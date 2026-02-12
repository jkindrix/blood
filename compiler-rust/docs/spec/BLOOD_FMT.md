# blood-fmt: Blood Code Formatter

**Version**: 1.0
**Status**: Specification
**Tool**: `blood fmt`

## Overview

`blood-fmt` is the official code formatter for Blood, providing consistent, opinionated formatting for all Blood source code. Like `gofmt` and `rustfmt`, it enforces a single canonical style, eliminating debates about formatting.

## Design Philosophy

### Principles

1. **One True Style**: There is exactly one correct way to format Blood code
2. **Zero Configuration** (by default): Works out of the box with no setup
3. **Deterministic**: Same input always produces same output
4. **Fast**: Formats large codebases in seconds
5. **Safe**: Never changes program semantics

### Non-Goals

- Configurable styles (minimal options only)
- Syntax highlighting
- Linting or error detection
- Code transformation beyond whitespace

## Style Rules

### Indentation

- **Indent size**: 4 spaces
- **No tabs**: Spaces only
- **Continuation indent**: 4 spaces (same as block indent)

```blood
fn example() {
    let x = long_function_call(
        argument_one,
        argument_two,
        argument_three,
    );
}
```

### Line Length

- **Maximum line length**: 100 characters
- **Soft limit**: 80 characters (preferred for readability)
- **Comments**: Also wrapped at 100 characters

### Braces

- **Opening brace**: Same line as statement
- **Closing brace**: Own line, aligned with statement
- **Single-expression bodies**: May omit braces if short

```blood
// Standard brace placement
fn add(a: i32, b: i32) -> i32 {
    a + b
}

// Single expression - braces optional
fn double(x: i32) -> i32 { x * 2 }

// If/else
if condition {
    do_something();
} else {
    do_other();
}
```

### Spacing

#### Around Operators

```blood
// Binary operators: space on both sides
let x = a + b;
let y = x * 2;
let z = a == b;

// Unary operators: no space
let neg = -x;
let not = !flag;
let ref_x = &x;
let deref = *ptr;

// Range operators: no spaces
for i in 0..10 { }
for i in 0..=10 { }
```

#### After Keywords

```blood
// Space after keywords
if condition { }
while running { }
for item in items { }
match value { }
return result;

// No space for function-like keywords
fn name() { }
struct Point { }
enum Option { }
```

#### In Function Calls

```blood
// No space before parentheses
function_call(arg1, arg2);
method.call(arg);

// Space after commas
let tuple = (a, b, c);
let array = [1, 2, 3];
```

#### Around Colons

```blood
// Type annotations: space after colon
let x: i32 = 42;
fn add(a: i32, b: i32) -> i32

// Struct fields: space after colon
struct Point {
    x: i32,
    y: i32,
}

// Match arms: space after colon equivalent (=>)
match value {
    1 => "one",
    _ => "other",
}
```

### Blank Lines

#### Between Items

```blood
// One blank line between top-level items
fn first() {
    // ...
}

fn second() {
    // ...
}

// No blank line for related one-liners
const A: i32 = 1;
const B: i32 = 2;
const C: i32 = 3;
```

#### Within Functions

```blood
fn example() {
    // Related statements: no blank line
    let x = 1;
    let y = 2;
    let z = x + y;

    // Logical sections: one blank line
    process_first_thing();

    process_second_thing();

    // No trailing blank line before closing brace
}
```

### Imports

#### Order

1. Standard library imports
2. External crate imports
3. Local imports

```blood
// Standard library
use std::collections::HashMap;
use std::io::Read;

// External crates
use http::Request;
use json::Value;

// Local modules
use crate::config::Config;
use crate::utils::helper;
```

#### Grouping

```blood
// Group imports from same module
use std::collections::{HashMap, HashSet, BTreeMap};

// Long import lists: one per line
use std::collections::{
    BTreeMap,
    BTreeSet,
    HashMap,
    HashSet,
    LinkedList,
    VecDeque,
};

// Alphabetical order within groups
use std::io::{BufRead, Read, Write};
```

### Structs

