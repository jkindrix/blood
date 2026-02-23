# Blood-Rust Bootstrap Compiler — Self-Hosting Feedback

**Date:** 2026-02-05
**blood-rust version:** 0.2.0 (commit `08067f6`)
**Self-hosted compiler:** 113 files, ~61,000 lines of Blood

---

## Context

The self-hosted Blood compiler (`src/selfhost/`) has reached the point where it can compile itself. The pipeline is:

```
blood-rust compiles main.blood → first_gen binary
first_gen compiles main.blood  → main.ll (LLVM IR)
main.ll → llvm-as → llc → link → second_gen binary
```

Steps 1-3 all succeed. The first_gen binary produces valid LLVM IR that assembles and links. However, the resulting second_gen binary **segfaults** due to a codegen bug in blood-rust (BUG-008) that causes the first_gen binary to emit incorrect LLVM IR for `&str` parameter types.

**We are one bug away from full self-hosting.**

This document catalogs all issues encountered during self-hosting development, ordered by severity, with reproduction steps and evidence where available.

---

## P0 — Blocks Self-Hosting

### BUG-008: If-Expression with Function Call Condition Returns Wrong Branch

**Severity:** Critical — the single remaining blocker for self-hosting

**Summary:** When an if-expression's condition is a function call, and the branches return `String` values (via `common::make_string`), blood-rust's codegen eliminates the conditional branch entirely. The generated code unconditionally executes the else branch.

**Affected self-hosted code:**

```blood
// codegen_types.blood:191-198
fn ref_to_llvm(inner: &hir_ty::Type) -> String {
    if is_unsized_type(inner) {
        common::make_string("{ ptr, i64 }")   // ← never reached
    } else {
        common::make_string("ptr")             // ← always taken
    }
}

// codegen_types.blood:202-209
fn ptr_to_llvm(inner: &hir_ty::Type) -> String {
    if is_unsized_type(inner) {
        common::make_string("{ ptr, i64 }")   // ← never reached
    } else {
        common::make_string("ptr")             // ← always taken
    }
}
```

Where `is_unsized_type` is:

```blood
// codegen_types.blood:172-187
fn is_unsized_type(ty: &hir_ty::Type) -> bool {
    match &ty.kind {
        &hir_ty::TypeKind::Primitive(ref prim) => {
            match prim {
                &hir_ty::PrimitiveTy::Str => true,
                _ => false,
            }
        }
        &hir_ty::TypeKind::Slice { element: _ } => true,
        &hir_ty::TypeKind::DynTrait { trait_id: _, auto_traits: _ } => true,
        _ => false,
    }
}
```

**Symptoms:**

1. The first_gen binary (compiled by blood-rust) always returns `"ptr"` from `ref_to_llvm`, even for `&str` types that should get `"{ ptr, i64 }"`.
2. This causes all `&str` parameters in foreign function declarations to be declared as `ptr` instead of `{ ptr, i64 }`.
3. Runtime functions like `print`, `panic`, `file_append_string`, etc. get wrong signatures.
4. The second_gen binary segfaults immediately due to calling convention mismatch.

**Evidence from the first_gen's LLVM IR output (`main_self.ll`):**

The first_gen binary emits these declarations (wrong):
```llvm
declare void @print_str(ptr)           ; should be: @print_str({ ptr, i64 })
declare void @panic({ ptr, i64 })      ; mixed: some get fat ptr, some don't
```

While blood-rust directly emits (correct):
```llvm
declare void @print_str({ ptr, i64 })
```

**Key characteristics of the triggering pattern:**
- If-expression (not if-statement)
- Condition is a function call returning `bool`
- Function takes a reference parameter (`&Type`)
- Branches return `String` (heap-allocated via `common::make_string`)
- Simple cases (returning `i32`, `bool`, `&str` literals) work correctly

**Note:** This bug does not reproduce in simple standalone programs. It manifests specifically when blood-rust compiles the self-hosted compiler's complex functions. This suggests the bug may be related to optimization passes interacting with specific code patterns, or to how the function call result is consumed in the presence of String return values.

**Suggested investigation areas:**
- LLVM IR generated for `ref_to_llvm` — check if the `br` instruction uses the function call result
- Check if the safe optimization pipeline (`bf4d19d`) inadvertently prunes the branch
- Check if String return value handling (alloca + store) interferes with the branch condition

---

### Memory Pressure During Self-Compilation

**Severity:** High — causes OOM on large compilations

**Summary:** When the first_gen binary self-compiles (`./first_gen build main.blood`), memory consumption grows without bound because the region allocator retains all allocations until region destruction. Blood-rust's Rust runtime uses drop semantics and stays at ~24 MB regardless of code size.

