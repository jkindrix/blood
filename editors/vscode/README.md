# Blood Language — VS Code Extension

Language support for the [Blood](https://github.com/blood-lang/blood) programming language.

## Features

- **Syntax highlighting** via TextMate grammar
- **Inline diagnostics** on save (via `blood check`)
- **Language Server** with hover, go-to-definition, completions, references, code lens, inlay hints, semantic tokens, rename
- **Format on save** (via `blood-fmt`)
- **Run / Check** commands with keybindings
- **Code snippets** for common patterns

## Installation

### Prerequisites

1. **Blood compiler** — install via `./build_selfhost.sh install` from the repo root, or set `blood.path` in VS Code settings to the compiler binary path.

2. **Blood Language Server** — build and install:
   ```bash
   cd src/bootstrap
   cargo build -p blood-lsp --release
   cp target/release/blood-lsp ~/.blood/bin/
   ```
   Or set `blood.lsp.path` in VS Code settings to the absolute path.

3. Ensure `~/.blood/bin` is in your `PATH`:
   ```bash
   export PATH="$HOME/.blood/bin:$PATH"
   ```

### Install the Extension

From the repository:
```bash
cd editors/vscode
npm install
npm run compile
```

Then install in VS Code:
```bash
# Development mode (symlink)
code --install-extension .

# Or package as .vsix
npx vsce package
code --install-extension blood-lang-0.1.0.vsix
```

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `blood.path` | `blood` | Path to the blood compiler |
| `blood.lsp.enable` | `true` | Enable the Language Server |
| `blood.lsp.path` | `blood-lsp` | Path to the LSP binary |
| `blood.checkOnSave` | `true` | Run `blood check` on save |
| `blood.format.onSave` | `true` | Format on save via `blood-fmt` |
| `blood.inlayHints.enable` | `true` | Show inlay hints |
| `blood.inlayHints.typeHints` | `true` | Show type hints for variables |
| `blood.inlayHints.effectHints` | `true` | Show effect hints for calls |
| `blood.trace.server` | `off` | LSP communication tracing |

## Keybindings

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+R` | Run current file |
| `Ctrl+Shift+F` | Format current file |

## Development

```bash
cd editors/vscode
npm install
npm run watch    # Recompile on changes
```

Press F5 in VS Code to launch an Extension Development Host for testing.
