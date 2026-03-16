# Design Evaluation: Blood Module System

**Version**: 0.1.0
**Status**: Draft
**Created**: 2026-03-15

## 1. Problem Statement

Blood's current module system was inherited from Rust without independent evaluation. The Design Space Audit (§1.8) identifies five axes as **Inherited** or **Defaulted**:

| Axis | Current State | Status |
|------|--------------|--------|
| Module hierarchy | File-based, Rust's file=module model | **Inherited** |
| Visibility | `pub`, `pub(crate)`, `pub(super)` | **Inherited** |
| Cyclic imports | Rejected (undocumented) | **Defaulted** |
| `mod.blood` convention | Rust's `mod.rs` renamed | **Inherited** |
| Functors/parameterized modules | Not addressed | **Defaulted** |

Meanwhile, Blood has design commitments that actively conflict with Rust's module model:

1. **Identity pillar**: Definitions are identified by content hash, not by name or path. Content-addressing explicitly decouples identity from filesystem location.
2. **`module` declarations**: Files declare `module std.collections.vec;` — a self-describing model (like Go/Java) that contradicts the parent-controlled model (`mod foo;` in a parent file, like Rust).
3. **Dot-separated paths**: Already decided (GRAMMAR.md v0.4.0). Not under evaluation here.

This document evaluates Blood's module system from first principles, identifies every Rust-ism, and proposes a coherent design aligned with Blood's pillars.

---

## 2. Current State

### 2.1 Two Contradictory Models

Blood currently has **two module models** that coexist awkwardly:

**Model A: Parent-controlled tree (from Rust)**
```blood
// main.blood — parent declares what modules exist
mod lexer;      // loads lexer.blood
mod parser;     // loads parser.blood
mod codegen;    // loads codegen.blood
```

The parent file controls the module tree. Children exist because a parent says they do. File resolution: `mod foo;` → `foo.blood` or `foo/mod.blood`. This is Rust's model verbatim with `.blood` instead of `.rs`.

**Model B: Self-declaring modules (from Go/Java)**
```blood
// stdlib/result.blood — file declares where it belongs
module std.result;

pub use std.core.result.{Result, Ok, Err};
```

Each file declares its own identity in the module namespace. The filesystem layout is organizational, not semantic. This is closer to Go packages or Java packages.

**The contradiction**: In Model A, a file's module identity is determined by which parent `mod` declaration includes it. In Model B, a file's module identity is what it says it is. These are philosophically opposed — one is top-down authority, the other is self-sovereignty.

**Current reality**: Model A is what the compilers actually implement. Model B (`module` declarations) exists in the grammar and some stdlib files but is **decorative** — the compilers don't use it for resolution or validation. A file can declare `module std.result;` but actually be loaded as part of a completely different module tree, and no error occurs.

### 2.2 Rust-isms Inventory

| Feature | Rust Origin | Blood Status |
|---------|------------|--------------|
| `mod foo;` parent-controlled loading | `mod foo;` in Rust | Implemented in both compilers |
| `foo/mod.blood` directory modules | `foo/mod.rs` in Rust | Implemented, not in spec |
| `crate` as root namespace | Rust's crate concept | In grammar as visibility scope + used in stdlib imports |
| `pub(crate)` visibility | Rust's crate-scoped visibility | In grammar, implemented |
| `pub(super)` visibility | Rust's parent-module visibility | In grammar, implemented |
| `self` in paths | Rust's self-referential paths | In grammar, parsed |
| `super` in paths | Rust's parent-referential paths | In grammar, parsed |
| `lib.blood` / `main.blood` convention | `lib.rs` / `main.rs` in Rust | In bootstrap resolver |
| Inline modules `mod foo { }` | `mod foo { }` in Rust | In grammar, implemented |
| Re-exports `pub use` | `pub use` in Rust | In grammar, implemented |
| Module = file, module tree = directory tree | Rust's core module model | Implemented |

### 2.3 What Blood Uses In Practice

**Selfhost compiler** (src/selfhost/): ~80 `.blood` files, all flat in one directory. Both `main.blood` and `driver.blood` repeat long lists of `mod` declarations. Files do NOT use `module` declarations. Cross-module references use dot notation: `hir_def.DefId`, `common.Span`.

**Stdlib** (stdlib/): Uses subdirectories (`collections/`, `effects/`, `core/`). Uses `mod.blood` files as directory entry points. Some files use `module` declarations (`module std.result;`). Uses `pub mod`, `pub use`, and `crate.` prefix in imports.

