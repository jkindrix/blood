# Blood Programming Language: Complete Toolchain & Naming Reference

*A comprehensive guide to naming tools for the Blood programming language*

**Document Version:** 3.0
**Last Updated:** January 2026
**Status:** Ready for Implementation

---

## Executive Summary

This document provides complete naming recommendations for the Blood programming language toolchain, including conflict analysis, CLI mockups, stdlib naming conventions, and rationale for each decision. All recommendations have been verified against existing tools and projects.

**Key Design Principles:**

1. **Hybrid CLI Approach** - The unified `blood` CLI provides conventional subcommands (`blood fmt`, `blood test`) for beginners, while thematic standalone tools (`clot`, `culture`) are available for power users
2. **Content-Addressed Code** - Blood uses Unison-style content addressing where code is stored by hash, not name—this fundamentally changes how the toolchain works
3. **Thematic Coherence** - All tool names relate to blood/medical terminology, reinforcing the "written in blood" safety-critical mission

### Quick Reference: Recommended Toolchain

| Category | Tool Name | CLI Alias | Purpose |
|----------|-----------|-----------|---------|
| Unified CLI | `blood` | — | Single entry point for all operations |
| Compiler | `bloodc` | `blood build` | Direct compiler invocation |
| Codebase Manager | `marrow` | `blood add/find/rename` | Content-addressed code management |
| Formatter | `clot` | `blood fmt` | Code formatting |
| Linter | `scan` | `blood lint` | Static analysis |
| REPL | `beat` | `blood repl` | Interactive evaluation |
| Language Server | `vessel-ls` | — | IDE integration (LSP) |
| Debugger | `draw` | `blood debug` | Interactive debugging |
| Test Runner | `culture` | `blood test` | Test execution |
| Doc Generator | `codex` | `blood doc` | Documentation generation |
| Code Registry | Bloodbank | — | Shared code by hash |

---

## Table of Contents

