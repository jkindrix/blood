# Blood Package Manifest Specification

**Version**: 0.1.0
**Status**: Draft
**Format**: TOML

## Overview

The Blood package manifest (`Blood.toml`) defines package metadata, dependencies, build configuration, and effect declarations for Blood projects. This specification draws inspiration from Cargo (Rust), with Blood-specific extensions for effects and content-addressed modules.

## File Structure

```
my_package/
├── Blood.toml          # Package manifest
├── Blood.lock          # Resolved dependency versions (auto-generated)
├── src/
│   ├── lib.blood       # Library root (for libraries)
│   └── main.blood      # Binary root (for applications)
├── tests/
│   └── *.blood         # Integration tests
├── benches/
│   └── *.blood         # Benchmarks
└── examples/
    └── *.blood         # Example programs
```

## Manifest Format

### Package Section

The `[package]` section contains metadata about the package itself.

```toml
[package]
name = "my_package"
version = "0.1.0"
edition = "2026"
description = "A brief description of the package"
documentation = "https://docs.example.com/my_package"
readme = "README.md"
homepage = "https://example.com"
repository = "https://github.com/user/my_package"
license = "MIT OR Apache-2.0"
license-file = "LICENSE"
keywords = ["effects", "systems", "embedded"]
categories = ["development-tools", "network-programming"]
authors = ["Author Name <author@example.com>"]
exclude = ["tests/fixtures/*"]
include = ["src/**/*", "Blood.toml"]
publish = true
```

#### Required Fields

| Field | Description |
|-------|-------------|
| `name` | Package name (lowercase, hyphens allowed, must start with letter) |
| `version` | Semantic version (MAJOR.MINOR.PATCH) |

#### Optional Fields

| Field | Description | Default |
|-------|-------------|---------|
| `edition` | Blood language edition | `"2026"` |
| `description` | Short package description | None |
| `documentation` | Documentation URL | None |
| `readme` | Path to README file | `"README.md"` |
| `homepage` | Project homepage URL | None |
| `repository` | Source repository URL | None |
| `license` | SPDX license expression | None |
| `license-file` | Path to license file | None |
| `keywords` | Search keywords (max 5) | `[]` |
| `categories` | Registry categories (max 5) | `[]` |
| `authors` | Package authors | `[]` |
| `exclude` | Patterns to exclude from publishing | `[]` |
| `include` | Patterns to include when publishing | `[]` |
| `publish` | Whether package can be published | `true` |

### Dependency Section

The `[dependencies]` section declares runtime dependencies.

```toml
[dependencies]
# Version from registry
json = "1.2.3"

# Version with constraints
http = ">=1.0, <2.0"

# Git dependency
logger = { git = "https://github.com/blood-lang/logger", tag = "v1.0.0" }

# Git with specific branch or revision
experimental = { git = "https://github.com/user/exp", branch = "main" }
pinned = { git = "https://github.com/user/pinned", rev = "a1b2c3d4" }

# Path dependency (local development)
my-utils = { path = "../my-utils" }

# Content-addressed dependency (Blood-specific)
verified = { hash = "blood:sha256:abc123...", version = "1.0.0" }

# Optional dependency
optional-feature = { version = "1.0.0", optional = true }

# Dependency with features
serde = { version = "2.0.0", features = ["derive", "json"] }

# Dependency without default features
minimal = { version = "1.0.0", default-features = false }
```

#### Version Constraints

| Syntax | Meaning |
|--------|---------|
| `"1.2.3"` | Exactly version 1.2.3 |
| `"^1.2.3"` | Compatible with 1.2.3 (>=1.2.3, <2.0.0) |
| `"~1.2.3"` | Approximately 1.2.3 (>=1.2.3, <1.3.0) |
| `">=1.0, <2.0"` | Range constraint |
| `"*"` | Any version (not recommended) |

#### Content-Addressed Dependencies

Blood supports content-addressed dependencies, where the dependency is identified by the hash of its compiled module:

```toml
[dependencies]
# Hash ensures exact reproducibility
core-utils = { hash = "blood:sha256:def456...", version = "1.5.0" }
```

The hash is computed from the module's source content, ensuring builds are reproducible regardless of when or where they occur.

### Development Dependencies

The `[dev-dependencies]` section declares dependencies only needed for development and testing.

```toml
[dev-dependencies]
test-framework = "1.0.0"
benchmark = "0.5.0"
mock-server = { git = "https://github.com/blood-lang/mock-server" }
```

