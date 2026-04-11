# Blood — Known Limitations

**Last updated:** 2026-04-10
**Scope:** honest enumeration of gaps between the spec and the current compiler artifact. This document exists because the older `docs/planning/IMPLEMENTATION_STATUS.md` has drifted from reality since January; a comprehensive working audit lives at `.tmp/AUDIT_2026-04-07.md` (not committed — it's a live session document).

The goal of this file is to answer honestly: *if you write a Blood program today, what won't work?*

## At a glance

- **Self-hosting:** verified. 103K lines of Blood compile themselves through a three-generation byte-identical bootstrap. See "Self-hosting feature coverage" below for which features are exercised.
- **Golden tests:** 544 pass, 0 fail. Golden tests cover program-level correctness, not systematic spec conformance.
- **Spec coverage:** of 78 surveyed normative claims across `docs/spec/*.md`, 39 have verifiable code evidence (~50%). The other 39 are partial, missing, or too vague to verify.
- **Rust bootstrap:** builds and runs simple programs. Used as an escape hatch; not the primary development target. Diverged from selfhost on type unification in April before being corrected.
- **Formal proofs:** 264 Coq theorems/lemmas across 22 theory files, 0 admitted, 0 axioms. The proofs cover a simplified formal model of the language, not the compiler artifact. The gap between model and implementation is real and significant.

## Known soundness gaps (compile-time or runtime correctness)

### ~~GAP-1: `&str` stale detection disabled for `String`/`Vec` data buffers~~ FIXED

**Fixed in commit 6080f21 (2026-04-10).** `rt_blood_alloc_simple` now registers allocations in the generation hash table via `rt_blood_register_allocation_tagged(addr, size, 2)`. When a `String` or `Vec` grows and reallocates, the old buffer's generation increments, invalidating any `&str` or `&[T]` pointing at it. The latent `&str`-lifetime bug that previously blocked re-enablement (reported in AUDIT_2026-04-07.md) was fixed by intervening work — self-compilation completes without stale reference panics. Three-generation byte-identical bootstrap verified. Test: `t03_genref_stale_str_realloc.blood`.

### GAP-2: `Frozen<T>` deep traversal is shallow

`blood_freeze()` marks only the root allocation as frozen (gen set to `0x7FFFFFFE`). Inner heap pointers inside the structure are not recursively frozen. A "frozen" value can contain pointers to mutable heap data, breaking the immutability guarantee.

**Needed:** runtime type-layout metadata so that freeze can walk fields and follow inner pointers. Not currently emitted by codegen.

### GAP-3: Aggregate operand escape analysis — partially fixed (`--no-parallel`)

Aggregate operands should be marked as escaping so their allocations are promoted to the correct tier (proof assumption in Coq). This is now **enabled when `--no-parallel` is set**, which forces sequential codegen.

**Still broken in parallel mode:** Enabling aggregate escape with 4-worker parallel codegen causes `corrupted size vs. prev_size` glibc heap corruption during self-compilation. Root cause: latent memory corruption in parallel codegen exposed by increased heap allocation pressure. The gen hash table and tier classification are correct — the glibc heap itself is corrupted by a threading bug.

**Impact:** In default (parallel) mode, aggregate operands may be misclassified to a lower tier. Use `--no-parallel` for correct tier classification at the cost of ~30% slower codegen.

### ~~GAP-4: Closure codegen regression — nested closures inside other closures~~ FIXED

**Fixed in commit 2b6d72e (2026-04-08).** `mir_lower_expr.blood` now uses `finish_nested(parent)` instead of `finish()` for nested `MirLowerCtx`, which propagates discovered closures to the parent context instead of silently dropping them. Transitive propagation verified through 3 nesting levels. Tests: `t04_nested_closure.blood`, `t04_doubly_nested_closure.blood`.

### ~~GAP-5: Function-call arity not checked~~ FIXED

**Fixed in commit f6285a5 (2026-04-08).** The arity check at `typeck_expr.blood:1252` always worked for main-file bodies. The actual bug was in `typeck_driver.blood:790-793`: Phase 2b discarded *all* errors from external module bodies, including arity mismatches. Fix: selectively keep `ArityMismatch` errors from Phase 2b while discarding other cross-module false positives. Test: `t06_err_wrong_arity.blood`.

### ~~GAP-6: Effect snapshot validation is a stub~~ FALSE POSITIVE

Investigation found the snapshot mechanism is fully implemented. Codegen adds entries for all gen-tracked locals (region, persistent, unsized refs) at every Perform site (`codegen_term.blood:136-258`). The runtime validates each entry against the generation hash table after perform returns (`rt_effect.blood:197-218`). Golden tests: `t03_genref_snapshot_effect.blood`, `t10_nested_effect_snapshot.blood`. The stale comment in `rt_effect.blood:33` claiming "selfhost creates snapshots but never adds entries" has been corrected.

### ~~GAP-7: Generation counter overflow panics instead of Tier 3 promotion~~ FIXED

**Fixed in commit 4dfe8d0 (2026-04-10).** Generation overflow (g >= 0x7FFFFFFE) now sets gen to -1 (permanently valid sentinel) instead of panicking. `blood_validate_generation` treats gen=-1 as always valid, equivalent to Tier 3 promotion. Applies to both heap allocations (alloc.blood) and region allocations (rt_region.blood).

### ~~GAP-8: Region virtual address space leak~~ FIXED

**Fixed in commit ebdac42 (2026-04-10).** Region destroy now calls `munmap` instead of `madvise(MADV_DONTNEED)`, releasing virtual address space back to the kernel. The validation array retains the region's (base, end, gen) entry so stale references are still detected. Region gen overflow also uses the -1 sentinel instead of panicking.

## Design decisions (intentional behavior, not gaps)

### gen=0 for stack-tier references

Stack-allocated locals (`alloca` in LLVM entry block) receive generation 0. The deref validation in `codegen_place.blood:870-874` skips generation checking when gen=0. This is intentional: stack allocas live for the entire function duration, so intra-function references to them are never dangling. Region blocks don't affect stack locals, and Blood's copy-by-default semantics means closures capture values, not references. `GlobalEscape` → Persistent correctly heap-allocates locals whose references escape the function.

### Untracked addresses return "valid"

`blood_validate_generation` at `runtime/blood-runtime/alloc.blood:424` returns 1 (valid) for addresses not in any registry (neither the per-allocation hash table nor the per-region gen array). This is intentional and covers three distinct categories of untracked pointers, each with its own soundness rationale:

1. **Stack allocations**: locals live on the current stack frame and cannot dangle within their owning function. Blood's MVS model captures values, not references; BC-01 (`E0503`) rejects any path where a `&local` escapes the stack-frame lifetime at compile time. Stack pointers never appear in the heap registry because we never `malloc()` for them. Returning 1 is correct by construction.

2. **Field refs within an allocation**: `&struct.field` produces a pointer offset from the base allocation. The gen table keys on base addresses, not field addresses. The containing allocation's liveness is verified separately when the base pointer is derived; returning 1 for the field ref is correct.

3. **FFI-owned pointers** (C's `malloc`, `mmap`, `open` → fd, `dlopen` → handle, etc.): memory the runtime doesn't own has no tracked generation. Treating unregistered addresses as invalid would break every FFI bridge that returns a pointer. This is the intentional boundary between Blood's tracked heap safety and `@unsafe`/bridge code.

**Trade-off**: dangling pointers to unregistered memory silently pass validation. Blood's generational-reference safety guarantees apply only to Blood-allocated memory. The spec (`docs/spec`, Pillar 1) frames this as "gen tracking covers Blood-managed allocations, not arbitrary address arithmetic."

**Not documented in-code**: the `rt_blood_validate_generation` function currently has only a short single-line comment in `alloc.blood` because the runtime build is sensitive to minor source changes in ways we don't fully understand — an attempt in session 7 to add a longer documentation block to this function produced a non-equivalent runtime archive that broke first_gen relink. Root cause unclear; interacts with the same runtime-archive-sensitivity class that blocked SOUND-04 mutex landing in session 6. See `.tmp/BUGS_OPEN.md` SOUND-04.

## Features that are specified but not implemented

### Concurrency primitives — partially implemented

The spec at `docs/spec/CONCURRENCY.md` describes fibers, channels, mutexes, atomic operations, and an M:N scheduler. Basic thread primitives work; higher-level abstractions are missing:

- `__builtin_fiber_spawn/join/yield/sleep/cancel` work via raw `pthread_create`/`pthread_join`
- 4 golden tests verify basic fiber operations (`t10_fiber_builtins`, `t10_fiber_spawn_join`, `t10_fiber_effect`, `t10_fiber_handle_wrap`)
- No cooperative M:N scheduler (fibers are OS threads, not green threads)
- No mutex, no channels, no atomics
- No safepoint mechanism for stop-the-world coordination

Async/await syntax is not implemented at any level.

### Macros — single-file user macros work, cross-module macros don't

Built-in macros (`format!`, `vec!`, `println!`, `assert!`, `dbg!`, `matches!`) work. User-defined declarative macros work within a single file — multi-rule, recursive expansion, and capture patterns (`$val:expr`) are all functional via `macro_expand.blood`. Cross-module macros (defined in one file, invoked in another) are not supported. Procedural macros are not implemented.

### FFI bridge blocks — mostly working, link specs not implemented

Struct, enum, type-alias, union, callback, opaque-type, and C-function bridge items work. Link specifications (linker directives for choosing libraries) are not implemented (`hir_lower_builtin.blood:870`).

### Standard library — small but honest

The `stdlib/` directory contains 23 Blood-syntax files outside `_rust_prototype/`
(counted via `find stdlib -name '*.blood' -not -path '*/_rust_prototype/*'
-not -path '*/tests/*'` on 2026-04-11). 56 Rust-syntax prototype files remain
in `stdlib/_rust_prototype/` (they use `::` path separators, `Vec::new()`,
`if let`, and other Rust patterns that don't compile in Blood); the prototypes
are retained as design notes, not as compilation targets.

Working modules: `prelude`, `string`, `math`, `convert`, `args`, `io`,
`testing`, `result` (documentation stub — `Result<T,E>` is a compiler built-in),
`collections/hashmap`, `collections/hashset`, `effects/cancel`, `mem/arena`,
`crypto/blake3`, `algorithms/sort`, `core/drop`, `core/fmt`, `traits/marker`,
`traits/clone`, plus `mod.blood` aggregators. The non-prototype files do not
use `use` statements — each module is self-contained. The compiler does not
depend on the stdlib: it has its own built-in implementations of `HashMap`,
`Vec`, `String`, etc. distributed across `src/selfhost/common.blood`,
`type_intern.blood`, and the runtime.

Known structural gaps (not bugs, but feature omissions):

- **No generic `HashMap<K, V>`**: the stdlib `hashmap.blood` is monomorphic
  (`HashMapU64U64` etc.); a generic HashMap requires `T::Item` projection on
  type parameters, which is not implemented.
- **No usable Iterator trait for user code**: the same `T::Item` projection
  gap blocks a general `Iterator<Item = T>` trait from being useful in
  user-written code. The compiler uses concrete iterators and `for i in 0..n`
  ranges instead.
- **No file I/O abstraction**: the stdlib exposes only raw FFI (`LibcIO.open`,
  `LibcIO.read`, `LibcIO.write`, `LibcIO.close` in `runtime/blood-runtime/libc.blood`).
  There is no `File` struct, no `BufReader`/`BufWriter`, no `Path` type.
- **No concurrency primitives in Blood source**: mutexes, channels, atomics,
  condvars — none of these exist above the raw `pthread_create`/`pthread_join`
  bridge in `runtime/blood-runtime/libc.blood`. See the "Concurrency primitives"
  entry above for the fiber layer's status.

### Generic associated types projections (`T::Item` for type parameters)

The compiler handles `Self::Item` in trait/impl bodies. It does NOT handle `T::Item` where `T` is a type parameter. This blocks the Iterator trait from being used with generic for-in desugaring in user code.

### Associated type bounds (`type Item: Display`)

The parser does not parse bounds on associated types. The spec allows them; the implementation rejects them.

### Local declarations inside function bodies

The following declaration kinds are explicitly rejected inside function bodies (`hir_lower_expr.blood:1388-1417`): struct, enum, type alias, const, static, trait, effect, handler. The compiler doesn't need them for its own source, but user code that wants a helper struct inside a function has to move it to module scope.

### Runtime multiple dispatch

Compile-time dispatch works (specificity ranking, constraint-based, retroactive conformance). Runtime fingerprint-based dispatch (the "dynamic dispatch" story from `docs/spec/DISPATCH.md`) is not implemented. `.tmp/GAPS.md` describes this as "deferred indefinitely."

### Content-addressing — plumbing without payoff

BLAKE3 hashing, codebase storage, `use hash("prefix")` imports, and VFT registration all work at the mechanism level. VFT registrations are emitted at program startup (`blood_vft_register` called from `__blood_vft_init` global constructor for each trait impl method) — but `blood_vft_lookup` is never called from generated code. All dispatch goes through direct calls, vtable GEP, or closure pointers. The registrations go into a table that nothing reads at runtime in normal operation.

Not wired: VFT dispatch during method calls, cross-compilation-unit hash-based linking, hot-swapping via `blood_vft_swap`, distributed codebase registry.

### WCET / real-time / certification path

Nothing is started. `docs/spec/WCET_REALTIME.md` is aspirational. Certification annotations (`requires`, `ensures`, `invariant`, `decreases`), SMT-backed verification, and proof-carrying code are all future work.

## Build-system limitations

- **`build_selfhost.sh` caches are wiped on every `build first_gen`** from seed. Use `build first_gen --relink` when only the runtime changed to skip the ~2-minute rebuild (drops cycle to ~5 seconds).
- **Self-hosting is not in CI.** CI only runs the Rust bootstrap tests (`cargo test -p bloodc`). Golden tests, self-hosting, and byte-identical verification are local-only. Regressions surface only when someone manually runs `./build_selfhost.sh gate`.
- **LLVM version defaults to 18 but is overridable.** Build scripts and the compiler resolve the `LLC`, `CLANG`, `OPT`, `FILECHECK`, `LLVM_AS`, `LLVM_EXTRACT`, and `LLVM_LINK` environment variables at invocation time, falling back to the `*-18` names when unset. Verified working on LLVM 17 and 18 as of 2026-04-11 (see `src/selfhost/_llvm_tools.sh` + `resolve_llc_tool`/`resolve_clang_tool` in `main.blood`). LLVM 19 support is implemented but not yet verified on a machine with LLVM 19 installed.
- **Error messages are basic.** E0201 now shows expected/found types. E0102 now suggests similar names. Other error codes still lack detailed context (e.g., trait bounds, exhaustiveness).

## What works that you can actually use

This is the honest complement: things that are genuinely working end-to-end and can be relied on.

- Parsing, type inference, type checking for the intersection of features exercised by the 103K-line selfhost compiler (see feature coverage below)
- Generics, monomorphization, method calls, generic impls (per-call-site fresh inference variables; see recent commit ca1f2aa)
- Deep and shallow effect handlers with perform/resume/abort semantics
- Pattern matching (exhaustive, or-patterns, nested destructuring)
- Closures (move semantics, capture by value, nested closures)
- For-in loops over Vec, `&Vec`, arrays, `&arrays`, slices
- Module system with dot-separated paths, grouped imports, glob imports
- Regions with generational reference invalidation on destroy
- Linear and affine types with consumption checking
- Array / Vec / slice bounds checking (default on)
- Definite initialization analysis (default on)
- Compile-time dangling reference rejection via E0503
- Runtime stale reference detection on deref for all reference types including String/Vec data buffers

## Self-hosting feature coverage

The compiler compiles itself using: structs (379), enums (129), functions (1,523), generics (`Vec<T>`, `Option<T>`, `HashMap`, 2,619 uses), closures (237), match expressions (2,304), impl blocks (258), and `@unsafe` blocks (159).

The compiler does **not** use in its own source: traits (0 trait definitions, 0 trait impls), effects (0 effect/handler/perform/resume), linear/affine types (0 uses as a consumer), explicit `region { }` blocks (0 — uses FFI calls instead), content-addressing (`use hash(...)`), dyn Trait, or fibers.

These unused features are validated through golden tests (541 total), proving ground programs (13 integration tests across all 5 pillars), and the Coq formalization — but not through self-compilation at 103K-line scale.

## Effect handler control flow

Effect handlers use `setjmp`/`longjmp` for non-resumptive control flow, not real delimited continuations. The continuation table infrastructure exists but callbacks are identity functions (`rt_continuation.blood:4-6`). `resume(value)` in a handler op body sets a flag and returns the value through the call stack — there is no stack capture, no suspended computation, no ability to resume later or elsewhere.

Multi-shot continuations are not supported. The continuation table marks entries as consumed and panics on second resume ("single-shot violation").

What works: tail-resumptive handlers (State, Reader, Writer — zero overhead), non-resumptive handlers (Cancel, Error, StaleReference — `longjmp` abort), single resume in non-tail position (immediate return). What doesn't: multi-shot, deferred resume, storing continuations, suspend/resume scheduling.

## How to read this document

A gap here doesn't mean the spec is wrong. It means the compiler hasn't caught up yet. The project's design hierarchy is `Correctness > Safety > Predictability > Performance > Ergonomics`. When the spec and the implementation disagree, the spec is authoritative — but the runtime behavior is whatever the code does today.

If you hit something that looks like a bug and isn't listed here, it's probably a real bug — the document is maintained best-effort, and known-unknowns are more common than known-knowns for a research compiler at this stage.
