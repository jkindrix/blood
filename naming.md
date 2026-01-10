# Blood Programming Language: Complete Toolchain & Naming Reference

*A comprehensive, research-backed guide to naming tools for the Blood programming language*

**Document Version:** 2.0  
**Last Updated:** January 2026  
**Status:** Ready for Implementation

---

## Executive Summary

This document provides complete naming recommendations for the Blood programming language toolchain, including conflict analysis, CLI mockups, stdlib naming conventions, and rationale for each decision. All recommendations have been verified against existing tools and projects.

### Quick Reference: Recommended Toolchain

| Category | Tool Name | Command | Status |
|----------|-----------|---------|--------|
| Unified CLI | `blood` | `blood build/run/test` | ✅ Clear |
| Compiler | `bloodc` | `bloodc src/main.blood` | ✅ Clear |
| Package Manager | `vein` | `vein add/install/publish` | ✅ Clear |
| Formatter | `clot` | `clot fmt .` | ✅ Clear |
| Linter | `scan` | `scan --strict src/` | ✅ Clear |
| REPL | `beat` | `beat` | ✅ Clear |
| Language Server | `vessel-ls` | `vessel-ls --stdio` | ✅ Clear |
| Debugger | `draw` | `draw ./app` | ✅ Clear |
| Test Runner | `culture` | `culture run` | ✅ Clear |
| Doc Generator | `codex` | `codex build` | ✅ Clear |
| Package Registry | Bloodbank | `bloodbank.dev` | ✅ Clear |

---

## Table of Contents