**Measurements (from 200-let-binding test):**

| Phase | Region Used | Per Statement |
|-------|-------------|---------------|
| After Parse | 16,252 KB | ~81 KB |
| After HIR Lower | 27,806 KB | ~58 KB |
| After Type Check | 47,469 KB | ~98 KB |
| **Total** | **47,469 KB** | **~237 KB** |

**Live measurement (self-compiling main.blood, 2026-02-05):**
```
PID       VSZ (KB)     RSS (KB)    %MEM   %CPU
4082503   53,969,840   35,744,172  54.4%  99.9%
```

That is **35 GB resident memory** (54% of a 64 GB machine) while self-compiling.
The self-compilation does complete successfully (producing 395,430 lines / 14 MB of LLVM IR), but a machine with less than ~40 GB of RAM would OOM. blood-rust compiles the same source in ~24 MB.

**Contributing factors:**
- AST `Statement` enum is sized to its largest variant (~500+ bytes)
- Vec growth leaks old backing buffers (region dealloc is a no-op)
- `copy_type()` creates many intermediate allocations that are never freed
- Token trivia Vecs are allocated per token

**What would help:**
1. **Phase-based region reset** — ability to destroy and recreate regions between compilation phases (parse → HIR → typeck → codegen) so that intermediate data structures can be reclaimed
2. **Vec realloc that frees old buffers** — when a Vec grows, the old buffer should be returned to a free list rather than leaked
3. **Region-aware `blood_free_simple`** — the runtime already has slab allocator infrastructure; making `blood_free_simple` actually reuse freed memory within a region would help significantly

**Note:** The runtime's Generation-Aware Slab Allocator exists and supports region-aware allocation, but compiled Blood programs need explicit region creation/activation at startup to benefit. Documentation or a standard library helper for this would be valuable.

---

## P1 — Major Productivity Impact

### `blood test` Linking Is Broken

**Severity:** Medium-High — prevents use of the test framework entirely

**Summary:** `blood test` fails with undefined reference errors when linking test binaries against `libblood_runtime.a`. The `#[test]` attribute, test runner generation, and assertion builtins all exist and work at the compiler level, but the final link step fails.

**Error:**
```
undefined reference to `blood_assert_eq_int`
undefined reference to `blood_assert`
undefined reference to `blood_init_args`
```

**Root cause:** The static library (`libblood_runtime.a`) is built with LTO in release mode, which causes Rust-mangled symbol names to replace the `#[no_mangle]` FFI exports. The dynamic library (`.so`) has the correct symbols.

**Evidence:**
```bash
# Symbols present in .so:
nm -D libblood_runtime.so | grep blood_assert  # found

# Symbols missing from .a:
nm libblood_runtime.a | grep blood_assert       # not found (mangled)
```

**Workaround:** We use `blood run` with `// EXPECT:` output markers instead of `#[test]` functions.

**Fix suggestion:** Either:
1. Disable LTO for the runtime crate's release build, or
2. Add `-C lto=off` for the runtime specifically, or
3. Use `--whole-archive` when linking test binaries against the static library, or
4. Link test binaries against the `.so` instead of the `.a`

---

### Multi-Error Reporting

**Severity:** Medium — significantly impacts iteration speed

**Summary:** blood-rust stops compilation at the first error encountered. When working on a 113-file, 61K-line compiler, this means each compilation cycle (19s with `--release`, 182s without) reveals at most one error.

**Impact:** A change that introduces 5 errors requires 5 full recompilation cycles to discover and fix all of them. With `--release`, that's ~95 seconds of waiting. Without, ~15 minutes.

**Request:** Collect errors during each phase and continue to the next phase where possible. Even reporting multiple errors within a single phase (e.g., all type errors in one pass) would be a significant improvement.

**Precedent:** The self-hosted compiler's reporter module (`reporter.blood`) collects diagnostics into a `Vec<Diagnostic>` and reports them all at the end. blood-rust already uses ariadne for pretty-printing — it would just need error collection instead of early termination.

---

### `--emit` Fails on Large Programs

**Severity:** Medium — prevents inspecting intermediate representations

**Summary:** Both `--emit llvm-ir` and `--emit llvm-ir-unopt` fail when applied to the self-hosted compiler, even though `blood build` (producing a binary) succeeds on the same source.

**Errors:**
```bash
blood build --emit llvm-ir-unopt main.blood
# → "Unknown const DefId(1961)"
# → "Failed to generate unoptimized LLVM IR."

blood build --emit llvm-ir main.blood
# → "Static DefId(1375) not found in globals"
# → "Failed to generate optimized LLVM IR."

blood build main.blood
# → succeeds, produces working binary
```

