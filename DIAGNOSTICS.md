# Blood Diagnostics Specification

**Version**: 0.1.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

This document specifies Blood's error messages, warnings, and diagnostic output format, designed for maximum developer productivity and clarity.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Diagnostic Levels](#2-diagnostic-levels)
3. [Message Format](#3-message-format)
4. [Error Categories](#4-error-categories)
5. [Fix-It Hints](#5-fix-it-hints)
6. [Multi-Location Diagnostics](#6-multi-location-diagnostics)
7. [Error Recovery](#7-error-recovery)
8. [Machine-Readable Output](#8-machine-readable-output)
9. [Internationalization](#9-internationalization)
10. [Testing Diagnostics](#10-testing-diagnostics)

---

## 1. Overview

### 1.1 Design Principles

Blood diagnostics follow these principles:

1. **Actionable** — Every message should help the user fix the problem
2. **Precise** — Point to the exact location of the issue
3. **Educational** — Explain why something is wrong, not just what
4. **Consistent** — Use uniform terminology and formatting
5. **Minimal Noise** — Avoid cascading errors and false positives

### 1.2 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [GRAMMAR.md](./GRAMMAR.md) — Parser error context
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — StaleReference diagnostics
- [DISPATCH.md](./DISPATCH.md) — Ambiguity error messages
- [FFI.md](./FFI.md) — FFI-related diagnostics
- [STDLIB.md](./STDLIB.md) — Standard error types

### 1.3 Diagnostic Philosophy

| Aspect | Blood Approach |
|--------|----------------|
| **Tone** | Respectful, not condescending |
| **Focus** | What to fix, not what's wrong |
| **Context** | Show relevant surrounding code |
| **Suggestions** | Provide fix-it hints when possible |
| **Cascading** | Suppress likely follow-on errors |

### 1.3 The Golden Rule

> "An error message is successful when the user can fix the problem without searching online."

---

## 2. Diagnostic Levels

### 2.1 Level Hierarchy

| Level | Code | Blocks Compilation | Description |
|-------|------|-------------------|-------------|
| **Error** | E | Yes | Code is invalid, cannot compile |
| **Warning** | W | No (by default) | Suspicious but valid code |
| **Info** | I | No | Informational note |
| **Hint** | H | No | Suggestion for improvement |

### 2.2 Warning Configuration

```bash
# Treat all warnings as errors
$ blood build --deny warnings

# Treat specific warning as error
$ blood build --deny W0042

# Allow specific warning
$ blood build --allow W0101

# Silence all warnings
$ blood build --cap-lints allow
```

### 2.3 Warning Categories

| Category | Prefix | Description |
|----------|--------|-------------|
| Unused | W01xx | Unused variables, imports, etc. |
| Naming | W02xx | Naming convention violations |
| Style | W03xx | Style inconsistencies |
| Performance | W04xx | Potential performance issues |
| Safety | W05xx | Suspicious safety patterns |
| Deprecated | W06xx | Use of deprecated features |
| Complexity | W07xx | Overly complex code |
| Documentation | W08xx | Missing documentation |

---

## 3. Message Format

### 3.1 Standard Format

```
level[CODE]: main message
  --> file.blood:line:column
   |
NN | source code line
   |        ^^^^ primary span label
   |
   = note: additional context
   = help: how to fix it
```

### 3.2 Format Components

```
┌─────────────────────────────────────────────────────────────────┐
│ error[E0123]: cannot add `String` to `Int`                      │
│   --> src/math.blood:15:12                                      │
│    |                                                            │
│ 15 |     let sum = x + name                                     │
│    |              ^ - ^^^^ String                                │
│    |              |                                             │
│    |              Int                                           │
│    |                                                            │
│    = note: the `+` operator is not defined for these types      │
│    = help: convert the String to Int: `name.parse::<Int>()?`    │
└─────────────────────────────────────────────────────────────────┘

Components:
1. Level and error code: "error[E0123]"
2. Main message: summary of the problem
3. Location: "file:line:column"
4. Source context: relevant code lines
5. Primary span: underlines the problem (^^^^)
6. Secondary spans: related locations
7. Notes: additional context
8. Help: suggestions for fixing
```

### 3.3 Span Styles

| Style | Character | Usage |
|-------|-----------|-------|
| Primary | `^` | Main error location |
| Secondary | `-` | Related location |
| Insertion | `+` | Suggested insertion point |
| Deletion | `~` | Suggested deletion |

### 3.4 Color Scheme

| Element | Color | ANSI Code |
|---------|-------|-----------|
| Error | Red | `\x1b[31m` |
| Warning | Yellow | `\x1b[33m` |
| Info | Blue | `\x1b[34m` |
| Hint | Cyan | `\x1b[36m` |
| Note | White | `\x1b[37m` |
| Code | Bold | `\x1b[1m` |
| Path | Underline | `\x1b[4m` |

---

## 4. Error Categories

### 4.1 Parse Errors (E01xx)

```
error[E0101]: expected `;` after statement
  --> src/main.blood:10:25
   |
10 |     let x = compute()
   |                      ^ expected `;`
   |
   = help: add `;` at the end of the statement

error[E0102]: mismatched brackets
  --> src/main.blood:15:1
   |
12 |     fn example() {
   |                  - opening brace
...
15 | fn another() {
   | ^ expected `}` before this
   |
   = note: the opening `{` on line 12 is never closed

error[E0103]: unexpected token
  --> src/main.blood:8:15
   |
 8 |     if x == 42 then {
   |                ^^^^ unexpected keyword `then`
   |
   = note: Blood uses `{` not `then` for if blocks
   = help: remove `then`: `if x == 42 {`
```

### 4.2 Type Errors (E02xx)

```
error[E0201]: type mismatch
  --> src/main.blood:20:16
   |
18 |     fn greet(name: String) -> i32 {
   |                               --- expected `i32` because of return type
19 |         println("Hello, {}", name)
20 |         name
   |         ^^^^ expected `i32`, found `String`
   |
   = help: consider using `name.len() as i32` if you want the length

error[E0202]: cannot infer type
  --> src/main.blood:5:9
   |
 5 |     let x = Vec::new()
   |         ^ cannot infer type for `x`
   |
   = note: type annotations needed
   = help: consider giving `x` an explicit type: `let x: Vec<i32> = Vec::new()`

error[E0203]: missing type parameter
  --> src/main.blood:12:20
   |
12 |     fn identity(x: T) -> T {
   |                    ^ undefined type `T`
   |
   = help: add a type parameter: `fn identity<T>(x: T) -> T`
```

### 4.3 Effect Errors (E03xx)

```
error[E0301]: unhandled effect `IO`
  --> src/main.blood:8:5
   |
 6 | fn pure_function() -> i32 / pure {
   |                            ---- function declared as `pure`
 7 |     let data = read_file("config.txt")
   |                ^^^^^^^^^ performs `IO` effect
 8 |     data.len()
   |
   = note: `read_file` has effect signature `/ {IO}`
   = help: either:
           - add `IO` to the effect row: `fn pure_function() -> i32 / {IO}`
           - handle the effect with a handler

error[E0302]: effect mismatch
  --> src/main.blood:15:12
   |
15 |     with StateHandler handle {
   |          ^^^^^^^^^^^^
   |          handles: {State<Int>}
   |          required: {State<String>}
   |
   = note: the handled computation requires `State<String>`
           but the handler provides `State<Int>`

error[E0303]: cannot resume with wrong type
  --> src/main.blood:22:16
   |
20 |     op get() -> String {
   |                 ------ operation returns `String`
21 |         let value = 42
22 |         resume(value)
   |                ^^^^^ expected `String`, found `i32`
```

### 4.4 Ownership Errors (E04xx)

```
error[E0401]: use of moved value
  --> src/main.blood:10:20
   |
 8 |     let data = vec![1, 2, 3]
   |         ---- move occurs here
 9 |     process(data)
   |             ---- value moved here
10 |     println("{:?}", data)
   |                     ^^^^ value used after move
   |
   = help: consider cloning: `process(data.clone())`

error[E0402]: cannot borrow as mutable
  --> src/main.blood:15:5
   |
13 |     let items = vec![1, 2, 3]
   |         ----- not declared as mutable
14 |
15 |     items.push(4)
   |     ^^^^^ cannot borrow as mutable
   |
   = help: consider making `items` mutable: `let mut items = ...`

error[E0403]: linear value not consumed
  --> src/main.blood:8:9
   |
 8 |     let handle: linear FileHandle = open("file.txt")?
   |         ^^^^^^ linear value must be used exactly once
   |
   = note: `handle` was never used or explicitly closed
   = help: either use `handle` or close it: `handle.close()`
```

### 4.5 Lifetime Errors (E05xx)

```
error[E0501]: reference outlives borrowed value
  --> src/main.blood:12:12
   |
10 |     fn get_ref() -> &String {
   |                     ------- lifetime of return value
11 |         let s = String::from("hello")
   |             - borrowed value created here
12 |         &s
   |          ^ returns reference to local variable
   |
   = note: `s` is dropped at the end of the function
   = help: consider returning an owned value: `fn get_ref() -> String`

error[E0502]: stale reference possible
  --> src/main.blood:18:9
   |
15 |     let ptr = &*box_value
   |               ----------- reference created here
16 |     yield()
   |     ------- effect may suspend here
17 |
18 |     *ptr
   |     ^^^^ reference may be stale after resume
   |
   = note: the memory may be freed during suspension
   = help: dereference before yield: `let value = *ptr; yield(); value`
```

### 4.6 Dispatch Errors (E06xx)

```
error[E0601]: ambiguous method call
  --> src/main.blood:25:5
   |
25 |     draw(shape)
   |     ^^^^
   |
   = note: multiple implementations match:
           - `draw(s: Circle)` in module `shapes`
           - `draw(s: impl Shape)` in module `rendering`
   |
   = help: use explicit type annotation: `draw(shape as Circle)`

error[E0602]: no matching implementation
  --> src/main.blood:30:5
   |
30 |     add(point, vector)
   |     ^^^
   |
   = note: required: `add(Point, Vector)`
   = note: available implementations:
           - `add(Int, Int)`
           - `add(Float, Float)`
           - `add(Point, Point)`
   |
   = help: implement `add` for `(Point, Vector)` or convert types
```

---

## 5. Fix-It Hints

### 5.1 Automatic Fixes

Blood can suggest and apply fixes automatically:

```bash
# Show suggested fixes
$ blood check --suggest-fixes

# Apply fixes interactively
$ blood fix --interactive

# Apply all safe fixes automatically
$ blood fix --apply
```

### 5.2 Fix-It Format

```
error[E0201]: type mismatch
  --> src/main.blood:15:12
   |
15 |     return "hello"
   |            ^^^^^^^ expected `i32`, found `&str`
   |
   = fix-it: parse the string
     |
  15 |     return "hello".parse().unwrap()
     |            ~~~~~~~~~~~~~~~~~~~~~~~~
```

### 5.3 Fix Categories

| Category | Confidence | Auto-apply |
|----------|------------|------------|
| **Certain** | Fix is definitely correct | Yes |
| **Likely** | Fix is probably correct | With `--apply` |
| **Suggestion** | One of several options | Interactive only |

### 5.4 Fix-It Examples

```
// Missing semicolon (Certain)
error[E0101]: expected `;`
  15 |     let x = 42
     |               ^ insert `;`
  fix-it (certain): insert ";"

// Type conversion (Likely)
error[E0201]: type mismatch: expected i32, found i64
  20 |     let x: i32 = large_value
     |                  ^^^^^^^^^^^
  fix-it (likely): add `as i32`
    |
  20 |     let x: i32 = large_value as i32

// Missing import (Suggestion)
error[E0701]: undefined name `HashMap`
  5  |     let map: HashMap<String, i32> = ...
     |              ^^^^^^^ not found
  fix-it (suggestion): add import
    |
  1+ | use std::collections::HashMap;
```

---

## 6. Multi-Location Diagnostics

### 6.1 Two-Location Errors

```
error[E0201]: type mismatch
  --> src/main.blood:20:12
   |
 8 | fn process(x: i32) -> String {
   |                       ------ expected `String` because of return type
...
20 |     return 42
   |            ^^ expected `String`, found `i32`
```

### 6.2 Many-Location Errors

```
error[E0601]: ambiguous dispatch
  --> src/main.blood:50:5
   |
10 | fn draw(s: Circle) { ... }
   | ----------------------- candidate #1
...
25 | fn draw(s: impl Shape) { ... }
   | --------------------------- candidate #2
...
50 |     draw(my_shape)
   |     ^^^^^^^^^^^^^^ ambiguous: matches both candidates
   |
   = note: both candidates have equal specificity
   = help: add explicit type annotation to disambiguate
```

### 6.3 Cross-File Diagnostics

```
error[E0301]: effect mismatch
  --> src/handlers.blood:15:5
   |
  ::: src/effects.blood:8:5
   |
 8 | effect MyEffect {
 9 |     op do_thing() -> String;
   |                      ------ expected type
   |
  ::: src/handlers.blood:15:5
   |
15 |         resume(42)
   |                ^^ expected `String`, found `i32`
```

---

## 7. Error Recovery

### 7.1 Recovery Strategies

The parser recovers from errors to report multiple issues:

| Strategy | When Used | Example |
|----------|-----------|---------|
| **Skip to delimiter** | Missing `;` | Skip to next `;` or `}` |
| **Insert expected token** | Missing `}` | Pretend `}` exists |
| **Skip malformed construct** | Unparseable expression | Skip entire statement |
| **Assume typo** | Unknown identifier | Suggest similar name |

### 7.2 Error Cascading Prevention

```
// BAD: Cascading errors
error[E0701]: undefined type `Strng`    // Typo
error[E0201]: cannot add `<error>` and `i32`  // Cascade
error[E0201]: cannot call method on `<error>` // Cascade

// GOOD: Suppress cascades
error[E0701]: undefined type `Strng`
  --> src/main.blood:5:12
   |
 5 |     let s: Strng = "hello"
   |            ^^^^^ did you mean `String`?
   |
   = note: subsequent errors in expressions using `s` are suppressed
```

### 7.3 Error Limits

```bash
# Stop after N errors (default: 20)
$ blood build --error-limit 50

# Show all errors (may be overwhelming)
$ blood build --error-limit 0
```

---

## 8. Machine-Readable Output

### 8.1 JSON Format

```bash
$ blood check --message-format json
```

```json
{
  "diagnostics": [
    {
      "level": "error",
      "code": "E0201",
      "message": "type mismatch: expected `i32`, found `String`",
      "spans": [
        {
          "file": "src/main.blood",
          "line_start": 15,
          "line_end": 15,
          "column_start": 12,
          "column_end": 18,
          "is_primary": true,
          "label": "expected `i32`",
          "suggested_replacement": null
        }
      ],
      "children": [
        {
          "level": "note",
          "message": "return type declared here",
          "spans": [
            {
              "file": "src/main.blood",
              "line_start": 8,
              "line_end": 8,
              "column_start": 25,
              "column_end": 28
            }
          ]
        }
      ],
      "rendered": "error[E0201]: type mismatch..."
    }
  ]
}
```

### 8.2 SARIF Format

Static Analysis Results Interchange Format for IDE integration:

```bash
$ blood check --message-format sarif
```

```json
{
  "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
  "version": "2.1.0",
  "runs": [
    {
      "tool": {
        "driver": {
          "name": "blood",
          "version": "0.1.0",
          "rules": [
            {
              "id": "E0201",
              "name": "TypeMismatch",
              "shortDescription": {
                "text": "Type mismatch in expression"
              }
            }
          ]
        }
      },
      "results": [...]
    }
  ]
}
```

### 8.3 LSP Diagnostics

```typescript
// Language Server Protocol diagnostic
interface Diagnostic {
  range: Range;
  severity: DiagnosticSeverity;
  code: string;
  source: "blood";
  message: string;
  relatedInformation?: DiagnosticRelatedInformation[];
  tags?: DiagnosticTag[];
  data?: {
    fixIts: FixIt[];
  };
}
```

---

## 9. Internationalization

### 9.1 Message Catalogs

Error messages support translation:

```
// messages/en.ftl (Fluent format)
E0201 = type mismatch: expected `{ $expected }`, found `{ $found }`
    .note = { $context }
    .help = { $suggestion }

// messages/es.ftl
E0201 = error de tipo: se esperaba `{ $expected }`, se encontró `{ $found }`
    .note = { $context }
    .help = { $suggestion }
```

### 9.2 Language Selection

```bash
# Use system locale
$ blood check

# Override language
$ LANG=es_ES blood check

# Force English
$ blood check --lang en
```

### 9.3 Fallback Chain

1. Requested language (e.g., `es_MX`)
2. Language family (e.g., `es`)
3. English (always available)

---

## 10. Testing Diagnostics

### 10.1 Diagnostic Tests

```blood
// test/diagnostics/type_mismatch.blood

// @error E0201 "type mismatch"
// @note "expected type"
// @at 5:12
fn test_case() -> i32 {
    "not an int"  // E0201
}
```

### 10.2 Test Assertions

```rust
#[test]
fn test_type_mismatch_error() {
    let source = r#"
        fn example() -> i32 { "hello" }
    "#;

    let diagnostics = compile_and_get_diagnostics(source);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "E0201");
    assert!(diagnostics[0].message.contains("type mismatch"));
    assert_eq!(diagnostics[0].primary_span.line, 2);
}
```

### 10.3 Snapshot Testing

```bash
$ blood test-diagnostics --update-snapshots
```

```
// tests/snapshots/E0201.snap
error[E0201]: type mismatch: expected `i32`, found `String`
  --> test.blood:2:23
   |
 2 |     fn example() -> i32 { "hello" }
   |                           ^^^^^^^ expected `i32`
```

---

## Appendix A: Error Code Registry

### Parse Errors (E01xx)

| Code | Description |
|------|-------------|
| E0101 | Expected token |
| E0102 | Mismatched brackets |
| E0103 | Unexpected token |
| E0104 | Invalid literal |
| E0105 | Unterminated string |
| E0106 | Invalid escape sequence |
| E0107 | Invalid number format |

### Type Errors (E02xx)

| Code | Description |
|------|-------------|
| E0201 | Type mismatch |
| E0202 | Cannot infer type |
| E0203 | Missing type parameter |
| E0204 | Invalid type application |
| E0205 | Recursive type without indirection |
| E0206 | Missing trait bound |
| E0207 | Conflicting trait implementations |

### Effect Errors (E03xx)

| Code | Description |
|------|-------------|
| E0301 | Unhandled effect |
| E0302 | Effect signature mismatch |
| E0303 | Resume type mismatch |
| E0304 | Linear value in multi-shot handler |
| E0305 | Missing effect handler |
| E0306 | Duplicate effect in row |

### Ownership Errors (E04xx)

| Code | Description |
|------|-------------|
| E0401 | Use of moved value |
| E0402 | Cannot borrow as mutable |
| E0403 | Linear value not consumed |
| E0404 | Double-free |
| E0405 | Mutable borrow while immutable borrow exists |
| E0406 | Return of borrowed value |

### Lifetime Errors (E05xx)

| Code | Description |
|------|-------------|
| E0501 | Reference outlives value |
| E0502 | Stale reference across effect |
| E0503 | Lifetime bound not satisfied |
| E0504 | Region escape |

### Dispatch Errors (E06xx)

| Code | Description |
|------|-------------|
| E0601 | Ambiguous method dispatch |
| E0602 | No matching implementation |
| E0603 | Type instability |
| E0604 | Circular dispatch |

### Name Resolution Errors (E07xx)

| Code | Description |
|------|-------------|
| E0701 | Undefined name |
| E0702 | Duplicate definition |
| E0703 | Private item access |
| E0704 | Circular import |

---

## Appendix B: Warning Code Registry

### Unused Warnings (W01xx)

| Code | Description | Default |
|------|-------------|---------|
| W0101 | Unused variable | Warn |
| W0102 | Unused import | Warn |
| W0103 | Unused function | Warn |
| W0104 | Unused type | Warn |
| W0105 | Unused effect | Warn |
| W0106 | Unreachable code | Warn |

### Naming Warnings (W02xx)

| Code | Description | Default |
|------|-------------|---------|
| W0201 | Non-snake_case variable | Warn |
| W0202 | Non-PascalCase type | Warn |
| W0203 | Non-SCREAMING_CASE constant | Allow |
| W0204 | Single-letter variable (outside loops) | Allow |

### Style Warnings (W03xx)

| Code | Description | Default |
|------|-------------|---------|
| W0301 | Unnecessary parentheses | Allow |
| W0302 | Unnecessary braces | Allow |
| W0303 | Mixed tabs and spaces | Warn |
| W0304 | Trailing whitespace | Allow |

### Performance Warnings (W04xx)

| Code | Description | Default |
|------|-------------|---------|
| W0401 | Clone in loop | Warn |
| W0402 | Box of small value | Allow |
| W0403 | Unnecessary allocation | Allow |
| W0404 | Effect in hot loop | Allow |

### Safety Warnings (W05xx)

| Code | Description | Default |
|------|-------------|---------|
| W0501 | Unchecked cast | Warn |
| W0502 | Potential integer overflow | Warn |
| W0503 | Division by zero possible | Warn |
| W0504 | Unhandled None/Error | Warn |

---

## Appendix C: References

Diagnostic design draws from:

- [GCC Guidelines for Diagnostics](https://gcc.gnu.org/onlinedocs/gccint/Guidelines-for-Diagnostics.html)
- [Rust Compiler Error Index](https://doc.rust-lang.org/error-index.html)
- [Elm's Compiler Errors for Humans](https://elm-lang.org/news/compiler-errors-for-humans)
- [Swift Diagnostic Guidelines](https://github.com/apple/swift/blob/main/docs/Diagnostics.md)
- [SARIF Specification](https://sarifweb.azurewebsites.net/)

---

*This document is part of the Blood Language Specification.*
