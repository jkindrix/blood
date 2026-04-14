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

The self-hosted compiler passes **576/576 golden tests**. It compiles itself through a three-generation byte-identical bootstrap, and the resulting binary has no Rust runtime dependency — the seed is self-sufficient and requires only LLVM 18 on the host.

| Component | Status | Details |
|-----------|--------|---------|
| Lexer & Parser | ✅ Working | 90% of normative grammar claims verified in code |
| Type Inference | ✅ Working | Algorithm W, bidirectional, 100% of normative claims verified |
| Type Checking | ✅ Working | Linearity, effects, exhaustiveness, generic impls |
| Code Generation | ⚠️ Mostly working | LLVM IR emission. Known gap: aggregate escape analysis disabled |
| Effects System | ✅ Working | Deep/shallow handlers, perform/resume, snapshots, per-op resume |
| Memory Model | ⚠️ Mostly working | Generational refs for heap/region. `&str` stale detection disabled pending snapshot liveness analysis (GAP-1) |
| Runtime | ✅ Working | 197KB Blood-native runtime (no Rust). Memory, effects, VFT |
| Multiple Dispatch | ⚠️ Partial | Compile-time dispatch works. Runtime dispatch (fingerprint-based) is deferred |
| Fibers / Concurrency | ❌ Not integrated | pthread-based spawn; no M:N scheduler, no mutex/channel primitives wired |
| Safety Checks | ✅ Default | Definite init, linearity, bounds, dangling ref rejection all enabled |
| Content Addressing | 🔶 Partial | BLAKE3 hashing, codebase storage. VFT dispatch wiring not hooked up |
| Formal Proofs | ⚠️ Mostly complete | 273 Coq theorems/lemmas (219 proved, 14 Admitted, 0 Axioms). Covers a core calculus formalization, not the compiler artifact |

**Legend**: ✅ Working | ⚠️ Mostly working with known gaps | 🔶 Partial | ❌ Not integrated

**Spec coverage:** 7 of 16 spec files fully implemented and tested, 3 partially implemented (Concurrency, Diagnostics, Stdlib), 1 untested (WCET/Real-time). See [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md) for the full breakdown and known soundness gaps.

[Tutorial](docs/TUTORIAL.md) | [Specification](docs/spec/SPECIFICATION.md) | [Known Limitations](docs/KNOWN_LIMITATIONS.md) | [Contributing](CONTRIBUTING.md)

## The Name

In engineering, regulations "written in blood" are those born from catastrophic failures — rules that exist because someone died or systems failed in ways that can never be allowed again.

Blood is for avionics, medical devices, financial infrastructure, autonomous vehicles, nuclear control systems. Systems where failure is not an option.

## Why Blood for Safety-Critical Systems?

If you're an avionics, medical, or financial engineer evaluating alternatives to C and Rust:

- **Algebraic effects** — every function's I/O, state mutations, and failure modes appear in its type signature. Auditors can mechanically verify which subsystems touch hardware or network — no grep, no manual review.
- **Content-addressed code** — builds are reproducible by construction. The same source hash always produces the same binary, satisfying DO-178C and IEC 62443 traceability requirements without a separate build-provenance toolchain.
- **No borrow checker** — domain experts write correct systems code without a PL PhD. Generational references enforce memory safety at runtime with deterministic, bounded overhead.
- **Generational memory safety** — no GC pauses, no use-after-free, no dangling pointers. Safety without the Rust learning curve or lifetime annotation burden.
- **Multiple dispatch** — extend a safety-critical codebase by adding implementations, not by modifying certified modules. Open/closed principle enforced at the language level.

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
| [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md) | Honest gap enumeration (current) |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contributing

We welcome contributions! See [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md) for the current state and [CONTRIBUTING.md](CONTRIBUTING.md) for areas that need help.

- **Bug reports**: Open an issue with reproduction steps
- **Feature requests**: Open a discussion first
- **Code contributions**: Fork, branch, and submit a PR

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
