# Blood Language Syntax Support Status

**Version**: 0.5.2
**Last Updated**: 2026-01-14
**Purpose**: Authoritative reference for what syntax IS and IS NOT supported by the Blood compiler

---

## Executive Summary

This document is the **single source of truth** for Blood language syntax support. The grammar specification (GRAMMAR.md) describes the *intended* language design. This document describes what the **parser actually accepts**.

**Key Finding**: Several examples in the repository use syntax that the parser does not support. These examples either need to be fixed to use valid Blood syntax, or marked as aspirational/future.

---

## 1. Syntax Actually Supported

### 1.1 Literals

| Literal Type | Syntax | Supported | Notes |
|--------------|--------|-----------|-------|
| Integer (decimal) | `42`, `1_000` | ✅ Yes | Underscores allowed |
| Integer (hex) | `0xFF`, `0xDEAD_BEEF` | ✅ Yes | |
| Integer (binary) | `0b1010` | ✅ Yes | |
| Integer (octal) | `0o777` | ✅ Yes | |
| Integer with suffix | `42i32`, `0u64` | ❌ **No** | Parser doesn't recognize type suffix |
| Float | `3.14`, `1.0e-5` | ✅ Yes | |
| String | `"hello"` | ✅ Yes | Standard escape sequences |
| Raw string | `r"..."`, `r#"..."#` | ✅ Yes | Up to `r##"..."##` |
| Byte string | `b"..."` | ❌ **No** | Not implemented |
| Char | `'a'`, `'\n'` | ✅ Yes | |
| Char (hex escape) | `'\xNN'` | ⚠️ Partial | May have issues |
| Bool | `true`, `false` | ✅ Yes | |

### 1.2 Paths

| Path Syntax | Example | Supported | Notes |
|-------------|---------|-----------|-------|
| Simple identifier | `foo` | ✅ Yes | |
| Module path | `std::vec::Vec` | ✅ Yes | In type position |
| Type path | `Option<T>` | ✅ Yes | Generic types |
| Path in function call | `Vec::new()` | ✅ Yes | Method-style |
| Path as argument | `foo(Bar::Baz)` | ❌ **No** | Parser fails: "expected `)`, found `::`" |
| Associated constant | `i64::MAX` | ❌ **No** | Same issue as above |
| Path in expression | `std::ptr::null()` | ❌ **No** | Same issue as above |

### 1.3 Macros

| Macro | Syntax | Supported | Notes |
|-------|--------|-----------|-------|
| `println!` | `println!("format", args)` | ✅ Yes | Built-in |
| `print!` | `print!("format", args)` | ✅ Yes | Built-in |
| `eprintln!` | `eprintln!("msg")` | ✅ Yes | Built-in |
| `format!` | `format!("template", args)` | ✅ Yes | Built-in |
| `vec!` | `vec![1, 2, 3]`, `vec![0; 10]` | ✅ Yes | Built-in |
| `matches!` | `matches!(expr, pattern)` | ✅ Yes | Built-in |
| `panic!` | `panic!("msg")` | ✅ Yes | Built-in |
| `assert!` | `assert!(cond)` | ✅ Yes | Built-in |
| `dbg!` | `dbg!(expr)` | ✅ Yes | Built-in |
| `todo!` | `todo!("msg")` | ❌ **No** | "user-defined macros are not yet supported" |
| `unimplemented!` | `unimplemented!()` | ❌ **No** | Same |
| Custom macros | `my_macro!(...)` | ❌ **No** | Same |

### 1.4 Keywords

The following are **reserved keywords** and CANNOT be used as identifiers:

```
fn, let, mut, const, static, struct, enum, union, type, trait, impl,
mod, use, pub, crate, super, self, Self, if, else, match, loop, while,
for, in, break, continue, return, effect, handler, handle, with, do,
resume, try, perform, as, where, dyn, true, false, box, move, ref, own,
async, await, spawn, extern, unsafe, pure, default
```

**Important**: Using reserved keywords as field names, parameter names, or function names will cause parser errors.

| Usage | Example | Result |
|-------|---------|--------|
| `default` as function name | `fn default() {}` | ❌ Parser error |
| `default` as parameter | `fn foo(default: i32)` | ❌ Parser error |
| `handle` as field name | `self.handle` | ❌ Parser error |
| `try` as expression | `let x = try { }` | ❌ Parser error |

### 1.5 Control Flow

