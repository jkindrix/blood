# Migrating from Rust to Blood

**Version**: 1.0.0
**Status**: Reference
**Last Updated**: 2026-01-13

This guide helps Rust developers transition to Blood by mapping familiar Rust patterns to their Blood equivalents. Blood shares many concepts with Rust but takes different approaches in key areas.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Syntax Differences](#2-syntax-differences)
3. [Ownership and Borrowing](#3-ownership-and-borrowing)
4. [Error Handling](#4-error-handling)
5. [Traits to Effects](#5-traits-to-effects)
6. [Concurrency](#6-concurrency)
7. [Memory Management](#7-memory-management)
8. [Pattern Matching](#8-pattern-matching)
9. [Common Patterns](#9-common-patterns)
10. [Crate to Module Migration](#10-crate-to-module-migration)

---

## 1. Overview

### 1.1 Key Differences

| Aspect | Rust | Blood |
|--------|------|-------|
| **Memory Safety** | Borrow checker (compile-time) | Generational pointers (runtime-checked) |
| **Error Handling** | Result/Option + ? operator | Effect system with handlers |
| **Concurrency** | async/await, threads | Fibers with effect handlers |
| **Side Effects** | Not tracked in type system | Explicit effect annotations |
| **FFI** | unsafe blocks | Bridge dialect + @unsafe |
| **Macros** | proc_macro, macro_rules! | Hygenic macros (planned) |

### 1.2 Why Migrate?

Blood offers advantages for certain use cases:

- **Algebraic effects** provide more flexible error handling than Result
- **Generational pointers** eliminate lifetime complexity
- **Effect tracking** makes side effects explicit in function signatures
- **Fiber-based concurrency** is simpler than async/await
- **Region-based memory** provides deterministic deallocation

### 1.3 What Stays the Same

Blood inherits many Rust concepts:

- Expression-based syntax
- Pattern matching with `match`
- Struct and enum definitions
- Trait-like interfaces
- Generic types and functions
- Module system basics

---

## 2. Syntax Differences

### 2.1 Variable Bindings

```rust
// Rust
let x: i32 = 42;
let mut y = 0;
y += 1;
```

```blood
// Blood - identical syntax
let x: i32 = 42;
let mut y = 0;
y += 1;
```

### 2.2 Function Definitions

```rust
// Rust
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn greet(name: &str) {
    println!("Hello, {}!", name);
}
```

```blood
// Blood - effect annotations added
fn add(a: i32, b: i32) -> i32 / pure {
    a + b
}

fn greet(name: &str) / {IO} {
    println_str("Hello, ");
    println_str(name);
}
```

**Key difference**: Blood requires effect annotations (`/ pure` or `/ {Effects}`).

### 2.3 Struct Definitions

```rust
// Rust
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    fn distance(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}
```

```blood
// Blood - nearly identical
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn new(x: f64, y: f64) -> Self / pure {
        Point { x, y }
    }

    fn distance(&self, other: &Point) -> f64 / pure {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        sqrt(dx * dx + dy * dy)
    }
}
```

### 2.4 Enums

```rust
// Rust
enum Option<T> {
    Some(T),
    None,
}

enum Result<T, E> {
    Ok(T),
    Err(E),
}
```

```blood
// Blood - identical syntax
enum Option<T> {
    Some(T),
    None,
}

enum Result<T, E> {
    Ok(T),
    Err(E),
}
```

### 2.5 Closures

```rust
// Rust
let add = |a, b| a + b;
let double = |x| x * 2;
let mut counter = 0;
let increment = || { counter += 1; counter };
```

```blood
// Blood - similar syntax, effects explicit
let add = |a, b| -> i32 / pure { a + b };
let double = |x| -> i32 / pure { x * 2 };
let mut counter = 0;
let increment = || -> i32 / {State<i32>} {
    counter += 1;
    counter
};
```

---

## 3. Ownership and Borrowing

### 3.1 The Core Difference

**Rust**: Compile-time borrow checker tracks ownership and lifetimes.

**Blood**: Runtime generational checks verify pointer validity.

```rust
// Rust - compile-time lifetime tracking
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}
```

```blood
// Blood - no lifetime annotations needed
fn longest(x: &str, y: &str) -> &str / pure {
    if x.len() > y.len() { x } else { y }
}
```

### 3.2 No Lifetime Annotations

Blood eliminates lifetime complexity:

```rust
// Rust - complex lifetimes
struct Parser<'input, 'cache> {
    input: &'input str,
    cache: &'cache mut HashMap<String, Value>,
}

impl<'input, 'cache> Parser<'input, 'cache> {
    fn parse(&mut self) -> Result<Value, Error> { ... }
}
```

```blood
// Blood - no lifetimes
struct Parser {
    input: &str,
    cache: &mut HashMap<String, Value>,
}

impl Parser {
    fn parse(&mut self) -> Value / {Error<ParseError>} { ... }
}
```

### 3.3 Move Semantics

```rust
// Rust
let s1 = String::from("hello");
let s2 = s1;  // s1 is moved, cannot use
// println!("{}", s1);  // ERROR: value moved
```

```blood
// Blood - similar move semantics for linear types
let s1 = String::from("hello");
let s2 = s1;  // s1 is moved
// println_str(s1);  // ERROR: value moved
```

### 3.4 Clone vs Copy

```rust
// Rust
#[derive(Clone, Copy)]
struct Point { x: i32, y: i32 }

#[derive(Clone)]
struct Buffer { data: Vec<u8> }
```

```blood
// Blood - similar
#[derive(Clone, Copy)]
struct Point { x: i32, y: i32 }

#[derive(Clone)]
struct Buffer { data: Vec<u8> }
```

### 3.5 Mutable References

```rust
// Rust - one mutable reference OR many immutable
let mut data = vec![1, 2, 3];
let r1 = &mut data;
// let r2 = &data;  // ERROR: cannot borrow while mutably borrowed
r1.push(4);
```

```blood
// Blood - similar exclusivity, but runtime checked
let mut data = vec![1, 2, 3];
let r1 = &mut data;
// let r2 = &data;  // Would create aliasing violation
r1.push(4);
```

---

## 4. Error Handling

### 4.1 Result vs Effects

**Rust**: Uses `Result<T, E>` and the `?` operator.

**Blood**: Uses effect handlers for structured error handling.

```rust
// Rust
fn read_file(path: &str) -> Result<String, io::Error> {
    let contents = fs::read_to_string(path)?;
    Ok(contents)
}

fn main() {
    match read_file("config.txt") {
        Ok(contents) => println!("{}", contents),
        Err(e) => eprintln!("Error: {}", e),
    }
}
```

```blood
// Blood - effect-based
fn read_file(path: &str) -> String / {Error<IoError>, IO} {
    fs::read_to_string(path)  // Error propagates via effect
}

fn main() / {IO} {
    handle {
        let contents = read_file("config.txt");
        println_str(contents);
    } with {
        Error {
            raise(e) => {
                eprintln_str("Error: ");
                eprintln_str(e.message());
            }
        }
    }
}
```

### 4.2 The ? Operator Equivalent

```rust
// Rust - ? for early return
fn process() -> Result<Data, Error> {
    let a = step1()?;
    let b = step2(a)?;
    let c = step3(b)?;
    Ok(c)
}
```

```blood
// Blood - effects propagate automatically
fn process() -> Data / {Error<ProcessError>} {
    let a = step1();  // Error effect propagates
    let b = step2(a);
    let c = step3(b);
    c
}
```

### 4.3 Multiple Error Types

```rust
// Rust - often needs error enums or Box<dyn Error>
#[derive(Debug)]
enum AppError {
    Io(io::Error),
    Parse(ParseError),
    Network(NetworkError),
}

fn complex_operation() -> Result<Data, AppError> {
    let file = read_file().map_err(AppError::Io)?;
    let parsed = parse(file).map_err(AppError::Parse)?;
    let result = send(parsed).map_err(AppError::Network)?;
    Ok(result)
}
```

```blood
// Blood - effects compose naturally
fn complex_operation() -> Data / {Error<IoError>, Error<ParseError>, Error<NetworkError>} {
    let file = read_file();    // Propagates Error<IoError>
    let parsed = parse(file);  // Propagates Error<ParseError>
    let result = send(parsed); // Propagates Error<NetworkError>
    result
}

// Or unify with a common handler
fn unified_operation() -> Data / {Error<AppError>} {
    handle {
        complex_operation()
    } with {
        Error<IoError> { raise(e) => raise(AppError::Io(e)) }
        Error<ParseError> { raise(e) => raise(AppError::Parse(e)) }
        Error<NetworkError> { raise(e) => raise(AppError::Network(e)) }
    }
}
```

### 4.4 Option Handling

```rust
// Rust
fn find_user(id: u32) -> Option<User> { ... }

fn get_user_name(id: u32) -> Option<String> {
    let user = find_user(id)?;
    Some(user.name)
}
```

```blood
// Blood - Option is still useful for nullable values
fn find_user(id: u32) -> Option<User> / {Database} { ... }

fn get_user_name(id: u32) -> Option<String> / {Database} {
    match find_user(id) {
        Some(user) => Some(user.name),
        None => None,
    }
}

// Or use the Nullable effect for optional values
fn get_user_name(id: u32) -> String / {Database, Nullable} {
    let user = find_user(id).unwrap_or_perform();  // Performs Nullable.none if None
    user.name
}
```

---

## 5. Traits to Effects

### 5.1 Traits for Behavior

**Rust**: Traits define shared behavior.

**Blood**: Traits exist, but effects handle cross-cutting concerns.

```rust
// Rust - trait for logging
trait Logger {
    fn log(&self, message: &str);
}

fn process<L: Logger>(data: Data, logger: &L) {
    logger.log("Starting process");
    // ... work ...
    logger.log("Process complete");
}
```

```blood
// Blood - logging as an effect
effect Log {
    fn log(message: &str);
}

fn process(data: Data) / {Log} {
    perform Log.log("Starting process");
    // ... work ...
    perform Log.log("Process complete");
}

// Handler provides implementation
handler ConsoleLog for Log {
    fn log(message) => {
        println_str(message);
        resume(())
    }
}
```

### 5.2 Dependency Injection via Effects

```rust
// Rust - trait objects for DI
trait Database {
    fn query(&self, sql: &str) -> Vec<Row>;
}

struct RealDb;
impl Database for RealDb {
    fn query(&self, sql: &str) -> Vec<Row> { /* real impl */ }
}

struct MockDb;
impl Database for MockDb {
    fn query(&self, sql: &str) -> Vec<Row> { /* mock impl */ }
}

fn get_users(db: &dyn Database) -> Vec<User> {
    db.query("SELECT * FROM users")
        .into_iter()
        .map(User::from_row)
        .collect()
}
```

```blood
// Blood - effects for DI
effect Database {
    fn query(sql: &str) -> Vec<Row>;
}

fn get_users() -> Vec<User> / {Database} {
    perform Database.query("SELECT * FROM users")
        .into_iter()
        .map(User::from_row)
        .collect()
}

// Production handler
handler RealDb for Database {
    fn query(sql) => {
        // Real database query
        resume(execute_real_query(sql))
    }
}

// Test handler
handler MockDb for Database {
    fn query(sql) => {
        // Return mock data
        resume(vec![mock_row()])
    }
}
```

### 5.3 Iterator Trait

```rust
// Rust - Iterator trait
impl Iterator for MyCollection {
    type Item = i32;
    fn next(&mut self) -> Option<Self::Item> { ... }
}
```

```blood
// Blood - similar trait syntax
impl Iterator for MyCollection {
    type Item = i32;
    fn next(&mut self) -> Option<Self::Item> / pure { ... }
}
```

---

## 6. Concurrency

### 6.1 Async/Await vs Fibers

**Rust**: Stackless async/await with executors.

**Blood**: Stackful fibers with effect handlers.

```rust
// Rust
async fn fetch_data(url: &str) -> Result<Data, Error> {
    let response = reqwest::get(url).await?;
    let data = response.json().await?;
    Ok(data)
}

#[tokio::main]
async fn main() {
    let result = fetch_data("https://api.example.com").await;
}
```

```blood
// Blood - fiber-based
fn fetch_data(url: &str) -> Data / {Fiber, Error<HttpError>, IO} {
    let response = http::get(url);
    let data = response.json();
    data
}

fn main() / {IO} {
    with FiberRuntime handle {
        let result = fetch_data("https://api.example.com");
    }
}
```

### 6.2 Spawning Tasks

```rust
// Rust
use tokio::spawn;

async fn main() {
    let handle1 = spawn(async { task1().await });
    let handle2 = spawn(async { task2().await });

    let (r1, r2) = tokio::join!(handle1, handle2);
}
```

```blood
// Blood
fn main() / {Fiber} {
    let handle1 = spawn(|| task1());
    let handle2 = spawn(|| task2());

    let r1 = await(handle1);
    let r2 = await(handle2);
}
```

### 6.3 Channels

```rust
// Rust
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        tx.send(42).unwrap();
    });

    let value = rx.recv().unwrap();
}
```

```blood
// Blood
fn main() / {Fiber} {
    let (tx, rx) = channel(10);

    spawn(move || {
        tx.send(42);
    });

    let value = rx.recv();
}
```

### 6.4 Mutex and Shared State

```rust
// Rust
use std::sync::{Arc, Mutex};

fn main() {
    let data = Arc::new(Mutex::new(0));
    let data_clone = Arc::clone(&data);

    thread::spawn(move || {
        let mut guard = data_clone.lock().unwrap();
        *guard += 1;
    });
}
```

```blood
// Blood - shared state is discouraged, use effects instead
effect State<T> {
    fn get() -> T;
    fn put(value: T);
}

fn main() / {Fiber} {
    // State is handled, not shared
    with StateHandler { initial: 0 } handle {
        spawn(|| {
            let x = perform State.get();
            perform State.put(x + 1);
        });
    }
}
```

---

## 7. Memory Management

### 7.1 Box, Rc, Arc

```rust
// Rust
let boxed: Box<Data> = Box::new(data);
let shared: Rc<Data> = Rc::new(data);
let atomic: Arc<Data> = Arc::new(data);
```

```blood
// Blood - regions replace most heap allocation patterns
region my_region {
    let data = allocate(Data { ... });  // Tier 1 region allocation
}

// Persistent for long-lived shared data
let shared = persist(data);  // Tier 3 persistent
```

### 7.2 Region-Based Memory

```rust
// Rust - manual scope management
fn process(data: Vec<u8>) -> Result<Output> {
    let temp_buffer = Vec::with_capacity(1024);
    // ... use temp_buffer ...
    // temp_buffer dropped at end of scope
    Ok(output)
}
```

```blood
// Blood - explicit regions
fn process(data: Vec<u8>) -> Output / {Allocate} {
    region temp {
        let temp_buffer = Vec::with_capacity(1024);
        // ... use temp_buffer ...
        // All temp region memory freed here
    }
    output
}
```

### 7.3 Memory Tiers

| Rust Equivalent | Blood Tier | Use Case |
|-----------------|------------|----------|
| Stack allocation | Tier 0 (Stack) | Local values, small data |
| `Box<T>` | Tier 1 (Region) | Function-scoped heap data |
| `Rc<T>`, `Arc<T>` | Tier 2 (Persistent) | Long-lived shared data |
| Static | Tier 3 (Content-addressed) | Immutable eternal data |

---

## 8. Pattern Matching

### 8.1 Match Expressions

```rust
// Rust
match value {
    Some(x) if x > 0 => handle_positive(x),
    Some(x) => handle_negative(x),
    None => handle_none(),
}
```

```blood
// Blood - identical syntax
match value {
    Some(x) if x > 0 => handle_positive(x),
    Some(x) => handle_negative(x),
    None => handle_none(),
}
```

### 8.2 If Let

```rust
// Rust
if let Some(value) = maybe_value {
    process(value);
}
```

```blood
// Blood - identical
if let Some(value) = maybe_value {
    process(value);
}
```

### 8.3 Destructuring

```rust
// Rust
let Point { x, y } = point;
let (first, second) = tuple;
let [a, b, rest @ ..] = slice;
```

```blood
// Blood - identical
let Point { x, y } = point;
let (first, second) = tuple;
let [a, b, rest @ ..] = slice;
```

---

## 9. Common Patterns

### 9.1 Builder Pattern

```rust
// Rust
struct RequestBuilder {
    url: String,
    method: Method,
    headers: HashMap<String, String>,
}

impl RequestBuilder {
    fn new(url: &str) -> Self { ... }
    fn method(mut self, method: Method) -> Self { ... }
    fn header(mut self, key: &str, value: &str) -> Self { ... }
    fn build(self) -> Request { ... }
}
```

```blood
// Blood - same pattern works
struct RequestBuilder {
    url: String,
    method: Method,
    headers: HashMap<String, String>,
}

impl RequestBuilder {
    fn new(url: &str) -> Self / pure { ... }
    fn method(mut self, method: Method) -> Self / pure { ... }
    fn header(mut self, key: &str, value: &str) -> Self / pure { ... }
    fn build(self) -> Request / pure { ... }
}
```

### 9.2 RAII / Drop

```rust
// Rust
struct FileHandle { fd: i32 }

impl Drop for FileHandle {
    fn drop(&mut self) {
        unsafe { close(self.fd); }
    }
}
```

```blood
// Blood - similar, but with effects
struct FileHandle { fd: i32 }

impl Drop for FileHandle {
    fn drop(&mut self) / {FFI} {
        @unsafe { close(self.fd); }
    }
}
```

### 9.3 From/Into Conversions

```rust
// Rust
impl From<i32> for MyType {
    fn from(value: i32) -> Self { ... }
}

let x: MyType = 42.into();
```

```blood
// Blood - identical
impl From<i32> for MyType {
    fn from(value: i32) -> Self / pure { ... }
}

let x: MyType = 42.into();
```

### 9.4 Interior Mutability

```rust
// Rust
use std::cell::{Cell, RefCell};

struct Counter {
    count: Cell<u32>,
}

struct Cache {
    data: RefCell<HashMap<String, Value>>,
}
```

```blood
// Blood - use effects instead
effect Counter {
    fn increment();
    fn get() -> u32;
}

fn use_counter() / {Counter} {
    perform Counter.increment();
    perform Counter.get()
}
```

---

## 10. Crate to Module Migration

### 10.1 Module Structure

```rust
// Rust - lib.rs
pub mod parser;
pub mod lexer;
pub mod codegen;

// Rust - parser/mod.rs
mod expr;
mod stmt;
pub use expr::*;
pub use stmt::*;
```

```blood
// Blood - similar structure
// lib.blood
pub mod parser;
pub mod lexer;
pub mod codegen;

// parser/mod.blood
mod expr;
mod stmt;
pub use expr::*;
pub use stmt::*;
```

### 10.2 Use Statements

```rust
// Rust
use std::collections::HashMap;
use crate::parser::Parser;
use super::utils::*;
```

```blood
// Blood - same syntax
use std::collections::HashMap;
use crate::parser::Parser;
use super::utils::*;
```

### 10.3 Visibility

```rust
// Rust
pub fn public_function() {}
pub(crate) fn crate_visible() {}
pub(super) fn parent_visible() {}
fn private() {}
```

```blood
// Blood - identical
pub fn public_function() / pure {}
pub(crate) fn crate_visible() / pure {}
pub(super) fn parent_visible() / pure {}
fn private() / pure {}
```

---

## Appendix A: Quick Reference

### A.1 Syntax Mapping

| Rust | Blood | Notes |
|------|-------|-------|
| `fn foo() -> T` | `fn foo() -> T / pure` | Effect annotation required |
| `fn foo() -> Result<T, E>` | `fn foo() -> T / {Error<E>}` | Effects replace Result |
| `async fn foo()` | `fn foo() / {Fiber}` | Fibers instead of async |
| `'a` lifetime | (none needed) | Generational pointers |
| `Box<T>` | Region allocation | `region r { allocate(...) }` |
| `Rc<T>` / `Arc<T>` | `persist(value)` | Tier 2/3 memory |
| `unsafe { }` | `@unsafe { }` | Different syntax |
| `?` operator | Effect propagation | Automatic with effects |
| `println!()` | `println_str()` | No macros yet |

### A.2 Effect Equivalents

| Rust Pattern | Blood Effect |
|--------------|--------------|
| `Result<T, E>` | `{Error<E>}` |
| `Option<T>` methods | `{Nullable}` |
| `println!`, `eprintln!` | `{IO}` |
| `std::fs::*` | `{IO, Error<IoError>}` |
| `async/await` | `{Fiber}` |
| `Mutex<T>`, `RwLock<T>` | `{State<T>}` |
| Random number generation | `{Random}` |
| Environment variables | `{Env}` |

---

## Appendix B: Migration Checklist

When porting Rust code to Blood:

- [ ] Add effect annotations to all functions
- [ ] Replace `Result<T, E>` returns with `{Error<E>}` effect
- [ ] Replace lifetime annotations with generational references
- [ ] Convert `async fn` to fiber-based `/ {Fiber}`
- [ ] Replace `unsafe` blocks with `@unsafe` blocks
- [ ] Convert trait-based DI to effect-based DI
- [ ] Replace `Box`/`Rc`/`Arc` with appropriate memory tiers
- [ ] Update macro invocations to function calls
- [ ] Add effect handlers at program entry points
- [ ] Update tests to use effect mocking

---

## Appendix C: Further Reading

- [Blood Language Specification](SPECIFICATION.md)
- [Effects Tutorial](EFFECTS_TUTORIAL.md)
- [Memory Model Guide](MEMORY_GUIDE.md)
- [Concurrency Specification](CONCURRENCY.md)
- [Getting Started Guide](GETTING_STARTED.md)

---

*This document is part of the Blood Language Specification.*
