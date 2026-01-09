# Blood Implementation Roadmap

**Version**: 0.1.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

This document outlines the implementation strategy for the Blood programming language, including compiler architecture, phases, milestones, and the bootstrap plan.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Compiler Architecture](#2-compiler-architecture)
3. [Implementation Phases](#3-implementation-phases)
4. [Phase 0: Foundation](#4-phase-0-foundation)
5. [Phase 1: Core Language](#5-phase-1-core-language)
6. [Phase 2: Effects System](#6-phase-2-effects-system)
7. [Phase 3: Memory Model](#7-phase-3-memory-model)
8. [Phase 4: Content Addressing](#8-phase-4-content-addressing)
9. [Phase 5: Runtime](#9-phase-5-runtime)
10. [Phase 6: Self-Hosting](#10-phase-6-self-hosting)
11. [Milestones](#11-milestones)
12. [Testing Strategy](#12-testing-strategy)
13. [Open Questions](#13-open-questions)

---

## 1. Overview

### 1.1 Implementation Strategy

Blood will be implemented in stages:

1. **Bootstrap Compiler** — Written in Rust, targeting LLVM
2. **Standard Library** — Core types and effects in Blood
3. **Self-Hosting** — Compiler rewritten in Blood
4. **Optimization** — Performance-focused development

### 1.2 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — Phase 3 implementation details
- [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) — Phase 4 implementation details
- [CONCURRENCY.md](./CONCURRENCY.md) — Phase 5 implementation details
- [DECISIONS.md](./DECISIONS.md) — Architectural decision records
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — Error infrastructure implementation

### 1.3 Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Bootstrap Language** | Rust | Memory safety, ecosystem, performance |
| **Initial Backend** | LLVM | Mature, multi-target, optimized |
| **Future Backend** | Custom / Cranelift | Faster compilation, better control |
| **Parser Generator** | Hand-written | Better errors, incremental parsing |
| **Type Checker** | Bidirectional | Simpler implementation, good inference |

### 1.3 Timeline Philosophy

This roadmap describes **what** must be done, not **when**. Implementation proceeds when contributors are available. Each phase has clear completion criteria rather than deadlines.

---

## 2. Compiler Architecture

### 2.1 Pipeline Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        BLOOD COMPILER PIPELINE                               │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Source Text                                                                 │
│       │                                                                      │
│       ▼                                                                      │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐                   │
│  │  Lexer  │───►│ Parser  │───►│  HIR    │───►│  Type   │                   │
│  │         │    │         │    │ Lower   │    │  Check  │                   │
│  └─────────┘    └─────────┘    └─────────┘    └─────────┘                   │
│                                                      │                       │
│                                                      ▼                       │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐                   │
│  │  LLVM   │◄───│  Code   │◄───│   MIR   │◄───│  MIR    │                   │
│  │   IR    │    │  Gen    │    │         │    │ Lower   │                   │
│  └─────────┘    └─────────┘    └─────────┘    └─────────┘                   │
│       │                                                                      │
│       ▼                                                                      │
│  ┌─────────────────────────────────────────────────────────────────┐        │
│  │                        LLVM Backend                              │        │
│  │  Optimization ──► Target Codegen ──► Linking ──► Executable     │        │
│  └─────────────────────────────────────────────────────────────────┘        │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Intermediate Representations

| IR Level | Description | Purpose |
|----------|-------------|---------|
| **CST** | Concrete Syntax Tree | Parsing, formatting |
| **AST** | Abstract Syntax Tree | Desugaring, macro expansion |
| **HIR** | High-level IR | Type checking, effect analysis |
| **MIR** | Mid-level IR | Optimization, borrow analysis |
| **LIR** | Low-level IR | Code generation |
| **LLVM IR** | LLVM's IR | Backend optimization, codegen |

### 2.3 Module Structure

```
blood/
├── bloodc/                    # Compiler crate
│   ├── src/
│   │   ├── lexer/            # Tokenization
│   │   ├── parser/           # Parsing to AST
│   │   ├── hir/              # High-level IR
│   │   ├── typeck/           # Type checking
│   │   ├── effects/          # Effect system
│   │   ├── mir/              # Mid-level IR
│   │   ├── codegen/          # LLVM codegen
│   │   ├── driver/           # Compilation orchestration
│   │   └── diagnostics/      # Error reporting
│   └── Cargo.toml
├── blood-runtime/             # Runtime library
│   ├── src/
│   │   ├── gc/               # Generational GC (Tier 3)
│   │   ├── regions/          # Region allocator
│   │   ├── fibers/           # Fiber scheduler
│   │   ├── effects/          # Effect runtime
│   │   └── ffi/              # FFI support
│   └── Cargo.toml
├── blood-std/                 # Standard library (in Blood)
│   ├── core/                 # Core types
│   ├── io/                   # IO effects
│   ├── collections/          # Data structures
│   └── ...
└── blood-tools/               # Tooling
    ├── lsp/                  # Language server
    ├── fmt/                  # Formatter
    └── ucm/                  # Codebase manager
```

---

## 3. Implementation Phases

### 3.1 Phase Dependencies

```
Phase 0 (Foundation)
    │
    ├──► Phase 1 (Core Language)
    │        │
    │        ├──► Phase 2 (Effects)
    │        │        │
    │        │        └──► Phase 3 (Memory Model)
    │        │                  │
    │        │                  └──► Phase 4 (Content Addressing)
    │        │                            │
    │        └──────────────────────────► Phase 5 (Runtime)
    │                                          │
    └─────────────────────────────────────────► Phase 6 (Self-Hosting)
```

### 3.2 Phase Completion Criteria

Each phase has explicit exit criteria:

| Phase | Exit Criteria |
|-------|---------------|
| **Phase 0** | Can parse and type-check simple programs |
| **Phase 1** | Can compile and run FizzBuzz |
| **Phase 2** | Effect handlers work correctly |
| **Phase 3** | Generational references pass all safety tests |
| **Phase 4** | Content-addressed codebase is functional |
| **Phase 5** | Fibers and concurrency work |
| **Phase 6** | Compiler compiles itself |

---

## 4. Phase 0: Foundation

### 4.1 Goals

- Project structure and build system
- Lexer and parser
- Basic AST representation
- Simple type checker (no effects yet)
- Error reporting infrastructure

### 4.2 Lexer Implementation

```rust
// Token types
pub enum Token {
    // Keywords
    Fn, Let, If, Else, Match, Effect, Handler, ...

    // Literals
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    BoolLit(bool),

    // Operators
    Plus, Minus, Star, Slash, ...

    // Delimiters
    LParen, RParen, LBrace, RBrace, ...

    // Identifiers
    Ident(String),
    TypeIdent(String),

    // Special
    Eof,
    Error(LexError),
}
```

### 4.3 Parser Implementation

Hand-written recursive descent parser with:
- **Pratt parsing** for expressions (precedence climbing)
- **Error recovery** for better diagnostics
- **Incremental parsing** hooks for LSP

```rust
pub struct Parser<'src> {
    lexer: Lexer<'src>,
    current: Token,
    previous: Token,
    errors: Vec<ParseError>,
}

impl<'src> Parser<'src> {
    fn parse_function(&mut self) -> Result<FnDecl, ParseError> {
        self.expect(Token::Fn)?;
        let name = self.parse_ident()?;
        let params = self.parse_params()?;
        let return_ty = if self.check(Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let effects = self.parse_effect_row()?;
        let body = self.parse_block()?;
        Ok(FnDecl { name, params, return_ty, effects, body })
    }
}
```

### 4.4 Initial Type System

Subset of full type system:
- Primitive types (i32, f64, bool, String)
- Function types (no effects yet)
- Simple generics
- Record types
- No effect rows (added in Phase 2)

### 4.5 Deliverables

- [ ] `blood check file.blood` — Type-check without compiling
- [ ] `blood parse file.blood` — Dump AST
- [ ] Basic error messages with source locations
- [ ] Test suite for lexer and parser

---

## 5. Phase 1: Core Language

### 5.1 Goals

- Complete type system (without effects)
- Basic code generation (LLVM)
- Hello World executable
- Simple I/O (hardcoded, not effect-based)

### 5.2 HIR Design

```rust
pub enum HirExpr {
    Literal(Literal),
    Var(VarId),
    Binary(BinOp, Box<HirExpr>, Box<HirExpr>),
    Call(Box<HirExpr>, Vec<HirExpr>),
    If(Box<HirExpr>, Box<HirExpr>, Option<Box<HirExpr>>),
    Match(Box<HirExpr>, Vec<MatchArm>),
    Block(Vec<HirStmt>, Option<Box<HirExpr>>),
    Lambda(Vec<Param>, Box<HirExpr>),
    // ... etc
}

pub struct HirFunction {
    pub name: Symbol,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub body: HirExpr,
    pub span: Span,
}
```

### 5.3 Type Inference

Bidirectional type checking:

```rust
impl TypeChecker {
    /// Check expression against expected type
    fn check(&mut self, expr: &HirExpr, expected: &Type) -> Result<(), TypeError> {
        match (expr, expected) {
            (HirExpr::Lambda(params, body), Type::Fn(param_tys, ret_ty)) => {
                // Check lambda body against return type
                for (param, ty) in params.iter().zip(param_tys) {
                    self.env.insert(param.name, ty.clone());
                }
                self.check(body, ret_ty)
            }
            _ => {
                let inferred = self.infer(expr)?;
                self.unify(&inferred, expected)
            }
        }
    }

    /// Infer expression type
    fn infer(&mut self, expr: &HirExpr) -> Result<Type, TypeError> {
        match expr {
            HirExpr::Literal(lit) => Ok(lit.ty()),
            HirExpr::Var(id) => self.env.lookup(*id),
            HirExpr::Binary(op, lhs, rhs) => {
                let lhs_ty = self.infer(lhs)?;
                let rhs_ty = self.infer(rhs)?;
                self.check_binary_op(*op, &lhs_ty, &rhs_ty)
            }
            // ... etc
        }
    }
}
```

### 5.4 LLVM Code Generation

```rust
impl<'ctx> CodeGen<'ctx> {
    fn compile_function(&mut self, func: &HirFunction) -> FunctionValue<'ctx> {
        let fn_type = self.llvm_fn_type(&func.params, &func.return_ty);
        let function = self.module.add_function(&func.name.as_str(), fn_type, None);

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        // Bind parameters
        for (i, param) in func.params.iter().enumerate() {
            let val = function.get_nth_param(i as u32).unwrap();
            self.locals.insert(param.name, val);
        }

        let result = self.compile_expr(&func.body);
        self.builder.build_return(Some(&result));

        function
    }
}
```

### 5.5 Deliverables

- [ ] `blood build file.blood` — Compile to executable
- [ ] `blood run file.blood` — Compile and run
- [ ] Hello World works
- [ ] Basic arithmetic and control flow
- [ ] Function calls (no closures yet)

---

## 6. Phase 2: Effects System

### 6.1 Goals

- Effect declarations and operations
- Deep and shallow handlers
- Row polymorphism for effects
- Effect inference

### 6.2 Effect Representation

```rust
pub struct Effect {
    pub name: Symbol,
    pub operations: Vec<Operation>,
}

pub struct Operation {
    pub name: Symbol,
    pub params: Vec<Type>,
    pub return_ty: Type,
}

pub struct EffectRow {
    pub effects: Vec<EffectRef>,
    pub row_var: Option<TypeVar>,  // For polymorphism
}
```

### 6.3 Handler Compilation

Effect handlers compile to continuation-passing style:

```rust
// Source
with StateHandler { state: 0 } handle {
    let x = get()
    put(x + 1)
    get()
}

// Compiled (conceptually)
fn run_with_state<T>(initial: i32, body: impl Fn(&dyn StateOps) -> T) -> T {
    let mut state = initial;

    // CPS-transformed body
    body(&StateOps {
        get: || state,
        put: |s| { state = s; },
    })
}
```

### 6.4 Continuation Capture

For deep handlers that capture continuations:

```rust
struct Continuation<'a> {
    stack: Vec<StackFrame>,
    registers: SavedRegisters,
    handler_chain: Vec<HandlerFrame>,
    generation_snapshot: GenerationSnapshot,  // For Phase 3
}
```

### 6.5 Deliverables

- [ ] Effect declarations work
- [ ] Handler syntax works
- [ ] `perform` operations work
- [ ] `resume` works in handlers
- [ ] Effect polymorphism works
- [ ] Standard effects: State, Error, IO

---

## 7. Phase 3: Memory Model

### 7.1 Goals

- 128-bit generational references
- Tier 0/1/2 memory hierarchy
- Escape analysis
- Generation snapshots for effects
- StaleReference effect

### 7.2 Pointer Representation

```rust
#[repr(C, align(16))]
pub struct BloodPtr {
    address: *mut u8,
    generation: u32,
    metadata: u32,
}

impl BloodPtr {
    pub fn deref<T>(&self) -> Result<&T, StaleReference> {
        let slot = unsafe { &*(self.address as *const Slot) };
        if self.generation != slot.generation {
            return Err(StaleReference {
                expected: self.generation,
                actual: slot.generation,
            });
        }
        Ok(unsafe { &*(slot.value as *const T) })
    }
}
```

### 7.3 Escape Analysis

```rust
impl EscapeAnalyzer {
    fn analyze(&mut self, func: &MirFunction) -> EscapeResults {
        let mut states = HashMap::new();

        for alloc in func.allocations() {
            states.insert(alloc.id, EscapeState::NoEscape);
        }

        // Iterate to fixed point
        loop {
            let mut changed = false;

            for block in func.blocks() {
                for stmt in block.statements() {
                    changed |= self.analyze_statement(stmt, &mut states);
                }
            }

            if !changed { break; }
        }

        states
    }
}
```

### 7.4 Generation Snapshot

```rust
impl ContinuationCapture {
    fn capture_snapshot(&self, env: &Environment) -> GenerationSnapshot {
        let mut entries = Vec::new();

        for binding in env.bindings() {
            if binding.ty.contains_genref() {
                for genref in extract_genrefs(&binding.value) {
                    entries.push((genref.address, genref.generation));
                }
            }
        }

        GenerationSnapshot { entries }
    }
}
```

### 7.5 Deliverables

- [ ] Generational references work
- [ ] Tier 0 (stack) allocation works
- [ ] Tier 1 (region) allocation works
- [ ] Tier 2 (persistent) allocation works
- [ ] Escape analysis optimizes allocations
- [ ] StaleReference detected correctly
- [ ] Generation snapshots captured and validated

---

## 8. Phase 4: Content Addressing

### 8.1 Goals

- AST canonicalization
- BLAKE3 hashing
- Codebase database
- Name-to-hash mappings
- VFT implementation

### 8.2 Canonicalization

```rust
impl Canonicalizer {
    fn canonicalize(&mut self, def: &Definition) -> CanonicalAST {
        // 1. Resolve all references to hashes
        let resolved = self.resolve_references(def);

        // 2. Convert local names to de Bruijn indices
        let debruijn = self.to_debruijn(resolved);

        // 3. Normalize types and effects
        let normalized = self.normalize(debruijn);

        // 4. Strip metadata
        self.strip_metadata(normalized)
    }
}
```

### 8.3 Hash Computation

```rust
fn compute_hash(canonical: &CanonicalAST) -> Hash {
    let mut hasher = Blake3::new();

    // Version prefix for future compatibility
    hasher.update(&[FORMAT_VERSION]);

    // Serialize and hash
    serialize_canonical(canonical, &mut hasher);

    Hash(hasher.finalize().into())
}
```

### 8.4 Codebase Storage

```rust
pub struct Codebase {
    db: sled::Db,
    terms: Tree,      // Hash -> CanonicalAST
    types: Tree,      // Hash -> Type
    names: Tree,      // Name -> Hash
    metadata: Tree,   // Hash -> Metadata
}

impl Codebase {
    fn add(&self, def: Definition) -> Hash {
        let canonical = canonicalize(&def);
        let hash = compute_hash(&canonical);

        self.terms.insert(hash.as_bytes(), serialize(&canonical))?;
        self.names.insert(def.name.as_bytes(), hash.as_bytes())?;

        hash
    }
}
```

### 8.5 Deliverables

- [ ] AST canonicalization works
- [ ] Hashes are deterministic
- [ ] Codebase storage works
- [ ] Name resolution via hashes
- [ ] Incremental compilation
- [ ] `blood` codebase manager CLI

---

## 9. Phase 5: Runtime

### 9.1 Goals

- Fiber scheduler
- Concurrency primitives
- I/O runtime
- FFI support
- Standard library

### 9.2 Fiber Implementation

```rust
pub struct Fiber {
    id: FiberId,
    stack: Stack,
    state: FiberState,
    context: Context,
}

pub struct Scheduler {
    workers: Vec<Worker>,
    global_queue: ConcurrentQueue<FiberId>,
    fibers: DashMap<FiberId, Fiber>,
}

impl Scheduler {
    fn run(&self) {
        // Spawn worker threads
        for worker in &self.workers {
            thread::spawn(|| worker.run_loop());
        }
    }
}

impl Worker {
    fn run_loop(&self) {
        loop {
            let fiber_id = self.get_next_fiber();
            let result = self.run_fiber(fiber_id);

            match result {
                FiberResult::Yielded => self.reschedule(fiber_id),
                FiberResult::Suspended(cond) => self.suspend(fiber_id, cond),
                FiberResult::Completed(val) => self.complete(fiber_id, val),
            }
        }
    }
}
```

### 9.3 I/O Integration

```rust
pub struct IoReactor {
    #[cfg(target_os = "linux")]
    driver: IoUring,

    #[cfg(target_os = "macos")]
    driver: Kqueue,

    #[cfg(target_os = "windows")]
    driver: Iocp,
}

impl IoReactor {
    fn submit(&self, op: IoOp) -> IoFuture { ... }
    fn poll(&self) -> Vec<IoCompletion> { ... }
}
```

### 9.4 FFI Runtime

```rust
pub fn ffi_call(
    symbol: &str,
    args: &[FfiValue],
    return_ty: &FfiType,
) -> FfiResult {
    // 1. Look up symbol
    let ptr = dlsym(symbol)?;

    // 2. Build argument buffer
    let arg_buf = marshal_args(args);

    // 3. Call using libffi
    let result = libffi::call(ptr, &arg_buf, return_ty);

    // 4. Unmarshal result
    unmarshal_result(result, return_ty)
}
```

### 9.5 Deliverables

- [ ] Fiber scheduler works
- [ ] Channels work
- [ ] Async I/O works
- [ ] FFI calls work
- [ ] Core standard library complete
- [ ] Benchmarks show competitive performance

---

## 10. Phase 6: Self-Hosting

### 10.1 Goals

- Rewrite compiler in Blood
- Compiler compiles itself
- Maintain feature parity with Rust bootstrap

### 10.2 Strategy

1. **Parallel Development** — Blood compiler alongside Rust compiler
2. **Feature Flags** — Enable Blood self-compilation incrementally
3. **Validation** — Bootstrap produces identical output
4. **Cutover** — Switch primary development to Blood

### 10.3 Milestones

1. Blood lexer in Blood
2. Blood parser in Blood
3. Blood type checker in Blood
4. Blood code generator in Blood
5. Full compiler in Blood
6. Rust bootstrap deprecated

### 10.4 Validation

```bash
# Stage 0: Rust compiler compiles Blood source
$ rustc bloodc -> bloodc_stage0

# Stage 1: Stage 0 compiler compiles Blood source
$ ./bloodc_stage0 blood-compiler.blood -> bloodc_stage1

# Stage 2: Stage 1 compiler compiles Blood source
$ ./bloodc_stage1 blood-compiler.blood -> bloodc_stage2

# Validation: Stage 1 and Stage 2 must be identical
$ diff bloodc_stage1 bloodc_stage2
# (no output = success)
```

---

## 11. Milestones

### 11.1 Milestone Summary

| Milestone | Description | Completion Criteria |
|-----------|-------------|---------------------|
| **M0** | Parser works | Can parse full grammar |
| **M1** | Type checker works | Can type-check standard library |
| **M2** | Hello World | Can compile and run simple programs |
| **M3** | Effects work | Effect handlers compile and run |
| **M4** | Memory safe | Generational references work |
| **M5** | Content addressed | Codebase manager functional |
| **M6** | Concurrent | Fibers and scheduler work |
| **M7** | Self-hosting | Compiler compiles itself |
| **M8** | Production ready | Stable API, documentation complete |

### 11.2 Quality Gates

Each milestone must pass:

- [ ] All existing tests pass
- [ ] New tests for milestone features
- [ ] Documentation updated
- [ ] No regressions in compile time (>10% increase requires justification)
- [ ] No regressions in runtime performance (>10% increase requires justification)

---

## 12. Testing Strategy

### 12.1 Test Categories

| Category | Description | Location |
|----------|-------------|----------|
| **Unit** | Individual function tests | `tests/unit/` |
| **Integration** | Full compilation tests | `tests/integration/` |
| **UI** | Error message tests | `tests/ui/` |
| **Conformance** | Spec compliance | `tests/conformance/` |
| **Performance** | Benchmark suite | `benches/` |
| **Fuzz** | Fuzzing inputs | `fuzz/` |

### 12.2 Test Infrastructure

```bash
# Run all tests
$ cargo test

# Run specific test category
$ cargo test --test integration

# Run UI tests with blessing
$ cargo test --test ui -- --bless

# Run benchmarks
$ cargo bench

# Run fuzzer
$ cargo fuzz run lexer
```

### 12.3 Continuous Integration

```yaml
# .github/workflows/ci.yml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]

    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --all
      - run: cargo test --all
      - run: cargo clippy -- -D warnings
```

---

## 13. Open Questions

### 13.1 Technical Questions

| Question | Options | Status |
|----------|---------|--------|
| **Incremental compilation granularity** | Function / Module / File | Under discussion |
| **Effect compilation strategy** | CPS / Stack copying / Delimited | Under discussion |
| **Fiber stack size** | Fixed / Growable / Segmented | Growable (decided) |
| **WASM backend priority** | Phase 5 / Phase 7 / Never | Under discussion |

### 13.2 Ecosystem Questions

| Question | Options | Status |
|----------|---------|--------|
| **Package manager** | Cargo-like / UCM-integrated / Nix-style | UCM-integrated (tentative) |
| **Documentation tool** | Custom / Rustdoc-like | Under discussion |
| **Build system** | Custom / Cargo-like | Custom (tentative) |

### 13.3 Research Needed

- **Effect compilation efficiency** — How to minimize continuation allocation?
- **Generation check elision** — How aggressive can escape analysis be?
- **Hot-swap mechanics** — How to handle in-flight requests during swap?
- **Distributed codebase** — How to sync codebases across machines?

---

## Appendix A: Technology Stack

### A.1 Rust Dependencies

| Crate | Purpose |
|-------|---------|
| `inkwell` | LLVM bindings |
| `salsa` | Incremental computation |
| `chumsky` | Parser combinator (backup) |
| `ariadne` | Error rendering |
| `logos` | Fast lexer |
| `sled` | Codebase database |
| `blake3` | Content hashing |
| `crossbeam` | Concurrency primitives |
| `dashmap` | Concurrent hashmap |

### A.2 External Dependencies

| Tool | Purpose |
|------|---------|
| LLVM 17+ | Backend |
| lld | Linker (optional) |
| libffi | FFI calls |

---

## Appendix B: Contributing

### B.1 Good First Issues

- Parser error recovery improvements
- Additional error messages
- Test case additions
- Documentation improvements
- Benchmark additions

### B.2 Development Setup

```bash
# Clone repository
$ git clone https://github.com/blood-lang/blood
$ cd blood

# Install Rust
$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install LLVM
$ brew install llvm@17          # macOS
$ apt install llvm-17-dev       # Ubuntu

# Build
$ cargo build

# Test
$ cargo test
```

---

## Appendix C: References

Implementation approach informed by:

- [Crafting Interpreters](https://craftinginterpreters.com/)
- [Rust Compiler Development Guide](https://rustc-dev-guide.rust-lang.org/)
- [LLVM Tutorial](https://llvm.org/docs/tutorial/)
- [Writing an Interpreter in Go](https://interpreterbook.com/)
- [Koka Compiler](https://github.com/koka-lang/koka)

---

*This document is part of the Blood Language Specification.*
