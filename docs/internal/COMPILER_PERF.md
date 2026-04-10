# Blood Compiler Performance Notes

This document captures perf patterns, instrumentation references, and traps
discovered while optimizing the Blood self-hosted compiler's build time. If
you're about to work on compiler perf, read this first.

Last updated: 2026-04-10 (wall time 354 s → 117 s over four sessions).

---

## 1. The "dead linear-scan fallback" trap

### What it is

Many hash-backed lookup functions in the compiler were written in this shape:

```blood
pub fn lookup(...) -> Option<T> {
    // Fast path: hash lookup
    match self.hash.get(key) {
        Option.Some(idx) => return Option.Some(self.vec[idx]),
        Option.None => {}
    }
    // Fallback to linear scan for collision handling
    for i in 0usize..self.vec.len() {
        if self.vec[i].key == key {
            return Option.Some(self.vec[i].value);
        }
    }
    Option.None
}
```

The pattern looks defensive — "if the hash misses, scan linearly just in case".
**It is almost always dead code.** `hashmap.HashMapU64U32` and `HashMapU64U64`
use **open addressing with linear probing**, so true hash collisions are
resolved inside `.get()` itself. A hash miss means the key is genuinely not
present. The linear scan fallback never finds anything the hash didn't — it
just walks the entire vec every time a lookup fails, returning None.

For hot functions called millions of times per build, this was the single
biggest source of lost wall time in the compiler. Removing one such fallback
(`SubstTable.lookup_ty_id`, 2026-04-09) dropped wall time from 5m12s to 2m48s
in a single commit — 144 seconds.

### How to detect it

Grep for the pattern:

```bash
grep -rn "Fallback to linear\|fall back to linear\|should not happen.*linear" \
    src/selfhost/*.blood
```

Any result in a function that's called from a hot path should be audited.

### How to verify a fallback is safe to remove

Two conditions must both hold:

1. **The hash is authoritative.** Every path that pushes to the backing vec
   must also update the hash. Grep for the vec's `.push(` sites and confirm
   each has a corresponding `hash.insert()`. If there are alternate insertion
   paths (e.g., builtin enum registration that bypasses the main `register_*`
   function), either populate the hash from them or leave the fallback.

2. **The call site doesn't depend on linear-scan first-match semantics.** If
   the same key is pushed twice with different values, the hash stores the
   LAST index and linear scan returns the FIRST match. For most lookup
   functions this doesn't happen, but verify before removing.

### Known exceptions (do NOT remove)

- **`mir_lower_ctx.lookup_method_def`** and **`mir_lower_ctx.lookup_field_idx`**
  — the linear scan is needed because of the closure rekeying trap (see §2).
- **`type_intern.intern`**, **`TypeInterner.find`**, **`intern_ty_list`**,
  **`StringInterner.intern`**, **`StringInterner.find`** — the linear scans
  handle *true hash collisions* (the hash key here is the FNV hash of the
  content, not the map bucket, so two distinct contents can share a key).

When in doubt: remove, run `./build_selfhost.sh test golden -q`, and if tests
pass run `./build_selfhost.sh gate --quick` to confirm byte-identical output.
If either fails, revert and document the dependency.

---

## 2. The closure rekeying trap

`mir_lower_ctx.blood` has a subtle issue that makes `lookup_method_def`,
`lookup_field_idx`, and (by extension) any future hash-backed lookup in this
file unsafe to remove their linear-scan fallbacks.

### The rekeying

When `mir_lower_expr.blood` creates a closure context (around line 2430), it
rewrites every parent method/field/coercion resolution with the closure's
`def_id`:

```blood
let mut closure_methods: Vec<common.MethodResEntry> = Vec.new();
while cmi < ctx.method_resolutions.len() {
    let me = &ctx.method_resolutions[cmi];
    closure_methods.push(common.MethodResEntry.new(
        closure_def_id.index,   // REKEYED to closure's def_id
        me.span_start,
        me.def_id_index,
    ));
    cmi += 1;
}
```

### Why it's a hash hazard