1. [Language Overview](#1-language-overview)
2. [Thematic Foundation](#2-thematic-foundation)
3. [Conflict Analysis](#3-conflict-analysis)
4. [Tool-by-Tool Deep Dive](#4-tool-by-tool-deep-dive)
5. [CLI Design & Help Text Mockups](#5-cli-design--help-text-mockups)
6. [Standard Library Naming](#6-standard-library-naming)
7. [Package & Project Conventions](#7-package--project-conventions)
8. [File Extensions & Terminology](#8-file-extensions--terminology)
9. [Alternative Naming Strategies](#9-alternative-naming-strategies)
10. [Implementation Roadmap](#10-implementation-roadmap)
11. [Appendices](#11-appendices)

---

## 1. Language Overview

### Core Identity

Blood is a statically-typed, functional-oriented systems programming language targeting **safety-critical domains**:
- Avionics
- Medical devices
- Financial infrastructure
- Embedded systems requiring formal verification

### Technical Innovations

| Feature | Inspiration | Implementation |
|---------|-------------|----------------|
| Content-Addressed Code | Unison | BLAKE3-256 AST hashing, no file paths |
| Generational Memory Safety | Vale | 128-bit fat pointers (~1-2 cycle checks) |
| Mutable Value Semantics | Hylo | Default copying, explicit borrowing |
| Algebraic Effects | Koka | Row-polymorphic, evidence passing |
| Multiple Dispatch | Julia | Type-stable, open methods |

### Origin of "Blood"

The name derives from **"written in blood"**—rules and protocols that emerged from hard-won experience, often from failures and disasters. In safety-critical domains:

- Aviation regulations are "written in blood" after crashes
- Medical protocols evolve from adverse events
- Financial compliance rules follow market failures

This origin provides rich thematic territory for toolchain naming while reinforcing the language's mission: **preventing the next disaster through better tooling and type systems**.

### Current Implementation Status

```
Phase 0: ✓ Lexer, parser, AST, type checker
Phase 1: ✓ blood build/run, FizzBuzz works
Phase 2: ✓ Effect system (handlers, evidence passing)
Phase 3: ✓ MIR with 128-bit generational pointers
Phase 4: ✓ Content addressing (BLAKE3 hashing)
Phase 5: ✓ Runtime (fiber scheduler, channels, I/O reactor)
Phase 6: → Self-hosting and standard library (current)
```

---

## 2. Thematic Foundation

### Primary Thematic Domains

#### A. Circulatory/Biological System

| Term | Definition | Tool Application |
|------|------------|------------------|
| **Vein** | Vessel carrying blood to the heart | Dependency flow, data streams |
| **Artery** | Vessel carrying blood from the heart | Output, distribution |
| **Capillary** | Tiny blood vessels | Fine-grained connections |
| **Plasma** | Liquid carrying cells/nutrients | Package transport |
| **Marrow** | Where blood cells originate | Source generation |
| **Pulse/Beat** | Rhythmic heartbeat | Real-time feedback, REPL |
| **Flow** | Movement of blood | Data flow, LSP |
| **Corpuscle** | Blood cell | Module, compilation unit |
| **Vessel** | Container for blood | LSP, workspace |

#### B. Medical Procedures & Diagnostics

| Term | Definition | Tool Application |
|------|------------|------------------|
| **Transfuse** | Transfer blood | Package installation |
| **Draw** | Extract blood sample | Debugging, inspection |
| **Screen** | Test for conditions | Linting, static analysis |
| **Scan** | Medical imaging | Static analysis |
| **Panel** | Set of blood tests | Test suite |
| **Type** | Blood type classification | Type checking |
| **Count** | Blood cell count | Metrics, profiling |
| **Clot** | Coagulation | Formatting, bundling |
| **Culture** | Blood culture growth | Test runner |
| **Lab** | Blood laboratory | Testing environment |

#### C. Idiomatic Expressions

| Term | Definition | Tool Application |
|------|------------|------------------|
| **Oath** | Blood oath | Documentation, contracts |
| **Pact** | Blood pact | Configuration |
| **Covenant** | Sacred agreement | Package manifest |
| **Lineage** | Bloodline | Version control |
| **Kin** | Blood relatives | Related packages |
| **Testament** | Sworn record | Changelog |
| **Codex** | Ancient manuscript | Documentation |
| **First Blood** | Initial victory | Project init |

#### D. Compound/Creative Terms

| Term | Derivation | Tool Application |
|------|------------|------------------|
| **Bloodbank** | Blood bank | Package registry |
| **Bloodline** | Family line | Version history |
| **Bloodwork** | Medical tests | Static analysis |
| **Lifeblood** | Essential element | Core library |
| **Bloodstream** | Circulation | Data pipeline |

---

## 3. Conflict Analysis

### Research Methodology

All tool names were checked against:
- GitHub repositories and topics
- PyPI, npm, crates.io, RubyGems
- System utilities (Linux, macOS, Windows)
- Domain-specific tools in the programming space

### Confirmed Conflicts

| Name | Conflict | Severity | Alternative |
|------|----------|----------|-------------|
| `plasma` | KDE Plasma Desktop | 🟡 Medium | `vein` (recommended) |
| `pulse` | PulseAudio | 🟡 Medium | `beat` (recommended) |
| `screen` | GNU Screen | 🔴 High | `scan` (recommended) |
| `lab` | GitLab CLI (`glab`) | 🟡 Medium | `culture` (recommended) |
| `oath` | OATH Toolkit (2FA) | 🔴 High | `codex` (recommended) |
| `sanguine` | Multiple projects | 🟡 Medium | Avoid for core tools |

### Confirmed Clear Names

| Name | Verification | Notes |
|------|--------------|-------|
| `blood` | ✅ Clear | No major conflicts as unified CLI |
| `bloodc` | ✅ Clear | Standard compiler naming |
| `vein` | ✅ Clear | Only a video game server tool |
| `clot` | ✅ Clear | No programming tool conflicts |
| `beat` | ✅ Clear | No programming tool conflicts |
| `scan` | ✅ Clear | Generic but available |
| `vessel` | ✅ Clear | No LSP conflicts |
| `draw` | ✅ Clear | No debugger conflicts |
| `culture` | ✅ Clear | Perfect for testing |
| `codex` | ✅ Clear | No doc generator conflicts |
| `lineage` | ✅ Clear | No version manager conflicts |
| `bloodbank` | ✅ Clear | Domain available |

### Existing "Blood" in Programming

A small Java project called "Blood" exists (NielsTilch/Compiler on GitHub) but appears inactive and unlikely to cause confusion given the scale difference.

---

## 4. Tool-by-Tool Deep Dive

### 4.1 Unified CLI: `blood`

**Purpose:** Single entry point for all Blood operations, following the Go/Zig/Cargo pattern.

**Subcommand Design:**
```bash
blood new       # Create new project
blood init      # Initialize in existing directory
blood build     # Compile project
blood run       # Build and run
blood test      # Run tests
blood check     # Type-check without codegen
blood fmt       # Format code (invokes clot)
blood lint      # Lint code (invokes scan)
blood repl      # Start REPL (invokes beat)
blood doc       # Generate docs (invokes codex)
blood add       # Add dependency (invokes vein)
blood remove    # Remove dependency
blood update    # Update dependencies
blood publish   # Publish to bloodbank
blood bench     # Run benchmarks
blood clean     # Clean build artifacts
```

**Rationale:** Developers shouldn't need to remember multiple tool names for daily workflows. The unified CLI wraps underlying tools while still exposing them for advanced use.

---

### 4.2 Compiler: `bloodc`

**Purpose:** Direct compiler invocation for advanced users and build systems.

**Why `bloodc`:**
- Follows established convention: `rustc`, `swiftc`, `clang`
- Clear distinction from the unified CLI
- Expected by build systems and IDE integrations

**Alternative Considered:** `bleed` (active verb, memorable) — rejected as too morbid for a safety-critical language.

---

### 4.3 Package Manager: `vein`

**Purpose:** Dependency management, package installation, publishing.

**Why `vein` over `plasma`:**
1. **No conflicts** — `plasma` collides with KDE Plasma
2. **Better metaphor** — Veins carry blood TO the heart (dependencies flow IN)
3. **Short, memorable** — 4 characters vs 6
4. **Verb potential** — "vein in a dependency"

**Commands:**
```bash
vein init           # Initialize Blood.toml
vein add <pkg>      # Add dependency
vein remove <pkg>   # Remove dependency
vein install        # Install all dependencies
vein update         # Update dependencies
vein search <query> # Search bloodbank
vein publish        # Publish to bloodbank
vein login          # Authenticate with bloodbank
vein audit          # Security audit
vein tree           # Dependency tree
```

**Alternative Names Considered:**

| Name | Pros | Cons | Decision |
|------|------|------|----------|
| `plasma` | Great metaphor | KDE conflict | ❌ Rejected |
| `transfuse` | Active verb | Too long (9 chars) | ❌ Rejected |
| `flow` | Dynamic | Generic | ❌ Rejected |
| `marrow` | Source metaphor | Obscure | ❌ Rejected |
| `vein` | Clear, short, no conflict | — | ✅ Selected |

---

### 4.4 Formatter: `clot`

**Purpose:** Code formatting for consistent style.

**Why `clot`:**
1. **Perfect metaphor** — Blood clots organize chaotic flow into solid form
2. **Memorable** — Short, punchy, unique
3. **No conflicts** — Verified clear
4. **Visual** — The action of "clotting" messy code

**Commands:**
```bash
clot .              # Format current directory
clot --check       # Check without modifying
clot --diff        # Show what would change
clot src/main.blood # Format specific file
```

**Configuration:** `clot.toml` or `[clot]` section in `Blood.toml`

```toml
[clot]
line_width = 100
indent = 4
trailing_comma = "always"
```

---

### 4.5 Linter: `scan`

**Purpose:** Static analysis, catching bugs and style issues.

**Why `scan` over `screen`:**
1. **No conflicts** — GNU `screen` is too prevalent
2. **Medical accuracy** — "Blood scan" is a real diagnostic procedure
3. **Action-oriented** — "Scan your code for issues"

**Commands:**
```bash
scan .                    # Lint entire project
scan --strict            # Enable all warnings as errors
scan --fix               # Auto-fix where possible
scan --explain E0001     # Explain an error code
scan src/module.blood    # Lint specific file
```

**Alternative Names Considered:**

| Name | Pros | Cons | Decision |
|------|------|------|----------|
| `screen` | Blood screening | GNU Screen conflict | ❌ Rejected |
| `panel` | Blood panel tests | Sounds passive | ❌ Rejected |
| `check` | Intuitive | Too generic | ❌ Rejected |
| `bloodwork` | Thematic | Too long | ❌ Rejected |
| `scan` | Clear, actionable | — | ✅ Selected |

---

### 4.6 REPL: `beat`

**Purpose:** Interactive evaluation, exploration, prototyping.

**Why `beat` over `pulse`:**
1. **No conflicts** — PulseAudio owns "pulse" in Linux space
2. **Equally thematic** — Heartbeat is rhythmic interaction
3. **Short** — 4 characters

**Behavior:**
```
$ beat
Blood 0.1.0 REPL
Type :help for commands, :quit to exit

>>> let x = 42
>>> x * 2
84

>>> :type x
Int64

>>> :effect
Current effects: IO, Console

>>> :quit
```

**REPL Commands:**
```
:help     Show help
:quit     Exit REPL
:type     Show type of expression
:effect   Show current effect handlers
:load     Load a Blood file
:reset    Reset REPL state
```

---

### 4.7 Language Server: `vessel-ls`

**Purpose:** IDE integration via Language Server Protocol.

**Why `vessel`:**
1. **Perfect metaphor** — Blood vessels carry blood everywhere; LSP carries information everywhere in the IDE
2. **No conflicts** — No existing LSP named "vessel"
3. **Visual** — Vessels are conduits, like LSP is a conduit

**Configuration:**
```json
// VS Code settings.json
{
  "blood.languageServer.path": "vessel-ls",
  "blood.languageServer.args": ["--stdio"]
}
```

**Editor Plugin Names:**
- VS Code: `blood-vscode` or `vessel-vscode`
- Neovim: `blood.nvim`
- Emacs: `blood-mode`

---

### 4.8 Debugger: `draw`

**Purpose:** Interactive debugging, inspection, breakpoints.

**Why `draw`:**
1. **Medical metaphor** — "Blood draw" extracts samples for analysis
2. **Action verb** — "Draw out" the bugs
3. **No conflicts** — No debugger called "draw"

**Commands:**
```bash
draw ./app                    # Start debug session
draw --attach <pid>          # Attach to running process
draw --core core.dump        # Analyze core dump
draw --remote host:port      # Remote debugging
```

**Interactive Commands:**
```
(draw) break main.blood:42   # Set breakpoint
(draw) run                    # Start execution
(draw) step                   # Step into
(draw) next                   # Step over
(draw) continue               # Continue execution
(draw) print x                # Print variable
(draw) backtrace              # Show call stack
(draw) effect                 # Show effect stack
(draw) quit                   # Exit debugger
```

---

### 4.9 Test Runner: `culture`

**Purpose:** Running tests, benchmarks, coverage.

**Why `culture` over `lab`:**
1. **No conflicts** — GitLab CLI uses `glab`/`lab`
2. **Perfect metaphor** — "Blood culture" grows samples to detect issues; tests "grow" scenarios to detect bugs
3. **Unique** — Distinctive in the testing tool space

**Commands:**
```bash
culture run                   # Run all tests
culture run --filter "auth*" # Filter tests
culture run --parallel       # Parallel execution
culture watch                # Watch mode
culture coverage             # Generate coverage report
culture bench                # Run benchmarks
culture list                 # List all tests
```

**Test Annotation:**
```blood
@test
fn test_addition() {
    assert_eq(2 + 2, 4)
}

@test
@ignore("flaky on CI")
fn test_network_call() {
    // ...
}

@bench
fn bench_sort() {
    // ...
}
```

---

### 4.10 Documentation Generator: `codex`

**Purpose:** Generate documentation from source code and markdown.

**Why `codex` over `oath`:**
1. **No conflicts** — OATH Toolkit is a well-known 2FA system
2. **Meaning** — A codex is an ancient bound manuscript; documentation is the project's manuscript
3. **Resonance** — "Blood codex" evokes important historical documents

**Commands:**
```bash
codex build                   # Build documentation
codex serve                   # Local doc server
codex open                    # Open docs in browser
codex test                    # Test doc examples
codex publish                 # Publish to bloodbank
```

**Doc Comments:**
```blood
/// Adds two integers.
///
/// # Examples
///
/// ```blood
/// assert_eq(add(2, 3), 5)
/// ```
///
/// # Effects
///
/// None - this is a pure function.
fn add(a: Int, b: Int) -> Int {
    a + b
}
```

---

### 4.11 Version Manager: `lineage`

**Purpose:** Managing multiple Blood toolchain versions.

**Why `lineage`:**
1. **Perfect metaphor** — Bloodline/lineage tracks ancestry; version manager tracks releases
2. **No conflicts** — Verified clear
3. **Evocative** — Suggests history, inheritance, continuity

**Commands:**
```bash
lineage install 1.0.0        # Install specific version
lineage install stable       # Install stable channel
lineage install nightly      # Install nightly
lineage use 1.0.0            # Set default version
lineage list                 # List installed versions
lineage current              # Show current version
lineage update               # Update all channels
lineage run 0.9.0 -- build   # Run with specific version
```

---

### 4.12 Package Registry: Bloodbank

**Purpose:** Central repository for Blood packages.

**Why `bloodbank`:**
1. **Perfect metaphor** — Blood banks store blood for when it's needed; package registries store code
2. **Memorable** — Single compound word
3. **Domain available** — `bloodbank.dev` likely available

**URLs:**
- Registry: `bloodbank.dev` or `bank.blood.dev`
- API: `api.bloodbank.dev`
- Documentation: `docs.bloodbank.dev`

---

## 5. CLI Design & Help Text Mockups

### 5.1 Main CLI (`blood --help`)

```
Blood - A safety-critical systems programming language

Usage: blood <COMMAND> [OPTIONS]

Commands:
  new        Create a new Blood project
  init       Initialize Blood in an existing directory
  build      Compile the current project
  run        Build and run the current project
  test       Run tests (alias for `culture run`)
  check      Type-check without generating code
  fmt        Format source code (alias for `clot`)
  lint       Run static analysis (alias for `scan`)
  repl       Start interactive REPL (alias for `beat`)
  doc        Generate documentation (alias for `codex`)
  add        Add a dependency
  remove     Remove a dependency
  update     Update dependencies
  publish    Publish package to bloodbank
  bench      Run benchmarks
  clean      Remove build artifacts

Options:
  -v, --verbose    Increase output verbosity
  -q, --quiet      Suppress non-error output
  -h, --help       Print help
  -V, --version    Print version

Learn more at https://blood.dev

Report bugs at https://github.com/blood-lang/blood/issues
```

### 5.2 Package Manager (`vein --help`)

```
vein - Package manager for Blood

Usage: vein <COMMAND> [OPTIONS]

Commands:
  init       Create a new Blood.toml
  add        Add a dependency
  remove     Remove a dependency
  install    Install all dependencies
  update     Update dependencies to latest compatible versions
  search     Search bloodbank for packages
  info       Show information about a package
  publish    Publish this package to bloodbank
  login      Authenticate with bloodbank
  logout     Remove authentication
  audit      Check for security vulnerabilities
  tree       Display dependency tree
  outdated   Show outdated dependencies
  why        Explain why a package is installed

Options:
  --manifest <PATH>  Path to Blood.toml [default: Blood.toml]
  --locked           Require Blood.lock to be up to date
  --offline          Run without network access
  -v, --verbose      Increase output verbosity
  -h, --help         Print help
  -V, --version      Print version

Examples:
  vein add http-client
  vein add crypto@2.0 --features tls,async
  vein update --aggressive
  vein tree --duplicates
```

### 5.3 Formatter (`clot --help`)

```
clot - Code formatter for Blood

Usage: clot [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Files or directories to format [default: .]

Options:
  --check        Check formatting without modifying files
  --diff         Show diff of what would change
  --write        Write formatted output (default)
  --stdin        Read from stdin, write to stdout
  --config <FILE>  Path to configuration file

Configuration:
  Create a clot.toml or add [clot] to Blood.toml

  [clot]
  line_width = 100
  indent = 4
  trailing_comma = "always"
  imports_granularity = "crate"

Examples:
  clot                     # Format entire project
  clot src/lib.blood       # Format single file
  clot --check             # CI mode: fail if not formatted
  clot --diff              # Preview changes
```

### 5.4 Test Runner (`culture --help`)

```
culture - Test runner for Blood

Usage: culture <COMMAND> [OPTIONS]

Commands:
  run        Run tests
  list       List all tests without running
  watch      Run tests on file changes
  coverage   Generate code coverage report
  bench      Run benchmarks

Run Options:
  --filter <PATTERN>    Filter tests by name
  --parallel <N>        Run N tests in parallel [default: auto]
  --fail-fast           Stop on first failure
  --no-capture          Don't capture stdout/stderr
  --show-output         Show output for passing tests
  --timeout <DURATION>  Timeout per test [default: 60s]

Coverage Options:
  --format <FORMAT>     Output format: html, lcov, json [default: html]
  --output <PATH>       Output path [default: target/coverage]
  --min-coverage <PCT>  Fail if coverage below threshold

Examples:
  culture run
  culture run --filter "auth::*"
  culture run --parallel 8 --fail-fast
  culture coverage --min-coverage 80
  culture bench --baseline previous
```

### 5.5 REPL (`beat --help`)

```
beat - Interactive REPL for Blood

Usage: beat [OPTIONS] [FILE]

Arguments:
  [FILE]  Preload a Blood file into the session

Options:
  --no-banner          Skip startup banner
  --no-colors          Disable colored output
  --effect <EFFECTS>   Pre-install effect handlers
  --history <FILE>     History file location
  -e <EXPR>            Evaluate expression and exit

REPL Commands:
  :help      Show available commands
  :quit      Exit the REPL
  :type <e>  Show the type of expression
  :effect    Show current effect handlers
  :load <f>  Load a Blood file
  :reset     Reset REPL state
  :clear     Clear screen
  :history   Show command history

Examples:
  beat                           # Interactive session
  beat prelude.blood             # Load file first
  beat -e "2 + 2"                # Evaluate and exit
```

---

## 6. Standard Library Naming

### 6.1 Module Hierarchy

The Blood standard library follows a hierarchical naming convention using the `blood::` namespace prefix:

```
blood::                    # Root namespace
├── core/                  # Primitives, no runtime
│   ├── types             # Int, Float, Bool, Char
│   ├── ops               # Operators, traits
│   ├── mem               # Memory primitives
│   └── marker            # Marker traits
├── alloc/                # Heap allocation
│   ├── box               # Owned heap pointers
│   ├── vec               # Growable arrays
│   └── string            # UTF-8 strings
├── collections/          # Data structures
│   ├── hash_map          # Hash tables
│   ├── btree_map         # Ordered maps
│   ├── set               # Set types
│   └── deque             # Double-ended queues
├── io/                   # Input/Output
│   ├── read              # Reader trait
│   ├── write             # Writer trait
│   ├── file              # File operations
│   ├── net               # Networking
│   └── buf               # Buffered I/O
├── async/                # Asynchronous primitives
│   ├── fiber             # Fibers/green threads
│   ├── channel           # MPMC channels
│   └── select            # Multiplexing
├── effect/               # Effect system
│   ├── handler           # Effect handlers
│   ├── evidence          # Evidence passing
│   └── builtin           # Builtin effects
├── fmt/                  # Formatting
│   ├── display           # Human-readable
│   ├── debug             # Debug output
│   └── write             # Format writing
├── hash/                 # Hashing
│   ├── hasher            # Hasher trait
│   ├── blake3            # BLAKE3 implementation
│   └── sip               # SipHash
├── math/                 # Mathematics
│   ├── num               # Numeric traits
│   ├── float             # Floating point
│   └── rand              # Random numbers
├── sync/                 # Synchronization
│   ├── mutex             # Mutual exclusion
│   ├── rwlock            # Reader-writer locks
│   └── atomic            # Atomic operations
├── time/                 # Time handling
│   ├── instant           # Monotonic time
│   ├── duration          # Time spans
│   └── system            # System clock
├── process/              # Process handling
│   ├── env               # Environment
│   ├── exit              # Exit codes
│   └── spawn             # Child processes
├── path/                 # Path manipulation
│   ├── path              # Path type
│   └── components        # Path parsing
├── ffi/                  # Foreign function interface
│   ├── c                 # C interop
│   └── abi               # ABI definitions
└── test/                 # Testing utilities
    ├── assert            # Assertions
    ├── mock              # Mocking
    └── prop              # Property testing
```

### 6.2 Naming Conventions

| Category | Convention | Example |
|----------|------------|---------|
| Modules | `snake_case` | `hash_map`, `btree_set` |
| Types | `PascalCase` | `HashMap`, `Vec`, `String` |
| Functions | `snake_case` | `read_line`, `to_string` |
| Constants | `SCREAMING_SNAKE` | `MAX_SIZE`, `PI` |
| Traits | `PascalCase` | `Read`, `Write`, `Hash` |
| Effects | `PascalCase` | `IO`, `State`, `Async` |
| Effect handlers | `snake_case` | `with_io`, `handle_state` |

### 6.3 Thematic Module Names (Optional)

For modules that could use thematic naming:

| Standard Name | Thematic Alternative | Notes |
|---------------|---------------------|-------|
| `prelude` | `lifeblood` | Auto-imported essentials |
| `core` | `marrow` | Foundation |
| `alloc` | `plasma` | Carries resources |
| `collections` | `vessels` | Containers |
| `test` | `culture` | Testing |

**Recommendation:** Use standard naming for discoverability, with optional thematic aliases.

---

## 7. Package & Project Conventions

### 7.1 Project Structure

```
my-project/
├── Blood.toml           # Project manifest
├── Blood.lock           # Dependency lock file
├── .bloodignore         # Files to ignore
├── src/
│   ├── main.blood       # Entry point (binary)
│   └── lib.blood        # Entry point (library)
├── tests/
│   ├── integration/
│   │   └── api.blood
│   └── unit/
│       └── parser.blood
├── benches/
│   └── performance.blood
├── examples/
│   └── basic.blood
└── docs/
    └── guide.md
```

### 7.2 Manifest File (Blood.toml)

```toml
[package]
name = "my-project"
version = "0.1.0"
edition = "2026"
authors = ["Alice <alice@example.com>"]
license = "MIT OR Apache-2.0"
description = "A short description of the project"
repository = "https://github.com/alice/my-project"
documentation = "https://docs.bloodbank.dev/my-project"
keywords = ["safety", "parsing"]
categories = ["safety-critical", "parsing"]

[dependencies]
regex = "1.0"
http-client = { version = "2.0", features = ["tls"] }
local-crate = { path = "../local-crate" }
git-dep = { git = "https://github.com/org/repo" }

[dev-dependencies]
test-utils = "0.5"
mock-io = "1.0"

[build-dependencies]
code-gen = "0.2"

[features]
default = ["std"]
std = []
alloc = []
async = ["tokio"]
unstable = []

[profile.release]
opt-level = 3
lto = true
debug = false

[profile.dev]
opt-level = 0
debug = true
overflow-checks = true

# Tool configurations can go here
[clot]
line_width = 100

[scan]
deny = ["unsafe_code"]
```

### 7.3 Lock File (Blood.lock)

```toml
# This file is automatically generated by vein.
# Do not edit manually.

[[package]]
name = "regex"
version = "1.0.5"
source = "bloodbank"
checksum = "blake3:abc123..."
dependencies = [
    "memchr 2.4.0",
]

[[package]]
name = "memchr"
version = "2.4.0"
source = "bloodbank"
checksum = "blake3:def456..."
```

### 7.4 Package Naming Conventions

| Rule | Example | Notes |
|------|---------|-------|
| Lowercase with hyphens | `http-client` | Primary separator |
| No underscores in names | ~~`http_client`~~ | Use hyphens |
| Descriptive prefixes | `blood-derive` | For extensions |
| Avoid generic names | ~~`utils`~~, ~~`common`~~ | Be specific |

---

## 8. File Extensions & Terminology

### 8.1 File Extensions

| Extension | Purpose | Example |
|-----------|---------|---------|
| `.blood` | Blood source files | `main.blood` |
| `.bloodi` | Interface files (for IDE) | `module.bloodi` |
| `Blood.toml` | Project manifest | — |
| `Blood.lock` | Dependency lock | — |
| `clot.toml` | Formatter config (optional) | — |
| `.bloodignore` | Ignore patterns | — |

**Why `.blood`:**
- Unique and unlikely to conflict
- Clear association with the language
- Memorable
- Not too long (6 characters including dot)

**Alternatives Considered:**
- `.bl` — Too short, potential conflicts
- `.bld` — Looks like "build"
- `.blod` — Typo-prone

### 8.2 Module System Terminology

| Term | Definition | Similar To |
|------|------------|------------|
| **Cell** | A single compilation unit | Rust's crate |
| **Module** | Namespace within a cell | Rust's module |
| **Package** | Published unit on bloodbank | npm package |
| **Workspace** | Multi-cell project | Cargo workspace |

**Usage in Code:**

```blood
// In src/lib.blood
pub mod parser;     // Declares submodule
pub use parser::*;  // Re-exports

// In src/parser.blood or src/parser/mod.blood
pub fn parse(input: String) -> Result<Ast, Error> {
    // ...
}
```

### 8.3 Terminology Glossary

| Term | Definition | Context |
|------|------------|---------|
| **Cell** | Compilation unit (blood cell) | Module system |
| **Vessel** | Workspace container | Project structure |
| **Transfusion** | Adding external code | Dependencies |
| **Typing** | Type checking | Compiler phase |
| **Screening** | Static analysis | Linting |
| **Culture** | Test growth/execution | Testing |
| **Effect** | Tracked side effect | Type system |
| **Handler** | Effect interpreter | Runtime |

---

## 9. Alternative Naming Strategies

### 9.1 Single-Binary Subcommand Style

Like Go's `go` command, everything under one tool:

```bash
blood build
blood test
blood type       # Type checking
blood clot       # Formatting
blood screen     # Linting
blood pulse      # REPL
blood bank       # Package management
```

**Pros:**
- Single install
- Unified interface
- No PATH conflicts

**Cons:**
- Harder to extend
- Subcommands may feel awkward (`blood clot`?)
- Lost thematic charm of standalone names

### 9.2 Pure Subcommand (No Themes)

Standard tool names as subcommands:

```bash
blood build
blood run
blood test
blood fmt
blood lint
blood repl
blood doc
blood add
blood publish
```

**Pros:**
- Familiar to Go/Zig users
- No thematic learning curve
- Maximally conventional

**Cons:**
- Loses Blood's personality
- Indistinguishable from other languages

### 9.3 Hybrid Approach (Recommended)

Use the themed standalone tools for advanced users, but expose them via standard subcommands:

```bash
# These are equivalent:
blood fmt   ↔  clot
blood lint  ↔  scan
blood repl  ↔  beat
blood test  ↔  culture run
blood doc   ↔  codex build
```

This gives beginners a familiar interface while allowing power users to use the themed tools directly.

---

## 10. Implementation Roadmap

### Phase 1: Core Toolchain (v0.1 - v0.5)

| Priority | Tool | Name | Status |
|----------|------|------|--------|
| P0 | Compiler | `bloodc` | In progress |
| P0 | Unified CLI | `blood` | In progress |
| P1 | Build system | `blood build` | Planned |
| P1 | Formatter | `clot` | Planned |
| P2 | REPL | `beat` | Planned |

### Phase 2: Developer Experience (v0.5 - v1.0)

| Priority | Tool | Name | Status |
|----------|------|------|--------|
| P1 | Package manager | `vein` | Planned |
| P1 | Language server | `vessel-ls` | Planned |
| P2 | Linter | `scan` | Planned |
| P2 | Test runner | `culture` | Planned |
| P3 | Doc generator | `codex` | Planned |

### Phase 3: Ecosystem (v1.0+)

| Priority | Tool | Name | Status |
|----------|------|------|--------|
| P1 | Package registry | bloodbank.dev | Planned |
| P2 | Debugger | `draw` | Planned |
| P2 | Version manager | `lineage` | Planned |
| P3 | Playground | play.blood.dev | Planned |
| P3 | Profiler | `count` | Planned |

---

## 11. Appendices

### A. Full Toolchain Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         blood (unified CLI)                          │
│      blood build | run | test | fmt | lint | repl | doc | add        │
├──────────┬──────────┬──────────┬──────────┬──────────┬───────────────┤
│  bloodc  │   vein   │   clot   │   scan   │   beat   │   vessel-ls   │
│ compiler │ packages │ formatter│  linter  │   REPL   │      LSP      │
├──────────┴──────────┼──────────┴──────────┼──────────┴───────────────┤
│       culture       │        codex        │          draw            │
│     test runner     │     doc generator   │        debugger          │
├─────────────────────┴─────────────────────┴──────────────────────────┤
│                          lineage (version manager)                    │
├──────────────────────────────────────────────────────────────────────┤
│                         bloodbank (registry)                          │
│                        https://bloodbank.dev                          │
└──────────────────────────────────────────────────────────────────────┘
```

### B. ASCII Art Visualization

```
                            ┌─────────────┐
                            │    blood    │
                            │ (unified)   │
                            └──────┬──────┘
                                   │
        ┌──────┬───────┬───────┬───┴───┬───────┬───────┬──────┐
        │      │       │       │       │       │       │      │
        ▼      ▼       ▼       ▼       ▼       ▼       ▼      ▼
    ┌──────┐┌─────┐┌──────┐┌──────┐┌──────┐┌──────┐┌──────┐┌──────┐
    │bloodc││vein ││ clot ││ scan ││ beat ││vessel││culture││codex │
    └──────┘└─────┘└──────┘└──────┘└──────┘└──────┘└──────┘└──────┘
        │      │       │       │       │       │       │      │
        │      └───────┴───────┴───────┴───────┴───────┴──────┘
        │                        │
        ▼                        ▼
    ┌──────┐                ┌──────────┐
    │target│                │bloodbank │
    │build │                │.dev      │
    │ dir  │                │          │
    └──────┘                └──────────┘
```

### C. Domain Availability Check

As of January 2026, recommended domains to secure:

| Domain | Purpose | Priority |
|--------|---------|----------|
| `blood.dev` | Main site | P0 |
| `bloodbank.dev` | Package registry | P0 |
| `blood-lang.org` | Alternative main | P1 |
| `bloodlang.dev` | Alternative main | P1 |

### D. Similar Language Comparison

| Aspect | Rust | Go | Zig | Blood |
|--------|------|----|----|-------|
| Unified CLI | `cargo` | `go` | `zig` | `blood` |
| Compiler | `rustc` | (part of `go`) | (part of `zig`) | `bloodc` |
| Package Manager | Cargo | Go Modules | Zig build | `vein` |
| Formatter | `rustfmt` | `go fmt` | `zig fmt` | `clot` |
| Linter | Clippy | `go vet` | — | `scan` |
| REPL | (none official) | — | — | `beat` |
| LSP | rust-analyzer | gopls | zls | `vessel-ls` |

### E. Error Message Style Guide

Blood error messages should follow this format:

```
error[E0001]: type mismatch
  --> src/main.blood:42:13
   |
42 |     let x: Int = "hello"
   |            ---   ^^^^^^^ expected `Int`, found `String`
   |            |
   |            expected due to this

help: try converting the string to an integer
   |
42 |     let x: Int = "hello".parse()?
   |                         ^^^^^^^^
```

### F. Community & Branding

| Resource | Name | URL |
|----------|------|-----|
| Package Registry | Bloodbank | `bloodbank.dev` |
| Documentation | Blood Book | `book.blood.dev` |
| Playground | The Donor Room | `play.blood.dev` |
| Community Forum | The Bloodline | `community.blood.dev` |
| Discord/Chat | Blood Vessels | `discord.gg/blood` |
| Blog | The Pulse | `blog.blood.dev` |

---

## Summary

This document provides a complete, research-backed naming system for the Blood programming language toolchain. The recommendations:

1. **Avoid known conflicts** (no `pulse`, `screen`, `oath`, `lab`)
2. **Maintain thematic coherence** (all names relate to blood/medical domain)
3. **Prioritize usability** (short names, familiar subcommands)
4. **Support both styles** (themed standalone tools + standard subcommands)

The naming balances Blood's unique identity as a safety-critical language with practical developer experience concerns.

---

*For questions or suggestions, open an issue at github.com/blood-lang/blood*