```blood
// Short structs: single line
struct Point { x: i32, y: i32 }

// Longer structs: multi-line
struct Config {
    name: String,
    version: String,
    debug: bool,
    max_connections: u32,
}

// Tuple structs
struct Color(u8, u8, u8);
struct Wrapper(Inner);
```

### Enums

```blood
// Simple enums: may be compact
enum Direction { North, South, East, West }

// Enums with data: multi-line
enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping { id: u64 },
    Quit,
}
```

### Functions

```blood
// Short function: single line OK
fn add(a: i32, b: i32) -> i32 { a + b }

// Standard function
fn process(input: String) -> Result<Output, Error> {
    // body
}

// Long parameter list: wrap
fn complex_function(
    first_param: FirstType,
    second_param: SecondType,
    third_param: ThirdType,
) -> ReturnType {
    // body
}

// Effects on same line if they fit
fn read_file(path: String) -> String with FileSystem {
    do FileSystem.read(path)
}

// Long effect list: wrap
fn complex_operation(
    input: String,
) -> Result<Output, Error> with Database, Network, Log {
    // body
}
```

### Match Expressions

```blood
// Simple match: compact if short
match x {
    1 => "one",
    2 => "two",
    _ => "other",
}

// Match with blocks
match value {
    Some(x) => {
        process(x);
        x
    }
    None => {
        log("nothing");
        default()
    }
}

// Pattern alignment
match result {
    Ok(value)   => handle_success(value),
    Err(error)  => handle_error(error),
}
```

### Closures

```blood
// Short closures: inline
let double = |x| x * 2;
let add = |a, b| a + b;

// Typed closures
let parse = |s: &str| -> i32 { s.parse().unwrap() };

// Multi-line closures
let complex = |x| {
    let y = transform(x);
    process(y)
};
```

### Chains

```blood
// Short chains: single line
let result = value.map(f).filter(g).collect();

// Long chains: one per line
let result = collection
    .iter()
    .filter(|x| x.is_valid())
    .map(|x| x.transform())
    .filter_map(|x| x.try_convert())
    .collect::<Vec<_>>();
```

### Comments

```blood
// Line comments: space after //
// This is a comment

/// Doc comments: space after ///
/// This documents the following item

/* Block comments: spaces inside */
/* This is a block comment */

// Trailing comments: align if nearby
let x = 1;  // first value
let y = 2;  // second value
```

### Attributes

```blood
// Single attribute: same line
#[derive(Debug, Clone)]
struct Point { x: i32, y: i32 }

// Multiple attributes: one per line
#[derive(Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Config {
    name: String,
}

// Inner attributes at top
#![allow(unused)]
#![feature(effects)]

mod implementation {
    // ...
}
```

### Effects and Handlers

```blood
// Effect definition
effect Log {
    fn log(level: Level, message: String);
    fn flush();
}

// Handler definition
handler ConsoleLog: Log {
    fn log(level: Level, message: String) {
        println!("[{}] {}", level, message);
        resume(())
    }

    fn flush() {
        // no-op
        resume(())
    }
}

// Handler usage
with handler ConsoleLog::new() {
    do Log.log(Level::Info, "Hello");
}
```

## Command Line Interface

### Basic Usage

```bash
# Format single file
blood fmt src/main.blood

# Format directory recursively
blood fmt src/

# Format entire project
blood fmt

# Check formatting without modifying
blood fmt --check

# Show diff of changes
blood fmt --diff
```

### Options

```
blood fmt [OPTIONS] [FILES]...

Arguments:
  [FILES]...  Files or directories to format (default: current directory)

Options:
  -c, --check         Check if files are formatted, exit with error if not
  -d, --diff          Show diff instead of modifying files
  -q, --quiet         Suppress non-error output
  -v, --verbose       Show files being processed
      --stdin         Read from stdin, write to stdout
      --config PATH   Path to blood-fmt.toml configuration
      --print-config  Print current configuration and exit
  -h, --help          Print help
  -V, --version       Print version
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (files formatted or already formatted) |
| 1 | Formatting differences found (with --check) |
| 2 | Error (invalid syntax, I/O error, etc.) |

## Configuration

While `blood-fmt` is opinionated by default, minimal configuration is available for special cases.

### Configuration File

`blood-fmt.toml` in project root or `~/.config/blood/fmt.toml`:

```toml
# blood-fmt.toml

