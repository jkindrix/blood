# blood-doc: Blood Documentation Generator

**Version**: 1.0
**Status**: Specification
**Tool**: `blood doc`

## Overview

`blood-doc` generates beautiful, searchable API documentation from Blood source code. It extracts documentation comments, type signatures, and examples to produce HTML documentation similar to Rust's `rustdoc`.

## Features

- Automatic API documentation from doc comments
- Cross-referenced type links
- Full-text search
- Example code extraction and testing
- Multiple output formats (HTML, JSON, Markdown)
- Custom themes and templates
- Effect documentation

## Documentation Comments

### Comment Syntax

```blood
/// Single-line doc comment for items
/// Multiple lines continue the documentation

/**
 * Block doc comment
 * Also valid but less common
 */

//! Module-level documentation
//! Applies to the containing module

/*!
 * Block module documentation
 */
```

### Markdown Support

Doc comments support CommonMark Markdown:

```blood
/// # Header
///
/// Regular paragraph with **bold**, *italic*, and `code`.
///
/// ## Lists
///
/// - Item one
/// - Item two
///   - Nested item
///
/// ## Code
///
/// ```blood
/// let x = 42;
/// ```
///
/// ## Links
///
/// See [`OtherType`] for more info.
/// External link: [Blood website](https://blood-lang.org)
///
/// ## Tables
///
/// | Column 1 | Column 2 |
/// |----------|----------|
/// | Cell 1   | Cell 2   |
fn documented_function() { }
```

### Special Sections

```blood
/// Creates a new instance of `Config`.
///
/// # Arguments
///
/// * `path` - Path to the configuration file
/// * `options` - Optional configuration options
///
/// # Returns
///
/// A new `Config` instance, or an error if the file is invalid.
///
/// # Errors
///
/// Returns `ConfigError::NotFound` if the file doesn't exist.
/// Returns `ConfigError::Parse` if the file is malformed.
///
/// # Panics
///
/// Panics if `path` contains null bytes.
///
/// # Examples
///
/// ```
/// let config = Config::new("config.toml", None)?;
/// assert!(config.is_valid());
/// ```
///
/// # Safety
///
/// This function is safe to call from any thread.
///
/// # See Also
///
/// * [`Config::load`] - Load from default location
/// * [`ConfigBuilder`] - Builder pattern alternative
fn new(path: &str, options: Option<Options>) -> Result<Config, ConfigError> {
    // ...
}
```

### Effect Documentation

```blood
/// Reads a file from the filesystem.
///
/// # Effects
///
/// This function performs the `FileSystem.read` effect operation.
/// The caller must provide a handler for the `FileSystem` effect.
///
/// # Example
///
/// ```
/// with handler LocalFileSystem::new() {
///     let content = read_file("data.txt")?;
/// }
/// ```
fn read_file(path: String) -> Result<String, IoError> with FileSystem {
    do FileSystem.read(path)
}
```

### Linking

```blood
/// Works with [`HashMap`] and [`Vec`].
///
/// See [`module::submodule::Type`] for details.
///
/// Links to methods: [`HashMap::insert`]
///
/// Links to modules: [`crate::utils`]
///
/// Links to effects: [`effect@FileSystem`]
///
/// Links to handlers: [`handler@LocalFileSystem`]
fn example() { }
```

## Generated Documentation Structure

### HTML Output

```
target/doc/
├── index.html              # Package documentation
├── all.html                # Index of all items
├── search-index.js         # Search index
├── settings.html           # Reader settings
├── src/                    # Source code viewer
│   └── my_package/
│       └── lib.blood.html
├── my_package/
│   ├── index.html          # Module index
│   ├── struct.Point.html   # Struct documentation
│   ├── enum.Option.html    # Enum documentation
│   ├── fn.process.html     # Function documentation
│   ├── trait.Display.html  # Trait documentation
│   ├── effect.Log.html     # Effect documentation
│   └── handler.Console.html # Handler documentation
├── static/
│   ├── style.css
│   ├── main.js
│   └── search.js
└── favicon.ico
```

### Documentation Page Sections

Each item's documentation page includes:

1. **Header**: Name, kind, and module path
2. **Signature**: Full type signature with syntax highlighting
3. **Description**: Rendered documentation comment
4. **Sections**: Arguments, Returns, Errors, etc.
5. **Examples**: Extracted and tested code examples
6. **Source Link**: Link to source code viewer
7. **Related Items**: Implementations, trait impls, etc.

## Command Line Interface

### Basic Usage

```bash
# Generate documentation
blood doc

