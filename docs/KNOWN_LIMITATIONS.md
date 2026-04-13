# Blood — Known Limitations

**Last updated:** 2026-04-13
**Scope:** honest enumeration of gaps between the spec and the current compiler artifact. This document exists because the older `docs/planning/IMPLEMENTATION_STATUS.md` has drifted from reality since January; a comprehensive working audit lives at `.tmp/DEEP_AUDIT_2026-04-10.md` (not committed — it's a live session document).

The goal of this file is to answer honestly: *if you write a Blood program today, what won't work?*

## At a glance

- **Self-hosting:** verified. 103K lines of Blood compile themselves through a three-generation byte-identical bootstrap. See "Self-hosting feature coverage" below for which features are exercised.
- **Golden tests:** 569 pass, 0 fail. Golden tests cover program-level correctness, not systematic spec conformance. Traceability matrix at `.tmp/SPEC_TRACEABILITY.md`.
- **Spec coverage:** 7 of 16 spec files fully implemented and tested. 3 partially implemented (Concurrency, Diagnostics, Stdlib). 1 has no tests (WCET/Real-time). See `.tmp/SPEC_TRACEABILITY.md` for details.
- **Rust bootstrap:** builds and runs simple programs. Used as an escape hatch; not the primary development target. Diverged from selfhost on type unification in April before being corrected.
- **Formal proofs:** 264 Coq theorems/lemmas across 22 theory files, 227 proved (Qed), 28 admitted. Three-tier structure (core soundness → feature interaction → composition). The proofs cover a core calculus formalization, not the compiler artifact directly. See `proofs/PROOF_ROADMAP.md`.
- **CI:** GitHub Actions at `.github/workflows/ci.yml` covering both bootstrap and selfhost (build, golden tests, gate). Fuzz testing at `fuzz.yml`.
- **Ecosystem:** No package manager, formatter, or documentation generator. Stdlib has 81 .blood files across 26 directories.

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

- **Generic `HashMap<K, V>` works for primitive and String keys** (session 18,
  2026-04-13): `HashMap<K, V>` is backed by a type-erased C runtime
  (`runtime/blood-runtime/rt_hashmap.c`) using open addressing with linear
  probing and FNV-1a hashing. Supported key types: all integer types, bool,
  and String. Methods: `new`, `insert`, `get`, `contains_key`, `len`,
  `is_empty`, `remove`. `get` returns `Option<V>` (by value, not by reference).
  Golden tests: `t05_hashmap_generic` (i32 keys), `t05_hashmap_string_keys`
  (String keys), `t05_hashmap_clone` (clone for primitive-key maps).
  `clone` works via shallow copy (correct for primitive types; String key
  cloning copies pointers). **Not yet supported**: arbitrary struct keys
  (needs Hash trait), iterator API (`for (k, v) in map`), and deep clone
  for non-primitive key types. The selfhost
  compiler's own hashmaps remain the monomorphic `HashMapU64U32` /
  `HashMapU64U64` / `HashMapU32U32` variants — these are performance-critical
  and won't be converted.
- **Iterator for-in works for concrete types and generic type parameters**:
  `for x in iter { }` works for Array, Vec, Slice, Range, custom concrete
  iterator types, and generic iterator type parameters (`fn f<T: Iterator>(iter: &mut T)`).
  Golden tests: `t05_for_in_custom_iterator`, `t05_generic_for_in_iterator`.
  Generic for-in desugars to method dispatch calls (not direct field access),
  emitting a degenerate placeholder in the type-erased body and resolving through
  the monomorphization pass.
- **No file I/O abstraction**: the stdlib exposes only raw FFI (`LibcIO.open`,
  `LibcIO.read`, `LibcIO.write`, `LibcIO.close` in `runtime/blood-runtime/libc.blood`).
  There is no `File` struct, no `BufReader`/`BufWriter`, no `Path` type.
- **No concurrency primitives in Blood source**: mutexes, channels, atomics,
  condvars — none of these exist above the raw `pthread_create`/`pthread_join`
  bridge in `runtime/blood-runtime/libc.blood`. See the "Concurrency primitives"
  entry above for the fiber layer's status.

### Generic associated type projections (`T.Item` for type parameters)

`T.Item` resolution works for type parameters with single trait bounds (both inline
`fn f<T: Trait>` and where-clause `where T: Trait`). Supported since session 12.
Golden tests: `t05_assoc_type_projection`, `t05_assoc_type_projection_param`,
`t05_assoc_type_where_clause`.

**Remaining gaps**: `<T as Trait>.Item` qualified projections, `where T.Item: Eq`
projection bounds, disambiguation when multiple bounds declare the same associated
type name. Scope cleanup (clearing `current_type_param_bounds` on exit) implemented
in session 13.

**Generic trait method return types**: Calling trait methods on `&T` or `&mut T` in
generic function bodies now type-checks correctly, including methods returning
`Option<T>` and other compound types (fixed session 16, commit a5f3e81). Golden test:
`t05_generic_trait_method_return`. Generic iterators work via the manual while-loop
pattern (`fn sum_all<T: Iterator>(iter: &mut T) -> i32`). The remaining gap is for-in
syntax desugaring, which only handles concrete ADTs in MIR lowering.

### Associated type bounds (`type Item: Display`)

The parser does not parse bounds on associated types. The spec allows them; the implementation rejects them.

### Local declarations inside function bodies

The following declaration kinds are explicitly rejected inside function bodies (`hir_lower_expr.blood:1388-1417`): struct, enum, type alias, const, static, trait, effect, handler. The compiler doesn't need them for its own source, but user code that wants a helper struct inside a function has to move it to module scope.

### Runtime multiple dispatch

Compile-time dispatch works (specificity ranking, constraint-based, retroactive conformance). Runtime fingerprint-based dispatch (the "dynamic dispatch" story from `docs/spec/DISPATCH.md`) is not implemented. `.tmp/GAPS.md` describes this as "deferred indefinitely."

### Content-addressing — partial delivery, one end-to-end demo

BLAKE3 hashing, codebase storage, and `use hash("prefix")` imports work **end-to-end**: the proving-ground test at `tests/proving/p5_identity.blood` imports `factorial`, `fibonacci`, and `gcd` from `tests/proving/mathlib.blood` by content hash and runs them successfully. This flow is now available as a one-line verification via `./build_selfhost.sh test pillar2`, which:

1. Compiles `mathlib.blood` with `--store-codebase`, populating `~/.blood/codebases/default/` by content hash.
2. Runs `p5_identity.blood` (which has `use hash("a13d")`, `use hash("40f0")`, `use hash("2ceb")`), verifying that cross-module hash-based linking resolves the imports and that the imported functions execute correctly.
3. Compares stdout against the expected fixture.

Hot-swap via `blood_vft_swap` is also functional (golden test `t05_vft_hot_swap.blood` exercises register→lookup→swap→verify).

**What's NOT wired**:

- **VFT lookup during method dispatch**: `blood_vft_lookup` is registered as a builtin (`hir_lower_builtin.blood:454`) but codegen never emits it. All method dispatch resolves at compile time to direct `FnDef(def_id)` calls, vtable GEPs, or closure pointers. VFT registrations go into a table that nothing reads at runtime in normal operation. This is documented as "deferred indefinitely" in `DISPATCH.md §10.10` because content-hash keys aren't known at the dispatch site (the caller doesn't know the hash — that's what the dispatcher is for), so VFT-as-dispatch is architecturally the wrong shape. VFT-as-hot-swap is the right shape and is already working.
- **Distributed codebase registry**: the codebase is a local flat-file at `~/.blood/codebases/default/`. There is no network layer, no remote fetch, no mirror registry. Single-machine only.
- **Codebase garbage collection / deduplication across projects**: each `--store-codebase` adds to the default codebase; there's no eviction, no reference counting, no per-project isolation.

The `--store-codebase` flag currently has a rough edge: it runs as a side effect of content-hash emission during codegen pass 2, which happens before llc runs. For a library-only input (no `main` function), the compilation's downstream llc step fails with `use of undefined value '@blood_main'` — but by that point the codebase has already been populated, so the side effect has fired. The `test pillar2` target above works around this by not treating the non-zero exit as fatal and verifying population via the codebase names index. A cleaner fix would be to detect library mode (no main) and skip the main trampoline emission in codegen, or to refactor `--store-codebase` into a separate store-only subcommand that skips codegen entirely.

### WCET / real-time / certification path

Nothing is started. `docs/spec/WCET_REALTIME.md` is aspirational. Certification annotations (`requires`, `ensures`, `invariant`, `decreases`), SMT-backed verification, and proof-carrying code are all future work.

## Rust bootstrap compiler (legacy fallback)

The original Blood compiler lives at `src/bootstrap/bloodc/`, written in Rust
(~97K lines of actual compiler logic, excluding parser snapshot test data).
It is **not the primary development target.** The selfhost Blood compiler
at `src/selfhost/` is the canonical implementation and is what receives all
new features, bug fixes, and soundness work.

The Rust bootstrap is maintained as a **functional fallback only**: it
still builds, and `cargo test -p bloodc` runs `93/93` unify tests +
`27/27` codegen-regression tests. If the selfhost compiler ever breaks so
thoroughly that it can't rebuild itself from its own seed, the Rust
bootstrap can be used to get back to a known-good state.

**Known bugs in the bootstrap compiler that are fixed in the selfhost:**

| ID | Bug | Selfhost status | Bootstrap status |
|---|---|---|---|
| `BC-01` | Dangling references (no borrow checker) | FIXED — `E0503` + runtime gen validation | OPEN |
| `BC-02` | Uninitialized variables | FIXED — definite-init analysis enforced | OPEN |
| `BC-03` | Out-of-bounds indexing | FIXED — bounds checks for `Vec`/arrays/slices | OPEN |
| `BC-04` | Generational references | DONE — fat refs + gen capture/validation | Codegen disabled |
| `BC-05` | Region stale references | FIXED — `E0502` + runtime gen checks | OPEN |

Additional bootstrap-only issues are tracked in `.tmp/BUGS_OPEN.md` (they
are not enumerated here because fixing them would not be useful — the
bootstrap exists as an escape hatch, not as a product). The
selfhost-fixed items above are listed because they are illustrative of
what would be lost if a user tried to use the bootstrap for real work.

**Policy**: if the bootstrap breaks in a way that blocks using it as a
recovery path (e.g., it no longer compiles, or it can't compile a
minimal hello-world), that is the trigger to fix it. Anything less
serious is documented and left alone. The bootstrap's role is minimal
by design — every session adds confidence to the selfhost, and the
bootstrap's historical bug count is frozen at the point where selfhost
took over.

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
- For-in loops over Vec, `&Vec`, arrays, `&arrays`, slices, custom iterators, and generic iterator type parameters
- Module system with dot-separated paths, grouped imports, glob imports
- Regions with generational reference invalidation on destroy
- Linear and affine types with consumption checking
- Array / Vec / slice bounds checking (default on)
- Definite initialization analysis (default on)
- Compile-time dangling reference rejection via E0503
- Runtime stale reference detection on deref for all reference types including String/Vec data buffers

## Self-hosting feature coverage

The compiler compiles itself using: structs (379), enums (129), functions (1,523), generics (`Vec<T>`, `Option<T>`, `HashMap`, 2,619 uses), closures (237), match expressions (2,304), impl blocks (258), and `@unsafe` blocks (159).

The compiler uses **traits** in its own source: 2 trait definitions (`Clone`, `Display` in `common.blood`), 6 trait impls (`Clone for String`, `Display for Span/Symbol/SpannedSymbol/SpannedString`, `Display for CompilePhase`). It does **not** use: effects (0 effect/handler/perform/resume), linear/affine types (0 uses as a consumer), explicit `region { }` blocks (0 — uses FFI calls instead), content-addressing (`use hash(...)`), dyn Trait, or fibers.

These features beyond the self-hosting subset are validated through golden tests (565 total), proving ground programs (13 integration tests across all 5 pillars), and the Coq formalization — but not through self-compilation at 103K-line scale.

## Effect handler control flow

Effect handlers use `setjmp`/`longjmp` for non-resumptive control flow, not real delimited continuations. The continuation table infrastructure exists but callbacks are identity functions (`rt_continuation.blood:4-6`). `resume(value)` in a handler op body sets a flag and returns the value through the call stack — there is no stack capture, no suspended computation, no ability to resume later or elsewhere.

Multi-shot continuations are not supported. The continuation table marks entries as consumed and panics on second resume ("single-shot violation").

What works: tail-resumptive handlers (State, Reader, Writer — zero overhead), non-resumptive handlers (Cancel, Error, StaleReference — `longjmp` abort), single resume in non-tail position (immediate return). What doesn't: multi-shot, deferred resume, storing continuations, suspend/resume scheduling.

## How to read this document

A gap here doesn't mean the spec is wrong. It means the compiler hasn't caught up yet. The project's design hierarchy is `Correctness > Safety > Predictability > Performance > Ergonomics`. When the spec and the implementation disagree, the spec is authoritative — but the runtime behavior is whatever the code does today.

If you hit something that looks like a bug and isn't listed here, it's probably a real bug — the document is maintained best-effort, and known-unknowns are more common than known-knowns for a research compiler at this stage.