The hash key is `combine_u32_usize(body_def_id, span_start)`. After rekeying,
every inherited entry has `body_def_id == closure_def_id`, and two parent
entries whose `span_start` happens to collide (possible across different
parent files since each file's spans start at byte 0) now hash to the same
bucket. The hash stores last-write-wins; the linear scan finds first-match.

### The reproducer

Removing the fallback in `lookup_method_def` breaks
`tests/golden/t01_generic_multi_param.blood` — output mismatch, NOT a type
error, because the wrong method gets resolved at runtime.

### The real fix (deferred)

Filter parent resolutions to only those whose `span_start` falls inside the
closure body's span range, before rekeying. That narrows the inherited set
and eliminates the collision window. This is a closure-lowering refactor,
not a session-sized change.

### Until then

Leave the linear-scan fallbacks in `lookup_method_def` and `lookup_field_idx`
alone. If you add a new hash-backed lookup in `mir_lower_ctx.blood`, either
keep a fallback or don't share the body_def_id + span_start keying scheme.

---

## 3. Instrumentation timers reference

Every `--timings` build (the default) prints a series of `[...]` lines in
`src/selfhost/.logs/build_*.log`. These are permanent regression beacons;
they add a few seconds of overhead per build but catch silent creep early.

### Top-level phases (source: `main.blood` after `codegen_pass2`)

```
Parse                Nms
HIR lowering         Nms
Type checking        Nms
Codegen              Nms
Compiler total       Nms
llc-18 (per-module)  Nms
clang-18 (link)      Nms
```

### HIR lowering breakdown (source: `hir_lower.blood`)

```
[hir phases] p0=N p1=N p2(decls)=N p3a(ext_decls)=N p3pre(prelude+imports)=N \
             p3b(main_decls)=N p4(main_bodies)=N p4b(ext_bodies)=N
[lem] calls=N read_ms=N parse_ms=N register_ms=N
```

- `p0` — builtin effect + hash import preload
- `p1` — `register_type_names` (recursively loads and parses external modules)
- `p2` — `register_declarations`
- `p3a` — external module declaration lowering (from cached parse)
- `p3pre` — stdlib prelude injection + `resolve_imports`
- `p3b` — `hir_lower_item.lower_declarations` (main file)
- `p4` — `lower_fn_bodies` (main)
- `p4b` — external module function bodies
- `[lem]` — aggregate sub-phase times inside `load_external_module`; note
  `register_ms` overlaps `parse_ms` because it recursively re-enters
  `load_external_module` for nested `mod foo;` declarations.

### Type checking breakdown (source: `typeck_driver.blood`)

```
[typeck phase 2]  bodies=N check_body_total_ms=N max_ms=N max_body_id=N slow>100ms=N very_slow>1000ms=N
[typeck phase 2b] bodies=N check_body_total_ms=N max_ms=N max_body_id=N slow>100ms=N very_slow>1000ms=N
[typeck phases]   p1(setup+builtins)=N p1de(dispatch+stability)=N body_cache=N p2(main_bodies)=N p2b(ext_bodies)=N p3(pending)=N p4(impl_table)=N
[check_body]      setup_ms=N infer_ms=N resolve_ms=N linearity_ms=N
```

- Phase 2 = main-file bodies (errors reported)
- Phase 2b = external bodies (errors mostly discarded; runs purely to
  populate method/field resolutions for codegen)
- `[check_body]` decomposes per-body cost: `setup_ms` is local registration,
  `infer_ms` is `infer_expr` + `unify` + effect subtyping, `resolve_ms` is
  numeric inference defaulting + unresolved-infer check, `linearity_ms` is
  `check_linearity`.

Historically `infer_ms` was 96%+ of `check_body` time. If that ratio changes
significantly, something new is slow in the resolve or linearity phases.

### Codegen pass2 breakdown (source: `codegen.blood` + `main.blood`)

```
[codegen pass2] functions=N cache_ms=N mir_lower_ms=N bc01_ms=N init_lin_ms=N codegen_ms=N
[mir_lower]     setup_ms=N expr_ms=N finish_ms=N resolve_locals_ms=N
[codegen fn]    setup_ms=N escape_ms=N allocas_ms=N blocks_ms=N footer_ms=N
[mir_init]      calls=N iter_total=N iter_max=N hit_cap=N setup_ms=N fixpoint_ms=N error_pass_ms=N
[stmt]          total_ms=N assign_ms=N calls=N assign_calls=N
[stmt assign]   place_ms=N type_ms=N rvalue_ms=N
[rvalue]        use=Nms/N ref=Nms/N binop=Nms/N agg=Nms/N cast=Nms/N
```

- `[codegen pass2]` — top-level decomposition of codegen's per-function pass
- `[mir_lower]` — sub-phases of `mir_lower.lower_body`
- `[codegen fn]` — sub-phases of `codegen.generate_function_with_ctx`
- `[mir_init]` — `analyze_init` convergence stats (`iter_max` > 2 is a red
  flag; means the dataflow is struggling) and sub-phase costs
- `[stmt]` — `emit_statement` call counts and Assign-arm time (100% of
  selfhost MIR statements are Assign)
- `[stmt assign]` — Assign sub-call breakdown (LHS place, get_local_type,
  RHS rvalue)
- `[rvalue]` — per-Rvalue-kind `emit_rvalue` time and call count; `Use` is
  typically the hottest by total time, `Ref` by per-call cost

### Adding your own timer

The pattern is consistent across the compiler. Example:

```blood
// At file level:
static mut FOO_T_BAR_MS: u64 = 0;
pub fn foo_t_bar_ms() -> u64 { @unsafe { FOO_T_BAR_MS } }

// At the call site:
let t = blood_clock_millis();
// ... work ...
@unsafe { FOO_T_BAR_MS += blood_clock_millis() - t; }

// In main.blood's codegen_pass2 end-print block (or the appropriate printer):
eprint_str("  [foo] bar_ms=");
eprint_str(int_to_string(foo_module.foo_t_bar_ms() as i32));
eprint_str("\n");
```

---

## 4. The build-time regression alarm

### Wall time alarm

`check_build_time_regression()` gates on total wall time. Current thresholds
(2026-04-10, matching ~117 s steady state with parallel codegen + parallel llc):

```
baseline         = 130 s
warn_threshold   = 182 s  (baseline × 1.4)
fail_threshold   = 260 s  (baseline × 2.0)
```

### Per-phase sub-alarms

`check_sub_phase_regression()` parses the compiler's sub-phase timers from
the build log and warns if any individual phase exceeds its baseline × 1.5.
This catches regressions masked in the total (e.g., codegen regresses 10 s
but typeck improves 10 s → net zero, but codegen alarm fires).

```
Phase            Baseline    Threshold (1.5×)
Parse               1 s          1.5 s
HIR lowering       22 s         33 s
Type checking      22 s         33 s
Codegen            70 s        105 s
llc-18              3 s          4.5 s
```

Set `BLOOD_NO_PERF_ALARM=1` to silence both alarms. **Update baselines in
lockstep with perf wins** — an alarm set above the current steady state
can't catch regressions at the current level.

### Metrics history

Each build writes JSON to `.logs/metrics.jsonl`. View recent trends with:

```bash
./src/selfhost/build_selfhost.sh metrics
```

---

## 5. The rules for a perf change to land

1. **Measure before and after.** Noise is real; a single measurement is not
   a signal. Run `build second_gen` (not `first_gen`) after your edit so your
   instrumentation actually runs.
2. **Golden suite must stay at 536/536.** No exceptions.
3. **Byte-identical gate.** For any non-instrumentation change, run
   `./build_selfhost.sh gate --quick` and verify second_gen == third_gen.
4. **Lower the alarm baseline** in the same commit as a perf win. Orphan
   wins invite future regressions.
5. **Don't add instrumentation-only commits** unless you label them clearly
   and they enable a follow-up fix. `[perf instrumentation]` is fine; a
   rename of the tag is not.

---

## 6. Known remaining hot spots (as of 2026-04-09)

After the 2026-04-08..09 sessions, the big single-threaded wins have been
extracted. Remaining targets (with the measurements that found them):

| Bucket | Time | Notes |
|---|---|---|
| HIR Phase 1 (`register_type_names`) | ~13.7 s | 13.3 s is `parser.parse_file`. Parser is 8k LOC; optimizing needs a dedicated session. |
| `check_body.infer_ms` | ~14 s | Already hit by `apply_substs` identity reuse, `unify_id` TyId pre-check, `count_method_matches` early exit. Further wins require walking `infer_expr` sub-arms. |
| Codegen `blocks_ms` | ~27 s | Fast paths in `emit_place_addr` and `has_any_escaped_locals` are in place. Remaining: `emit_rvalue.Use` (5.5 s) and `emit_rvalue.Ref` (3.5 s); see `[rvalue]` timers. |
| `codegen fn allocas_ms` | ~9 s | `is_always_stack_type_id` + `is_signed_id` are TyId-native. |
| llc-18 | ~18 s | External tool; only reducible via smaller IR. |

**Parallel codegen pass2** — partially implemented (2026-04-09 session):

Infrastructure in place but disabled (main.blood `use_parallel = false`):

- `blood_thread_spawn`/`blood_thread_join` — already working, golden test passes
- Type interner — already read-only during codegen (no `intern()` calls)
- `alloc bypass tracking` — runtime flag that makes alloc/free/realloc skip the
  gen tracking hash table (not thread-safe). Bypass checks added to all entry
  points: `dispatch_alloc_or_abort`, `blood_free`, `blood_free_simple`,
  `blood_realloc`, `blood_lazy_register_gen`, `blood_register_allocation`,
  `blood_unregister_allocation`, `blood_validate_generation`, etc.
- `codegen_thread_worker` — worker function that processes a chunk of functions
  (MIR lower + codegen), using packed args via alloc/ptr_write raw memory.
- `codegen_pass2` parallel dispatch — creates N worker ctxs, packs args,
  spawns threads, joins, merges (string tables, fn_ptr_wrappers, inline_stubs,
  fn_signatures).

**Remaining blockers:**

1. **Bypass realloc corruption** — `malloc_usable_size` in the bypass realloc
   path causes heap metadata corruption ("corrupted size vs. prev_size").
   Crashes around function 2400 out of 2598 with 1 or 4 workers. Without
   bypass (normal alloc + regions), all 2598 functions complete. Fix: either
   debug `malloc_usable_size` usage or replace with a size-tracking approach.

2. **Generated IR value error** — the parallel path produces `undefined value
   '%tmpN'` in some functions. Likely related to worker ctx state (value_counter
   or string constant numbering). Needs investigation.

Estimated payoff once enabled: codegen 38s → ~10s (4 workers), wall 164s → ~135s.
