# Blood Bootstrap Compiler

The Rust implementation of the Blood compiler. This is the bootstrap compiler — it compiles the [self-hosted compiler](../selfhost/) written in Blood, which then compiles itself.

## Role in the Bootstrap Chain

```
Rust source (this directory)
    → cargo build → blood (bootstrap binary)
        → compiles src/selfhost/ → first_gen
            → compiles src/selfhost/ → second_gen
                → compiles src/selfhost/ → third_gen (must be byte-identical to second_gen)
```

The bootstrap compiler defines language semantics. If blood-rust has a bug, the self-hosted compiler will be miscompiled. See [DEVELOPMENT.md](../../DEVELOPMENT.md) for the Bootstrap Gate protocol.

## Building

Requires Rust 1.77+ (stable) and LLVM 18.

```bash
cd src/bootstrap
cargo build --release
```

The compiler binary is produced at `target/release/blood`.

## Usage

```bash
# Check a Blood source file
./target/release/blood check ../../examples/hello.blood

# Build to executable
./target/release/blood build ../../examples/hello.blood

# Build and run
./target/release/blood run ../../examples/hello.blood
```

## Testing

```bash
# Run unit tests (~2,000 tests)
cargo test --workspace

# Run ground-truth integration tests (requires built compiler)
make ground-truth

# Full test suite (unit + ground-truth)
make test

# Full bootstrap cycle (build → compile self-hosted → self-compile)
make bootstrap
```

See the [Makefile](Makefile) for all available targets.

## Workspace Structure

```
src/bootstrap/
├── bloodc/              # Compiler crate (binary: blood)
│   └── src/
│       ├── main.rs      # CLI entry point
│       ├── lexer.rs     # Logos-based lexer
│       ├── parser/      # Recursive descent parser
│       ├── hir/         # High-level IR
│       ├── mir/         # Mid-level IR
│       ├── codegen/     # LLVM codegen (via Inkwell)
│       ├── effects/     # Effect system
│       └── content/     # Content-addressed storage
├── blood-runtime/       # Runtime library (memory, scheduler, FFI)
├── blood-tools/         # Tooling (fmt, lsp, ucm)
├── Cargo.toml           # Workspace manifest
└── Makefile             # Build and test targets
```

## Relationship to the Self-Hosted Compiler

| Aspect | Bootstrap (this) | Self-hosted |
|--------|------------------|-------------|
| Language | Rust | Blood |
| LLVM integration | Inkwell bindings (C API) | Text-based IR emission |
| Location | `src/bootstrap/` | `src/selfhost/` |
| Purpose | Compile the self-hosted compiler | Compile itself and user programs |

Both compilers must produce identical behavior. Mismatches are bugs — see [COMPILER_NOTES.md](../selfhost/COMPILER_NOTES.md) for known differences and [tools/FAILURE_LOG.md](../../tools/FAILURE_LOG.md) for bug history.

## Documentation

- [Root README](../../README.md) — Project overview and design principles
- [Language specification](../../docs/spec/SPECIFICATION.md) — Core language semantics
- [Grammar](../../docs/spec/GRAMMAR.md) — Surface syntax
- [Getting started](../../docs/guides/GETTING_STARTED.md) — Tutorial