# Maximum line width (default: 100)
max_width = 100

# Use tabs instead of spaces (default: false, strongly discouraged)
use_tabs = false

# Tab width if use_tabs = true (default: 4)
tab_width = 4

# Files to ignore (in addition to .gitignore)
ignore = [
    "generated/**",
    "vendor/**",
]

# Per-file overrides
[overrides."tests/fixtures/**"]
# Allow longer lines in test fixtures
max_width = 120
```

### Per-File Directives

```blood
// Disable formatting for a section
// blood-fmt: off
fn generated_code() {
    // This won't be formatted
}
// blood-fmt: on

// Skip next item
// blood-fmt: skip
const LOOKUP_TABLE: [u8; 256] = [
    0x00, 0x01, 0x02, 0x03, // ...
];
```

## Integration

### Editor Integration

#### VS Code

```json
// settings.json
{
    "blood.formatOnSave": true,
    "blood.formatter.path": "blood"
}
```

#### Vim/Neovim

```vim
" Format on save
autocmd BufWritePre *.blood execute "!blood fmt %"

" Format command
command! BloodFmt execute "!blood fmt %"
```

#### Emacs

```elisp
(defun blood-fmt-buffer ()
  "Format current buffer with blood-fmt."
  (interactive)
  (shell-command-on-region
   (point-min) (point-max)
   "blood fmt --stdin" t t))
```

### CI Integration

#### GitHub Actions

```yaml
- name: Check formatting
  run: blood fmt --check
```

#### Pre-commit Hook

```bash
#!/bin/sh
# .git/hooks/pre-commit

blood fmt --check
if [ $? -ne 0 ]; then
    echo "Code is not formatted. Run 'blood fmt' before committing."
    exit 1
fi
```

## Implementation Notes

### Parser Requirements

The formatter requires a full parse of the source file. It will fail if:
- Syntax errors are present
- Invalid tokens exist

For partial formatting during editing, the LSP server provides incremental formatting.

### Preservation Guarantees

The formatter preserves:
- Comments (including position relative to code)
- Semantic meaning (AST equivalence)
- User's choice of single vs double quotes (if both valid)

The formatter modifies:
- Whitespace (spaces, tabs, newlines)
- Import order
- Trailing commas
- Unnecessary parentheses

### Performance Targets

| Metric | Target |
|--------|--------|
| Single file | < 10ms |
| Large file (10K LOC) | < 100ms |
| Large project (100K LOC) | < 5s |
| Memory per file | < 10x file size |

## Style Rationale

### Why These Choices?

**4-space indent**: Matches most popular styles, balances readability with horizontal space.

**100-character lines**: Wide enough for modern monitors, narrow enough for side-by-side diffs.

**Braces on same line**: Reduces vertical space, widely accepted in C-family languages.

**No configurable style**: Eliminates bikeshedding, ensures all Blood code looks the same.

### Deviations from Rust/Go

| Aspect | Blood | Rust | Go |
|--------|-------|------|-----|
| Indent | 4 spaces | 4 spaces | Tabs |
| Line length | 100 | 100 | None |
| Import grouping | 3 groups | 2 groups | 3 groups |
| Trailing commas | Always in multi-line | Always | N/A |

## Future Considerations

### Potential Additions

1. **Sort struct fields**: Alphabetically or by size
2. **Align assignments**: Controversial, probably not
3. **Format macros**: When macro system is stable
4. **Semantic formatting**: Use type information for better decisions

### Not Planned

1. **Configurable brace style**: One style only
2. **Configurable indent**: 4 spaces, always
3. **Multiple formatters**: One official formatter
4. **Partial formatting of broken code**: Requires valid syntax

## Version History

| Version | Changes |
|---------|---------|
| 1.0 | Initial specification |

---

*The best style is the one you don't have to think about. `blood-fmt` makes all Blood code consistent.*