### Build Dependencies

The `[build-dependencies]` section declares dependencies for build scripts.

```toml
[build-dependencies]
code-gen = "0.2.0"
```

### Target-Specific Dependencies

Dependencies can be declared for specific targets:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
linux-sys = "1.0.0"

[target.'cfg(target_arch = "arm")'.dependencies]
arm-intrinsics = "0.1.0"

[target.'cfg(all(target_os = "linux", target_arch = "x86_64"))'.dependencies]
simd-ops = "1.0.0"
```

### Effects Section (Blood-Specific)

The `[effects]` section declares effect capabilities required or provided by the package.

```toml
[effects]
# Effects this package requires (must be handled by users)
requires = ["IO", "Net", "FileSystem"]

# Effects this package provides handlers for
provides = ["Cache", "Logger", "Retry"]

# Effect re-exports (effects from dependencies exposed to users)
re-exports = ["json.ParseError"]
```

#### Effect Requirements

When a package declares required effects, users must provide handlers:

```toml
# In library Blood.toml
[effects]
requires = ["Database"]

# User must handle Database effect when using this library
```

#### Effect Handlers

Packages providing handlers should declare them:

```toml
[effects]
provides = ["InMemoryCache", "RedisCache"]

# Documentation for provided handlers
[effects.handlers.InMemoryCache]
description = "In-memory LRU cache with configurable size"
effect = "Cache"

[effects.handlers.RedisCache]
description = "Redis-backed distributed cache"
effect = "Cache"
requires = ["Net"]
```

### Features Section

The `[features]` section defines conditional compilation features.

```toml
[features]
# Default features enabled when package is used
default = ["std", "logging"]

# Individual features
std = []
logging = ["dep:logger"]
async = ["dep:async-runtime"]
json = ["dep:json-parser"]
full = ["logging", "async", "json"]

# Feature enables optional dependency
derive = ["dep:derive-macro"]
```

#### Feature-Gated Dependencies

```toml
[dependencies]
logger = { version = "1.0.0", optional = true }

[features]
logging = ["dep:logger"]
```

### Targets Section

The `[[bin]]`, `[[lib]]`, `[[test]]`, `[[bench]]`, and `[[example]]` sections define build targets.

#### Library Target

```toml
[lib]
name = "my_package"
path = "src/lib.blood"
```

#### Binary Targets

```toml
[[bin]]
name = "my_app"
path = "src/main.blood"

[[bin]]
name = "my_tool"
path = "src/bin/tool.blood"
required-features = ["cli"]
```

#### Test Targets

```toml
[[test]]
name = "integration"
path = "tests/integration.blood"

[[test]]
name = "effects"
path = "tests/effects.blood"
required-features = ["test-utils"]
```

#### Benchmark Targets

```toml
[[bench]]
name = "performance"
path = "benches/perf.blood"
harness = false  # Custom benchmark harness
```

#### Example Targets

```toml
[[example]]
name = "basic"
path = "examples/basic.blood"

[[example]]
name = "advanced"
path = "examples/advanced.blood"
required-features = ["full"]
```

### Profile Section

The `[profile]` sections configure compilation profiles.

```toml
[profile.dev]
opt-level = 0
debug = true
generation-checks = true
overflow-checks = true
lto = false

[profile.release]
opt-level = 3
debug = false
generation-checks = false  # Disable for performance
overflow-checks = false
lto = true
strip = true

[profile.test]
opt-level = 1
debug = true
generation-checks = true

[profile.bench]
opt-level = 3
debug = true  # For profiling
generation-checks = false
```

#### Profile Options

| Option | Description | Values |
|--------|-------------|--------|
| `opt-level` | Optimization level | `0`, `1`, `2`, `3`, `"s"`, `"z"` |
| `debug` | Include debug symbols | `true`, `false` |
| `generation-checks` | Enable generation safety checks | `true`, `false` |
| `overflow-checks` | Enable integer overflow checks | `true`, `false` |
| `lto` | Link-time optimization | `true`, `false`, `"thin"` |
| `strip` | Strip symbols from binary | `true`, `false` |
| `codegen-units` | Parallel codegen units | `1`-`256` |

### Workspace Section

For multi-package projects, the `[workspace]` section defines shared configuration.

```toml
[workspace]
members = [
    "packages/core",
    "packages/cli",
    "packages/gui",
]
exclude = ["experimental/*"]

