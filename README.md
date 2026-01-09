# Blood

**A systems programming language for environments where failure is not an option.**

Blood synthesizes five cutting-edge programming language innovations:

- **Content-Addressed Code** (Unison) — Code identity via BLAKE3-256 hashes
- **Generational Memory Safety** (Vale) — 128-bit fat pointers, no GC
- **Mutable Value Semantics** (Hylo) — Simple ownership without borrow checker complexity
- **Algebraic Effects** (Koka) — All side effects typed and composable
- **Multiple Dispatch** (Julia) — Type-stable open extensibility

## Status

**Pre-implementation design phase.** We are specifying the language formally before writing the first line of compiler code.

See [SPECIFICATION.md](./SPECIFICATION.md) for the complete technical specification.

## The Name

In engineering, regulations "written in blood" are those born from catastrophic failures — rules that exist because someone died or systems failed in ways that can never be allowed again.

Blood is for avionics, medical devices, financial infrastructure, autonomous vehicles, nuclear control systems. Systems where failure is not an option.

## Design Principles

1. **No Hidden Costs** — Every abstraction has predictable, visible cost
2. **Failure is Data** — All errors tracked in the type system via effects
3. **Zero-Cost When Provable** — Compile-time proofs eliminate runtime checks
4. **Effects are Universal** — IO, state, exceptions, async — one unified mechanism
5. **Interop is First-Class** — C FFI designed from day one

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

## Documentation

### Core Specifications

| Document | Description |
|----------|-------------|
| [SPECIFICATION.md](./SPECIFICATION.md) | Core language specification |
| [MEMORY_MODEL.md](./MEMORY_MODEL.md) | Synthetic Safety Model (generational references) |
| [DISPATCH.md](./DISPATCH.md) | Multiple dispatch and type stability |
| [GRAMMAR.md](./GRAMMAR.md) | Complete surface syntax grammar |
| [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) | Operational semantics and type rules |

### System Specifications

| Document | Description |
|----------|-------------|
| [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) | Content-addressed storage and VFT |
| [CONCURRENCY.md](./CONCURRENCY.md) | Fiber model and scheduler |
| [FFI.md](./FFI.md) | Foreign function interface |
| [STDLIB.md](./STDLIB.md) | Standard library design |
| [DIAGNOSTICS.md](./DIAGNOSTICS.md) | Error messages and diagnostics |

### Planning

| Document | Description |
|----------|-------------|
| [ROADMAP.md](./ROADMAP.md) | Implementation plan and milestones |
| [DECISIONS.md](./DECISIONS.md) | Architecture decision records |

## License

To be determined.

## Contributing

Blood is in early design. We welcome feedback on the specification via issues and discussions.
