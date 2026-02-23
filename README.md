# Blood

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

**A systems programming language for environments where failure is not an option.**

Blood synthesizes five cutting-edge programming language innovations:

- **Content-Addressed Code** (Unison) — Code identity via BLAKE3-256 hashes
- **Generational Memory Safety** (Vale) — 128-bit fat pointers, no GC
- **Hybrid Ownership Model** (Hylo + Rust + Vale) — Move semantics with escape-analyzed allocation, no borrow checker
- **Algebraic Effects** (Koka) — All side effects typed and composable
- **Multiple Dispatch** (Julia) — Type-stable open extensibility

## Status

> **Version: 0.2.0**

Core compiler is functional and tested. Programs compile and run with full type checking, effect tracking, and generational memory safety. Bootstrap compiler passes 2,047 unit tests; self-hosted compiler passes 271/317 ground-truth integration tests. See [IMPLEMENTATION_STATUS.md](docs/spec/IMPLEMENTATION_STATUS.md) for detailed component status.

| Component | Status | Details |
|-----------|--------|---------|
| Lexer & Parser | ✅ Complete | Production-tested |
| Type Checker | ✅ Complete | Bidirectional + unification |
| Code Generation | ✅ Complete | LLVM backend |
| Effects System | 🔶 Partial | Bootstrap: evidence passing inline; self-hosted: runtime ABI in progress |
| Memory Model | ✅ Integrated | Generational pointers in codegen (blood_alloc/blood_free) |
| Runtime | ✅ Integrated | Memory management, FFI exports linked to programs |
| Multiple Dispatch | 🔶 Partial | Compile-time dispatch complete; runtime dispatch in progress |
| Closures | 🔶 Partial | Bootstrap: environment capture and codegen; self-hosted: parser support pending |

**Legend**: ✅ = Implemented and integrated | 🔶 = Partially integrated

**[Getting Started](docs/spec/GETTING_STARTED.md)** | [Specification](docs/spec/SPECIFICATION.md) | [Implementation Status](docs/spec/IMPLEMENTATION_STATUS.md)

## The Name

In engineering, regulations "written in blood" are those born from catastrophic failures — rules that exist because someone died or systems failed in ways that can never be allowed again.

Blood is for avionics, medical devices, financial infrastructure, autonomous vehicles, nuclear control systems. Systems where failure is not an option.

## Design Principles

1. **No Hidden Costs** — Every abstraction has predictable, visible cost
2. **Failure is Data** — All errors tracked in the type system via effects
3. **Zero-Cost When Provable** — Compile-time proofs eliminate runtime checks
4. **Effects are Universal** — IO, state, exceptions, async — one unified mechanism
5. **Interop is First-Class** — C FFI designed from day one

## Repository Structure

This is a **monorepo** containing both the Blood language project and the Rust bootstrap compiler.

```
blood/
├── blood-std/              # Standard library (Blood source)
│   └── std/
│       ├── compiler/       # Self-hosted compiler (written in Blood)
│       ├── core/           # Core types (Option, String, Box, etc.)
│       ├── collections/    # Vec, HashMap, LinkedList, etc.
│       ├── effects/        # Effect system primitives
│       ├── sync/           # Concurrency primitives
│       └── ...
├── src/bootstrap/          # Rust bootstrap compiler (git subtree)
│   ├── bloodc/src/         # Compiler source (Rust)
│   ├── runtime/            # C runtime library
│   ├── blood-std/          # Stdlib copy for compiler tests
│   └── Cargo.toml          # Workspace manifest
├── docs/                   # Language specification & documentation
│   ├── spec/               # Core specs (SPECIFICATION, MEMORY_MODEL, etc.)
│   ├── comparisons/        # Blood vs other languages
│   └── postmortem/         # Bug investigation records
├── examples/               # Blood language examples
└── editors/                # Editor support (VS Code, etc.)
```

See [`src/bootstrap/README.md`](src/bootstrap/README.md) for Rust-compiler-specific details.

## Quick Example

```blood
effect Error<E> {
    op raise(err: E) -> never
}

effect IO {
    op read_file(path: Path) -> Bytes
}

fn load_config(path: Path) -> Config / {IO, Error<ParseError>} {
    let data = read_file(path)
    parse_config(data)
}

fn main() / {IO, Error<AppError>} {
    let config = with ParseErrorHandler handle {
        load_config("config.toml")
    }
    run_app(config)
}
```

## Quick Start

```bash
# Build the bootstrap compiler
cd src/bootstrap
cargo build --release

# Compile and run a program
cargo run -- run examples/fizzbuzz.blood

# Run the test suite
cargo test --workspace
```

See **[GETTING_STARTED.md](docs/spec/GETTING_STARTED.md)** for the full tutorial.

## Documentation

### Core Specifications

| Document | Description |
|----------|-------------|
| [SPECIFICATION.md](docs/spec/SPECIFICATION.md) | Core language specification |
| [MEMORY_MODEL.md](docs/spec/MEMORY_MODEL.md) | Synthetic Safety Model (generational references) |
| [DISPATCH.md](docs/spec/DISPATCH.md) | Multiple dispatch and type stability |
| [GRAMMAR.md](docs/spec/GRAMMAR.md) | Complete surface syntax grammar |
| [FORMAL_SEMANTICS.md](docs/spec/FORMAL_SEMANTICS.md) | Operational semantics and type rules |

### System Specifications

| Document | Description |
|----------|-------------|
| [CONTENT_ADDRESSED.md](docs/spec/CONTENT_ADDRESSED.md) | Content-addressed storage and VFT |
| [CONCURRENCY.md](docs/spec/CONCURRENCY.md) | Fiber model and scheduler |
| [FFI.md](docs/spec/FFI.md) | Foreign function interface |
| [STDLIB.md](docs/spec/STDLIB.md) | Standard library design |
| [DIAGNOSTICS.md](docs/spec/DIAGNOSTICS.md) | Error messages and diagnostics |
| [UCM.md](docs/spec/UCM.md) | Codebase Manager (tooling) |

### Planning & Status

| Document | Description |
|----------|-------------|
| [GETTING_STARTED.md](docs/spec/GETTING_STARTED.md) | Tutorial and quick start guide |
| [ROADMAP.md](docs/spec/ROADMAP.md) | Implementation plan and milestones |
| [DECISIONS.md](docs/spec/DECISIONS.md) | Architecture decision records |
| [IMPLEMENTATION_STATUS.md](docs/spec/IMPLEMENTATION_STATUS.md) | Detailed implementation audit |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contributing

We welcome contributions! See the [implementation status](docs/spec/IMPLEMENTATION_STATUS.md) for areas that need work.

- **Bug reports**: Open an issue with reproduction steps
- **Feature requests**: Open a discussion first
- **Code contributions**: Fork, branch, and submit a PR

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
