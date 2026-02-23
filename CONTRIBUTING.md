# Contributing to Blood

Thank you for your interest in contributing to the Blood programming language! This guide will help you get started.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Project Structure](#project-structure)
3. [Development Workflow](#development-workflow)
4. [Compiler Architecture](#compiler-architecture)
5. [Testing](#testing)
6. [Code Style](#code-style)
7. [Pull Request Process](#pull-request-process)
8. [Finding Issues to Work On](#finding-issues-to-work-on)

---

## Getting Started

### Prerequisites

- **Rust** 1.77+ (install via [rustup](https://rustup.rs/))
- **LLVM 14** (for code generation via inkwell)
- **Git**

### Building from Source

```bash
# Clone the repository
git clone https://github.com/jkindrix/blood.git
cd blood/src/bootstrap

# Build everything
cargo build --release

# Run tests
cargo test --workspace

# Run the compiler
cargo run --release -- check ../examples/hello.blood
```

### Quick Verification

```bash
# From src/bootstrap/
# Type check an example
cargo run --release -- check ../examples/binary_tree_benchmark.blood

# Run all tests
cargo test --workspace

# Run benchmarks
cargo bench --bench runtime_bench
```

---

## Project Structure

```
blood/                          # Repository root (monorepo)
├── blood-std/                  # Standard library (Blood source)
│   └── std/
│       ├── core/               # Core types (Option, String, Box, etc.)
│       ├── collections/        # Vec, HashMap, LinkedList, etc.
│       └── effects/            # Effect system primitives
├── src/
│   ├── selfhost/               # Self-hosted compiler (written in Blood)
│   └── bootstrap/              # Rust bootstrap compiler (git subtree)
│   ├── bloodc/                 # The main compiler
│   │   ├── src/
│   │   │   ├── main.rs         # CLI entry point
│   │   │   ├── lexer.rs        # Tokenization
│   │   │   ├── parser/         # AST parsing
│   │   │   ├── hir/            # High-level IR
│   │   │   ├── mir/            # Mid-level IR
│   │   │   │   └── lowering/   # HIR → MIR transformation
│   │   │   ├── typeck/         # Type checking
│   │   │   │   ├── context/    # Type checking context
│   │   │   │   ├── dispatch/   # Multiple dispatch resolution
│   │   │   │   └── exhaustiveness.rs
│   │   │   ├── codegen/        # LLVM code generation
│   │   │   │   ├── context/    # Codegen context
│   │   │   │   └── mir_codegen/ # MIR → LLVM
│   │   │   ├── effects/        # Effect system
│   │   │   ├── diagnostics.rs  # Error reporting
│   │   │   └── span.rs         # Source locations
│   │   ├── tests/              # Integration tests
│   │   └── benches/            # Performance benchmarks
│   ├── blood-runtime/          # Runtime library
│   │   └── src/
│   │       ├── memory.rs       # Memory management
│   │       ├── continuation.rs # Effect continuations
│   │       ├── fiber.rs        # Fiber scheduling
│   │       └── ffi_exports.rs  # C FFI exports
│   ├── blood-std/              # Standard library copy for tests
│   ├── runtime/                # C runtime stubs
│   └── Cargo.toml              # Workspace manifest
├── docs/                       # Language specification & documentation
│   ├── spec/                   # Core specs
│   └── postmortem/             # Bug investigation records
├── examples/                   # Blood language examples
└── editors/                    # Editor support (VS Code, etc.)
```

> **Note:** The Cargo workspace is at `src/bootstrap/Cargo.toml`. All `cargo` commands must be run from the `src/bootstrap/` directory.

### Key Crates

| Crate | Purpose |
|-------|---------|
| `bloodc` | Main compiler: lexer, parser, type checker, codegen |
| `blood-runtime` | Runtime library: memory, effects, fibers |
| `blood-std` | Standard library written in Blood |

---

## Development Workflow

### Making Changes

1. **Create a feature branch**
   ```bash
   git checkout -b feature/my-feature
   ```

2. **Make your changes**
   - Write code
   - Add tests
   - Update documentation

3. **Run tests locally** (from `src/bootstrap/`)
   ```bash
   cargo test --workspace
   cargo clippy --all-targets
   ```

4. **Commit with conventional commits**
   ```bash
   git commit -m "feat(parser): add support for tuple patterns"
   ```

### Commit Message Format

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `refactor`: Code change that neither fixes a bug nor adds a feature
- `docs`: Documentation only
- `test`: Adding or updating tests
- `perf`: Performance improvement
- `chore`: Maintenance tasks

**Examples:**
```
feat(typeck): implement exhaustiveness checking for enum patterns
fix(parser): handle trailing commas in function arguments
docs(spec): update effect system specification
refactor(mir): extract common lowering utilities
test(codegen): add tests for closure capture
```

---

## Compiler Architecture

### Compilation Phases

```
Source Code (.blood)
       │
       ▼
┌─────────────┐
│   Lexer     │  Tokenize source into tokens
└─────────────┘
       │
       ▼
┌─────────────┐
│   Parser    │  Build AST from tokens
└─────────────┘
       │
       ▼
┌─────────────┐
│   HIR       │  High-level IR with resolved names
│   Lowering  │
└─────────────┘
       │
       ▼
┌─────────────┐
│   Type      │  Type inference and checking
│   Checker   │
└─────────────┘
       │
       ▼
┌─────────────┐
│   MIR       │  Mid-level IR for optimization
│   Lowering  │
└─────────────┘
       │
       ▼
┌─────────────┐
│   Codegen   │  Generate LLVM IR
└─────────────┘
       │
       ▼
   Object Code
```

### Key Data Structures

**DefId** - Unique identifier for definitions:
```rust
pub struct DefId {
    pub index: u32,  // Index in definition table
}
```

**Type** - Type representation:
```rust
pub enum Type {
    Int(IntTy),      // i8, i16, i32, i64
    Float(FloatTy),  // f32, f64
    Bool,
    Str,
    Unit,
    Tuple(Vec<Type>),
    Array(Box<Type>, usize),
    Function(FnType),
    Named(DefId, Vec<Type>),  // struct/enum with generics
    // ... more variants
}
```

**HIR Expr** - Expression representation:
```rust
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

pub enum ExprKind {
    Literal(Literal),
    Local(LocalId),
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    // ... more variants
}
```

### Important Modules

| Module | Purpose | Key Types |
|--------|---------|-----------|
| `typeck/context/` | Type checking state | `TypeckContext` |
| `typeck/dispatch/` | Multiple dispatch | `DispatchTable` |
| `mir/lowering/` | HIR → MIR | `MirBuilder` |
| `codegen/context/` | LLVM codegen state | `CodegenContext` |
| `effects/` | Effect inference | `EffectSet` |

---

## Testing

### Test Categories

All test commands run from `src/bootstrap/`:

1. **Unit Tests** - In `src/` alongside code
   ```bash
   cargo test --lib
   ```

2. **Integration Tests** - In `tests/`
   ```bash
   cargo test --test pipeline_integration
   ```

3. **Benchmarks** - In `benches/`
   ```bash
   cargo bench --bench runtime_bench
   ```

### Writing Tests

**Parser tests:**
```rust
#[test]
fn test_parse_function() {
    let source = "fn foo(x: i32) -> i32 { x + 1 }";
    let ast = parse(source).unwrap();
    assert!(matches!(ast.items[0], Item::Function { .. }));
}
```

**Type checking tests:**
```rust
#[test]
fn test_type_mismatch() {
    let source = "fn main() { let x: i32 = \"hello\"; }";
    let result = typecheck(source);
    assert!(result.is_err());
    assert!(result.unwrap_err()[0].message.contains("type mismatch"));
}
```

**End-to-end tests:**
```rust
#[test]
fn test_e2e_binary_tree() {
    let source = include_str!("../../examples/binary_tree_benchmark.blood");
    let result = compile_and_check(source);
    assert!(result.is_ok());
}
```

### Test Naming Convention

```
test_<phase>_<feature>_<scenario>
```

Examples:
- `test_parser_function_with_generics`
- `test_typeck_effect_inference`
- `test_codegen_closure_capture`
- `test_e2e_stale_reference_detection`

---

## Code Style

### Rust Guidelines

- Use `rustfmt` for formatting
- Use `clippy` for linting
- Write documentation for public APIs

```bash
# From src/bootstrap/
cargo fmt
cargo clippy --all-targets
```

### Project-Specific Guidelines

1. **No shortcuts** (per CLAUDE.md):
   - Every pattern match must be exhaustive
   - No silent failures (`_ => Ok(())`)
   - No placeholder returns (`Type::error()`)

2. **Error handling**:
   - Use `Result<T, Vec<Diagnostic>>` for recoverable errors
   - Include helpful error messages with source spans
   - Never panic in compiler code (except ICE)

3. **Documentation**:
   - Document public functions with `///`
   - Add module-level docs with `//!`
   - Reference spec sections in complex algorithms

### Example Code

```rust
/// Lower a pattern from AST to HIR.
///
/// # Algorithm
///
/// Pattern lowering transforms AST patterns into HIR patterns while:
/// 1. Type checking - validates pattern structure
/// 2. Variable binding - creates LocalIds
/// 3. Exhaustiveness - contributes to coverage analysis
///
/// # Errors
///
/// Returns diagnostics for:
/// - Type mismatches between pattern and expected type
/// - Invalid pattern structure for the type
///
/// See SPECIFICATION.md Section 5.3 for pattern semantics.
pub fn lower_pattern(
    &mut self,
    pattern: &ast::Pattern,
    expected_ty: &Type,
) -> Result<hir::Pattern, Vec<Diagnostic>> {
    match &pattern.kind {
        ast::PatternKind::Literal(lit) => {
            // Handle literal pattern...
        }
        ast::PatternKind::Binding(name) => {
            // Handle binding pattern...
        }
        // Explicitly handle all variants - no catch-all
    }
}
```

---

## Pull Request Process

### Before Submitting

- [ ] All tests pass (`cd src/bootstrap && cargo test --workspace`)
- [ ] No clippy warnings (`cargo clippy --all-targets`)
- [ ] Code is formatted (`cargo fmt --check`)
- [ ] Documentation is updated
- [ ] Commit messages follow convention

### PR Description Template

```markdown
## Summary

Brief description of the changes.

## Changes

- List of specific changes
- Another change

## Testing

How the changes were tested.

## Related Issues

Closes #123
```

### Review Process

1. Create PR against `main`
2. Automated CI checks run
3. Maintainer reviews code
4. Address feedback
5. PR merged when approved

### Code Review Guidelines

#### For Authors

**Before requesting review:**
- Self-review your changes first
- Ensure all CI checks pass
- Keep PRs focused - one logical change per PR
- Large changes should be split into reviewable chunks

**Responding to feedback:**
- Address all comments before re-requesting review
- Explain your reasoning if you disagree with feedback
- Mark resolved conversations as resolved
- Don't force-push after review has started (makes it hard to see changes)

#### For Reviewers

**Review checklist:**

1. **Correctness**
   - [ ] Does the code do what it claims?
   - [ ] Are edge cases handled?
   - [ ] Are there potential panics or unwraps that should be Results?

2. **Safety (per CLAUDE.md)**
   - [ ] No silent failures (`_ => Ok(())`, `_ => continue`)
   - [ ] No catch-all patterns that should be explicit
   - [ ] No `unwrap_or_default()` hiding real errors
   - [ ] No placeholder returns (`Type::error()` without justification)

3. **Testing**
   - [ ] Are new features tested?
   - [ ] Are bug fixes accompanied by regression tests?
   - [ ] Do tests cover edge cases?

4. **Design**
   - [ ] Does the change fit the existing architecture?
   - [ ] Is the code appropriately modular?
   - [ ] Are abstractions at the right level?

5. **Documentation**
   - [ ] Are public APIs documented?
   - [ ] Are complex algorithms explained?
   - [ ] Are spec references included where appropriate?

6. **Performance**
   - [ ] Are there obvious performance issues?
   - [ ] Are allocations minimized in hot paths?
   - [ ] Is the algorithmic complexity appropriate?

**Review etiquette:**
- Be constructive and specific
- Distinguish between blocking issues and suggestions
- Use prefixes: `nit:` (minor), `suggestion:` (optional), `blocking:` (must fix)
- Acknowledge good work and clever solutions
- Ask questions rather than making assumptions

#### Review Labels

| Label | Meaning |
|-------|---------|
| `needs-review` | Ready for review |
| `changes-requested` | Author needs to address feedback |
| `approved` | Ready to merge |
| `needs-discussion` | Requires broader input |
| `blocked` | Waiting on external dependency |

#### Merge Requirements

PRs require:
- At least 1 approving review
- All CI checks passing
- No unresolved blocking comments
- Up-to-date with target branch

#### Review Response Time

- **Initial review**: Within 3 business days
- **Follow-up reviews**: Within 1 business day
- If no review after 5 days, ping in comments

---

## Finding Issues to Work On

### Good First Issues

Look for issues labeled:
- `good-first-issue` - Simple, well-defined tasks
- `help-wanted` - Tasks that need contributors
- `documentation` - Docs improvements

### Action Items

See `docs/spec/ACTION_ITEMS.md` for a prioritized list of work items:

| Priority | Description |
|----------|-------------|
| P1 | High priority - needed for beta |
| P2 | Medium priority - needed for 1.0 |
| P3 | Low priority - nice to have |

### Areas Needing Help

1. **Standard Library** - Implementing collections, I/O
2. **Documentation** - Tutorials, guides, examples
3. **Tooling** - LSP, formatter, REPL
4. **Testing** - More integration tests, fuzzing
5. **Optimization** - Performance improvements

---

## Getting Help

- **Documentation**: Check `docs/spec/` for specifications
- **Architecture**: See `docs/spec/DECISIONS.md` for ADRs
- **Questions**: Open a discussion or issue

---

## Code of Conduct

We follow the [Contributor Covenant](https://www.contributor-covenant.org/). Be respectful and constructive.

---

*Last updated: 2026-02-12*