# Generate and open in browser
blood doc --open

# Document specific package
blood doc --package my-lib

# Document all dependencies too
blood doc --all

# Generate JSON output
blood doc --output-format json
```

### Options

```
blood doc [OPTIONS]

Options:
  -p, --package <NAME>     Document only the specified package
      --all                Document all packages including dependencies
      --no-deps            Don't document dependencies
      --open               Open documentation in browser after generating
      --output-format FMT  Output format: html (default), json, markdown
  -o, --output <DIR>       Output directory (default: target/doc)
      --theme <THEME>      Documentation theme: light, dark, auto (default)
      --document-private   Include private items in documentation
      --test               Run documentation tests
      --no-default-features Don't document default features
      --features <FEAT>    Document with specific features enabled
      --all-features       Document with all features enabled
  -j, --jobs <N>           Number of parallel jobs
  -v, --verbose            Verbose output
  -q, --quiet              Suppress output
  -h, --help               Print help
  -V, --version            Print version
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Documentation generation error |
| 2 | Doc test failure |
| 3 | Invalid options |

## Documentation Testing

### Doc Test Syntax

```blood
/// # Examples
///
/// ```
/// let x = 42;
/// assert_eq!(x, 42);
/// ```
///
/// Ignored test:
/// ```ignore
/// let dangerous = do_something_dangerous();
/// ```
///
/// Should panic:
/// ```should_panic
/// panic!("this is expected");
/// ```
///
/// Should fail to compile:
/// ```compile_fail
/// let x: i32 = "not an integer";
/// ```
///
/// No run (only compile):
/// ```no_run
/// loop { } // Would run forever
/// ```
fn example() { }
```

### Running Doc Tests

```bash
# Run all doc tests
blood doc --test

# Run doc tests for specific package
blood doc --test --package my-lib

# Verbose test output
blood doc --test --verbose
```

### Doc Test Output

```
   Doc-tests my-package

running 5 doc tests
test src/lib.blood - Config::new (line 15) ... ok
test src/lib.blood - HashMap::insert (line 42) ... ok
test src/lib.blood - process (line 78) ... ok
test src/lib.blood - ignored ... ignored
test src/lib.blood - compile_fail ... ok

doc test result: ok. 4 passed; 0 failed; 1 ignored
```

## Search Functionality

### Search Features

- Full-text search across all documentation
- Type signature search
- Fuzzy matching for typos
- Filter by item type (struct, fn, trait, etc.)
- Keyboard navigation

### Search Index

Generated `search-index.js`:

```javascript
window.searchIndex = {
  "my_package": {
    "doc": "Package description",
    "items": [
      [0, "Config", "struct", "Configuration struct"],
      [1, "new", "fn", "Create new config"],
      [2, "Log", "effect", "Logging effect"],
      // ...
    ],
    "paths": [
      ["my_package", "Config"],
      ["my_package", "Config", "new"],
      ["my_package", "Log"],
    ]
  }
};
```

### Search Syntax

```
# Basic search
Config

# Type filter
struct:Config
fn:new
effect:Log

# Module filter
my_package::Config

# Signature search
fn(String) -> Result
```

## Themes and Customization

### Built-in Themes

- **Light**: Default light theme
- **Dark**: Dark theme for low-light environments
- **Auto**: Follows system preference

### Custom CSS

```css
/* custom.css */
:root {
  --main-color: #007acc;
  --code-bg: #f5f5f5;
}

.docblock {
  font-size: 16px;
}
```

```bash
blood doc --css custom.css
```

### Custom Templates

Templates use a simple template language:

```html
<!-- templates/page.html -->
<!DOCTYPE html>
<html>
<head>
    <title>{{title}} - {{package}}</title>
    <link rel="stylesheet" href="{{root}}static/style.css">
</head>
<body>
    <nav>{{> navigation}}</nav>
    <main>{{content}}</main>
    <footer>Generated by blood-doc {{version}}</footer>
</body>
</html>
```

## JSON Output Format

For tooling integration:

```json
{
  "name": "my-package",
  "version": "1.0.0",
  "modules": [
    {
      "name": "my_package",
      "doc": "Package documentation",
      "items": [
        {
          "kind": "struct",
          "name": "Config",
          "doc": "Configuration struct",
          "signature": "struct Config { ... }",
          "fields": [...],
          "impls": [...]
        },
        {
          "kind": "function",
          "name": "process",
          "doc": "Process data",
          "signature": "fn process(input: String) -> Result<Output, Error>",
          "effects": ["FileSystem"],
          "arguments": [...],
          "returns": {...}
        },
        {
          "kind": "effect",
          "name": "Log",
          "doc": "Logging effect",
          "operations": [...]
        }
      ]
    }
  ]
}
```

## Integration

### Hosting Documentation

#### GitHub Pages

```yaml
# .github/workflows/docs.yml
name: Documentation