**Golden tests** (tests/golden/): Uses `mod` for multi-file tests (`mod t07_glob_import_lib;`). Uses inline modules (`mod inner { ... }`).

---

## 3. Design Space

### 3.1 Axis 1: Module Identity — Who Decides What a Module Is?

**Option A: Parent-controlled tree (current/Rust)**
Parent files declare `mod child;`, building an explicit tree. The compiler walks the tree from a root file.

- *Pro*: Explicit dependency ordering. The tree structure is visible.
- *Pro*: No ambiguity — exactly one way to reach each module.
- *Con*: Requires repeating `mod` lists (main.blood has 34 declarations, driver.blood has 25, codegen.blood has 21 — all overlapping).
- *Con*: Parent authority contradicts content-addressing. If a definition's identity is its hash, why does it matter which parent declares it?
- *Con*: Moving a file between directories changes its module path, which shouldn't matter if identity is content-addressed.

**Option B: Self-declaring modules (Go-style)**
Each file declares `module std.collections.vec;`. The compiler discovers files by convention or configuration, not by parent declaration.

- *Pro*: Aligns with content-addressing — files are self-describing, identity is intrinsic.
- *Pro*: No redundant `mod` lists. No `mod.blood` files needed.
- *Pro*: Moving a file between directories doesn't change its module path (you update the declaration).
- *Con*: Requires a discovery mechanism (project config, convention, or glob).
- *Con*: Two files could claim the same module path — needs conflict detection.
- *Con*: Build order is implicit (derived from `use` dependencies) rather than explicit (`mod` tree).

**Option C: Hybrid — self-declaring with explicit roots**
Files self-declare their module path. A project manifest (`Blood.toml`) or entry point specifies root files. The compiler discovers modules from `use` dependencies starting at roots.

- *Pro*: Self-describing files + deterministic discovery.
- *Pro*: No `mod` declarations, no `mod.blood` files.
- *Pro*: `Blood.toml` can specify multiple entry points (binary, library, tests).
- *Con*: More complex implementation than either pure model.

**Option D: No modules — flat namespace with content-addressed definitions**
Blood's Identity pillar taken to its logical conclusion: definitions are identified by hash, not by module path. "Modules" become organizational metadata (like tags), not namespace containers. All definitions live in a global content-addressed store (Marrow).

- *Pro*: Most aligned with the Identity pillar and content-addressing spec.
- *Pro*: No module conflicts, no visibility hierarchy, no path resolution.
- *Con*: Radical departure. No existing language does this at scale.
- *Con*: Human-readability suffers — people think in hierarchies, not hash sets.
- *Con*: Loss of access control (visibility modifiers need a hierarchy to be meaningful).
- *Con*: Premature — Marrow/Bloodbank are unimplemented. Cannot build on infrastructure that doesn't exist yet.

### 3.2 Axis 2: File ↔ Module Mapping

**Option A: One file = one module (current)**
Each `.blood` file is a module. Directory structure mirrors module hierarchy.

**Option B: One file = one module, but hierarchy is declared not structural**
Each `.blood` file is a module, but its position in the hierarchy comes from its `module` declaration, not its filesystem path. Files can live anywhere.

**Option C: Multiple modules per file**
A file can contain multiple `module` blocks. Useful for small related modules.

**Option D: One directory = one module (Go-style)**
All `.blood` files in a directory share a module identity. No file-level module boundaries.

### 3.3 Axis 3: Directory Module Convention