| Construct | Syntax | Supported |
|-----------|--------|-----------|
| If/else | `if cond { } else { }` | ✅ Yes |
| If-let | `if let pat = expr { }` | ✅ Yes |
| Match | `match expr { pat => expr }` | ✅ Yes |
| Loop | `loop { }` | ✅ Yes |
| While | `while cond { }` | ✅ Yes |
| While-let | `while let pat = expr { }` | ✅ Yes |
| For | `for x in iter { }` | ✅ Yes |
| Break | `break`, `break 'label`, `break value` | ✅ Yes |
| Continue | `continue`, `continue 'label` | ✅ Yes |
| Return | `return`, `return expr` | ✅ Yes |
| Labeled loops | `'label: loop { }` | ✅ Yes |

### 1.6 Effects

| Construct | Syntax | Supported |
|-----------|--------|-----------|
| Effect declaration | `effect Name { op foo() -> T; }` | ✅ Yes |
| Handler declaration | `deep handler Name for Effect { }` | ✅ Yes |
| Handler declaration | `shallow handler Name for Effect { }` | ✅ Yes |
| Perform | `perform Effect.op(args)` | ✅ Yes |
| Resume | `resume(value)` | ✅ Yes |
| Handle | `with Handler { } handle { }` | ✅ Yes |

### 1.7 Operators

All standard operators are supported:
- Arithmetic: `+`, `-`, `*`, `/`, `%`
- Comparison: `==`, `!=`, `<`, `>`, `<=`, `>=`
- Logical: `&&`, `||`, `!`
- Bitwise: `&`, `|`, `^`, `~`, `<<`, `>>`
- Assignment: `=`, `+=`, `-=`, etc.
- Range: `..`, `..=`
- Pipe: `|>`
- Cast: `as`
- Reference: `&`, `&mut`
- Dereference: `*`

---

## 2. Syntax NOT Supported

### 2.1 Path Syntax Limitations

**Problem**: The parser does not support `::` path syntax in function call argument position.

```blood
// DOES NOT WORK
let x = foo(Bar::Baz);           // Error: expected `)`, found `::`
let max = i64::MAX;              // Error: expected `)`, found `::`
let ptr = std::ptr::null();      // Error: expected `)`, found `::`
let ch = char::from_u32(code);   // Error: expected `)`, found `::`

// WORKAROUNDS
let baz = Bar::Baz;              // Bind to variable first
let x = foo(baz);                // Then use variable
```

### 2.2 Integer Type Suffixes

**Problem**: The parser does not recognize type suffixes on integer literals.

```blood
// DOES NOT WORK
let x = 42i32;     // Error: expected `;`, found identifier
let y = 0u64;      // Error: expected `]`, found identifier
let z = [0u32; 10]; // Error: expected `]`, found identifier

// WORKAROUNDS
let x: i32 = 42;   // Use type annotation instead
let y: u64 = 0;
let z: [u32; 10] = [0; 10];
```

### 2.3 Byte String Literals

**Problem**: The parser does not support byte string literals.

```blood
// DOES NOT WORK
let bytes = b"hello";  // Error: expected `;`, found string literal

// WORKAROUNDS
// Use regular string and convert, or use array literal
let bytes = [104, 101, 108, 108, 111];  // "hello" as bytes
```

### 2.4 User-Defined Macros

**Problem**: Only built-in macros are supported. Custom macros produce an explicit error.

```blood
// DOES NOT WORK
todo!("implement this");         // Error: user-defined macros are not yet supported
unimplemented!();               // Same error
my_custom_macro!(x, y, z);       // Same error

// WORKAROUNDS
panic!("not yet implemented: implement this");  // Use panic! instead of todo!
```

### 2.5 Reserved Keyword Conflicts

**Problem**: Several common identifier names are reserved keywords.

```blood
// DOES NOT WORK
fn default() { }                 // Error: expected function name, found keyword
fn foo(default: i32) { }         // Error: expected identifier, found keyword
struct Foo { handle: Handle }    // Error: expected `}`, found keyword
let try = try_something();       // Error: reserved keyword

// WORKAROUNDS
fn default_value() { }           // Rename to avoid keyword
fn foo(default_val: i32) { }
struct Foo { file_handle: Handle }
let try_result = try_something();
```

### 2.6 Try Expression

**Problem**: `try` is a reserved keyword but not implemented as an expression.

```blood
// DOES NOT WORK
let result = try { risky_operation() };  // Error: expected expression, found keyword `try`

// WORKAROUNDS
// Use match or explicit error handling
match risky_operation() {
    Ok(v) => v,
    Err(e) => return Err(e),
}
```

---

## 3. Example File Status

### 3.1 Failing Examples (Need Fixes)

