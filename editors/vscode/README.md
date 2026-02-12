# Blood Language Support for VS Code

Full language support for the [Blood programming language](https://blood-lang.org) in Visual Studio Code.

## Features

- **Syntax Highlighting**: Full syntax highlighting for Blood source files
- **Language Server Protocol**: Rich IDE features via blood-lsp
- **Code Completion**: Intelligent auto-completion
- **Go to Definition**: Navigate to symbol definitions
- **Find References**: Find all references to a symbol
- **Hover Information**: Type information and documentation on hover
- **Signature Help**: Function signature help while typing
- **Code Formatting**: Format code with blood-fmt
- **Linting**: Real-time error checking
- **Inlay Hints**: Type and effect hints inline
- **Snippets**: Common code patterns

## Installation

### From VS Code Marketplace

1. Open VS Code
2. Go to Extensions (Ctrl+Shift+X)
3. Search for "Blood Language"
4. Click Install

### From VSIX

1. Download the `.vsix` file from releases
2. In VS Code, run "Extensions: Install from VSIX"
3. Select the downloaded file

### Prerequisites

- [Blood compiler](https://blood-lang.org/install) installed
- `blood` and `blood-lsp` in your PATH

## Usage

### Running Blood Code

- **Ctrl+Shift+R** (Cmd+Shift+R on macOS): Run current file
- Right-click in editor and select "Run Current File"

### Formatting

- **Ctrl+Shift+F** (Cmd+Shift+F on macOS): Format current file
- Enable format on save in settings

### Commands

| Command | Description |
|---------|-------------|
| `Blood: Restart Language Server` | Restart the Blood LSP server |
| `Blood: Run Current File` | Execute the current file |
| `Blood: Check Current File` | Type-check without running |
| `Blood: Format Current File` | Format with blood-fmt |
| `Blood: Show Blood Output` | View compiler output |
| `Blood: Open Blood Documentation` | Open docs in browser |
| `Blood: Show Effects for Function` | Display effects at cursor |

## Configuration

### Settings

```json
{
  // Path to blood executable
  "blood.path": "blood",

  // Enable/disable language server
  "blood.lsp.enable": true,

  // Path to blood-lsp executable
  "blood.lsp.path": "blood-lsp",

  // Enable formatting
  "blood.format.enable": true,

  // Format on save
  "blood.format.onSave": true,

  // Enable linting
  "blood.lint.enable": true,

  // Run check on save
  "blood.checkOnSave": true,

  // Inlay hints
  "blood.inlayHints.enable": true,
  "blood.inlayHints.typeHints": true,
  "blood.inlayHints.effectHints": true,
  "blood.inlayHints.parameterHints": true
}
```

## Syntax Highlighting

The extension provides highlighting for:

- Keywords (`fn`, `let`, `struct`, `enum`, etc.)
- Effect keywords (`effect`, `handler`, `do`, `resume`, `with`)
- Types (built-in and user-defined)
- Functions and methods
- Strings and characters
- Numbers (decimal, hex, binary, octal)
- Comments (line, block, documentation)
- Attributes
- Operators

## Snippets

Type these prefixes and press Tab:

| Prefix | Description |
|--------|-------------|
| `fn` | Function definition |
| `fneff` | Function with effects |
| `main` | Main function |
| `struct` | Struct definition |
| `enum` | Enum definition |
| `effect` | Effect definition |
| `handler` | Handler definition |
| `with` | Handler scope |
| `do` | Effect operation |
| `match` | Match expression |
| `test` | Test function |
| `derive` | Derive attribute |

## Troubleshooting

### Language Server Not Starting

1. Verify `blood-lsp` is installed: `which blood-lsp`
2. Check the path in settings
3. View output: "Blood: Show Blood Output"

### Formatting Not Working

1. Verify `blood` is installed: `blood --version`
2. Check for syntax errors (formatter requires valid code)
3. Ensure `blood.format.enable` is true

### Inlay Hints Not Showing

1. Ensure `blood.inlayHints.enable` is true
2. Some hints only show on hover (VS Code behavior)
3. Check individual hint settings

## Development

### Building from Source

```bash
cd editors/vscode
npm install
npm run compile
```

### Testing

```bash
npm run test
```

### Packaging

```bash
npm run package
```

## Contributing

Contributions welcome! Please see our [contributing guide](../../CONTRIBUTING.md).

## License

MIT License - see [LICENSE](LICENSE)

## Links

- [Blood Language](https://blood-lang.org)
- [Documentation](https://blood-lang.org/docs)
- [GitHub](https://github.com/blood-lang/blood)
- [Issue Tracker](https://github.com/blood-lang/blood/issues)