on:
  push:
    branches: [main]

jobs:
  docs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Generate docs
        run: blood doc
      - name: Deploy to GitHub Pages
        uses: peaceiris/actions-gh-pages@v3
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: ./target/doc
```

#### docs.rs Equivalent

The Blood package registry hosts documentation automatically:

```
https://docs.blood-lang.org/my-package/latest/
```

### IDE Integration

#### VS Code

Hover documentation from doc comments:

```json
{
  "blood.hover.showDocs": true,
  "blood.hover.showSignature": true,
  "blood.hover.showEffects": true
}
```

#### LSP

The LSP provides:
- Hover documentation
- Signature help with doc comments
- Go to documentation

## Linting Documentation

### Documentation Lints

```bash
blood doc --lint
```

Checks for:
- Missing documentation on public items
- Broken links
- Missing examples
- Outdated examples (won't compile)
- Missing argument documentation

### Configuration

In `Blood.toml`:

```toml
[doc.lints]
missing_docs = "warn"
broken_links = "deny"
missing_examples = "allow"

# Require docs on public items
[doc]
require_docs = true
```

## Effect-Specific Documentation

### Effect Operations

```blood
/// Logging effect for structured logging.
///
/// # Operations
///
/// - [`log`]: Log a message at a given level
/// - [`flush`]: Flush any buffered logs
///
/// # Example Handler
///
/// ```
/// handler ConsoleLog: Log {
///     fn log(level: Level, msg: String) {
///         println!("[{}] {}", level, msg);
///         resume(())
///     }
///     fn flush() { resume(()) }
/// }
/// ```
effect Log {
    /// Log a message.
    ///
    /// # Arguments
    ///
    /// * `level` - Severity level (Debug, Info, Warn, Error)
    /// * `message` - The message to log
    fn log(level: Level, message: String);

    /// Flush buffered logs.
    fn flush();
}
```

### Handler Documentation

```blood
/// Console logging handler.
///
/// Writes log messages to stdout with color support.
///
/// # Configuration
///
/// - `colored`: Enable ANSI colors (default: true if TTY)
/// - `timestamps`: Include timestamps (default: false)
///
/// # Example
///
/// ```
/// with handler ConsoleLog::new() {
///     do Log.log(Level::Info, "Hello!");
/// }
/// ```
handler ConsoleLog: Log {
    colored: bool,
    timestamps: bool,

    /// Create a new console logger with default settings.
    fn new() -> ConsoleLog {
        ConsoleLog { colored: true, timestamps: false }
    }

    fn log(level: Level, message: String) {
        // ...
        resume(())
    }

    fn flush() {
        resume(())
    }
}
```

## Performance

### Build Performance

| Metric | Target |
|--------|--------|
| Small package (1K LOC) | < 1s |
| Medium package (10K LOC) | < 5s |
| Large package (100K LOC) | < 30s |
| Incremental rebuild | < 100ms per changed file |

### Output Size

- HTML: ~10KB per documented item
- Search index: ~1KB per item
- Source views: ~2x source size

## Accessibility

### Features

- Semantic HTML structure
- ARIA labels
- Keyboard navigation
- High contrast themes
- Screen reader friendly
- Reduced motion support

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `/` or `s` | Focus search |
| `Esc` | Close search / dialogs |
| `?` | Show keyboard shortcuts |
| `j` / `k` | Navigate items |
| `Enter` | Select item |
| `t` | Toggle theme |

## Future Enhancements

### Planned Features

1. **Interactive examples**: Run examples in browser
2. **Version diffing**: Show API changes between versions
3. **Dependency graph**: Visual module dependencies
4. **Usage examples**: Real-world usage from registry

### Not Planned

1. **Wiki features**: Separate concern
2. **Comments/discussions**: External tools handle this
3. **Video tutorials**: Markdown-based docs only

## Version History

| Version | Changes |
|---------|---------|
| 1.0 | Initial specification |

---

*Good documentation is a feature. `blood-doc` makes it easy to write and beautiful to read.*
