# Contributing to Blood

## Getting Started

### Prerequisites

- **LLVM 18** (llc-18 and clang-18 must be on PATH)
- **Git**
- No Rust toolchain required — the compiler bootstraps from a prebuilt seed binary.

### Building from Source

```bash
git clone https://github.com/jkindrix/blood.git
cd blood/src/selfhost

# Build the compiler from the bootstrap seed
./build_selfhost.sh build first_gen

# Run the golden test suite
./build_selfhost.sh test golden

# Compile and run a program
build/first_gen run ../../tests/golden/t00_hello_world.blood
```

### Quick Verification

```bash
cd src/selfhost

# Check a file (syntax, types, init, linearity, dangling refs)
build/first_gen check ../../tests/golden/t05_linear_affine.blood

# Build and run
build/first_gen run ../../tests/golden/t01_struct_basic.blood

# Full test suite
./build_selfhost.sh test golden
```

---

## Project Structure

```
blood/
├── src/
│   ├── selfhost/               # Self-hosted compiler (Blood) — primary
│   │   ├── main.blood          # Entry point, CLI, build pipeline
│   │   ├── parser.blood        # Recursive descent parser
│   │   ├── hir_lower.blood     # AST → HIR lowering
│   │   ├── typeck_driver.blood # Type checking orchestration
│   │   ├── mir_lower.blood     # HIR → MIR lowering
│   │   ├── codegen.blood       # MIR → LLVM IR generation
│   │   ├── mir_init.blood      # Definite init + linearity analysis
│   │   └── build_selfhost.sh   # Build script (all commands)
│   └── bootstrap/bloodc/src/   # Rust bootstrap compiler (legacy)
├── bootstrap/seed              # Prebuilt compiler binary (bootstrap fixed point)
├── stdlib/                     # Standard library (Blood source)
├── runtime/blood-runtime/      # Runtime library (Blood source)
├── tests/golden/               # Golden integration tests (576 pass)
├── proofs/theories/            # Coq formal proofs (22 files, 10K lines, 264 theorems)
├── docs/spec/                  # Language specifications
├── tools/                      # Development & debugging tools
└── CLAUDE.md                   # Compiler dev reference
```

### Key Components

| Component | Location | Language | Purpose |
|-----------|----------|----------|---------|
| Selfhost compiler | `src/selfhost/` | Blood | Primary compiler |
| Bootstrap seed | `bootstrap/seed` | — | Prebuilt binary for bootstrapping |
| Blood runtime | `runtime/blood-runtime/` | Blood | Memory, effects, fibers |
| Stdlib | `stdlib/` | Blood | Collections, effects, I/O |
| Golden tests | `tests/golden/` | Blood | 576 integration tests |
| Formal proofs | `proofs/theories/` | Coq | 273 mechanized theorems (219 proved, 14 admitted) |

---

## Development Workflow

The compiler is developed using a **self-compilation loop**:

```bash
cd src/selfhost

# 1. Edit source files

# 2. Quick check (seconds)
build/first_gen check file.blood

# 3. Incremental self-compilation (~1 min with cache, ~5 min clean)
./build_selfhost.sh build second_gen

# 4. Test
./build_selfhost.sh test golden second_gen

# 5. Before pushing: full bootstrap gate
./build_selfhost.sh gate
```

### Build Commands

```bash
./build_selfhost.sh build first_gen      # Seed → first_gen
./build_selfhost.sh build second_gen     # first_gen → second_gen (self-compile)
./build_selfhost.sh build third_gen      # second_gen → third_gen (verify)
./build_selfhost.sh test golden          # Run 576 golden tests
./build_selfhost.sh test golden second_gen  # Test with second_gen
./build_selfhost.sh gate                 # Full bootstrap + update seed
./build_selfhost.sh status               # Show compiler state
./build_selfhost.sh clean                # Remove build artifacts
```

### Build Caching

The build system has three cache layers:
- **Module-level source hashes** (`build/obj/.hashes/`) — skip unchanged modules
- **Per-function content hashes** (`build/.content_hashes/`) — skip unchanged functions via BLAKE3 canonical AST hashing
- **Source-level cache** (`build/.blood-cache`)

Caches are **compiler-version-specific** and automatically cleared between generations during `gate` and `build all`. When manually testing across generations, clear caches first:

```bash
rm -rf build/.content_hashes build/obj/.hashes build/.blood-cache
```

### Commit Messages

