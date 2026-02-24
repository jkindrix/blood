# Blood Codebase Manager (UCM) Specification

**Version**: 0.3.0
**Status**: Specified (Implementation Planned)
**Implementation**: 📋 Planned
**Last Updated**: 2026-01-10

**Implementation Status**: This document is a design specification for a planned component. Current Blood development uses standard file-based compilation via `bloodc`. UCM implementation is planned for a future release. See [ROADMAP.md](./ROADMAP.md).

**Revision 0.3.0 Changes**:
- Added implementation status clarification (specification only, not implemented)
- Added cross-reference to ROADMAP.md for implementation timeline

This document specifies the Blood Codebase Manager (UCM), the primary tool for managing Blood codebases, editing definitions, running programs, and collaborating with others.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Architecture](#2-architecture)
3. [Codebase Structure](#3-codebase-structure)
4. [Core Commands](#4-core-commands)
5. [Project Management](#5-project-management)
6. [History & Versioning](#6-history--versioning)
7. [Collaboration](#7-collaboration)
8. [Editor Integration](#8-editor-integration)
9. [Configuration](#9-configuration)
10. [Error Handling](#10-error-handling)

---

## 1. Overview

### 1.1 What is UCM?

UCM (Blood Codebase Manager) is the all-in-one tool for working with Blood code. Unlike traditional compilers that operate on files, UCM manages a content-addressed codebase where definitions are identified by hash.

```
┌─────────────────────────────────────────────────────────────┐
│                     UCM ARCHITECTURE                         │
│                                                              │
│  ┌──────────┐    ┌──────────┐    ┌──────────────────────┐   │
│  │  Editor  │◄──►│   UCM    │◄──►│  Codebase Database   │   │
│  │ (scratch │    │  (CLI/   │    │  (content-addressed) │   │
│  │  files)  │    │   LSP)   │    │                      │   │
│  └──────────┘    └──────────┘    └──────────────────────┘   │
│                       │                                      │
│                       ▼                                      │
│               ┌──────────────┐                              │
│               │   Runtime    │                              │
│               │ (execute,    │                              │
│               │  test, run)  │                              │
│               └──────────────┘                              │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 Design Philosophy

1. **Codebase, not files**: Code lives in a structured database, not loose files
2. **Names are metadata**: Definitions are identified by hash; names can change freely
3. **Structured editing**: UCM understands code semantically, not just textually
4. **Safe by default**: Operations are reversible; history is preserved
5. **Collaborative**: Built for sharing and distributed development

### 1.3 Related Specifications

- [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) — Hash computation, VFT, storage format
- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — Error message format
- [ROADMAP.md](./ROADMAP.md) — Implementation timeline

---

## 2. Architecture

### 2.1 Component Overview

```
blood/
├── ucm/                      # Codebase Manager
│   ├── cli/                  # Command-line interface
│   │   ├── commands/         # Individual command implementations
│   │   ├── repl/             # Interactive REPL
│   │   └── completion/       # Shell completions
│   ├── lsp/                  # Language Server Protocol
│   ├── core/                 # Core codebase operations
│   │   ├── codebase.rs       # Codebase abstraction
│   │   ├── namespace.rs      # Namespace management
│   │   ├── branch.rs         # Branch operations
│   │   └── history.rs        # Reflog and undo
│   ├── sync/                 # Collaboration/sharing
│   │   ├── push.rs
│   │   ├── pull.rs
│   │   └── share.rs
│   └── ui/                   # Local web UI (optional)
```

### 2.2 Codebase Abstraction

```rust
/// Core codebase interface
pub trait Codebase {
    /// Retrieve a definition by hash
    fn get(&self, hash: &Hash) -> Option<Definition>;

    /// Store a new definition, returns its hash
    fn put(&mut self, def: Definition) -> Hash;

    /// Resolve a name to a hash in the current namespace
    fn resolve(&self, name: &Name) -> Option<Hash>;

    /// Add a name binding
    fn bind(&mut self, name: Name, hash: Hash);

    /// Get all dependents of a definition
    fn dependents(&self, hash: &Hash) -> Vec<Hash>;

    /// Get all dependencies of a definition
    fn dependencies(&self, hash: &Hash) -> Vec<Hash>;
}
```

### 2.3 Session Model

UCM maintains session state for interactive use:

```rust
pub struct Session {
    /// Current project
    project: ProjectId,

    /// Current branch within project
    branch: BranchId,

    /// Current namespace path (e.g., "lib.collections")
    namespace: NamespacePath,

    /// Pending changes from scratch files
    pending: ScratchState,

    /// Undo/redo stack
    history: SessionHistory,
}
```

---

## 3. Codebase Structure

### 3.1 Directory Layout

```
~/.blood/
├── config.toml               # Global configuration
├── codebases/
│   └── default/              # Default codebase
│       ├── v1/               # Codebase format version
│       │   ├── terms/        # Term definitions (by hash prefix)
│       │   │   ├── a7/
│       │   │   │   └── f2k9m3xp5jht2ngqw4bc8rv6ys7dz1ef0il.term
│       │   │   └── ...
│       │   ├── types/        # Type definitions
│       │   ├── effects/      # Effect definitions
│       │   ├── patches/      # Update patches
│       │   └── branches/     # Branch state
│       ├── projects/         # Project metadata
│       └── cache/            # Compiled artifacts
└── share/                    # Shared/published code
```

### 3.2 Namespace Model

Namespaces provide hierarchical organization of names:

```
.                           # Root namespace
├── lib                     # Libraries
│   ├── std                 # Standard library
│   │   ├── io
│   │   ├── collections
│   │   └── ...
│   └── external            # External dependencies
│       ├── http@v2.1.0     # Versioned dependency
│       └── json@v1.5.0
├── src                     # Project source
│   ├── main
│   └── utils
└── test                    # Tests
```

### 3.3 Project Structure

```rust
pub struct Project {
    /// Unique project identifier
    id: ProjectId,

    /// Human-readable name
    name: String,

    /// Project metadata
    metadata: ProjectMetadata,

    /// Branches in this project
    branches: Vec<Branch>,

    /// Default branch (typically "main")
    default_branch: BranchId,
}

pub struct Branch {
    /// Branch identifier
    id: BranchId,

    /// Human-readable name
    name: String,

    /// Root namespace hash for this branch
    root: Hash,

    /// Parent branch (for feature branches)
    parent: Option<BranchId>,

    /// Creation timestamp
    created_at: Timestamp,
}
```

---

## 4. Core Commands

### 4.1 Definition Management

#### `blood add`

Adds new definitions from a scratch file to the codebase.

```bash
$ blood add
  ✓ Added: factorial #qr7st8uvwx
  ✓ Added: fibonacci #xy9ab0cdef

$ blood add path/to/scratch.blood
  ✓ Added 5 definitions
```

**Behavior:**
- Parses and type-checks scratch file
- Computes hashes for new definitions
- Stores definitions in codebase
- Binds names to hashes in current namespace

#### `blood update`

Updates existing definitions, propagating changes to dependents.

```bash
$ blood update
  ✓ Updated: add #a7f2k9 → #b3c1xp
  ✓ Propagated to 3 dependents:
    - sum #old123 → #new456
    - average #old789 → #newdef
    - total #oldabc → #newghi
```

**Behavior:**
- Like `add`, but replaces existing name bindings
- Automatically updates dependents if types are compatible
- Reports conflicts if type changes break dependents

#### `blood view`

Displays a definition by name or hash.

```bash
$ blood view factorial
fn factorial(n: i32) -> i32 / pure {
    if n <= 1 { 1 }
    else { n * factorial(n - 1) }
}

$ blood view #qr7st8uvwx
fn factorial(n: i32) -> i32 / pure { ... }

$ blood view --type factorial
factorial : fn(i32) -> i32 / pure
```

**Options:**
- `--type`: Show type signature only
- `--hash`: Show hash only
- `--deps`: Show dependencies
- `--raw`: Show canonical AST

#### `blood find`

Searches for definitions by name pattern.

```bash
$ blood find add
  add : #a7f2k9m3xp (lib.math)
  add : #c4d2yr6kiu (lib.vector)
  addAll : #e5f3zs7lmn (lib.collections)

$ blood find --type "fn(i32, i32) -> i32"
  add : #a7f2k9m3xp
  subtract : #b3c1xp5jht
  multiply : #c4d2yr6kiu
```

**Options:**
- `--type`: Search by type signature
- `--effect`: Search by effect signature
- `--namespace`: Limit to namespace
- `--limit`: Maximum results (default: 20)

#### `blood rename`

Renames a definition (changes metadata only, hash unchanged).

```bash
$ blood rename factorial fact
  ✓ Renamed: factorial → fact
  Hash unchanged: #qr7st8uvwx
```

#### `blood delete`

Removes a name binding (definition remains in codebase if referenced elsewhere).

```bash
$ blood delete old_function
  ✓ Deleted name binding: old_function
  Note: Definition #abc123 still referenced by 2 dependents

$ blood delete --force unused_function
  ✓ Deleted: unused_function #def456
  Definition removed (no remaining references)
```

#### `blood move`

Moves a definition to a different namespace.

```bash
$ blood move helpers.add lib.math.add
  ✓ Moved: helpers.add → lib.math.add
```

### 4.2 Running Code

#### `blood run`

Executes a term that requires IO effects.

```bash
$ blood run main
Hello, World!

$ blood run --args "input.txt" processFile
Processing: input.txt
Done.
```

#### `blood eval`

Evaluates a pure expression interactively.

```bash
$ blood eval "factorial(10)"
3628800

$ blood eval "fibonacci().take(10).collect()"
[0, 1, 1, 2, 3, 5, 8, 13, 21, 34]
```

#### `blood test`

Runs tests in the current namespace.

```bash
$ blood test
  ✓ test.factorial_zero (0.1ms)
  ✓ test.factorial_one (0.1ms)
  ✓ test.factorial_ten (0.2ms)
  ✗ test.factorial_negative (0.1ms)
    Expected: Error<InvalidInput>
    Got: 1

  3 passed, 1 failed
```

**Options:**
- `--filter`: Run matching tests only
- `--parallel`: Run tests in parallel
- `--coverage`: Report code coverage

### 4.3 Exploration

#### `blood ls`

Lists contents of current namespace.

```bash
$ blood ls
  Types:
    Point, Vector, Matrix

  Terms:
    add, subtract, multiply, divide
    factorial, fibonacci

  Effects:
    MathError

$ blood ls lib.std.io
  Types:
    File, Path, IoError

  Effects:
    IO
```

#### `blood deps`

Shows dependency graph.

```bash
$ blood deps factorial
  factorial #qr7st8uvwx
    └── multiply #c4d2yr6kiu
        └── Int.mul (builtin)

$ blood deps --reverse factorial
  Dependents of factorial #qr7st8uvwx:
    └── combinatorics #abc123
        └── probability #def456
```

#### `blood diff`

Shows differences between definitions or branches.

```bash
$ blood diff add#old add#new
  - fn add(x: i32, y: i32) -> i32 / pure {
  -     x + y
  - }
  + fn add(x: i32, y: i32) -> i32 / {Log} {
  +     log("Adding {} + {}", x, y)
  +     x + y
  + }

$ blood diff main feature/new-parser
  + parser.tokenize
  + parser.parse
  ~ parser.Expression (modified)
  - parser.old_helper
```

---

## 5. Project Management

### 5.1 Project Commands

#### `blood project.create`

Creates a new project.

```bash
$ blood project.create myapp
  ✓ Created project: myapp
  ✓ Created branch: main
  Switched to myapp/main
```

#### `blood project.list`

Lists all projects.

```bash
$ blood project.list
  * myapp (current)
    webapp
    library
```

#### `blood switch`

Switches between projects and branches.

```bash
$ blood switch webapp
  Switched to webapp/main

$ blood switch /feature
  Switched to myapp/feature

$ blood switch webapp/develop
  Switched to webapp/develop
```

### 5.2 Branch Commands

#### `blood branch`

Lists or creates branches.

```bash
$ blood branch
  * main
    feature/new-parser
    bugfix/memory-leak

$ blood branch feature/effects
  ✓ Created branch: feature/effects
  (based on main)
```

#### `blood merge`

Merges branches.

```bash
$ blood merge feature/new-parser
  ✓ Merged feature/new-parser into main
  + 5 definitions added
  ~ 2 definitions updated
  - 1 definition removed
```

---

## 6. History & Versioning

### 6.1 Reflog

Every codebase change is recorded in the reflog:

```bash
$ blood reflog
  #1a2b3c (HEAD) add: factorial, fibonacci
  #4d5e6f update: parser.Expression
  #7g8h9i delete: old_helper
  #0j1k2l merge: feature/effects
  #3m4n5o initial commit

$ blood reflog --branch main
$ blood reflog --project myapp
```

### 6.2 Undo/Redo

```bash
$ blood undo
  ✓ Undid: add factorial, fibonacci
  State restored to #4d5e6f

$ blood redo
  ✓ Redid: add factorial, fibonacci
  State restored to #1a2b3c
```

### 6.3 Reset

```bash
$ blood reset #4d5e6f
  ✓ Reset to #4d5e6f
  Warning: 2 commits will be orphaned

$ blood reset --hard #4d5e6f
  ✓ Hard reset to #4d5e6f
  Removed 2 unreferenced definitions
```

### 6.4 Tagging

```bash
$ blood tag v1.0.0
  ✓ Tagged current state as v1.0.0

$ blood tag --list
  v1.0.0 → #1a2b3c
  v0.9.0 → #7g8h9i
```

---

## 7. Collaboration

### 7.1 Sharing

#### `blood push`

Pushes code to a remote share.

```bash
$ blood push
  Pushing to share.blood-lang.org/username/myapp...
  ✓ Pushed 42 definitions

$ blood push --to share.company.com
  ✓ Pushed to private share
```

#### `blood pull`

Pulls code from a remote share.

```bash
$ blood pull username/library
  ✓ Pulled 15 definitions
  ✓ Installed as lib.library

$ blood pull username/library@v2.0.0
  ✓ Pulled specific version
```

### 7.2 Dependencies

#### `blood lib.install`

Installs a library dependency.

```bash
$ blood lib.install http
  ✓ Installed http@latest as lib.http

$ blood lib.install json@1.5.0
  ✓ Installed json@1.5.0 as lib.json
```

#### `blood lib.upgrade`

Upgrades a dependency.

```bash
$ blood lib.upgrade http
  Upgrading http 2.0.0 → 2.1.0...
  ✓ Updated 3 dependents automatically
  ⚠ 1 dependent requires manual update:
    - myHandler: new parameter required
```

#### `blood lib.list`

Lists installed dependencies.

```bash
$ blood lib.list
  lib.http     v2.1.0  (share.blood-lang.org/std/http)
  lib.json     v1.5.0  (share.blood-lang.org/std/json)
  lib.testing  v1.0.0  (share.blood-lang.org/std/testing)
```

---

## 8. Editor Integration

### 8.1 Language Server Protocol (LSP)

UCM provides an LSP server for editor integration:

```bash
$ blood lsp
  Blood LSP server listening on stdio...
```

**Supported features:**
- Go to definition (resolves hash, opens in codebase UI)
- Find references (across entire codebase)
- Hover (shows type, effects, documentation)
- Completion (namespace-aware, effect-aware)
- Diagnostics (type errors, effect mismatches)
- Code actions (extract function, inline, rename)
- Semantic highlighting

### 8.2 Scratch Files

Code is edited in scratch files (`.blood` extension) that are type-checked on save:

```bash
# Watch a scratch file for changes
$ blood watch scratch.blood

# Manually load a scratch file
$ blood load scratch.blood
  Parsing...
  Type-checking...
  ✓ Ready to add: 3 new definitions
```

### 8.3 LSP Extensions

Blood-specific LSP extensions:

```json
{
  "method": "blood/resolveHash",
  "params": { "hash": "#a7f2k9m3xp" },
  "result": {
    "name": "add",
    "type": "fn(i32, i32) -> i32 / pure",
    "namespace": "lib.math",
    "definition": "fn add(x, y) = x + y"
  }
}

{
  "method": "blood/showDependents",
  "params": { "hash": "#a7f2k9m3xp" },
  "result": {
    "dependents": [
      { "hash": "#qr7st8", "name": "sum" },
      { "hash": "#xy9ab0", "name": "average" }
    ]
  }
}

{
  "method": "blood/effectInfo",
  "params": { "position": { "line": 10, "character": 5 } },
  "result": {
    "effects": ["IO", "Error<ParseError>"],
    "handled": ["Error<ParseError>"],
    "unhandled": ["IO"]
  }
}
```

---

## 9. Configuration

### 9.1 Global Configuration

`~/.blood/config.toml`:

```toml
[user]
name = "Jane Developer"
email = "jane@example.com"

[defaults]
project = "myapp"
share = "share.blood-lang.org"

[editor]
command = "code --wait"
scratch_dir = "~/blood-scratch"

[cache]
max_size = "10GB"
compiled_artifacts = true

[lsp]
port = 8765
log_level = "info"

[share.blood-lang-org]
url = "https://share.blood-lang.org"
token_file = "~/.blood/tokens/blood-lang-org"

[share.company]
url = "https://share.company.com"
token_file = "~/.blood/tokens/company"
```

### 9.2 Project Configuration

`.blood/project.toml` in project root:

```toml
[project]
name = "myapp"
version = "1.0.0"
description = "My Blood application"

[dependencies]
http = "^2.0"
json = "1.5.0"

[dev-dependencies]
testing = "^1.0"
benchmark = "^0.5"

[build]
target = "x86_64-linux"
optimization = "release"

[test]
parallel = true
timeout = "30s"
```

---

## 10. Error Handling

### 10.1 Error Categories

| Category | Exit Code | Description |
|----------|-----------|-------------|
| Success | 0 | Operation completed |
| Parse Error | 1 | Syntax error in scratch file |
| Type Error | 2 | Type checking failed |
| Name Error | 3 | Unresolved name |
| Conflict | 4 | Merge conflict or incompatible update |
| Network Error | 5 | Failed to connect to share |
| IO Error | 6 | File system error |
| Internal Error | 100 | UCM bug (please report) |

### 10.2 Error Format

Errors follow the format specified in [DIAGNOSTICS.md](./DIAGNOSTICS.md):

```
error[E0001]: type mismatch
  --> scratch.blood:10:5
   |
10 |     add("hello", 42)
   |         ^^^^^^^ expected i32, found String
   |
   = note: function signature: fn add(i32, i32) -> i32
   = help: convert string to integer with parse()?
```

### 10.3 Recovery

UCM is designed for safe operation:

- All mutations are logged to reflog
- `undo` always available for recent changes
- `reset --hard` required for destructive operations
- Garbage collection only runs on orphaned definitions after retention period

---

## Appendix A: Command Reference Quick Reference

| Command | Description |
|---------|-------------|
| `blood add` | Add definitions from scratch file |
| `blood update` | Update definitions, propagate changes |
| `blood view <name>` | View definition |
| `blood find <pattern>` | Search for definitions |
| `blood rename <old> <new>` | Rename definition |
| `blood delete <name>` | Delete name binding |
| `blood move <from> <to>` | Move to namespace |
| `blood run <term>` | Run IO term |
| `blood eval <expr>` | Evaluate pure expression |
| `blood test` | Run tests |
| `blood ls` | List namespace contents |
| `blood deps <name>` | Show dependencies |
| `blood diff` | Show differences |
| `blood project.create` | Create project |
| `blood switch` | Switch project/branch |
| `blood branch` | List/create branches |
| `blood merge` | Merge branches |
| `blood reflog` | Show history |
| `blood undo` / `blood redo` | Undo/redo changes |
| `blood reset` | Reset to state |
| `blood tag` | Create/list tags |
| `blood push` | Push to share |
| `blood pull` | Pull from share |
| `blood lib.install` | Install dependency |
| `blood lib.upgrade` | Upgrade dependency |
| `blood lsp` | Start LSP server |
| `blood watch` | Watch scratch file |

---

## Appendix B: Comparison with Unison UCM

Blood's UCM is inspired by [Unison's UCM](https://www.unison-lang.org/docs/ucm-commands/) with adaptations for Blood's features:

| Feature | Unison UCM | Blood UCM |
|---------|------------|-----------|
| Content addressing | SHA3-512 | BLAKE3-256 |
| Effect tracking | Abilities | Algebraic effects |
| Hot-swap | Not supported | Supported (see VFT) |
| Type system | Unison types | Blood types + effects |
| Memory model | GC | Generational references |
| Multiple dispatch | No | Yes |

---

## Revision History

| Version | Date | Changes |
|---------|------|---------|
| 0.3.0 | 2026-01-11 | Updated status, implementation planned |
| 0.1.0 | 2026-01-09 | Initial specification |

---

*This is a design specification for planned functionality. See [ROADMAP.md](./ROADMAP.md) for implementation timeline.*
