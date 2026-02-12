# Blood Developer Tooling Specification

**Version**: 0.1.0
**Status**: Specified
**Last Updated**: 2026-01-14

This document provides comprehensive documentation for Blood's developer tooling ecosystem: the Language Server Protocol (LSP) implementation, code formatter, and IDE integrations.

---

## Table of Contents

1. [Overview](#1-overview)
2. [blood-lsp: Language Server](#2-blood-lsp-language-server)
3. [blood-fmt: Code Formatter](#3-blood-fmt-code-formatter)
4. [IDE Integration](#4-ide-integration)
5. [CI/CD Integration](#5-cicd-integration)
6. [Configuration Reference](#6-configuration-reference)

---

## 1. Overview

### 1.1 Tooling Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Developer Workflow                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐ │
│   │  Editor  │    │  blood   │    │  blood   │    │  blood   │ │
│   │  (IDE)   │    │   lsp    │    │   fmt    │    │   cli    │ │
│   └────┬─────┘    └────┬─────┘    └────┬─────┘    └────┬─────┘ │
│        │               │               │               │        │
│        └───────┬───────┴───────┬───────┴───────┬───────┘        │
│                │               │               │                 │
│                ▼               ▼               ▼                 │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │                      bloodc                              │   │
│   │              (Compiler Infrastructure)                   │   │
│   │                                                          │   │
│   │  ┌─────┐  ┌──────┐  ┌───────┐  ┌───────┐  ┌─────────┐  │   │
│   │  │Lexer│  │Parser│  │TypeCk │  │MIR/HIR│  │ CodeGen │  │   │
│   │  └─────┘  └──────┘  └───────┘  └───────┘  └─────────┘  │   │
│   └─────────────────────────────────────────────────────────┘   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 1.2 Tool Summary

| Tool | Purpose | Status |
|------|---------|--------|
| `blood-lsp` | Real-time IDE support via LSP | Implemented |
| `blood-fmt` | Automatic code formatting | Implemented |
| `bloodc` | Core compiler | Implemented |
| `blood run` | Build and run | Implemented |
| `blood check` | Type checking without codegen | Implemented |
| `blood test` | Test runner | Planned |
| `blood doc` | Documentation generator | Planned |

### 1.3 Related Documentation

- [DEBUGGING_GUIDE.md](../tutorials/DEBUGGING_GUIDE.md) - Debugging and profiling
- [BLOOD_FMT.md](./BLOOD_FMT.md) - Formatter style reference
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) - Error code reference

---

## 2. blood-lsp: Language Server

### 2.1 Overview

`blood-lsp` implements the Language Server Protocol, providing IDE features for Blood source files. It runs as a separate process, communicating with editors via JSON-RPC over stdin/stdout.

### 2.2 Supported Features

| Feature | Status | Description |
|---------|--------|-------------|
| **Diagnostics** | ✅ Implemented | Real-time error and warning reporting |
| **Hover** | ✅ Implemented | Documentation for keywords, types |
| **Completion** | ✅ Implemented | Keyword and basic completions |
| **Go to Definition** | ✅ Implemented | Navigate to symbol definitions |
| **Document Symbols** | ✅ Implemented | Outline view (functions, types, effects) |
| **Semantic Tokens** | ✅ Implemented | Full syntax highlighting |
| **Inlay Hints** | ✅ Implemented | Type and effect annotations |
| **Code Lens** | ✅ Implemented | Run/Test buttons, handler navigation |
| **Folding Ranges** | ✅ Implemented | Code folding support |
| **Signature Help** | ❌ Not yet | Function parameter help |
| **Find References** | ❌ Not yet | Find all usages |
| **Rename** | ❌ Not yet | Rename symbols |
| **Code Actions** | ❌ Not yet | Quick fixes |
| **Formatting** | ❌ Not yet | Via LSP (use blood-fmt directly) |

### 2.3 Installation

```bash
# Build from source
$ cd blood/blood-tools/lsp
$ cargo build --release

# Install to PATH
$ cargo install --path .

# Verify installation
$ blood-lsp --version
blood-lsp 0.1.0
```

### 2.4 Running the Server

```bash
# Standard mode (stdio)
$ blood-lsp --stdio

# With logging
$ RUST_LOG=blood_lsp=debug blood-lsp --stdio 2>lsp.log

# TCP mode (for debugging)
$ blood-lsp --tcp --port 9257
```

### 2.5 Feature Details

#### 2.5.1 Diagnostics

The LSP provides real-time diagnostics from the Blood compiler:

**Supported Diagnostic Types:**
- Parse errors (E0100-E0199)
- Type errors (E0200-E0299)
- Effect errors (E0300-E0399)
- Warnings (W0xxx)

**Update Triggers:**
- File open
- File save
- Document change (debounced, 300ms)

**Example Diagnostic:**
```json
{
  "range": {
    "start": { "line": 5, "character": 12 },
    "end": { "line": 5, "character": 18 }
  },
  "severity": 1,
  "code": "E0201",
  "source": "blood",
  "message": "type mismatch: expected `i32`, found `String`"
}
```

#### 2.5.2 Hover Information

Hover provides documentation for Blood constructs:

**Keyword Documentation:**
- `fn`, `let`, `struct`, `enum`, `trait`, `impl`
- `effect`, `handler`, `perform`, `resume`, `handle`
- `match`, `if`, `while`, `for`, `loop`
- `pure`, `linear`

**Type Documentation:**
- `Option<T>`, `Result<T, E>`
- `Box<T>`, `Vec<T>`, `String`
- `Frozen<T>`

**Example Hover (Markdown):**
```markdown
Declares a function.

```blood
fn name(params) -> ReturnType / Effects {
    body
}
```
```

#### 2.5.3 Completions

Completion triggers and types:

| Trigger | Completion Type |
|---------|-----------------|
| `.` | Member access |
| `::` | Path completion |
| `/` | Effect annotation |
| `<` | Type parameters |
| `(` | Function arguments |

**Keyword Completions:**
- Control flow: `if`, `else`, `match`, `while`, `for`, `loop`
- Declarations: `fn`, `let`, `struct`, `enum`, `trait`, `impl`
- Effects: `effect`, `handler`, `perform`, `resume`, `handle`, `with`
- Modifiers: `pub`, `mut`, `pure`, `linear`

#### 2.5.4 Inlay Hints

Automatic hints displayed inline:

**Type Hints:**
```blood
let x = 42;        // Displays: `: i32` after `x`
let y = compute(); // Displays: `: ReturnType` after `y`
```

**Effect Hints:**
```blood
fn process() -> i32 {  // Displays: `/ pure` before `{`
    42
}
```

**Parameter Hints:**
```blood
calculate(42, "name");  // Displays: `count: `, `name: `
```

#### 2.5.5 Code Lens

Code lens annotations:

| Context | Lens | Command |
|---------|------|---------|
| `fn main()` | "Run" | `blood.run` |
| `fn test_*()` | "Run Test" | `blood.runTest` |
| `#[test]` | "Run Test" | `blood.runTest` |
| `effect Name` | "Find Handlers" | `blood.findHandlers` |
| `handler X for Y` | "Go to Effect" | `blood.goToEffect` |

#### 2.5.6 Document Symbols

Symbol kinds supported:

| Blood Construct | Symbol Kind |
|-----------------|-------------|
| `fn` | Function |
| `struct` | Struct |
| `enum` | Enum |
| `trait` | Interface |
| `effect` | Interface |
| `handler` | Class |
| `const` | Constant |
| `type` | TypeParameter |

### 2.6 Configuration

**blood-lsp.toml** (project root):
```toml
[diagnostics]
# Enable/disable specific diagnostics
enable_warnings = true
enable_hints = false

[inlay_hints]
# Type hint settings
show_type_hints = true
show_effect_hints = true
show_parameter_hints = true

# Maximum hint length before truncation
max_hint_length = 30

[completion]
# Enable auto-import suggestions
auto_import = true

# Maximum completion items
max_completions = 100
```

### 2.7 Troubleshooting

**Common Issues:**

| Issue | Cause | Solution |
|-------|-------|----------|
| LSP not starting | Not in PATH | Add blood-lsp to PATH |
| No diagnostics | File not saved | Save file or enable incremental sync |
| Wrong errors | Stale state | Restart LSP server |
| High CPU | Large project | Increase debounce timeout |
| Completions slow | Complex project | Check workspace size |

**Debug Logging:**
```bash
# Enable all logging
$ RUST_LOG=blood_lsp=trace blood-lsp --stdio 2>lsp.log

# View logs in real-time
$ tail -f lsp.log
```

---

## 3. blood-fmt: Code Formatter

### 3.1 Overview

`blood-fmt` enforces a single canonical style for Blood code. It's opinionated by design—there's one correct way to format Blood code.

### 3.2 Key Style Rules

| Aspect | Rule |
|--------|------|
| Indentation | 4 spaces (no tabs) |
| Line length | 100 characters max |
| Braces | Same line as statement |
| Trailing commas | Always in multi-line |
| Import order | Std → External → Local |

### 3.3 Command Reference

```bash
# Format single file
$ blood fmt src/main.blood

# Format directory
$ blood fmt src/

# Format entire project
$ blood fmt

# Check without modifying (CI mode)
$ blood fmt --check

# Show diff of changes
$ blood fmt --diff

# Read from stdin
$ cat file.blood | blood fmt --stdin

# Verbose output
$ blood fmt --verbose
```

### 3.4 Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (formatted or already formatted) |
| 1 | Check failed (files need formatting) |
| 2 | Error (syntax error, I/O error) |

### 3.5 Configuration

**blood-fmt.toml** (project root):
```toml
# Maximum line width
max_width = 100

# Files to ignore
ignore = [
    "generated/**",
    "vendor/**",
]

# Per-path overrides
[overrides."tests/fixtures/**"]
max_width = 120
```

**Inline Directives:**
```blood
// Disable formatting for section
// blood-fmt: off
fn generated() { /* ... */ }
// blood-fmt: on

// Skip next item
// blood-fmt: skip
const TABLE: [u8; 256] = [ /* ... */ ];
```

### 3.6 Performance

| Metric | Target |
|--------|--------|
| Single file | < 10ms |
| 10K LOC file | < 100ms |
| 100K LOC project | < 5s |

---

## 4. IDE Integration

### 4.1 Visual Studio Code

**Extension Setup:**

1. Install the Blood extension from VS Code Marketplace
2. Configure settings:

```json
// .vscode/settings.json
{
  // LSP settings
  "blood.lsp.path": "blood-lsp",
  "blood.lsp.trace.server": "verbose",

  // Formatting
  "blood.formatOnSave": true,
  "blood.formatter.path": "blood",

  // Inlay hints
  "blood.inlayHints.typeHints": true,
  "blood.inlayHints.effectHints": true,
  "blood.inlayHints.parameterHints": true,

  // Diagnostics
  "blood.diagnostics.enableWarnings": true
}
```

**Recommended Extensions:**
- Blood Language Support (official)
- Error Lens (inline error display)

### 4.2 Neovim

**With nvim-lspconfig:**

```lua
-- init.lua or lua/plugins/lsp.lua

local lspconfig = require('lspconfig')

-- Add Blood LSP configuration
local configs = require('lspconfig.configs')
if not configs.blood_lsp then
  configs.blood_lsp = {
    default_config = {
      cmd = { 'blood-lsp', '--stdio' },
      filetypes = { 'blood' },
      root_dir = lspconfig.util.root_pattern('Blood.toml', '.git'),
      settings = {},
    },
  }
end

lspconfig.blood_lsp.setup{
  on_attach = function(client, bufnr)
    -- Enable inlay hints (Neovim 0.10+)
    if client.server_capabilities.inlayHintProvider then
      vim.lsp.inlay_hint.enable(true, { bufnr = bufnr })
    end
  end,
  capabilities = require('cmp_nvim_lsp').default_capabilities(),
}
```

**File type detection:**
```lua
-- ftdetect/blood.lua
vim.filetype.add({
  extension = {
    blood = 'blood',
  },
})
```

**Format on save:**
```lua
-- Auto-format Blood files on save
vim.api.nvim_create_autocmd("BufWritePre", {
  pattern = "*.blood",
  callback = function()
    vim.lsp.buf.format({ async = false })
  end,
})
```

### 4.3 Helix

**Configuration:**

```toml
# ~/.config/helix/languages.toml

[[language]]
name = "blood"
scope = "source.blood"
injection-regex = "blood"
file-types = ["blood"]
roots = ["Blood.toml"]
comment-token = "//"
indent = { tab-width = 4, unit = "    " }
language-servers = ["blood-lsp"]
formatter = { command = "blood", args = ["fmt", "--stdin"] }
auto-format = true

[language-server.blood-lsp]
command = "blood-lsp"
args = ["--stdio"]
```

### 4.4 Emacs

**With lsp-mode:**

```elisp
;; Blood language support
(use-package blood-mode
  :mode "\\.blood\\'"
  :hook (blood-mode . lsp-deferred))

;; LSP configuration
(with-eval-after-load 'lsp-mode
  (add-to-list 'lsp-language-id-configuration
    '(blood-mode . "blood"))

  (lsp-register-client
    (make-lsp-client
      :new-connection (lsp-stdio-connection '("blood-lsp" "--stdio"))
      :major-modes '(blood-mode)
      :server-id 'blood-lsp)))

;; Format on save
(add-hook 'blood-mode-hook
  (lambda ()
    (add-hook 'before-save-hook #'blood-format-buffer nil t)))

(defun blood-format-buffer ()
  "Format buffer with blood-fmt."
  (interactive)
  (let ((point (point)))
    (shell-command-on-region
      (point-min) (point-max)
      "blood fmt --stdin" (current-buffer) t)
    (goto-char point)))
```

### 4.5 JetBrains IDEs (IntelliJ, CLion)

**Plugin Setup:**

1. Install Blood plugin from JetBrains Marketplace
2. Configure external tools:

```
Settings > Tools > External Tools > Add

Name: Blood Format
Program: blood
Arguments: fmt $FilePath$
Working directory: $ProjectFileDir$

Settings > Keymap
Search: Blood Format
Assign shortcut: Ctrl+Alt+L (or preferred)
```

**File Watcher for auto-format:**
```
Settings > Tools > File Watchers > Add

File type: Blood
Program: blood
Arguments: fmt $FilePath$
Output paths: $FilePath$
```

### 4.6 Sublime Text

**Package Setup:**

```json
// Blood.sublime-settings
{
  "extensions": ["blood"],
  "tab_size": 4,
  "translate_tabs_to_spaces": true
}
```

**LSP Configuration:**
```json
// LSP.sublime-settings
{
  "clients": {
    "blood": {
      "enabled": true,
      "command": ["blood-lsp", "--stdio"],
      "selector": "source.blood"
    }
  }
}
```

---

## 5. CI/CD Integration

### 5.1 GitHub Actions

**Format Check:**
```yaml
# .github/workflows/check.yml
name: Check

on: [push, pull_request]

jobs:
  format:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Blood
        run: |
          curl -sSf https://blood-lang.org/install.sh | sh
          echo "$HOME/.blood/bin" >> $GITHUB_PATH

      - name: Check formatting
        run: blood fmt --check

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Blood
        run: |
          curl -sSf https://blood-lang.org/install.sh | sh
          echo "$HOME/.blood/bin" >> $GITHUB_PATH

      - name: Type check
        run: blood check --deny warnings
```

**Full CI Pipeline:**
```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: blood-lang/setup-blood@v1
        with:
          version: 'stable'

      - name: Format check
        run: blood fmt --check

      - name: Type check
        run: blood check

      - name: Build
        run: blood build --release

      - name: Test
        run: blood test
```

### 5.2 GitLab CI

```yaml
# .gitlab-ci.yml
stages:
  - check
  - build
  - test

format:
  stage: check
  image: blood-lang/blood:latest
  script:
    - blood fmt --check

typecheck:
  stage: check
  image: blood-lang/blood:latest
  script:
    - blood check --deny warnings

build:
  stage: build
  image: blood-lang/blood:latest
  script:
    - blood build --release
  artifacts:
    paths:
      - target/release/

test:
  stage: test
  image: blood-lang/blood:latest
  script:
    - blood test
```

### 5.3 Pre-commit Hooks

**Setup:**
```bash
# Install pre-commit
$ pip install pre-commit

# Create configuration
$ cat > .pre-commit-config.yaml << 'EOF'
repos:
  - repo: local
    hooks:
      - id: blood-fmt
        name: blood-fmt
        entry: blood fmt --check
        language: system
        files: \.blood$
        pass_filenames: false
EOF

# Install hooks
$ pre-commit install
```

**Manual Git Hook:**
```bash
#!/bin/sh
# .git/hooks/pre-commit

# Check formatting
blood fmt --check
if [ $? -ne 0 ]; then
    echo "Error: Code is not formatted."
    echo "Run 'blood fmt' to fix formatting."
    exit 1
fi

# Type check
blood check
if [ $? -ne 0 ]; then
    echo "Error: Type checking failed."
    exit 1
fi

exit 0
```

### 5.4 Docker Integration

**Dockerfile for Blood projects:**
```dockerfile
# Build stage
FROM blood-lang/blood:latest AS builder

WORKDIR /app
COPY . .

RUN blood build --release

# Runtime stage
FROM debian:bookworm-slim

COPY --from=builder /app/target/release/myapp /usr/local/bin/

ENTRYPOINT ["/usr/local/bin/myapp"]
```

**Multi-stage with caching:**
```dockerfile
FROM blood-lang/blood:latest AS deps

WORKDIR /app
COPY Blood.toml Blood.lock ./
RUN blood fetch

FROM deps AS builder
COPY . .
RUN blood build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/myapp /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/myapp"]
```

---

## 6. Configuration Reference

### 6.1 Project Configuration

**Blood.toml** (project manifest):
```toml
[package]
name = "my-project"
version = "0.1.0"
authors = ["Author <author@example.com>"]
edition = "2026"

[dependencies]
http = "1.0"
json = "2.0"

[dev-dependencies]
test-utils = "0.1"

[build]
# Optimization level for release builds
opt-level = 3

# Enable LTO
lto = true

[lsp]
# LSP-specific settings
inlay_hints = true
```

### 6.2 Editor-Agnostic Settings

**blood.toml** (user config in ~/.config/blood/):
```toml
[formatter]
# Use tabs (not recommended)
use_tabs = false
tab_width = 4
max_width = 100

[lsp]
# Diagnostics settings
diagnostics.enable_warnings = true
diagnostics.enable_hints = false

# Inlay hint settings
inlay_hints.type_hints = true
inlay_hints.effect_hints = true
inlay_hints.parameter_hints = true

[build]
# Default optimization
opt_level = 2

# Number of parallel jobs
jobs = 4
```

### 6.3 Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `BLOOD_HOME` | Blood installation directory | `~/.blood` |
| `BLOOD_CACHE` | Build cache directory | `~/.blood/cache` |
| `BLOOD_LOG` | Log level (error, warn, info, debug, trace) | `warn` |
| `BLOOD_VERBOSE` | Enable verbose output | `0` |
| `BLOOD_COLOR` | Color output (auto, always, never) | `auto` |

### 6.4 LSP Environment Variables

| Variable | Description |
|----------|-------------|
| `BLOOD_LSP_LOG` | LSP-specific log level |
| `BLOOD_LSP_CACHE` | LSP analysis cache directory |
| `RUST_LOG` | Rust logging (for debugging) |

---

## Appendix A: File Patterns

| Pattern | Description |
|---------|-------------|
| `*.blood` | Blood source files |
| `Blood.toml` | Project manifest |
| `Blood.lock` | Dependency lock file |
| `blood-fmt.toml` | Formatter configuration |
| `blood-lsp.toml` | LSP configuration |

---

## Appendix B: Syntax Highlighting

Blood syntax highlighting uses semantic tokens:

| Token Type | Blood Constructs |
|------------|------------------|
| `keyword` | `fn`, `let`, `struct`, `if`, `match`, etc. |
| `type` | Type names, generics |
| `function` | Function names |
| `variable` | Variable names |
| `parameter` | Function parameters |
| `property` | Struct fields |
| `enumMember` | Enum variants |
| `string` | String literals |
| `number` | Numeric literals |
| `comment` | Comments |
| `operator` | Operators |
| `namespace` | Module paths |

**Effect-specific tokens:**
| Token Type | Blood Constructs |
|------------|------------------|
| `interface` | Effect definitions |
| `class` | Handler definitions |
| `macro` | `perform`, `resume` |

---

*Last updated: 2026-01-14*