1. [Language Overview](#1-language-overview)
2. [Content-Addressed Architecture](#2-content-addressed-architecture)
3. [Thematic Foundation](#3-thematic-foundation)
4. [Conflict Analysis](#4-conflict-analysis)
5. [Hybrid CLI Design](#5-hybrid-cli-design)
6. [Tool-by-Tool Deep Dive](#6-tool-by-tool-deep-dive)
7. [CLI Help Text Mockups](#7-cli-help-text-mockups)
8. [Standard Library Naming](#8-standard-library-naming)
9. [Project Conventions](#9-project-conventions)
10. [File Extensions & Terminology](#10-file-extensions--terminology)
11. [Implementation Roadmap](#11-implementation-roadmap)
12. [Appendices](#12-appendices)

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
| Content-Addressed Code | Unison | BLAKE3-256 AST hashing, names as metadata |
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
✓ Lexer, parser, AST, type checker
✓ blood build/run, FizzBuzz works
✓ Effect system (handlers, evidence passing)
✓ MIR with 128-bit generational pointers
✓ Content addressing (BLAKE3 hashing)
✓ Runtime (fiber scheduler, channels, I/O reactor)
→ Self-hosting and standard library (current)
```

---

## 2. Content-Addressed Architecture

### The Paradigm Shift

Blood uses **content-addressed code** inspired by Unison. This is fundamentally different from traditional languages:

| Traditional Approach | Blood's Content-Addressed Approach |
|---------------------|-----------------------------------|
| Code identified by file path | Code identified by hash of AST |
| Names are identities | Names are metadata/pointers to hashes |
| Renaming breaks references | Renaming is free and instant |
| Version conflicts ("dependency hell") | Exact hashes, no conflicts possible |
| Merge conflicts on refactoring | Structure-aware, conflict-free merges |

### How It Works

```
┌─────────────────────────────────────────────────────────────┐
│                     Blood Codebase                           │
├─────────────────────────────────────────────────────────────┤
│  Definitions (stored by hash)                                │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ #a7f3b2c1 → fn double(x: Int) -> Int { x * 2 }         ││
│  │ #c9e4d8a2 → fn triple(x: Int) -> Int { x * 3 }         ││
│  │ #b1d2e3f4 → struct Point { x: Int, y: Int }            ││
│  └─────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────┤
│  Names (pointers to hashes)                                  │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ math.double     → #a7f3b2c1                             ││
│  │ math.triple     → #c9e4d8a2                             ││
│  │ geometry.Point  → #b1d2e3f4                             ││
│  │ utils.double    → #a7f3b2c1  (same code, different name)││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

### Implications for Tooling

**No Traditional Package Manager Needed:**
- Traditional package managers resolve version conflicts
- With content-addressing, there are no version conflicts—each dependency is an exact hash
- Instead, we have a **codebase manager** that handles names, history, and sharing

**No Separate Version Manager Needed:**
- Code history is built into the codebase
- Every change creates a new hash
- "Versions" are just named points in the hash history

**Sharing Code:**
- Bloodbank stores definitions by hash
- "Publishing" means making your hashes available
- "Depending" means referencing specific hashes
- No semver, no compatibility matrices, no resolution algorithms

### Why "Marrow"?

The codebase manager is named `marrow` because:
- **Bone marrow is where blood cells originate** — it's the source of all blood
- **The codebase is where all code originates** — it's the source of all definitions
- The metaphor is perfect: marrow generates and maintains the lifeblood of the system

---

## 3. Thematic Foundation

### Primary Thematic Domains

#### A. Circulatory/Biological System

| Term | Definition | Tool Application |
|------|------------|------------------|
| **Marrow** | Where blood cells originate | Codebase manager (source of code) |
| **Vein** | Vessel carrying blood to the heart | Data flow, streams |
| **Artery** | Vessel carrying blood from the heart | Output, distribution |
| **Plasma** | Liquid carrying cells/nutrients | Transport layer |
| **Pulse/Beat** | Rhythmic heartbeat | Real-time feedback, REPL |
| **Flow** | Movement of blood | Data flow, LSP |
| **Corpuscle** | Blood cell | Module, compilation unit |
| **Vessel** | Container for blood | LSP, workspace |

#### B. Medical Procedures & Diagnostics

| Term | Definition | Tool Application |
|------|------------|------------------|
| **Draw** | Extract blood sample | Debugging, inspection |
| **Scan** | Medical imaging | Static analysis |
| **Panel** | Set of blood tests | Test suite |
| **Type** | Blood type classification | Type checking |
| **Count** | Blood cell count | Metrics, profiling |
| **Clot** | Coagulation | Formatting (organizing chaos) |
| **Culture** | Blood culture growth | Test runner (growing test cases) |

#### C. Idiomatic Expressions

| Term | Definition | Tool Application |
|------|------------|------------------|
| **Codex** | Ancient manuscript | Documentation |
| **Lineage** | Bloodline | History tracking |
| **Kin** | Blood relatives | Related definitions |
| **First Blood** | Initial victory | Project initialization |

#### D. Compound/Creative Terms

| Term | Derivation | Tool Application |
|------|------------|------------------|
| **Bloodbank** | Blood bank | Code registry |
| **Bloodline** | Family line | Definition history |
| **Lifeblood** | Essential element | Core library |

---

## 4. Conflict Analysis

### Research Methodology

All tool names were checked against:
- GitHub repositories and topics
- PyPI, npm, crates.io, RubyGems
- System utilities (Linux, macOS, Windows)
- Domain-specific tools in the programming space

### Confirmed Conflicts

| Name | Conflict | Severity | Alternative |
|------|----------|----------|-------------|
| `plasma` | KDE Plasma Desktop | 🔴 High | Avoid |
| `pulse` | PulseAudio | 🔴 High | `beat` |
| `screen` | GNU Screen | 🔴 High | `scan` |
| `lab` | GitLab CLI (`glab`) | 🟡 Medium | `culture` |
| `oath` | OATH Toolkit (2FA) | 🔴 High | `codex` |

### Confirmed Clear Names

| Name | Verification | Notes |
|------|--------------|-------|
| `blood` | ✅ Clear | No major conflicts |
| `bloodc` | ✅ Clear | Standard compiler naming |
| `marrow` | ✅ Clear | No programming tool conflicts |
| `clot` | ✅ Clear | No programming tool conflicts |
| `beat` | ✅ Clear | No programming tool conflicts |
| `scan` | ✅ Clear | Generic but available |
| `vessel` | ✅ Clear | No LSP conflicts |
| `draw` | ✅ Clear | No debugger conflicts |
| `culture` | ✅ Clear | Perfect for testing |
| `codex` | ✅ Clear | No doc generator conflicts |
| `bloodbank` | ✅ Clear | Domain likely available |

---

## 5. Hybrid CLI Design

### Philosophy

The hybrid approach gives **beginners a familiar interface** while **power users get thematic tools**:

```bash
# These pairs are equivalent:
blood fmt      ↔  clot
blood lint     ↔  scan
blood test     ↔  culture run
blood repl     ↔  beat
blood doc      ↔  codex build
blood add      ↔  marrow add
blood find     ↔  marrow find
blood rename   ↔  marrow rename
```

### Why Both?

**For beginners:**
- `blood fmt` is instantly understandable
- No need to learn thematic naming
- Familiar to Go/Rust/Zig users

**For power users:**
- `clot` is memorable and distinctive
- Direct invocation for scripting
- Full access to tool-specific features

### Implementation

The `blood` CLI is a thin wrapper that delegates to standalone tools:

```rust
// Simplified implementation
fn main() {
    match args.command {
        "fmt" => exec("clot", &args.rest),
        "lint" => exec("scan", &args.rest),
        "test" => exec("culture", &["run", &args.rest].concat()),
        "repl" => exec("beat", &args.rest),
        "doc" => exec("codex", &["build", &args.rest].concat()),
        "add" | "find" | "rename" => exec("marrow", &args.all),
        // ... etc
    }
}
```

---

## 6. Tool-by-Tool Deep Dive

### 6.1 Unified CLI: `blood`

**Purpose:** Single entry point for all Blood operations.

**Subcommands:**
```bash
# Building & Running
blood build     # Compile project
blood run       # Build and run
blood check     # Type-check without codegen

# Code Quality
blood fmt       # Format code (→ clot)
blood lint      # Static analysis (→ scan)

# Testing & Docs
blood test      # Run tests (→ culture run)
blood bench     # Run benchmarks (→ culture bench)
blood doc       # Generate docs (→ codex build)

# Interactive
blood repl      # Start REPL (→ beat)
blood debug     # Start debugger (→ draw)

# Codebase Management (→ marrow)
blood add       # Add a definition by name
blood find      # Find definitions
blood rename    # Rename a definition
blood history   # Show definition history
blood deps      # Show dependencies
blood sync      # Sync with bloodbank

# Project Setup
blood new       # Create new project
blood init      # Initialize in existing directory
blood clean     # Remove build artifacts
```

---

### 6.2 Compiler: `bloodc`

**Purpose:** Direct compiler invocation for build systems and advanced users.

**Why `bloodc`:**
- Follows convention: `rustc`, `swiftc`, `clang`
- Clear distinction from unified CLI
- Expected by build systems

---

### 6.3 Codebase Manager: `marrow`

**Purpose:** Content-addressed codebase management—the heart of Blood's code organization.

**Why `marrow`:**
- Bone marrow is where blood cells **originate**
- The codebase is where all definitions **originate**
- Perfect metaphor for the source/origin of code

**Key Concepts:**
- **Definitions** are stored by hash, not name
- **Names** are pointers to hashes (metadata)
- **History** is the chain of hashes over time
- **Sharing** means making hashes available on Bloodbank

**Commands:**
```bash
# Adding code
marrow add <source>           # Add definition, get hash
marrow add --name <n> <src>   # Add with name

# Finding code
marrow find <query>           # Search by name or hash
marrow view <name-or-hash>    # View definition source
marrow list                   # List all names
marrow list --prefix std.io   # List names under prefix

# Naming
marrow rename <old> <new>     # Rename (instant, free)
marrow alias <name> <hash>    # Add name for hash
marrow unname <name>          # Remove a name

# Dependencies
marrow deps <name>            # Show what this uses
marrow dependents <name>      # Show what uses this
marrow tree <name>            # Full dependency tree

# History
marrow history <name>         # Show history of a name
marrow diff <hash1> <hash2>   # Diff two definitions
marrow rollback <name> <hash> # Point name to old hash

# Sharing (Bloodbank)
marrow push                   # Push local definitions
marrow pull <hash>            # Pull definition by hash
marrow pull <name>            # Pull by name from registry
marrow sync                   # Bidirectional sync

# Project
marrow init                   # Initialize codebase
marrow stats                  # Codebase statistics
```

**Example Session:**
```bash
$ marrow add "fn double(x: Int) -> Int { x * 2 }"
Added: #a7f3b2c1

$ marrow alias math.double #a7f3b2c1
Aliased: math.double → #a7f3b2c1

$ marrow rename math.double utils.double
Renamed: math.double → utils.double
# This is INSTANT - no refactoring needed!

$ marrow find double
  utils.double → #a7f3b2c1

$ marrow deps utils.double
  (none - pure function)

$ marrow history utils.double
  2026-01-10 14:32  #a7f3b2c1  (current)
  2026-01-10 14:30  renamed from math.double
  2026-01-10 14:28  #a7f3b2c1  created
```

---

### 6.4 Formatter: `clot`

**Purpose:** Code formatting for consistent style.

**Why `clot`:**
- Blood clots organize chaotic flow into solid form
- Formatting organizes chaotic code into consistent form
- Short, memorable, unique

**Commands:**
```bash
clot .                  # Format current directory
clot --check           # Check without modifying
clot --diff            # Show what would change
clot src/main.blood    # Format specific file
```

**Configuration:** `[clot]` section in `Blood.toml`:
```toml
[clot]
line_width = 100
indent = 4
trailing_comma = "always"
effect_annotation_style = "postfix"  # fn foo() -> Int / IO
```

---

### 6.5 Linter: `scan`

**Purpose:** Static analysis, catching bugs and style issues.

**Why `scan`:**
- "Blood scan" is a real diagnostic procedure
- Action-oriented: "Scan your code for issues"
- No conflicts (unlike `screen`)

**Commands:**
```bash
scan .                    # Lint entire project
scan --strict            # All warnings as errors
scan --fix               # Auto-fix where possible
scan --explain E0001     # Explain an error code
```

---

### 6.6 REPL: `beat`

**Purpose:** Interactive evaluation, exploration, prototyping.

**Why `beat`:**
- Heartbeat represents rhythmic interaction
- No conflicts (unlike `pulse` → PulseAudio)
- Short and memorable

**Behavior:**
```
$ beat
Blood 0.1.0 REPL (content-addressed)
Type :help for commands, :quit to exit

>>> let x = 42
>>> x * 2
84

>>> :type x
Int64

>>> :effect
Current effects: pure

>>> :hash
Current expression hash: #f7a2b3c4

>>> :quit
```

---

### 6.7 Language Server: `vessel-ls`

**Purpose:** IDE integration via Language Server Protocol.

**Why `vessel`:**
- Blood vessels carry blood everywhere in the body
- LSP carries code intelligence everywhere in the IDE
- Natural metaphor for a conduit/transport system

**Configuration (VS Code):**
```json
{
  "blood.languageServer.path": "vessel-ls",
  "blood.languageServer.args": ["--stdio"]
}
```

---

### 6.8 Debugger: `draw`

**Purpose:** Interactive debugging, inspection, breakpoints.

**Why `draw`:**
- "Blood draw" extracts samples for analysis
- Debugging "draws out" bugs for inspection
- Action verb, no conflicts

**Commands:**
```bash
draw ./app                # Start debug session
draw --attach <pid>       # Attach to process
```

---

### 6.9 Test Runner: `culture`

**Purpose:** Running tests, benchmarks, coverage.

**Why `culture`:**
- Blood cultures grow samples to detect infections
- Tests "grow" scenarios to detect bugs
- Perfect medical metaphor for testing

**Commands:**
```bash
culture run                   # Run all tests
culture run --filter "auth*"  # Filter tests
culture watch                 # Watch mode
culture coverage              # Coverage report
culture bench                 # Benchmarks
```

---

### 6.10 Documentation Generator: `codex`

**Purpose:** Generate documentation from source code.

**Why `codex`:**
- A codex is an ancient bound manuscript
- Documentation is the project's manuscript
- "Blood codex" evokes important historical documents

**Commands:**
```bash
codex build    # Build documentation
codex serve    # Local doc server
codex test     # Test doc examples
```

---

### 6.11 Code Registry: Bloodbank

**Purpose:** Shared registry of content-addressed definitions.

**How It Differs From npm/crates.io:**
- Stores **hashes**, not versioned packages
- No version resolution needed
- Pull exactly the code you reference
- Publish means making hashes available

**URLs:**
- Registry: `bloodbank.dev`
- API: `api.bloodbank.dev`
- Docs: `docs.bloodbank.dev`

**Usage:**
```bash
# Push your codebase's definitions to bloodbank
marrow push

# Pull a specific hash
marrow pull #a7f3b2c1

# Pull by name (looks up hash in registry)
marrow pull std.json.parse
```

---

## 7. CLI Help Text Mockups

### 7.1 Main CLI (`blood --help`)

```
Blood - A safety-critical systems programming language

Usage: blood <COMMAND> [OPTIONS]

Building:
  build      Compile the current project
  run        Build and run the current project
  check      Type-check without generating code
  clean      Remove build artifacts

Code Quality:
  fmt        Format source code (invokes clot)
  lint       Run static analysis (invokes scan)

Testing:
  test       Run tests (invokes culture)
  bench      Run benchmarks

Documentation:
  doc        Generate documentation (invokes codex)
  repl       Start interactive REPL (invokes beat)

Codebase (content-addressed):
  add        Add a definition to the codebase
  find       Find definitions by name or hash
  rename     Rename a definition (instant, free)
  deps       Show dependencies of a definition
  history    Show history of a name
  sync       Sync with bloodbank

Project:
  new        Create a new Blood project
  init       Initialize Blood in an existing directory

Options:
  -v, --verbose    Increase output verbosity
  -q, --quiet      Suppress non-error output
  -h, --help       Print help
  -V, --version    Print version

The `blood` CLI wraps thematic standalone tools. Power users
can invoke them directly:

  blood fmt   ↔  clot
  blood lint  ↔  scan
  blood test  ↔  culture run
  blood repl  ↔  beat
  blood add   ↔  marrow add

Learn more at https://blood.dev
```

### 7.2 Codebase Manager (`marrow --help`)

```
marrow - Content-addressed codebase manager for Blood

Blood stores code by hash, not by name. Names are just pointers.
This means renaming is free, sharing is trivial, and there's no
"dependency hell" - every reference is an exact hash.

Usage: marrow <COMMAND> [OPTIONS]

Adding Code:
  add <source>         Add definition, returns hash
  add --file <path>    Add from file

Finding Code:
  find <query>         Search by name, hash prefix, or pattern
  view <ref>           View source of a definition
  list                 List all names in codebase

Naming:
  rename <old> <new>   Rename a definition (instant!)
  alias <name> <hash>  Add a name pointing to hash
  unname <name>        Remove a name

Dependencies:
  deps <ref>           What does this definition use?
  dependents <ref>     What uses this definition?
  tree <ref>           Full dependency tree

History:
  history <name>       Show how a name evolved
  diff <h1> <h2>       Diff two definitions
  rollback <n> <h>     Point name to previous hash

Sharing (Bloodbank):
  push                 Push definitions to bloodbank
  pull <ref>           Pull definition by hash or name
  sync                 Bidirectional sync

Project:
  init                 Initialize new codebase
  stats                Show codebase statistics

Options:
  --codebase <PATH>    Path to codebase [default: .blood]
  -v, --verbose        Verbose output
  -h, --help           Print help

Examples:
  marrow add "fn double(x: Int) -> Int { x * 2 }"
  marrow alias math.double #a7f3b2c1
  marrow rename math.double utils.double  # Instant!
  marrow find double
  marrow deps utils.double
```

### 7.3 Formatter (`clot --help`)

```
clot - Code formatter for Blood

Usage: clot [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Files or directories to format [default: .]

Options:
  --check        Check formatting without modifying files
  --diff         Show diff of what would change
  --stdin        Read from stdin, write to stdout
  --config       Path to configuration file

Configuration ([clot] in Blood.toml):
  line_width = 100
  indent = 4
  trailing_comma = "always"

Examples:
  clot                     # Format entire project
  clot src/lib.blood       # Format single file
  clot --check             # CI mode: fail if unformatted
```

---

## 8. Standard Library Naming

### 8.1 Module Hierarchy

```
std::                      # Root namespace (or blood::)
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
│   ├── hash_map
│   ├── btree_map
│   ├── set
│   └── deque
├── io/                   # Input/Output
│   ├── read
│   ├── write
│   ├── file
│   └── net
├── effect/               # Effect system
│   ├── handler
│   ├── evidence
│   └── builtin
├── sync/                 # Synchronization
│   ├── mutex
│   ├── channel
│   └── atomic
├── hash/                 # Hashing (BLAKE3)
└── test/                 # Testing utilities
```

### 8.2 Naming Conventions

| Category | Convention | Example |
|----------|------------|---------|
| Modules | `snake_case` | `hash_map` |
| Types | `PascalCase` | `HashMap` |
| Functions | `snake_case` | `read_line` |
| Constants | `SCREAMING_SNAKE` | `MAX_SIZE` |
| Traits | `PascalCase` | `Read`, `Hash` |
| Effects | `PascalCase` | `IO`, `State` |

---

## 9. Project Conventions

### 9.1 Project Structure

```
my-project/
├── .blood/              # Codebase database (content-addressed)
│   ├── codebase.db     # SQLite: definitions, names, history
│   └── objects/        # Large binary objects
├── Blood.toml           # Project manifest
├── src/
│   ├── main.blood      # Entry point (binary)
│   └── lib.blood       # Entry point (library)
├── tests/
│   └── integration.blood
└── docs/
    └── guide.md
```

### 9.2 Manifest File (Blood.toml)

```toml
[package]
name = "my-project"
version = "0.1.0"
edition = "2026"
authors = ["Alice <alice@example.com>"]
license = "MIT OR Apache-2.0"

# Content-addressed dependencies (by hash)
[dependencies]
# Pull these hashes from bloodbank on build
http-client = "#a7f3b2c1d4e5f6..."
json-parser = "#b8c9d0e1f2a3..."

# Or reference by name (resolved to hash)
[dependencies.named]
regex = "std.regex"

# Tool configurations
[clot]
line_width = 100

[scan]
deny = ["unsafe_code"]
```

**Note:** In a content-addressed world, `version = "0.1.0"` is for human reference only. The actual identity of code is its hash.

---

## 10. File Extensions & Terminology

### 10.1 File Extensions

| Extension | Purpose |
|-----------|---------|
| `.blood` | Blood source files |
| `Blood.toml` | Project manifest |
| `.blood/` | Codebase directory |

### 10.2 Terminology

| Term | Definition | Context |
|------|------------|---------|
| **Definition** | A function, type, effect, or handler | Stored by hash |
| **Name** | A human-readable pointer to a hash | Metadata |
| **Hash** | BLAKE3-256 of AST structure | Identity |
| **Codebase** | Database of definitions and names | Local storage |
| **Bloodbank** | Shared registry of hashes | Remote storage |
| **Cell** | Compilation unit | Module system |

---

## 11. Implementation Roadmap

### Phase 1: Core (Current)

| Tool | Thematic Name | CLI Alias | Status |
|------|---------------|-----------|--------|
| Compiler | `bloodc` | `blood build` | ✓ Implemented |
| Unified CLI | `blood` | — | ✓ Implemented |
| Codebase Manager | `marrow` | `blood add/find` | In Progress |
| Formatter | `clot` | `blood fmt` | In Progress |

### Phase 2: Developer Experience

| Tool | Thematic Name | CLI Alias | Status |
|------|---------------|-----------|--------|
| Language Server | `vessel-ls` | — | Planned |
| Linter | `scan` | `blood lint` | Planned |
| REPL | `beat` | `blood repl` | Planned |
| Test Runner | `culture` | `blood test` | Planned |

### Phase 3: Ecosystem

| Tool | Thematic Name | Status |
|------|---------------|--------|
| Doc Generator | `codex` | Planned |
| Debugger | `draw` | Planned |
| Registry | bloodbank.dev | Planned |

---

## 12. Appendices

### A. Full Toolchain Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         blood (unified CLI)                           │
│    blood build | run | fmt | lint | test | repl | add | find          │
├──────────┬──────────┬──────────┬──────────┬──────────┬────────────────┤
│  bloodc  │  marrow  │   clot   │   scan   │   beat   │   vessel-ls    │
│ compiler │ codebase │ formatter│  linter  │   REPL   │      LSP       │
├──────────┴──────────┼──────────┴──────────┼──────────┴────────────────┤
│       culture       │        codex        │          draw             │
│     test runner     │     doc generator   │        debugger           │
├─────────────────────┴─────────────────────┴───────────────────────────┤
│                                                                        │
│    ┌────────────────────────────────────────────────────────────┐     │
│    │                    .blood/ (codebase)                       │     │
│    │  Definitions stored by BLAKE3 hash, names as metadata       │     │
│    └────────────────────────────────────────────────────────────┘     │
│                                  ↕                                     │
│    ┌────────────────────────────────────────────────────────────┐     │
│    │                bloodbank.dev (registry)                     │     │
│    │              Shared definitions by hash                     │     │
│    └────────────────────────────────────────────────────────────┘     │
└────────────────────────────────────────────────────────────────────────┘
```

### B. Comparison With Other Languages

| Aspect | Rust | Go | Unison | Blood |
|--------|------|----|----|-------|
| Unified CLI | `cargo` | `go` | `ucm` | `blood` |
| Package Manager | Cargo | Go Modules | Built-in | `marrow` |
| Content-Addressed | ❌ | ❌ | ✅ | ✅ |
| Formatter | `rustfmt` | `go fmt` | Built-in | `clot` |
| LSP | rust-analyzer | gopls | — | `vessel-ls` |

### C. Hybrid CLI Mapping

| Conventional | Thematic | Notes |
|--------------|----------|-------|
| `blood fmt` | `clot` | Formatter |
| `blood lint` | `scan` | Linter |
| `blood test` | `culture run` | Test runner |
| `blood bench` | `culture bench` | Benchmarks |
| `blood repl` | `beat` | REPL |
| `blood doc` | `codex build` | Documentation |
| `blood add` | `marrow add` | Add definition |
| `blood find` | `marrow find` | Find definition |
| `blood rename` | `marrow rename` | Rename (free!) |
| `blood deps` | `marrow deps` | Dependencies |
| `blood history` | `marrow history` | Name history |
| `blood sync` | `marrow sync` | Bloodbank sync |

### D. Community & Branding

| Resource | Name | URL |
|----------|------|-----|
| Package Registry | Bloodbank | `bloodbank.dev` |
| Documentation | Blood Book | `book.blood.dev` |
| Playground | The Donor Room | `play.blood.dev` |
| Community | The Bloodline | `community.blood.dev` |
| Blog | The Pulse | `blog.blood.dev` |

---

## Summary

This document provides a complete naming system for Blood's toolchain:

1. **Hybrid CLI** - `blood fmt` for beginners, `clot` for power users
2. **Content-Addressed** - `marrow` manages code by hash, not version
3. **Thematic Coherence** - All names reinforce the "written in blood" mission
4. **No Conflicts** - All names verified clear of major conflicts

The naming balances Blood's unique identity with practical developer experience.

---

*Document Version 3.0 — Updated to properly reflect content-addressed architecture*

*For questions or suggestions, open an issue at github.com/blood-lang/blood*