| File | Issue | Fix Required |
|------|-------|--------------|
| `argparse.blood` | Missing closing brace | Fix structure |
| `blood_lexer.blood` | `char::from_u32` path syntax | Bind to variable |
| `blood_parser.blood` | `todo!()` macro, `effects` field name | Use `panic!()`, rename field |
| `blood_typeck.blood` | `handler` field name | Rename field |
| `config_parser.blood` | `default` parameter name | Rename parameter |
| `gpio_driver.blood` | `reg::GPPUD` path syntax | Bind to variable |
| `gzip_compression.blood` | `0u32` suffix | Use type annotation |
| `http_client.blood` | `b"..."` byte string | Use byte array |
| `http_server.blood` | Structure mismatch | Fix structure |
| `json_parser.blood` | `'\x08'` escape | Check escape support |
| `markdown_parser.blood` | `try { }` expression | Use match/if-let |
| `order_book.blood` | `i64::MAX` constant | Define constant |
| `sqlite_driver.blood` | `std::ptr::null()` | Bind to variable |
| `state_machine.blood` | `handle` field name | Rename to `file_handle` |
| `web_scraper.blood` | `default` function name | Rename function |

### 3.2 Codegen Gaps (Not Syntax Issues)

| File | Issue | Category |
|------|-------|----------|
| `forall_types.blood` | Unknown function DefId | Codegen bug |
| `simple_effect_test.blood` | undefined reference to handler | Codegen/linking |

### 3.3 Correct Rejections

| File | Issue | Status |
|------|-------|--------|
| `shallow_multi_resume_error.blood` | Multi-resume in shallow handler | ✅ Correctly rejected |

### 3.4 Passing Examples (For Reference)

The following examples compile and run successfully, demonstrating valid Blood syntax:

- `algebraic_effects.blood`
- `array_pattern_test.blood`
- `basic_array.blood`
- `binary_tree_benchmark.blood`
- `concurrent_fibers.blood`
- `const_add.blood`, `const_simple.blood`, `const_static_test.blood`
- `content_addressing.blood`
- `data_structures.blood`
- `fannkuch_benchmark.blood`, `fasta_benchmark.blood`
- `ffi_interop.blood`
- `fizzbuzz.blood`
- `generational_memory.blood`
- `hello.blood`
- `math_algorithms.blood`
- `minimal.blood`
- `multi_perform_test.blood`, `multi_shot_resume.blood`
- `multiple_dispatch.blood`
- `nbody_benchmark.blood`
- `nested_handler_test.blood`
- `non_tail_resume.blood`
- `pipe_test.blood`
- `record_types.blood`
- `shallow_single_resume_ok.blood`
- `simple.blood`, `simple_deep.blood`, `simple_match.blood`
- `simple_nbody_test.blood`, `simple_nbody_test2.blood`, `simple_nbody_test3.blood`
- `simple_tuple.blood`
- `slice_pattern_test.blood`
- `sorting.blood`
- `spectral_norm_benchmark.blood`
- `static_test.blood`
- `strings.blood`
- `struct_ref_test.blood`
- `tuple_match.blood`, `tuple_match_nobind.blood`, `tuple_pass.blood`

---

## 4. Planned Syntax Additions

The following syntax is documented in GRAMMAR.md but not yet implemented:

| Feature | Status | Priority |
|---------|--------|----------|
| Path syntax in arguments | Not implemented | High |
| Integer type suffixes | Not implemented | Medium |
| Byte string literals | Not implemented | Medium |
| User-defined macros | Explicitly not supported | Low (design needed) |

---

## 5. How to Write Valid Blood Code

### 5.1 Avoid Path Syntax in Arguments

```blood
// Instead of:
foo(SomeType::SomeVariant)

// Do this:
let variant = SomeType::SomeVariant;
foo(variant)
```

### 5.2 Use Type Annotations Instead of Suffixes

```blood
// Instead of:
let x = 42i32;
let arr = [0u8; 256];

// Do this:
let x: i32 = 42;
let arr: [u8; 256] = [0; 256];
```

### 5.3 Avoid Reserved Keywords as Names

Check the keyword list in section 1.4. Common problematic names:
- `default` -> `default_value`, `defaults`
- `handle` -> `file_handle`, `resource_handle`
- `try` -> `attempt`, `try_op`
- `type` -> `kind`, `ty`

### 5.4 Use Built-in Macros Only

```blood
// Supported:
println!("message");
format!("template {}", value);
vec![1, 2, 3];
panic!("error");

// NOT supported (use panic! instead):
todo!("implement");
unimplemented!();
```

---

## 6. Reporting Issues

If you encounter a syntax error that you believe should work:

1. Check this document first
2. If the syntax is documented as supported but fails, report a bug
3. If the syntax is documented as not supported, this is expected behavior

---

*Document created 2026-01-14 based on comprehensive analysis of parser capabilities and example file testing.*
