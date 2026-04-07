# Blood

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

**A systems programming language for environments where failure is not an option.**

Blood synthesizes five cutting-edge programming language innovations:

- **Content-Addressed Code** (Unison) — Code identity via BLAKE3-256 hashes
- **Generational Memory Safety** (Vale) — Fat pointer refs with generation tracking, no GC
- **Hybrid Ownership Model** (Hylo + Rust + Vale) — Mutable value semantics with escape-analyzed allocation, no borrow checker
- **Algebraic Effects** (Koka) — All side effects typed and composable
- **Multiple Dispatch** (Julia) — Type-stable open extensibility

## Status

> **Pre-release — research compiler.** No version tag yet. See [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md) for the honest status of each component.

The self-hosted compiler passes **533/534 golden tests** (1 XFAIL tracking a known closure-codegen bug). It compiles itself through a three-generation byte-identical bootstrap, and the resulting binary has no Rust runtime dependency — the seed is self-sufficient and requires only LLVM 18 on the host.

| Component | Status | Details |
|-----------|--------|---------|
| Lexer & Parser | ✅ Working | 90% of normative grammar claims verified in code |
| Type Inference | ✅ Working | Algorithm W, bidirectional, 100% of normative claims verified |
| Type Checking | ✅ Working | Linearity, effects, exhaustiveness, generic impls |
| Code Generation | ⚠️ Mostly working | LLVM IR emission. Known gaps: nested-closure codegen, aggregate escape analysis disabled |
| Effects System | ✅ Working | Deep/shallow handlers, perform/resume, snapshots, per-op resume |
| Memory Model | ⚠️ Mostly working | Generational refs for heap/region. Stale &str detection for String buffers currently disabled pending a latent bug fix |
| Runtime | ✅ Working | 181KB Blood-native runtime (no Rust). Memory, effects, VFT |
| Multiple Dispatch | ⚠️ Partial | Compile-time dispatch works. Runtime dispatch (fingerprint-based) is deferred |
| Fibers / Concurrency | ❌ Not integrated | pthread-based spawn; no M:N scheduler, no mutex/channel primitives wired |
| Safety Checks | ✅ Default | Definite init, linearity, bounds, dangling ref rejection all enabled |
| Content Addressing | 🔶 Partial | BLAKE3 hashing, codebase storage. VFT dispatch wiring not hooked up |
| Formal Proofs | ✅ Complete | 60 Coq theorems, 0 Admitted, 0 Axioms (covers a simplified model of the language, not the compiler artifact) |

**Legend**: ✅ Working | ⚠️ Mostly working with known gaps | 🔶 Partial | ❌ Not integrated

**Spec coverage (approx):** 39 of 78 surveyed normative claims across `docs/spec/*.md` have verifiable code evidence today (~50%). See [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md) for the full breakdown by spec file and the known soundness gaps that remain open.

[Specification](docs/spec/SPECIFICATION.md) | [Known Limitations](docs/KNOWN_LIMITATIONS.md) | [Contributing](CONTRIBUTING.md)

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
├── stdlib/                 # Standard library (Blood source)
│   ├── core/               # Core types (Option, String, Box, etc.)
│   ├── collections/        # Vec, HashMap, LinkedList, etc.
│   ├── effects/            # Effect system primitives
│   ├── sync/               # Concurrency primitives
│   └── ...
├── src/
│   ├── bootstrap/          # Rust bootstrap compiler (Rust)
│   │   ├── bloodc/src/     # Compiler source (Rust)
│   │   └── Cargo.toml      # Workspace manifest
│   └── selfhost/           # Self-hosted compiler (written in Blood)
├── docs/                   # Language specification & documentation
│   ├── spec/               # Core language specifications
│   ├── design/             # Design evaluations and decisions
│   ├── planning/           # Roadmaps, status, decisions
│   ├── internal/           # Compiler internals
│   └── ...
├── examples/               # Blood language examples
└── tools/                  # Development & debugging tools
```

See [`src/bootstrap/README.md`](src/bootstrap/README.md) for Rust-compiler-specific details.

## Quick Example

```blood
effect Error<E> {
    op raise(err: E) -> !
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

### Using the Self-Hosted Compiler (recommended)

```bash
# Build the compiler from the bootstrap seed (requires LLVM 18)
cd src/selfhost
./build_selfhost.sh build first_gen

# Compile and run a program
build/first_gen run ../../examples/fizzbuzz.blood

# Development workflow: edit source, rebuild incrementally, test
./build_selfhost.sh build second_gen    # Incremental self-compilation
./build_selfhost.sh test golden second_gen
```

### Using the Bootstrap Compiler (legacy, requires Rust)

```bash
cd src/bootstrap
cargo build --release
cargo run -- run ../../examples/fizzbuzz.blood
```

See the [Specification](docs/spec/SPECIFICATION.md) for language details.

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

### Planning & Status

| Document | Description |
|----------|-------------|
| [ROADMAP.md](docs/planning/ROADMAP.md) | Implementation plan and milestones |
| [DECISIONS.md](docs/planning/DECISIONS.md) | Architecture decision records |
| [IMPLEMENTATION_STATUS.md](docs/planning/IMPLEMENTATION_STATUS.md) | Detailed implementation audit |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contributing

We welcome contributions! See the [implementation status](docs/planning/IMPLEMENTATION_STATUS.md) for areas that need work.

- **Bug reports**: Open an issue with reproduction steps
- **Feature requests**: Open a discussion first
- **Code contributions**: Fork, branch, and submit a PR

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