# Shared dependencies across workspace
[workspace.dependencies]
json = "1.2.3"
logger = { version = "2.0.0", features = ["color"] }

# Workspace metadata
[workspace.package]
authors = ["Team <team@example.com>"]
repository = "https://github.com/org/monorepo"
license = "MIT"
```

Member packages can inherit workspace settings:

```toml
# In packages/core/Blood.toml
[package]
name = "core"
version.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
json.workspace = true
```

### Metadata Section

The `[package.metadata]` section stores tool-specific configuration.

```toml
[package.metadata.docs]
# Documentation generation settings
features = ["full"]
default-target = "x86_64-unknown-linux"
targets = ["x86_64-unknown-linux", "aarch64-apple-darwin"]

[package.metadata.blood-fmt]
# Formatter settings
max-width = 100
tab-size = 4

[package.metadata.blood-lint]
# Linter settings
deny = ["unsafe-ffi", "missing-effect-bounds"]
warn = ["unused-variables"]
```

## Lock File Format

The `Blood.lock` file records exact resolved versions. This file should be committed for applications but not for libraries.

```toml
# Blood.lock - auto-generated, do not edit manually

[[package]]
name = "my_package"
version = "0.1.0"
source = "registry+https://blood-lang.org/packages"
checksum = "sha256:abc123..."

[[package]]
name = "json"
version = "1.2.3"
source = "registry+https://blood-lang.org/packages"
checksum = "sha256:def456..."
dependencies = [
    "unicode 1.0.0",
]

[[package]]
name = "logger"
version = "2.0.0"
source = "git+https://github.com/blood-lang/logger?tag=v2.0.0#a1b2c3d4"
dependencies = [
    "time 0.5.0",
]
```

## Complete Example

```toml
[package]
name = "web-server"
version = "0.1.0"
edition = "2026"
description = "A high-performance web server with algebraic effects"
authors = ["Jane Developer <jane@example.com>"]
license = "MIT"
repository = "https://github.com/jane/web-server"
keywords = ["web", "server", "effects", "async"]
categories = ["web-programming", "network-programming"]

[dependencies]
http = "1.0.0"
json = { version = "1.2.3", features = ["derive"] }
logger = { version = "2.0.0", optional = true }
router = { git = "https://github.com/blood-lang/router", tag = "v0.5.0" }

[dev-dependencies]
test-server = "0.1.0"
mock-http = "1.0.0"

[features]
default = ["logging"]
logging = ["dep:logger"]
tls = ["dep:tls-native"]

[effects]
requires = ["IO", "Net"]
provides = ["HttpServer", "Router", "Middleware"]

[effects.handlers.HttpServer]
description = "HTTP/1.1 and HTTP/2 server handler"
effect = "Net"

[lib]
name = "web_server"
path = "src/lib.blood"

[[bin]]
name = "server"
path = "src/main.blood"

[[example]]
name = "hello"
path = "examples/hello.blood"

[[example]]
name = "rest-api"
path = "examples/rest-api.blood"
required-features = ["logging"]

[[bench]]
name = "throughput"
path = "benches/throughput.blood"

[profile.release]
opt-level = 3
lto = true
generation-checks = false
```

## Command-Line Interface

The `blood` command-line tool interacts with the manifest:

```bash
# Initialize new package
blood new my-package
blood new --lib my-library

# Build package
blood build
blood build --release
blood build --features "logging,tls"

# Run binary
blood run
blood run --bin my-tool

# Test package
blood test
blood test --test integration

# Run benchmarks
blood bench
blood bench --bench throughput

# Run examples
blood run --example hello

# Manage dependencies
blood add json@1.2.3
blood add logger --features color
blood add --dev test-framework
blood remove unused-dep
blood update

# Publish package
blood publish
blood publish --dry-run

# Generate documentation
blood doc
blood doc --open
```

## Future Extensions

### Planned Features

1. **Build Scripts**: Custom build logic in `build.blood`
2. **Proc Macros**: Procedural macro packages
3. **Native Dependencies**: C library linking configuration
4. **Cross-Compilation**: Target specification
5. **Artifact Dependencies**: Binary distribution

### Reserved Fields

The following fields are reserved for future use:

- `[package.build]`
- `[package.links]`
- `[package.resolver]`
- `[target.*.build-dependencies]`

## Version History

| Version | Changes |
|---------|---------|
| 0.1.0 | Initial specification |

---

*This specification is subject to change as Blood's package ecosystem matures.*