[Conventional Commits](https://www.conventionalcommits.org/) format:

```
feat(parser): add support for tuple patterns
fix(codegen): correct &str type in fat pointer emission
refactor(typeck): extract dispatch resolution into module
test(golden): add array bounds check tests
docs(spec): update effect system specification
```

---

## Compiler Architecture

```
Source (.blood) → Lexer → Parser → AST → HIR → Type Check → MIR → Codegen → LLVM IR → Binary
```

### Pipeline Phases

| Phase | Files | Purpose |
|-------|-------|---------|
| Parse | `parser.blood`, `parser_expr.blood` | Source → AST |
| HIR Lower | `hir_lower.blood`, `hir_lower_*.blood` | AST → HIR (name resolution) |
| Type Check | `typeck_driver.blood`, `typeck_expr.blood`, `unify.blood` | Type inference, dispatch resolution |
| MIR Lower | `mir_lower.blood`, `mir_lower_*.blood` | HIR → MIR (control flow) |
| Safety | `mir_init.blood`, `validate_mir.blood` | Init checking, linearity, dangling refs |
| Codegen | `codegen.blood`, `codegen_*.blood` | MIR → LLVM IR text |

### Safety Checks (run by default)

- **Definite initialization** — rejects use of uninitialized variables
- **Linearity checking** — linear values consumed exactly once, affine at most once
- **Array bounds checking** — runtime panic on out-of-bounds array/Vec access
- **Dangling reference rejection** — rejects `return &local` patterns (E0503)
- **Cast linearity stripping** — rejects casts that remove linear/affine qualifiers
- **Gen ref validation** — runtime stale reference detection via generational checks

---

## Testing

### Golden Tests

Golden tests are Blood programs in `tests/golden/` with expected behavior:

```blood
// Runtime test — checks output
// EXPECT: 42
fn main() -> i32 {
    println_int(42);
    0
}
```

```blood
// Compile-fail test — checks error message
// COMPILE_FAIL: E0221
// EXPECT: linear value not consumed
fn main() -> i32 {
    let x: linear i32 = 42;
    0
}
```

```blood
// Runtime panic test — checks nonzero exit
// EXPECT_EXIT: nonzero
fn main() -> i32 {
    let arr: [i32; 3] = [1, 2, 3];
    let i: i32 = 10;
    let x: i32 = arr[i];  // panics
    0
}
```

### Running Tests

```bash
cd src/selfhost
./build_selfhost.sh test golden              # All tests through first_gen
./build_selfhost.sh test golden second_gen   # All tests through second_gen
./build_selfhost.sh test golden -q           # Quiet (failures only)
```

### Adding Tests

Place new test files in `tests/golden/` with the naming convention:
- `t00_*` — basics (arithmetic, control flow)
- `t01_*` — primitives (structs, enums, generics)
- `t02_*` — regions
- `t03_*` — effects
- `t05_*` — features (dispatch, modules, strings)
- `t06_err_*` — compile-fail error tests
- `t07_*` — stdlib
- `t10_*` — traits and dynamic dispatch
- `t11_*` — advanced (dispatch specificity, generics)

---

## Code Style

### Blood Code (selfhost compiler)

- Match existing patterns — read the surrounding code first
- Exhaustive pattern matching — no catch-all `_ =>` unless justified
- No shortcuts — see CLAUDE.md for the full policy
- Use `.clone()` to copy Strings (Clone trait implemented for String)

### Pull Request Checklist

- [ ] All golden tests pass (`./build_selfhost.sh test golden`)
- [ ] Self-compilation succeeds (`./build_selfhost.sh build second_gen`)
- [ ] Commit messages follow conventional commits
- [ ] New features have golden tests
- [ ] Bug fixes have regression tests

For changes touching codegen, type layouts, or runtime FFI:
- [ ] Bootstrap gate passes (`./build_selfhost.sh gate`)

---

## Areas Needing Help

1. **Standard Library** — expanding collections, I/O, effects
2. **Tooling** — LSP server, formatter, REPL
3. **Documentation** — tutorials, guides, examples
4. **Testing** — more golden tests, edge cases, fuzzing
5. **Specs** — reviewing and improving language specifications

See [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md) for the current gap enumeration.

---

## Getting Help

- **Build reference**: See `CLAUDE.md` for commands, patterns, gotchas
- **Methodology**: See `DEVELOPMENT.md` for the CCV protocol
- **Specs**: See `docs/spec/` for language specifications
- **Bug history**: See `tools/FAILURE_LOG.md` for past bugs and resolutions