**Impact:** Cannot inspect the LLVM IR that blood-rust generates for the self-hosted compiler. This makes debugging codegen issues like BUG-008 much harder — we can't see exactly what IR blood-rust produces for `ref_to_llvm` and related functions without `--emit`.

**Possible cause:** The `--emit` code path handles static globals and const evaluation differently from the binary-producing `build` path.

---

## P2 — Quality of Life

### `--quiet` Still Emits to stdout

**Severity:** Low — requires workaround in test automation

**Summary:** `blood run --quiet <file>` suppresses most build messages but still emits `"Build successful: ..."` and `"Running: ..."` to stdout, mixed with program output. This makes it impossible to capture only program output without grep filtering.

**Current workaround:**
```bash
actual=$("$bin" run --quiet "$src" 2>/dev/null | grep -v '^Build successful:\|^Running:')
```

**Request:** Either:
1. `--quiet` suppresses all non-program output, or
2. Build messages go to stderr instead of stdout, or
3. Add `--silent` flag that suppresses everything except program output

---

### Module Resolution Limits

**Severity:** Low — constrains code organization

**Summary:** Adding module imports to files that already have many imports can cause previously-resolvable symbols to become unresolvable. Specifically, adding `mod codegen_ctx;` to `driver.blood` caused `source::read_file` and `source::parent_dir` to become unresolvable in later functions.

**Impact:** We cannot freely modularize large files because adding a new `mod` declaration might break resolution of existing cross-module references. This forces some files to remain larger than ideal.

**Request:** If there is a hard limit on module imports or resolution scope, document it. If it's a bug, a fix would allow better code organization for large codebases.

---

### Codegen Performance

**Severity:** Low — addressable with `--release` but still noticeable

**Summary:** Codegen accounts for 99.8% of build time. Comparison with `--release`:

```
Timings (--release, compiling the 61K-line self-hosted compiler):
  Parse                      1ms
  Type check               119ms
  MIR lowering              20ms
  Codegen                18974ms   ← 99.8%
  Total                  19251ms   (~19s)

Without --release:
  Total                 ~182000ms  (~3 min)
```

The `--release` flag helps enormously (9.5x speedup). If there are opportunities for incremental codegen (only recompile changed definitions), that would further improve iteration speed. The content-addressed per-definition caching system already exists for object file compilation — extending that caching to the single-module `--emit` path would be the logical next step.

---

## What Works Well

To be clear: blood-rust is remarkably capable. The following features work well and have been essential to self-hosting progress:

| Feature | Assessment |
|---------|------------|
| **Lexer/Parser** | Handles all Blood syntax correctly |
| **Type system** | Inference, unification, generics, traits all work |
| **Module system** | Cross-module types, chained paths, transitive deps all work |
| **Pattern matching** | Exhaustiveness checking, enum payloads, ref bindings work |
| **Builtins** | 109 core builtins registered and callable |
| **CLI** | `--emit`, `--timings`, `--release`, `--quiet` are all valuable |
| **Incremental compilation** | Content-addressed per-definition caching works |
| **Error diagnostics** | ariadne pretty-printing with source spans |
| **Runtime** | File I/O, string ops, Vec, Option, HashMap all functional |
| **Ground-truth tests** | 307 test programs provide excellent regression coverage |
| **MIR validation** | Catches well-formedness issues before codegen |
| **Safe LLVM pipeline** | Removing dangerous optimization passes prevents miscompilation |

The compiler is close to achieving full self-hosting. BUG-008 is the single remaining obstacle.

---

## Summary

| Priority | Issue | Impact | Effort Estimate |
|----------|-------|--------|-----------------|
| **P0** | BUG-008: if-expr codegen eliminates branch | Blocks self-hosting | Investigation needed |
| **P0** | Region memory pressure / no phase reset | OOM on large compilations | Medium |
| **P1** | `blood test` linking broken (LTO strips FFI) | Can't use test framework | Small (build config) |
| **P1** | Single-error-then-stop | 5x slower error fixing | Medium |
| **P1** | `--emit` fails on large programs | Can't inspect generated IR | Unknown |
| **P2** | `--quiet` leaks build messages to stdout | Test automation friction | Small |
| **P2** | Module resolution limits | Constrains code organization | Unknown |
| **P2** | Codegen is 99.8% of build time | Slow iteration | Large (incremental codegen) |

**The #1 request: Fix BUG-008.** Everything else is a productivity improvement. BUG-008 is what stands between the current state and a fully self-hosting Blood compiler.
