# Blood — Known Limitations

**Last updated:** 2026-04-10
**Scope:** honest enumeration of gaps between the spec and the current compiler artifact. This document exists because the older `docs/planning/IMPLEMENTATION_STATUS.md` has drifted from reality since January; a comprehensive working audit lives at `.tmp/AUDIT_2026-04-07.md` (not committed — it's a live session document).

The goal of this file is to answer honestly: *if you write a Blood program today, what won't work?*

## At a glance

- **Self-hosting:** verified. 95K lines of Blood compile themselves through a three-generation byte-identical bootstrap.
- **Golden tests:** 543 pass, 0 XFAIL. Golden tests cover program-level correctness, not systematic spec conformance.
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

### GAP-7: Generation counter overflow panics instead of Tier 3 promotion

When a region slot is freed ~2 billion times, the generation counter wraps at `0x7FFFFFFE`. The runtime panics (`alloc.blood:391-395`) instead of promoting the allocation to reference-counted Tier 3 as the spec envisions.

**Impact:** long-lived processes will eventually crash. Not a concern for short-lived compilations, but relevant for the project's target domain (avionics, medical devices).

### GAP-8: Region virtual address space leak

Region destroy calls `madvise(MADV_DONTNEED)` but never `munmap` (`rt_region.blood:205-211`). The comment says "Keep virtual mapping for stale ref safety." Virtual address space is exhausted after enough region create/destroy cycles.

**Impact:** long-lived processes with many region lifecycles will exhaust virtual address space.

## Features that are specified but not implemented

### Concurrency primitives (0% implemented)

The spec at `docs/spec/CONCURRENCY.md` describes fibers, channels, mutexes, atomic operations, and an M:N scheduler. None are wired:

- `__builtin_fiber_*` symbols are declared in `build_runtime.py` but never generated
- Fiber spawn currently uses `pthread_create` directly (not cooperative M:N)
- No mutex, no channels, no atomics
- No safepoint mechanism for stop-the-world coordination
- 0 concurrency tests in the golden suite

Async/await syntax is not implemented at any level.

### Macros — single-file user macros work, cross-module macros don't

Built-in macros (`format!`, `vec!`, `println!`, `assert!`, `dbg!`, `matches!`) work. User-defined declarative macros work within a single file — multi-rule, recursive expansion, and capture patterns (`$val:expr`) are all functional via `macro_expand.blood`. Cross-module macros (defined in one file, invoked in another) are not supported. Procedural macros are not implemented.

### FFI bridge blocks — mostly working, link specs not implemented

Struct, enum, type-alias, union, callback, opaque-type, and C-function bridge items work. Link specifications (linker directives for choosing libraries) are not implemented (`hir_lower_builtin.blood:870`).

### Standard library — small but honest

The `stdlib/` directory contains 14 Blood-syntax files after cleanup. 56 Rust-syntax prototype files have been moved to `stdlib/_rust_prototype/` (they use `::` path separators, `Vec::new()`, `if let`, and other Rust patterns that don't compile in Blood).

Working modules: `core/drop`, `effects/cancel`, `math`, `prelude`, `string`. Additional modules (`crypto/blake3`, `traits/clone`, `traits/marker`, `result`) have minor issues (not Rust syntax but use features like `let` at module level or `?` operator that aren't implemented yet). The compiler does not depend on the stdlib — it has its own built-in implementations of HashMap, Vec, String, etc.

### Generic associated types projections (`T::Item` for type parameters)

The compiler handles `Self::Item` in trait/impl bodies. It does NOT handle `T::Item` where `T` is a type parameter. This blocks the Iterator trait from being used with generic for-in desugaring in user code.

### Associated type bounds (`type Item: Display`)

The parser does not parse bounds on associated types. The spec allows them; the implementation rejects them.

### Local declarations inside function bodies

The following declaration kinds are explicitly rejected inside function bodies (`hir_lower_expr.blood:1388-1417`): struct, enum, type alias, const, static, trait, effect, handler. The compiler doesn't need them for its own source, but user code that wants a helper struct inside a function has to move it to module scope.

### Runtime multiple dispatch

Compile-time dispatch works (specificity ranking, constraint-based, retroactive conformance). Runtime fingerprint-based dispatch (the "dynamic dispatch" story from `docs/spec/DISPATCH.md`) is not implemented. `.tmp/GAPS.md` describes this as "deferred indefinitely."

### Content-addressing — partial

BLAKE3 hashing, codebase storage, `use hash("prefix")` imports, and VFT registration all work at the mechanism level. What's NOT wired: the actual VFT dispatch lookup during method calls. Registrations are emitted but never consulted. No cross-compilation-unit hash-based linking. No distributed codebase registry.

### WCET / real-time / certification path

Nothing is started. `docs/spec/WCET_REALTIME.md` is aspirational. Certification annotations (`requires`, `ensures`, `invariant`, `decreases`), SMT-backed verification, and proof-carrying code are all future work.

## Build-system limitations

- **`build_selfhost.sh` caches are wiped on every `build first_gen`** from seed. Use `build first_gen --relink` when only the runtime changed to skip the ~2-minute rebuild (drops cycle to ~5 seconds).
- **Self-hosting is not in CI.** CI only runs the Rust bootstrap tests (`cargo test -p bloodc`). Golden tests, self-hosting, and byte-identical verification are local-only. Regressions surface only when someone manually runs `./build_selfhost.sh gate`.
- **LLVM version is hardcoded to 18** (llc-18, clang-18, opt-18, etc.) in build scripts. Breaks on systems with LLVM 17 or 19.
- **Error messages are basic.** E0201 now shows expected/found types. E0102 now suggests similar names. Other error codes still lack detailed context (e.g., trait bounds, exhaustiveness).

## What works that you can actually use

This is the honest complement: things that are genuinely working end-to-end and can be relied on.

- Parsing, type inference, type checking for the intersection of features exercised by the 95K-line selfhost compiler
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

## How to read this document

A gap here doesn't mean the spec is wrong. It means the compiler hasn't caught up yet. The project's design hierarchy is `Correctness > Safety > Predictability > Performance > Ergonomics`. When the spec and the implementation disagree, the spec is authoritative — but the runtime behavior is whatever the code does today.

If you hit something that looks like a bug and isn't listed here, it's probably a real bug — the document is maintained best-effort, and known-unknowns are more common than known-knowns for a research compiler at this stage.
