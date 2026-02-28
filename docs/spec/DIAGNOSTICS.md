# Blood Diagnostics Specification

**Version**: 0.4.0
**Status**: Implemented
**Implementation**: Implemented (core diagnostics complete)
**Last Updated**: 2026-01-12

**Revision 0.4.0 Changes**:
- Added comprehensive error code documentation with examples
- Documented all implemented error codes (E0001-E0237)
- Added suggested fixes for each error
- Added related errors cross-references
- Reorganized error code registry for clarity

**Implementation Scope**: Core error codes (E00xx, E01xx, E02xx, E03xx) are fully implemented. Effect and memory error codes (E04xx-E07xx) are in development.

This document specifies Blood's error messages, warnings, and diagnostic output format, designed for maximum developer productivity and clarity.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Diagnostic Levels](#2-diagnostic-levels)
3. [Message Format](#3-message-format)
4. [Error Code Reference](#4-error-code-reference)
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

### 1.4 The Golden Rule

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

## 4. Error Code Reference

### 4.1 Lexer Errors (E0001-E0099)

#### E0001: Unexpected Character

Occurs when the lexer encounters a character that is not valid in Blood source code.

**Example:**
```blood
fn main() {
    let x = 42 @ 10;  // '@' is not a valid operator
}
```

**Error message:**
```
error[E0001]: unexpected character in source
  --> src/main.blood:2:16
   |
 2 |     let x = 42 @ 10;
   |                ^ unexpected character `@`
```

**Fix:** Remove the invalid character or replace with a valid operator.

**Related:** E0100 (unexpected token)

---

#### E0002: Unclosed Block Comment

Occurs when a block comment `/* */` is opened but never closed.

**Example:**
```blood
fn main() {
    /* This comment is never closed
    let x = 42;
}
```

**Error message:**
```
error[E0002]: unclosed block comment
  --> src/main.blood:2:5
   |
 2 |     /* This comment is never closed
   |     ^^ block comment starts here
   |
   = help: add `*/` to close the block comment
```

**Fix:** Add `*/` to close the block comment.

**Related:** E0003 (unclosed string)

---

#### E0003: Unclosed String Literal

Occurs when a string literal is opened but never closed with a matching quote.

**Example:**
```blood
fn main() {
    let name = "hello world;
}
```

**Error message:**
```
error[E0003]: unclosed string literal
  --> src/main.blood:2:16
   |
 2 |     let name = "hello world;
   |                ^ string literal starts here
   |
   = help: add a closing `"` to complete the string
```

**Fix:** Add a closing `"` at the end of the string.

**Related:** E0007 (unclosed character literal)

---

#### E0004: Invalid Escape Sequence

Occurs when a string or character contains an unrecognized escape sequence.

**Example:**
```blood
fn main() {
    let path = "C:\Users\name";  // \U and \n are escape sequences
}
```

**Error message:**
```
error[E0004]: invalid escape sequence
  --> src/main.blood:2:19
   |
 2 |     let path = "C:\Users\name";
   |                   ^^ unknown escape sequence `\U`
   |
   = help: valid escape sequences are: \n, \r, \t, \\, \', \", \0, \x##, \u{####}
```

**Fix:** Use valid escape sequences. For Windows paths, use `\\` for backslashes or use a raw string.

**Related:** E0003 (unclosed string)

---

#### E0005: Invalid Integer Literal

Occurs when an integer literal is malformed.

**Example:**
```blood
fn main() {
    let x = 0x;        // hex prefix without digits
    let y = 123abc;    // invalid suffix
}
```

**Error message:**
```
error[E0005]: invalid integer literal
  --> src/main.blood:2:13
   |
 2 |     let x = 0x;
   |             ^^ expected hexadecimal digits after `0x`
```

**Fix:** Provide valid integer digits in the appropriate format (decimal, hex `0x`, octal `0o`, or binary `0b`).

**Related:** E0006 (invalid float)

---

#### E0006: Invalid Float Literal

Occurs when a floating-point literal is malformed.

**Example:**
```blood
fn main() {
    let x = 3.;     // missing digits after decimal
    let y = .5;     // missing digits before decimal
    let z = 1e;     // missing exponent
}
```

**Error message:**
```
error[E0006]: invalid float literal
  --> src/main.blood:2:13
   |
 2 |     let x = 3.;
   |             ^^ expected digits after decimal point
```

**Fix:** Provide a complete float literal with digits on both sides of the decimal point (e.g., `3.0`).

**Related:** E0005 (invalid integer)

---

#### E0007: Unclosed Character Literal

Occurs when a character literal is opened but never closed.

**Example:**
```blood
fn main() {
    let c = 'a;
}
```

**Error message:**
```
error[E0007]: unclosed character literal
  --> src/main.blood:2:13
   |
 2 |     let c = 'a;
   |             ^ character literal starts here
   |
   = help: add a closing `'` to complete the character literal
```

**Fix:** Add a closing `'` after the character.

**Related:** E0008 (empty character literal)

---

#### E0008: Empty Character Literal

Occurs when a character literal contains no characters.

**Example:**
```blood
fn main() {
    let c = '';
}
```

**Error message:**
```
error[E0008]: empty character literal
  --> src/main.blood:2:13
   |
 2 |     let c = '';
   |             ^^ character literals must contain exactly one character
   |
   = help: character literals must contain exactly one character
```

**Fix:** Add exactly one character between the quotes.

**Related:** E0007 (unclosed character literal)

---

### 4.2 Parser Errors (E0100-E0199)

#### E0100: Unexpected Token

Occurs when the parser encounters a token that doesn't fit the expected grammar.

**Example:**
```blood
fn main() {
    let x = + 42;  // expression cannot start with `+`
}
```

**Error message:**
```
error[E0100]: expected literal, identifier, `(`, `[`, `{`, `if`, or `match`, found `+`
  --> src/main.blood:2:13
   |
 2 |     let x = + 42;
   |             ^ unexpected token
```

**Fix:** Provide a valid expression. If you meant a positive number, simply write `42`.

**Related:** E0101 (unexpected EOF), E0103 (invalid item)

---

#### E0101: Unexpected End of File

Occurs when the parser expects more tokens but reaches the end of the file.

**Example:**
```blood
fn main() {
    let x = 42
```

**Error message:**
```
error[E0101]: unexpected end of file
  --> src/main.blood:2:14
   |
 2 |     let x = 42
   |              ^ expected `}`, found end of file
```

**Fix:** Complete the code structure. In this case, add `}` to close the function body.

**Related:** E0100 (unexpected token), E0104 (unclosed delimiter)

---

#### E0102: Missing Required Item

Occurs when a required syntactic element is missing.

**Example:**
```blood
fn main() {
    let x = 42
    let y = 10
}
```

**Error message:**
```
error[E0102]: expected `;` after statement
  --> src/main.blood:2:14
   |
 2 |     let x = 42
   |              ^ expected `;`
   |
   = help: add `;` at the end of the statement
```

**Fix:** Add the missing semicolon.

**Related:** E0100 (unexpected token)

---

#### E0103: Invalid Item in Context

Occurs when a syntactic construct appears in an invalid context.

**Example:**
```blood
fn main() {
    effect MyEffect {  // effects cannot be declared inside functions
        op get() -> i32;
    }
}
```

**Error message:**
```
error[E0103]: invalid item in this context
  --> src/main.blood:2:5
   |
 2 |     effect MyEffect {
   |     ^^^^^^ effects cannot be declared inside functions
```

**Fix:** Move the effect declaration to module scope.

**Related:** E0100 (unexpected token)

---

#### E0104: Unclosed Delimiter

Occurs when an opening delimiter has no matching closing delimiter.

**Example:**
```blood
fn main() {
    let items = [1, 2, 3;
}
```

**Error message:**
```
error[E0104]: unclosed delimiter
  --> src/main.blood:2:17
   |
 2 |     let items = [1, 2, 3;
   |                 ^ unclosed `[`
   |
   = help: check for matching opening and closing delimiters
```

**Fix:** Add the matching closing delimiter `]`.

**Related:** E0101 (unexpected EOF), E0102 (missing item)

---

#### E0105: Missing Type Annotation

Occurs when a type annotation is required but not provided.

**Example:**
```blood
fn process(x) -> i32 {  // parameter needs type annotation
    x * 2
}
```

**Error message:**
```
error[E0105]: missing type annotation
  --> src/main.blood:1:12
   |
 1 | fn process(x) -> i32 {
   |            ^ expected type annotation
   |
   = help: add a type annotation after the `:`
```

**Fix:** Add a type annotation: `fn process(x: i32) -> i32`.

**Related:** E0202 (cannot infer type)

---

#### E0106: Invalid Pattern

Occurs when a pattern is syntactically invalid.

**Example:**
```blood
fn main() {
    let 42 + x = value;  // not a valid pattern
}
```

**Error message:**
```
error[E0106]: invalid pattern
  --> src/main.blood:2:9
   |
 2 |     let 42 + x = value;
   |         ^^^^^^ expected pattern, found expression
```

**Fix:** Use a valid pattern such as a variable binding, tuple destructuring, or struct pattern.

**Related:** E0226 (pattern mismatch)

---

#### E0107: Invalid Expression

Occurs when an expression is syntactically invalid.

**Example:**
```blood
fn main() {
    let x = if;  // if without condition
}
```

**Error message:**
```
error[E0107]: invalid expression
  --> src/main.blood:2:13
   |
 2 |     let x = if;
   |             ^^ expected condition after `if`
```

**Fix:** Provide a complete expression.

**Related:** E0100 (unexpected token)

---

#### E0108: Expected Identifier

Occurs when an identifier is expected but something else is found.

**Example:**
```blood
fn 42() {  // function name must be an identifier
}
```

**Error message:**
```
error[E0108]: expected identifier
  --> src/main.blood:1:4
   |
 1 | fn 42() {
   |    ^^ expected identifier, found integer literal
```

**Fix:** Use a valid identifier (starts with a letter or underscore).

**Related:** E0100 (unexpected token)

---

#### E0109: Expected Type

Occurs when a type is expected but something else is found.

**Example:**
```blood
fn process(x: 42) -> i32 {  // 42 is not a type
    x
}
```

**Error message:**
```
error[E0109]: expected type
  --> src/main.blood:1:15
   |
 1 | fn process(x: 42) -> i32 {
   |               ^^ expected type, found integer literal
```

**Fix:** Use a valid type name.

**Related:** E0203 (type not found)

---

#### E0110: Expected Expression

Occurs when an expression is expected but something else is found.

**Example:**
```blood
fn main() {
    let x = fn;  // `fn` is a keyword, not an expression
}
```

**Error message:**
```
error[E0110]: expected expression
  --> src/main.blood:2:13
   |
 2 |     let x = fn;
   |             ^^ expected expression, found keyword `fn`
```

**Fix:** Provide a valid expression.

**Related:** E0107 (invalid expression)

---

#### E0111: Expected Pattern

Occurs when a pattern is expected but something else is found.

**Example:**
```blood
fn main() {
    match value {
        fn => println("matched"),  // `fn` is not a pattern
    }
}
```

**Error message:**
```
error[E0111]: expected pattern
  --> src/main.blood:3:9
   |
 3 |         fn => println("matched"),
   |         ^^ expected pattern, found keyword `fn`
```

**Fix:** Use a valid pattern.

**Related:** E0106 (invalid pattern)

---

#### E0112: Duplicate Modifier

Occurs when a modifier keyword appears more than once.

**Example:**
```blood
pub pub fn greet() {
    println("hello");
}
```

**Error message:**
```
error[E0112]: duplicate modifier
  --> src/main.blood:1:5
   |
 1 | pub pub fn greet() {
   |     ^^^ duplicate `pub` modifier
```

**Fix:** Remove the duplicate modifier.

**Related:** E0113 (invalid visibility)

---

#### E0113: Invalid Visibility Specifier

Occurs when a visibility modifier is used incorrectly.

**Example:**
```blood
fn main() {
    pub let x = 42;  // local variables cannot be pub
}
```

**Error message:**
```
error[E0113]: invalid visibility specifier
  --> src/main.blood:2:5
   |
 2 |     pub let x = 42;
   |     ^^^ visibility modifiers are not allowed on local variables
```

**Fix:** Remove the visibility modifier from local declarations.

**Related:** E0112 (duplicate modifier)

---

#### E0114: Missing Function Body

Occurs when a function declaration lacks a body where one is required.

**Example:**
```blood
fn compute() -> i32
```

**Error message:**
```
error[E0114]: missing function body
  --> src/main.blood:1:1
   |
 1 | fn compute() -> i32
   | ^^^^^^^^^^^^^^^^^^^ missing function body
   |
   = help: add a function body `{ ... }` or use `;` for a declaration
```

**Fix:** Add a function body or a semicolon if this is a declaration.

**Related:** E0102 (missing item)

---

#### E0115: Invalid Match Arm

Occurs when a match arm is syntactically invalid.

**Example:**
```blood
fn main() {
    match x {
        1, 2, 3 => println("small"),  // should use `|` not `,`
    }
}
```

**Error message:**
```
error[E0115]: invalid match arm
  --> src/main.blood:3:9
   |
 3 |         1, 2, 3 => println("small"),
   |         ^^^^^^^ invalid match arm syntax
   |
   = help: use `|` to match multiple patterns: `1 | 2 | 3 => ...`
```

**Fix:** Use correct match arm syntax with `|` for alternatives.

**Related:** E0106 (invalid pattern)

---

### 4.3 Type Errors (E0200-E0299)

#### E0201: Type Mismatch

Occurs when the expected type doesn't match the actual type of an expression.

**Example:**
```blood
fn greet() -> String {
    42  // expected String, found i32
}
```

**Error message:**
```
error[E0201]: type mismatch: expected `String`, found `i32`
  --> src/main.blood:2:5
   |
 1 | fn greet() -> String {
   |               ------ expected `String` because of return type
 2 |     42
   |     ^^ expected `String`, found `i32`
```

**Fix:** Return a value of the expected type, or change the function's return type.

**Related:** E0220 (return type mismatch)

---

#### E0202: Cannot Infer Type

Occurs when the type checker cannot determine a type from context.

**Example:**
```blood
fn main() {
    let x = Vec::new();  // what type of Vec?
}
```

**Error message:**
```
error[E0202]: type annotations needed
  --> src/main.blood:2:9
   |
 2 |     let x = Vec::new();
   |         ^ cannot infer type
   |
   = help: consider giving `x` an explicit type: `let x: Vec<i32> = Vec::new()`
```

**Fix:** Add a type annotation to provide enough information for type inference.

**Related:** E0105 (missing type annotation)

---

#### E0203: Type Not Found

Occurs when a type name is not defined in the current scope.

**Example:**
```blood
fn process(x: Strng) -> i32 {  // typo: `Strng` instead of `String`
    x.len() as i32
}
```

**Error message:**
```
error[E0203]: cannot find type `Strng` in this scope
  --> src/main.blood:1:15
   |
 1 | fn process(x: Strng) -> i32 {
   |               ^^^^^ not found in this scope
   |
   = help: did you mean `String`?
```

**Fix:** Use the correct type name or import the type if it's defined elsewhere.

**Related:** E0207 (trait not found), E0208 (value not found)

---

#### E0204: Wrong Number of Type Arguments

Occurs when a generic type is instantiated with the wrong number of type arguments.

**Example:**
```blood
fn main() {
    let x: HashMap<String> = HashMap::new();  // missing value type
}
```

**Error message:**
```
error[E0204]: wrong number of type arguments: expected 2, found 1
  --> src/main.blood:2:12
   |
 2 |     let x: HashMap<String> = HashMap::new();
   |            ^^^^^^^^^^^^^^^ expected 2 type arguments
   |
   = note: `HashMap` requires 2 type parameters: key type and value type
```

**Fix:** Provide the correct number of type arguments.

**Related:** E0211 (wrong arity for function)

---

#### E0205: Recursive Type Has Infinite Size

Occurs when a type is defined recursively without indirection (Box, Rc, etc.).

**Example:**
```blood
struct Node {
    value: i32,
    next: Node,  // infinite recursion without Box
}
```

**Error message:**
```
error[E0205]: recursive type `Node` has infinite size
  --> src/main.blood:1:1
   |
 1 | struct Node {
   | ^^^^^^^^^^^ recursive type has infinite size
   |
   = note: recursive types must use indirection
   = help: use `Box<Node>` for the recursive field
```

**Fix:** Use `Box<Node>` or another pointer type for the recursive field.

**Related:** None

---

#### E0206: Trait Bound Not Satisfied

Occurs when a type doesn't implement a required trait.

**Example:**
```blood
fn print_debug<T: Debug>(x: T) {
    println("{:?}", x);
}

fn main() {
    print_debug(MyStruct { value: 42 });  // MyStruct doesn't implement Debug
}
```

**Error message:**
```
error[E0206]: the trait bound `MyStruct: Debug` is not satisfied
  --> src/main.blood:6:17
   |
 6 |     print_debug(MyStruct { value: 42 });
   |                 ^^^^^^^^^^^^^^^^^^^^^^^ `MyStruct` does not implement `Debug`
   |
   = help: implement `Debug` for `MyStruct` or derive it with `#[derive(Debug)]`
```

**Fix:** Implement the required trait or use a different type.

**Related:** E0207 (trait not found), E0232 (missing trait method)

---

#### E0207: Trait Not Found

Occurs when a trait name is not defined in the current scope.

**Example:**
```blood
fn process<T: Seralize>(x: T) {  // typo: `Seralize` instead of `Serialize`
    // ...
}
```

**Error message:**
```
error[E0207]: cannot find trait `Seralize` in this scope
  --> src/main.blood:1:15
   |
 1 | fn process<T: Seralize>(x: T) {
   |               ^^^^^^^^ not found
   |
   = help: did you mean `Serialize`?
```

**Fix:** Use the correct trait name or import it.

**Related:** E0203 (type not found), E0206 (trait bound not satisfied)

---

#### E0208: Value Not Found

Occurs when a name is used but not defined in the current scope.

**Example:**
```blood
fn main() {
    println(messge);  // typo: `messge` instead of `message`
}
```

**Error message:**
```
error[E0208]: cannot find value `messge` in this scope
  --> src/main.blood:2:13
   |
 2 |     println(messge);
   |             ^^^^^^ not found in this scope
   |
   = help: did you mean `message`?
```

**Fix:** Define the variable before using it, or fix the typo.

**Related:** E0203 (type not found), E0229 (unresolved name)

---

#### E0209: Not a Type

Occurs when a value or function is used where a type is expected.

**Example:**
```blood
fn main() {
    let x: println = 42;  // `println` is a function, not a type
}
```

**Error message:**
```
error[E0209]: `println` is not a type
  --> src/main.blood:2:12
   |
 2 |     let x: println = 42;
   |            ^^^^^^^ expected type, found function
```

**Fix:** Use a valid type name.

**Related:** E0203 (type not found)

---

#### E0210: Expected Function

Occurs when something that is not a function is called as if it were.

**Example:**
```blood
fn main() {
    let x = 42;
    x();  // i32 is not callable
}
```

**Error message:**
```
error[E0210]: expected function, found `i32`
  --> src/main.blood:3:5
   |
 3 |     x();
   |     ^^^ expected function, found `i32`
```

**Fix:** Ensure you're calling a function or callable value.

**Related:** E0211 (wrong arity)

---

#### E0211: Wrong Number of Arguments

Occurs when a function is called with the wrong number of arguments.

**Example:**
```blood
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    add(1);  // missing second argument
}
```

**Error message:**
```
error[E0211]: this function takes 2 argument(s) but 1 were supplied
  --> src/main.blood:6:5
   |
 1 | fn add(a: i32, b: i32) -> i32 {
   |        ------  ------ expected 2 arguments
   |
 6 |     add(1);
   |     ^^^^^^ supplied 1 argument
```

**Fix:** Provide the correct number of arguments.

**Related:** E0204 (wrong type arguments), E0210 (not a function)

---

#### E0212: Not a Struct

Occurs when struct syntax is used on a non-struct type.

**Example:**
```blood
fn main() {
    let x = i32 { value: 42 };  // i32 is not a struct
}
```

**Error message:**
```
error[E0212]: `i32` is not a struct
  --> src/main.blood:2:13
   |
 2 |     let x = i32 { value: 42 };
   |             ^^^ expected struct type
```

**Fix:** Use a struct type or use the appropriate literal syntax.

**Related:** E0213 (no field)

---

#### E0213: No Field on Type

Occurs when accessing a field that doesn't exist on a type.

**Example:**
```blood
struct Point { x: i32, y: i32 }

fn main() {
    let p = Point { x: 1, y: 2 };
    println(p.z);  // Point has no field `z`
}
```

**Error message:**
```
error[E0213]: no field `z` on type `Point`
  --> src/main.blood:5:15
   |
 5 |     println(p.z);
   |               ^ unknown field
   |
   = note: available fields: `x`, `y`
```

**Fix:** Use a field that exists on the struct.

**Related:** E0225 (missing field), E0230 (unknown field)

---

#### E0214: Duplicate Definition

Occurs when the same name is defined multiple times in the same scope.

**Example:**
```blood
fn main() {
    let x = 42;
    let x = "hello";  // this is allowed (shadowing)
}

fn foo() {}
fn foo() {}  // ERROR: duplicate definition
```

**Error message:**
```
error[E0214]: the name `foo` is defined multiple times
  --> src/main.blood:7:1
   |
 6 | fn foo() {}
   | ----------- previous definition here
 7 | fn foo() {}
   | ^^^^^^^^^^^ duplicate definition
```

**Fix:** Rename one of the definitions or remove the duplicate.

**Related:** E0234 (overlapping impls)

---

#### E0215: Cannot Dereference

Occurs when attempting to dereference a type that doesn't support it.

**Example:**
```blood
fn main() {
    let x = 42;
    let y = *x;  // i32 cannot be dereferenced
}
```

**Error message:**
```
error[E0215]: type `i32` cannot be dereferenced
  --> src/main.blood:3:13
   |
 3 |     let y = *x;
   |             ^^ cannot dereference
   |
   = note: only pointer and reference types can be dereferenced
```

**Fix:** Ensure you're dereferencing a pointer or reference type.

**Related:** E0216 (invalid binary op)

---

#### E0216: Invalid Binary Operator

Occurs when a binary operator is used with incompatible types.

**Example:**
```blood
fn main() {
    let result = "hello" - "world";  // cannot subtract strings
}
```

**Error message:**
```
error[E0216]: cannot apply `-` to `String` and `String`
  --> src/main.blood:2:18
   |
 2 |     let result = "hello" - "world";
   |                  ^^^^^^^ ^ ^^^^^^^ String
   |                  |
   |                  String
   |
   = note: the `-` operator is not defined for `String`
```

**Fix:** Use compatible types or implement the operator for your types.

**Related:** E0217 (invalid unary op)

---

#### E0217: Invalid Unary Operator

Occurs when a unary operator is used with an incompatible type.

**Example:**
```blood
fn main() {
    let x = !"hello";  // cannot negate a string
}
```

**Error message:**
```
error[E0217]: cannot apply `!` to `String`
  --> src/main.blood:2:13
   |
 2 |     let x = !"hello";
   |             ^^^^^^^^ `!` cannot be applied to `String`
```

**Fix:** Use a compatible type (e.g., `bool` for `!`).

**Related:** E0216 (invalid binary op)

---

#### E0218: Not Indexable

Occurs when indexing into a type that doesn't support indexing.

**Example:**
```blood
fn main() {
    let x = 42;
    let y = x[0];  // i32 is not indexable
}
```

**Error message:**
```
error[E0218]: cannot index into a value of type `i32`
  --> src/main.blood:3:13
   |
 3 |     let y = x[0];
   |             ^^^^ `i32` cannot be indexed
   |
   = note: only arrays, slices, and types implementing `Index` can be indexed
```

**Fix:** Use an indexable type like an array or slice.

**Related:** E0213 (no field)

---

#### E0219: Wrong Main Signature

Occurs when the `main` function has an incorrect signature.

**Example:**
```blood
fn main(x: i32) {  // main cannot take arguments
    println(x);
}
```

**Error message:**
```
error[E0219]: `main` function has wrong signature
  --> src/main.blood:1:1
   |
 1 | fn main(x: i32) {
   | ^^^^^^^^^^^^^^^ `main` function has wrong signature
   |
   = note: `main` must have signature `fn main()` or `fn main() -> i32`
```

**Fix:** Use the correct signature for `main`.

**Related:** E0221 (no main function)

---

#### E0220: Return Type Mismatch

Occurs when a function's return value doesn't match its declared return type.

**Example:**
```blood
fn compute() -> i32 {
    "hello"  // returns String, declared as i32
}
```

**Error message:**
```
error[E0220]: return type mismatch: expected `i32`, found `String`
  --> src/main.blood:2:5
   |
 1 | fn compute() -> i32 {
   |                 --- expected `i32` because of return type
 2 |     "hello"
   |     ^^^^^^^ expected `i32`, found `String`
```

**Fix:** Return a value of the correct type or change the return type annotation.

**Related:** E0201 (type mismatch)

---

#### E0221: No Main Function

Occurs when compiling an executable that lacks a `main` function.

**Example:**
```blood
fn greet() {
    println("Hello!");
}
// no main function
```

**Error message:**
```
error[E0221]: `main` function not found
  --> src/main.blood
   |
   = note: a `main` function is required for executable programs
```

**Fix:** Add a `main` function to your program.

**Related:** E0219 (main signature)

---

#### E0222: Break Outside Loop

Occurs when `break` is used outside of a loop.

**Example:**
```blood
fn main() {
    break;  // not inside a loop
}
```

**Error message:**
```
error[E0222]: `break` outside of loop
  --> src/main.blood:2:5
   |
 2 |     break;
   |     ^^^^^ cannot `break` outside of a loop
```

**Fix:** Use `break` only inside `loop`, `while`, or `for` constructs.

**Related:** E0223 (continue outside loop), E0224 (return outside function)

---

#### E0223: Continue Outside Loop

Occurs when `continue` is used outside of a loop.

**Example:**
```blood
fn main() {
    continue;  // not inside a loop
}
```

**Error message:**
```
error[E0223]: `continue` outside of loop
  --> src/main.blood:2:5
   |
 2 |     continue;
   |     ^^^^^^^^ cannot `continue` outside of a loop
```

**Fix:** Use `continue` only inside `loop`, `while`, or `for` constructs.

**Related:** E0222 (break outside loop)

---

#### E0224: Return Outside Function

Occurs when `return` is used outside of a function body.

**Example:**
```blood
return 42;  // at module level

fn main() {}
```

**Error message:**
```
error[E0224]: `return` outside of function
  --> src/main.blood:1:1
   |
 1 | return 42;
   | ^^^^^^^^^ cannot `return` outside of a function
```

**Fix:** Use `return` only inside function bodies.

**Related:** E0222 (break outside loop)

---

#### E0225: Missing Field in Initializer

Occurs when a struct initializer is missing a required field.

**Example:**
```blood
struct Point { x: i32, y: i32 }

fn main() {
    let p = Point { x: 1 };  // missing `y`
}
```

**Error message:**
```
error[E0225]: missing field `y` in initializer of `Point`
  --> src/main.blood:4:13
   |
 4 |     let p = Point { x: 1 };
   |             ^^^^^^^^^^^^^^ missing field `y`
```

**Fix:** Provide all required fields in the initializer.

**Related:** E0213 (no field), E0230 (unknown field)

---

#### E0226: Pattern Mismatch

Occurs when a pattern doesn't match the type being matched against.

**Example:**
```blood
fn main() {
    let x: i32 = 42;
    match x {
        Some(v) => println(v),  // i32 is not Option
        None => {},
    }
}
```

**Error message:**
```
error[E0226]: pattern `Some(_)` not covered by type `i32`
  --> src/main.blood:4:9
   |
 4 |         Some(v) => println(v),
   |         ^^^^^^^ pattern not applicable to `i32`
```

**Fix:** Use patterns that match the type being matched.

**Related:** E0236 (non-exhaustive patterns)

---

#### E0227: Not a Tuple

Occurs when tuple access syntax is used on a non-tuple type.

**Example:**
```blood
fn main() {
    let x: i32 = 42;
    let y = x.0;  // i32 is not a tuple
}
```

**Error message:**
```
error[E0227]: `i32` is not a tuple
  --> src/main.blood:3:13
   |
 3 |     let y = x.0;
   |             ^^^ expected tuple type
```

**Fix:** Use tuple types for tuple field access.

**Related:** E0212 (not a struct)

---

#### E0228: Unsupported Feature

Occurs when a language feature is not yet implemented.

**Example:**
```blood
fn main() {
    async fn fetch() { }  // async not yet supported
}
```

**Error message:**
```
error[E0228]: unsupported feature: async functions
  --> src/main.blood:2:5
   |
 2 |     async fn fetch() { }
   |     ^^^^^ async functions are not yet supported
```

**Fix:** Use an alternative approach or wait for the feature to be implemented.

**Related:** None

---

#### E0229: Unresolved Name

Occurs when a name cannot be resolved in the current scope.

**Example:**
```blood
fn main() {
    let x = unknown_function();
}
```

**Error message:**
```
error[E0229]: cannot find `unknown_function` in this scope
  --> src/main.blood:2:13
   |
 2 |     let x = unknown_function();
   |             ^^^^^^^^^^^^^^^^ not found in this scope
```

**Fix:** Define the function or import it from another module.

**Related:** E0208 (value not found)

---

#### E0230: Unknown Field

Occurs when an unknown field is used in a struct expression.

**Example:**
```blood
struct Point { x: i32, y: i32 }

fn main() {
    let p = Point { x: 1, y: 2, z: 3 };  // `z` is not a field
}
```

**Error message:**
```
error[E0230]: unknown field `z` on type `Point`
  --> src/main.blood:4:33
   |
 4 |     let p = Point { x: 1, y: 2, z: 3 };
   |                                 ^ unknown field
   |
   = note: available fields: `x`, `y`
```

**Fix:** Remove the unknown field or add it to the struct definition.

**Related:** E0213 (no field), E0225 (missing field)

---

#### E0231: Method Not Found

Occurs when calling a method that doesn't exist on a type.

**Example:**
```blood
fn main() {
    let x = 42;
    x.push(1);  // i32 has no method `push`
}
```

**Error message:**
```
error[E0231]: no method named `push` found for type `i32` in the current scope
  --> src/main.blood:3:7
   |
 3 |     x.push(1);
   |       ^^^^ method not found
```

**Fix:** Use a method that exists on the type, or implement the method.

**Related:** E0208 (value not found)

---

#### E0232: Missing Trait Method

Occurs when a trait implementation is missing a required method.

**Example:**
```blood
trait Greet {
    fn greet(&self) -> String;
    fn farewell(&self) -> String;
}

impl Greet for Person {
    fn greet(&self) -> String {
        "Hello!".to_string()
    }
    // missing `farewell` implementation
}
```

**Error message:**
```
error[E0232]: not all trait items implemented, missing: `farewell` from trait `Greet`
  --> src/main.blood:6:1
   |
 6 | impl Greet for Person {
   | ^^^^^^^^^^^^^^^^^^^^^ missing `farewell`
```

**Fix:** Implement all required trait methods.

**Related:** E0233 (missing associated type)

---

#### E0233: Missing Associated Type

Occurs when a trait implementation is missing a required associated type.

**Example:**
```blood
trait Container {
    type Item;
    fn get(&self) -> Self::Item;
}

impl Container for MyBox {
    // missing `type Item = ...`
    fn get(&self) -> i32 { self.value }
}
```

**Error message:**
```
error[E0233]: not all trait items implemented, missing: `type Item` from trait `Container`
  --> src/main.blood:6:1
   |
 6 | impl Container for MyBox {
   | ^^^^^^^^^^^^^^^^^^^^^^^^ missing `type Item`
```

**Fix:** Define all required associated types.

**Related:** E0232 (missing trait method)

---

#### E0234: Overlapping Implementations

Occurs when two trait implementations could apply to the same type.

**Example:**
```blood
trait Process {}

impl<T> Process for T {}
impl Process for i32 {}  // overlaps with the blanket impl
```

**Error message:**
```
error[E0234]: conflicting implementations of trait `Process` for type `i32` and `T`
  --> src/main.blood:4:1
   |
 3 | impl<T> Process for T {}
   | ----------------------- first implementation here
 4 | impl Process for i32 {}
   | ^^^^^^^^^^^^^^^^^^^^^^ conflicting implementation
```

**Fix:** Remove one of the conflicting implementations or add trait bounds to make them non-overlapping.

**Related:** E0214 (duplicate definition)

---

#### E0235: Const Evaluation Error

Occurs when a compile-time const evaluation fails.

**Example:**
```blood
const X: i32 = 1 / 0;  // division by zero
```

**Error message:**
```
error[E0235]: const evaluation failed: division by zero
  --> src/main.blood:1:16
   |
 1 | const X: i32 = 1 / 0;
   |                ^^^^^ attempt to divide by zero
```

**Fix:** Ensure const expressions can be evaluated at compile time without errors.

**Related:** None

---

#### E0236: Non-Exhaustive Patterns

Occurs when a match expression doesn't cover all possible patterns.

**Example:**
```blood
enum Color { Red, Green, Blue }

fn main() {
    let c = Color::Red;
    match c {
        Color::Red => println("red"),
        Color::Green => println("green"),
        // missing Blue
    }
}
```

**Error message:**
```
error[E0236]: non-exhaustive patterns: `Blue` not covered
  --> src/main.blood:5:11
   |
 5 |     match c {
   |           ^ pattern `Blue` not covered
   |
   = note: ensure all variants are handled
   = help: add a `Color::Blue => ...` arm or use `_ => ...` for a catch-all
```

**Fix:** Add the missing pattern or use a wildcard `_` pattern.

**Related:** E0237 (unreachable pattern)

---

#### E0237: Unreachable Pattern

Occurs when a pattern can never be matched because it's covered by a previous pattern.

**Example:**
```blood
fn main() {
    let x = 42;
    match x {
        _ => println("anything"),
        42 => println("forty-two"),  // unreachable
    }
}
```

**Error message:**
```
error[E0237]: unreachable pattern
  --> src/main.blood:5:9
   |
 4 |         _ => println("anything"),
   |         - matches any value
 5 |         42 => println("forty-two"),
   |         ^^ unreachable pattern
```

**Fix:** Reorder patterns so more specific patterns come before general ones, or remove the unreachable pattern.

**Related:** E0236 (non-exhaustive patterns)

---

### 4.4 Effect Errors (E0300-E0399)

#### E0301: Unhandled Effect

Occurs when a function performs an effect that isn't handled or declared.

**Example:**
```blood
effect IO {
    op read_line() -> String;
}

fn pure_function() -> String / pure {  // declared as pure
    perform IO::read_line()  // but performs IO
}
```

**Error message:**
```
error[E0301]: unhandled effect `IO`
  --> src/main.blood:6:5
   |
 5 | fn pure_function() -> String / pure {
   |                                ---- function declared as `pure`
 6 |     perform IO::read_line()
   |     ^^^^^^^^^^^^^^^^^^^^^^^ performs `IO` effect
   |
   = note: `read_line` has effect signature `/ {IO}`
   = help: either:
           - add `IO` to the effect row: `fn pure_function() -> String / {IO}`
           - handle the effect with a handler
```

**Fix:** Add the effect to the function's effect row or handle it with a handler.

**Related:** E0302 (effect mismatch), E0308 (undeclared effects)

---

#### E0302: Effect Signature Mismatch

Occurs when effect types don't match between a handler and the computation it handles.

**Example:**
```blood
effect State<T> {
    op get() -> T;
    op set(value: T);
}

handler IntState for State<i32> {
    // handles State<i32>
}

fn main() {
    with IntState handle {
        let s: String = perform State::get();  // expects State<String>
    }
}
```

**Error message:**
```
error[E0302]: effect mismatch: expected `State<String>`, found `State<i32>`
  --> src/main.blood:11:5
   |
11 |     with IntState handle {
   |          ^^^^^^^^
   |          handles: `State<i32>`
   |          required: `State<String>`
```

**Fix:** Use a handler that matches the required effect type.

**Related:** E0301 (unhandled effect), E0303 (resume type mismatch)

---

#### E0303: Resume Type Mismatch

Occurs when the value passed to `resume` doesn't match the expected type.

**Example:**
```blood
effect Ask {
    op ask() -> String;
}

handler AnswerHandler for Ask {
    op ask() -> String {
        resume(42)  // should be String, not i32
    }
}
```

**Error message:**
```
error[E0303]: resume type mismatch: expected `String`, found `i32`
  --> src/main.blood:8:16
   |
 3 |     op ask() -> String;
   |                 ------ expected `String` from operation signature
   |
 8 |         resume(42)
   |                ^^ expected `String`, found `i32`
```

**Fix:** Pass a value of the correct type to `resume`.

**Related:** E0302 (effect mismatch)

---

#### E0305: Invalid Effect Handler

Occurs when a handler is structurally invalid.

**Example:**
```blood
effect Counter {
    op increment();
    op get() -> i32;
}

handler BadHandler for Counter {
    // missing increment operation
    op get() -> i32 {
        resume(0)
    }
}
```

**Error message:**
```
error[E0305]: invalid effect handler: missing operation `increment`
  --> src/main.blood:7:1
   |
 7 | handler BadHandler for Counter {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ handler missing operations
   |
   = note: handler must implement all operations from effect `Counter`
```

**Fix:** Implement all operations from the effect being handled.

**Related:** E0232 (missing trait method)

---

#### E0306: Not an Effect

Occurs when a non-effect type is used where an effect is expected.

**Example:**
```blood
struct MyStruct {}

handler BadHandler for MyStruct {  // MyStruct is not an effect
    // ...
}
```

**Error message:**
```
error[E0306]: `MyStruct` is not an effect
  --> src/main.blood:3:24
   |
 3 | handler BadHandler for MyStruct {
   |                        ^^^^^^^^ expected effect type
   |
   = note: use `effect` to declare an effect type
```

**Fix:** Use an effect type, not a struct or other type.

**Related:** E0209 (not a type)

---

#### E0307: Invalid Effect Type Syntax

Occurs when the effect type syntax is malformed.

**Example:**
```blood
fn perform_io() -> String / i32 {  // i32 is not an effect
    // ...
}
```

**Error message:**
```
error[E0307]: invalid effect type syntax: expected a named effect like `State<T>`, found i32
  --> src/main.blood:1:29
   |
 1 | fn perform_io() -> String / i32 {
   |                             ^^^ expected effect type
```

**Fix:** Use a valid effect name in the effect row.

**Related:** E0306 (not an effect)

---

#### E0308: Undeclared Effects

Occurs when a function performs effects not declared in its signature.

**Example:**
```blood
effect Log {
    op log(msg: String);
}

fn process() -> i32 {  // no effect declaration
    perform Log::log("processing");  // but performs Log effect
    42
}
```

**Error message:**
```
error[E0308]: function performs undeclared effects: Log
  --> src/main.blood:5:1
   |
 5 | fn process() -> i32 {
   | ^^^^^^^^^^^^^^^^^^^^ performs undeclared effect `Log`
 6 |     perform Log::log("processing");
   |     ------------------------------ effect performed here
   |
   = help: add effect to signature: `fn process() -> i32 / {Log}`
```

**Fix:** Declare the effects in the function signature.

**Related:** E0301 (unhandled effect)

---

### 4.5 Ownership Errors (E0400-E0499)

#### E0401: Cannot Borrow as Mutable

Occurs when attempting to mutably borrow something that cannot be borrowed mutably.

**Example:**
```blood
fn main() {
    let items = vec![1, 2, 3];  // immutable binding
    items.push(4);  // requires mutable borrow
}
```

**Error message:**
```
error[E0401]: cannot borrow as mutable: `items` is not declared as mutable
  --> src/main.blood:3:5
   |
 2 |     let items = vec![1, 2, 3];
   |         ----- help: consider changing this to `let mut items`
 3 |     items.push(4);
   |     ^^^^^ cannot borrow as mutable
```

**Fix:** Declare the binding as mutable with `let mut`.

**Related:** E0801 (unused linear value), E0802 (multiple linear use)

---

### 4.6 Internal Compiler Errors (E9000-E9999)

#### E9000: Internal Compiler Error

Indicates a bug in the Blood compiler itself. This should never be triggered by user code.

**Example output:**
```
internal compiler error: type variable should have been resolved before codegen
  --> bloodc/src/codegen/mod.rs:142

This is a bug in the Blood compiler. Please report it at:
https://github.com/blood-lang/blood/issues

Context:
  - type: TypeVar(42)
  - expected: concrete type
```

**What to do:** Report the bug at the Blood issue tracker with the full error message and if possible, a minimal reproducing example.

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
error[E0203]: cannot find type `HashMap`
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
error[E0203]: undefined type `Strng`    // Typo
error[E0201]: cannot add `<error>` and `i32`  // Cascade
error[E0201]: cannot call method on `<error>` // Cascade

// GOOD: Suppress cascades
error[E0203]: undefined type `Strng`
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

**Legend**: Implemented and tested, In development, Planned

### Lexer Errors (E0001-E0099) — Implemented

| Code | Description | Status |
|------|-------------|--------|
| E0001 | Unexpected character | Implemented |
| E0002 | Unclosed block comment | Implemented |
| E0003 | Unclosed string literal | Implemented |
| E0004 | Invalid escape sequence | Implemented |
| E0005 | Invalid integer literal | Implemented |
| E0006 | Invalid float literal | Implemented |
| E0007 | Unclosed character literal | Implemented |
| E0008 | Empty character literal | Implemented |

### Parser Errors (E0100-E0199) — Implemented

| Code | Description | Status |
|------|-------------|--------|
| E0100 | Unexpected token | Implemented |
| E0101 | Unexpected end of file | Implemented |
| E0102 | Missing required item | Implemented |
| E0103 | Invalid item in context | Implemented |
| E0104 | Unclosed delimiter | Implemented |
| E0105 | Missing type annotation | Implemented |
| E0106 | Invalid pattern | Implemented |
| E0107 | Invalid expression | Implemented |
| E0108 | Expected identifier | Implemented |
| E0109 | Expected type | Implemented |
| E0110 | Expected expression | Implemented |
| E0111 | Expected pattern | Implemented |
| E0112 | Duplicate modifier | Implemented |
| E0113 | Invalid visibility | Implemented |
| E0114 | Missing function body | Implemented |
| E0115 | Invalid match arm | Implemented |

### Type Errors (E0200-E0299) — Implemented

| Code | Description | Status |
|------|-------------|--------|
| E0201 | Type mismatch | Implemented |
| E0202 | Cannot infer type | Implemented |
| E0203 | Type not found | Implemented |
| E0204 | Wrong type argument count | Implemented |
| E0205 | Recursive/infinite type | Implemented |
| E0206 | Trait bound not satisfied | Implemented |
| E0207 | Trait not found | Implemented |
| E0208 | Value not found | Implemented |
| E0209 | Not a type | Implemented |
| E0210 | Expected function | Implemented |
| E0211 | Wrong argument count | Implemented |
| E0212 | Not a struct | Implemented |
| E0213 | No field on type | Implemented |
| E0214 | Duplicate definition | Implemented |
| E0215 | Cannot dereference | Implemented |
| E0216 | Invalid binary operator | Implemented |
| E0217 | Invalid unary operator | Implemented |
| E0218 | Not indexable | Implemented |
| E0219 | Wrong main signature | Implemented |
| E0220 | Return type mismatch | Implemented |
| E0221 | No main function | Implemented |
| E0222 | Break outside loop | Implemented |
| E0223 | Continue outside loop | Implemented |
| E0224 | Return outside function | Implemented |
| E0225 | Missing field in initializer | Implemented |
| E0226 | Pattern mismatch | Implemented |
| E0227 | Not a tuple | Implemented |
| E0228 | Unsupported feature | Implemented |
| E0229 | Unresolved name | Implemented |
| E0230 | Unknown field | Implemented |
| E0231 | Method not found | Implemented |
| E0232 | Missing trait method | Implemented |
| E0233 | Missing associated type | Implemented |
| E0234 | Overlapping implementations | Implemented |
| E0235 | Const evaluation error | Implemented |
| E0236 | Non-exhaustive patterns | Implemented |
| E0237 | Unreachable pattern | Implemented |

### Effect Errors (E0300-E0399) — Implemented

| Code | Description | Status |
|------|-------------|--------|
| E0301 | Unhandled effect | Implemented |
| E0302 | Effect signature mismatch | Implemented |
| E0303 | Resume type mismatch | Implemented |
| E0304 | Linear value in multi-shot handler | Planned |
| E0305 | Invalid effect handler | Implemented |
| E0306 | Not an effect | Implemented |
| E0307 | Invalid effect type syntax | Implemented |
| E0308 | Undeclared effects | Implemented |

### Ownership Errors (E0400-E0499) — In Development

| Code | Description | Status |
|------|-------------|--------|
| E0401 | Cannot borrow as mutable | Implemented |

Note: Blood uses Mutable Value Semantics (MVS) with copy-by-default, not Rust-style
move semantics. Ownership errors for linear/affine types are in the E0800 range.
Blood does NOT implement compile-time borrow checking (see ADR-001).

### Lifetime Errors (E0500-E0599) — Planned

| Code | Description | Status |
|------|-------------|--------|
| E0501 | Reference outlives value | Planned |
| E0502 | Stale reference across effect | Planned |
| E0503 | Lifetime bound not satisfied | Planned |
| E0504 | Region escape | Planned |

### Dispatch Errors (E0600-E0699) — Planned

| Code | Description | Status |
|------|-------------|--------|
| E0601 | Ambiguous method dispatch | Planned |
| E0602 | No matching implementation | Planned |
| E0603 | Type instability | Planned |
| E0604 | Circular dispatch | Planned |

### Name Resolution Errors (E0700-E0799) — Implemented

| Code | Description | Status |
|------|-------------|--------|
| E0701 | Undefined name | Implemented |
| E0702 | Duplicate definition | Implemented |
| E0703 | Private item access | In Development |
| E0704 | Circular import | Planned |

### Internal Compiler Errors (E9000-E9999)

| Code | Description | Status |
|------|-------------|--------|
| E9000 | Internal compiler error | Implemented |

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