**Option A: `mod.blood` (current, from Rust's `mod.rs`)**
`mod foo;` falls back to `foo/mod.blood` if `foo.blood` doesn't exist.

**Option B: `_init.blood` or `_module.blood`**
A different sentinel filename that doesn't carry Rust connotations.

**Option C: Named entry file (`foo/foo.blood`)**
The entry point for directory `foo/` is `foo/foo.blood`. Redundant but unambiguous.

**Option D: No directory modules**
Subdirectories are organizational only. The compiler doesn't treat them specially. Discovery comes from `module` declarations or project config.

### 3.4 Axis 4: The `crate` Concept

**Option A: Keep `crate` (current, from Rust)**
`crate` is the root of the module tree. `pub(crate)` means "visible within this compilation unit."

- *Con*: "Crate" is Rust jargon. Blood calls compilation units "definitions" or "modules."
- *Con*: If Blood content-addresses individual definitions, what is "the crate"?

**Option B: Replace `crate` with `package`**
`package` is the Blood project unit. `pub(package)` replaces `pub(crate)`.

- *Pro*: Standard terminology (Go, Java, Python, Swift all use "package").
- *Con*: Still assumes a single-unit compilation model that may not match content-addressing.

**Option C: Replace `crate` with `module` root**
`pub(module)` means "visible within the current module subtree." The root module is whatever `Blood.toml` defines.

**Option D: Drop it entirely**
Content-addressed definitions don't have a "crate root." Visibility is `pub` (global) or `pub(module.path)` (scoped to a specific module). No special root concept.

### 3.5 Axis 5: Visibility Model

**Option A: Keep Rust's model (current)**
`pub`, `pub(crate)`, `pub(super)`, `pub(self)`, private.

**Option B: Simplified two-level**
`pub` (visible outside the module) and private (default). No fine-grained scoping.

- *Pro*: Simplest possible model. Go uses this and it works.
- *Con*: No way to share internals between sibling modules without making them public.

**Option C: Path-scoped visibility**
`pub(std.collections)` — visible to a specific module subtree. The grammar already has this (`VisScope ::= ... | ModulePath`), but it's unimplemented.

- *Pro*: Fine-grained without special keywords.
- *Pro*: Works with any module hierarchy model.
- *Con*: Verbose for common cases.

**Option D: Two-level + `internal`**
`pub` (global), `internal` (within the package/project), private (within the file/module). Three levels. Swift uses this model.

### 3.6 Axis 6: Cyclic Dependencies

**Option A: Reject cycles (current, undocumented)**
Both compilers detect and reject circular `mod` imports.

**Option B: Allow cycles within a module group**
Files within the same module subtree can reference each other. Cross-module cycles are rejected.

**Option C: Allow cycles everywhere**
The compiler resolves declarations in dependency order regardless of file boundaries.

### 3.7 Axis 7: Parameterized Modules (Functors)

**Option A: Not needed**
Blood's generics + traits + effects cover the use cases ML functors address.

**Option B: Module-level type parameters**
```blood
module SortedSet<T: Ord> {
    struct Set { items: Vec<T> }
    fn insert(set: &mut Set, item: T) { ... }
}
```

**Option C: Defer**
Evaluate after the module system fundamentals are settled.

---

## 4. Evaluation Against Blood's Pillars

### 4.1 Identity (Content-Addressed AST)

This is the decisive pillar. Blood's content-addressing spec says:

> *"Blood identifies all definitions by a cryptographic hash of their canonicalized AST, not by name."*

Implications:
- **Module paths are metadata, not identity.** A definition's hash doesn't change when you move it between modules. Names are human-facing labels mapped to hashes.
- **Parent-controlled trees are irrelevant to identity.** Whether `main.blood` says `mod parser;` has no bearing on what `parser.blood` contains or how it's hashed.
- **`mod.blood` is an implementation detail, not a semantic concept.** The content-addressing system doesn't care about directory entry points.
- **`crate` as a boundary is arbitrary.** If definitions are individually hashed, the crate boundary has no semantic meaning for identity. It's only relevant for visibility scoping.

**The current parent-controlled model (Rust's) is misaligned with the Identity pillar.** Files should be self-describing. The module path should come from the file's own declaration, not from which parent includes it. This doesn't mean we need Option D (pure content-addressing with no modules) — humans need hierarchies. But the hierarchy should be declared by the definition, not imposed by a parent.

### 4.2 Composability (Algebraic Effects)

Effects are declared on function signatures: `fn read() -> Data / {IO, Error<E>}`. The module system should make effect dependencies visible and manageable.

No strong preference between module models here. Both parent-controlled and self-declaring work equally well for effect visibility.

### 4.3 Isolation (Linear Types + Regions)

Regions are runtime scoped, not module scoped. Linear types are enforced by the type checker regardless of module boundaries.

No strong preference between module models here. Visibility affects whether you can *name* a linear type, but linearity enforcement is orthogonal.

### 4.4 Predictability (Design Hierarchy)

> Correctness > Safety > **Predictability** > Performance > Ergonomics

Self-declaring modules are more predictable than parent-controlled trees:
- A file's identity doesn't change when a parent file is edited.
- No need to trace the `mod` tree to understand where a file belongs.
- The `module` declaration at the top of every file tells you immediately.

### 4.5 Simplicity (Principle 4)

> *"Blood should be simpler than Rust to learn, but not by hiding complexity — by eliminating unnecessary complexity."*

The parent-controlled model has unnecessary complexity:
- Redundant `mod` declaration lists (the selfhost compiler repeats these across 3+ files).
- The `mod.blood` convention requires knowing that directories need sentinel files.
- `pub(crate)` vs `pub(super)` vs `pub` requires understanding the tree structure to reason about visibility.

Self-declaring modules eliminate the `mod` boilerplate entirely. You write `module` at the top, `use` for dependencies, and the compiler figures out the rest.

### 4.6 DWARF Debug Info

Self-declaring modules provide a direct benefit to debuggability. Each file's `module` declaration IS the canonical source filename for DWARF metadata. Today, DWARF source locations must reconstruct module paths from the `mod` tree — a fragile process that breaks when files are loaded from multiple parents. With self-declaring modules, the mapping from module path to source file is 1:1 and explicit.

---

## 5. Proposal

### 5.1 Core Decision: Self-Declaring Modules

**Every `.blood` file declares its own module identity:**

```blood
module std.collections.vec;
```

This is **the** module identity mechanism. Not `mod` in a parent. Not filesystem position. The declaration.

**Rationale**: Aligns with the Identity pillar (definitions are self-describing), eliminates redundant `mod` lists, and is simpler than Rust's model.

### 5.2 Module Declaration Rules

1. **Every `.blood` file MUST have a `module` declaration** (first non-comment item).
2. The module path is dot-separated: `module project.subsystem.component;`
3. Two files MUST NOT declare the same module path (compiler error).
4. The module path is metadata — it provides human-readable naming and visibility scoping. A definition's content hash is still its true identity.

### 5.3 Discovery Mechanism

The compiler needs to know which files to compile. Three approaches, in order of priority:

1. **Entry point**: `blood build main.blood` or `blood build lib.blood` — the specified file is the root.
2. **`use` dependencies**: Starting from the root, the compiler follows `use` declarations to discover needed modules.
3. **Project manifest** (optional): `Blood.toml` can list source roots, entry points, and dependencies.

**How `use` resolution works:**

```blood
// In main.blood:
module myproject.main;

use myproject.parser.Parser;   // compiler must find a file declaring `module myproject.parser;`
use std.collections.HashMap;   // compiler must find a file declaring `module std.collections;`
```

The compiler searches for files declaring the target module. Search locations:
1. **Same directory** as the importing file.
2. **Source root** directory (from `Blood.toml` or inferred from entry point).
3. **Standard library** path.
4. **Dependency paths** (from `Blood.toml`).

Within each search location, the compiler checks all `.blood` files for matching `module` declarations. This can be cached across compilations.

### 5.4 No `mod` Declarations for File Loading

`mod name;` (external file loading) is **removed**. Files are discovered through `use` dependencies and `module` declarations, not through parent `mod` trees.

`mod name { ... }` (inline modules) is **retained**. Inline modules are useful for small, closely-related items:

```blood
module myproject.parser;

mod tokens {
    pub enum TokenKind { Ident, Number, String }
}

use tokens.TokenKind;
```

Inline modules are scoped to the file. They don't affect the filesystem.

### 5.5 No `mod.blood` Convention

Directory entry point files are not needed. Any `.blood` file in a directory can declare any module path. The directory structure is for human organization only.

**Convention** (not enforced): name files after their module's leaf segment. `std.collections.vec` lives in `vec.blood`. This is a convention, not a rule.

### 5.6 Replace `crate` with `package`

Blood projects are **packages**, not crates:

```blood
// Before (Rust-ism):
pub(crate) fn internal_helper() { }
use crate.core.Option;

// After:
pub(package) fn internal_helper() { }
use package.core.Option;    // or just: use mypackage.core.Option;
```

A **package** is defined by `Blood.toml`. It has a name, version, and content hash. `pub(package)` means "visible within this package."

If no `Blood.toml` exists (single-file compilation), the package is the file itself.

### 5.7 Visibility Model

**Retain the current model with `crate` → `package` rename:**

| Visibility | Meaning |
|-----------|---------|
| (none) | Private to the current module (file + inline modules) |
| `pub` | Public to all consumers |
| `pub(package)` | Public within the package |
| `pub(super)` | Public to the parent module |
| `pub(self)` | Explicit private (same as default) |
| `pub(path.to.module)` | Public to a specific module subtree |

`pub(super)` is meaningful for inline modules. For file-level modules, `super` refers to the parent in the module path (e.g., in `module a.b.c;`, `super` is `a.b`).

### 5.8 Cyclic Dependencies

**Cycles between module files are rejected.** If two modules need each other, they should be one module or share a common dependency.

**Rationale**: While Blood's multi-pass compilation could technically resolve cycles, the practical costs outweigh the theoretical benefit:
- Type checking across cyclic modules requires a fixpoint algorithm, adding compiler complexity and unpredictable error messages.
- Every language that allows cyclic imports (Python, JavaScript) has painful runtime edge cases. Every language that forbids them (Go, Rust) considers it a feature.
- The selfhost compiler already has a documented module resolution limit; cycles would worsen this.
- Acyclicity forces cleaner architecture — it's a design constraint that produces better code.

**Cross-package cycles are also rejected** (packages are independent compilation units).

### 5.9 Parameterized Modules

**Deferred.** Blood's generics, traits, and effects cover the primary use cases. Revisit after the base module system is stable.

---

## 6. Migration Path

### 6.1 What Changes

| Before | After |
|--------|-------|
| `mod foo;` loads `foo.blood` | Removed. Use `use` + `module` declarations. |
| `foo/mod.blood` directory entry | Removed. Any file can declare any module path. |
| `crate` keyword in paths/visibility | `package` keyword |
| `pub(crate)` | `pub(package)` |
| Files without `module` declarations | Now required (compiler error) |
| `mod` lists in parent files | Removed entirely |

### 6.2 What Stays

| Feature | Status |
|---------|--------|
| `mod name { ... }` inline modules | Kept |
| `use path.to.item;` imports | Kept |
| `pub use` re-exports | Kept |
| Dot-separated paths | Kept (already decided) |
| `pub`, `pub(super)`, `pub(self)` | Kept |
| `pub(module.path)` | Kept |

### 6.3 Migration for Selfhost Compiler

**Before** (main.blood):
```blood
mod common;
mod ast;
mod hir;
mod hir_def;
mod parser;
mod codegen;
// ... 30 more mod declarations
```

**After** (main.blood):
```blood
module blood.main;

use blood.common.Span;
use blood.parser.parse;
use blood.codegen.compile;
// ... only import what you use
```

Each file (e.g., `common.blood`) gains a `module` declaration:
```blood
module blood.common;

// ... definitions as before
```

The `mod` lists vanish. Files reference each other through `use` declarations. The compiler discovers files by scanning the source directory for `module blood.*` declarations.

### 6.4 Migration for Stdlib

**Before** (stdlib/mod.blood):
```blood
pub mod algorithms;
pub mod collections;
pub mod core;
```

**After** (no `mod.blood` needed):
```blood
// stdlib/algorithms/sort.blood
module std.algorithms.sort;

pub fn quicksort<T: Ord>(items: &mut [T]) { ... }
```

```blood
// stdlib/collections/hashmap.blood
module std.collections.hashmap;

pub struct HashMap<K, V> { ... }
```

The stdlib directory structure becomes purely organizational. The compiler finds modules by their declarations.

---

## 7. Open Questions

### Q1: How does the compiler efficiently discover modules?

When the compiler encounters `use foo.bar.Baz;`, it needs to find the file declaring `module foo.bar;`. Options:

**A. Scan-and-cache**: On first compilation, scan all `.blood` files in the source root(s) for `module` declarations. Cache the mapping. Incremental: re-scan only changed files.

**B. Convention-guided**: Look for `bar.blood` in directories that could match `foo/bar/`. Fall back to full scan if not found.

**C. Manifest-listed**: `Blood.toml` lists source directories. Only those are scanned.

Recommendation: **A with C as optimization**. Scanning `.blood` files for their first line is fast — the `module` declaration is required to be the first non-comment item, so the scanner reads at most a few lines per file. For a 1000-file project, this is ~10ms of I/O (well under the noise floor of compilation). Caching strategy: write a `module_index.cache` file mapping `(file_path, mtime) → module_path`. On subsequent compilations, re-scan only files whose mtime changed. This makes warm discovery O(changed_files), not O(all_files).

### Q2: Can multiple files contribute to the same module?

Go allows multiple files in a directory to share a package name. Should Blood allow:
```blood
// vec_core.blood
module std.collections.vec;
pub struct Vec<T> { ... }

// vec_iter.blood
module std.collections.vec;
pub fn iter<T>(v: &Vec<T>) -> Iterator<T> { ... }
```

This document proposes **no** — one file per module path. This is simpler and avoids partial-module compilation complexity.

**Trade-off: large files.** The selfhost's `codegen_expr.blood` is 3700+ lines. With one-file-per-module, it stays that way. The mitigation is inline modules — a large file can be internally organized with `mod codegen_place { ... }`, `mod codegen_index { ... }`, etc. This provides logical grouping without splitting the compilation unit. If a file grows beyond ~4000 lines, that's a signal to extract a new module (a new file with its own `module` declaration), not to allow multi-file modules.

### Q3: What about `super` and `self` in paths?

With self-declaring modules, `super` is well-defined: the parent segment of the module path. `self` is the current module. Both can remain useful:

```blood
module std.collections.vec;

use super.hashmap.HashMap;    // → std.collections.hashmap.HashMap
use self.VecIter;             // → std.collections.vec.VecIter (same module)
```

Recommend: **Keep both**, they're useful shorthands.

### Q4: What replaces `pub mod` re-exports?

Currently, `pub mod foo;` makes module `foo` visible to consumers. With self-declaring modules, re-exports use `pub use`:

```blood
// stdlib/collections.blood (or any file declaring module std.collections)
module std.collections;

pub use std.collections.hashmap.HashMap;
pub use std.collections.hashset.HashSet;
```

This is already how Blood re-exports work. No change needed.

---

## 8. Implementation Priority

This is a design document, not an implementation plan. The current module system works — it's ugly with the `mod` lists, but it compiles code correctly. **This redesign should be deferred** until generational references and the iterator protocol are stable, since those directly affect the language's usability and safety story. The module system is an ergonomic improvement, not a correctness fix.

When the time comes, **prototype before spec change.** The spec-first principle says "spec prescribes, implementation conforms" — but for a module system redesign, implementation feedback is needed to get the spec right. Build a working prototype in the bootstrap compiler for a subset of the selfhost before committing to a GRAMMAR.md update.

Suggested order if/when implemented:

1. **Prototype**: Implement `module`-based discovery in the bootstrap compiler for a test subset. Validate that use-based resolution works for real code.
2. **Add `module` declarations** to all existing `.blood` files (mechanical, no behavior change under the current system — this can be done today as a preparatory step).
3. **Spec update**: Update GRAMMAR.md §2.2 and §3.7 with the new module model, informed by prototype findings.
4. **Add `package` keyword** alongside `crate` (backwards-compatible addition).
5. **Implement `module`-based discovery** fully in the bootstrap compiler.
6. **Deprecate `mod name;`** (external file loading) — warning, then error.
7. **Remove `mod.blood` fallback** from both compilers.
8. **Migrate stdlib** to pure `module` + `use` model.
9. **Rename `crate` → `package`** in remaining code.
10. **Port changes** to selfhost compiler.
11. **Update golden tests**.

---

## 9. Alternatives Considered and Rejected

### 9.1 Keep Rust's Model With Cosmetic Changes

Replace `crate` with `package`, remove `mod.blood`, but keep the parent-controlled `mod` tree.

**Rejected**: This addresses symptoms (Rust jargon) but not the root cause (the module model contradicts content-addressing). The redundant `mod` lists remain. The fundamental misalignment with the Identity pillar remains.

### 9.2 Go-Style Directory = Module

All `.blood` files in a directory share a module identity. No per-file declarations.

**Rejected**: Go's model works because Go has no generics that create cross-file dependencies within a package. Blood's generics, traits, and effects create complex intra-module dependencies. Per-file modules give the compiler clearer compilation boundaries.

### 9.3 Pure Content-Addressed (No Module Hierarchy)

Definitions are identified purely by hash. Module paths are optional metadata.

**Rejected as premature**: This is where Blood's Identity pillar ultimately points, but it requires Marrow (the codebase manager) to be functional. Today, the compiler needs a filesystem-based module system. The self-declaring model proposed here is a stepping stone — when Marrow is ready, `module` declarations become the name↔hash mapping that Marrow indexes.

---

## 10. References

- GRAMMAR.md §2.2 (Module System), §3.7 (Module Declarations)
- SPECIFICATION.md §2.2 (Five Pillars), §2.3 (Design Principles)
- CONTENT_ADDRESSED.md §1 (Overview), §10.5 (Marrow)
- DESIGN_SPACE_AUDIT.md §1.8 (Module System), §3.3 (Tensions)
- ADR-003 (Content-Addressed Code via BLAKE3-256)
- ADR-037 (Compiler-as-a-Library)
