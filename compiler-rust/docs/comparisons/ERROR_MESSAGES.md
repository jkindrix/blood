# Error Message Comparison: Blood vs Other Languages

**Purpose**: Compare error message quality across Blood, Rust, Go, and other languages to evaluate developer experience.

Good error messages are crucial for developer productivity. This document compares how different languages report common programming errors.

---

## Evaluation Criteria

Error messages are evaluated on:

| Criterion | Description |
|-----------|-------------|
| **Clarity** | Is it immediately obvious what's wrong? |
| **Location** | Does it point to the exact problem location? |
| **Suggestion** | Does it suggest how to fix the issue? |
| **Context** | Does it show relevant surrounding code? |
| **Terminology** | Does it use accessible language? |

---

## 1. Type Mismatch Errors

### Blood

```blood
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let result = add(5, "hello");
}
```

**Blood Error:**
```
error[E0308]: mismatched types
   --> src/main.blood:7:24
    |
  7 |     let result = add(5, "hello");
    |                        ^^^^^^^^ expected `i32`, found `String`
    |
    = note: expected type `i32`
               found type `String`
    = help: consider using `.parse()` if converting from string to number
```

### Rust

```rust
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let result = add(5, "hello");
}
```

**Rust Error:**
```
error[E0308]: mismatched types
 --> src/main.rs:6:24
  |
6 |     let result = add(5, "hello");
  |                  ---    ^^^^^^^ expected `i32`, found `&str`
  |                  |
  |                  arguments to this function are incorrect
  |
note: function defined here
 --> src/main.rs:1:4
  |
1 | fn add(a: i32, b: i32) -> i32 {
  |    ^^^ ------  ------
help: consider using `parse` if you are trying to parse to `i32`
  |
6 |     let result = add(5, "hello".parse().unwrap());
  |                        +++++++++++++++++++++++++
```

### Go

```go
func add(a int, b int) int {
    return a + b
}

func main() {
    result := add(5, "hello")
}
```

**Go Error:**
```
./main.go:8:22: cannot use "hello" (untyped string constant) as int value in argument to add
```

### Comparison

| Aspect | Blood | Rust | Go |
|--------|-------|------|-----|
| Clarity | Excellent | Excellent | Good |
| Location | Exact span | Exact span | Line only |
| Suggestion | Yes | Yes, with code | No |
| Context | Shows code | Shows definition | No |

**Winner**: Blood and Rust (tied) - both provide helpful suggestions

---

## 2. Use Before Declaration

### Blood

```blood
fn main() {
    println!("{}", x);
    let x = 5;
}
```

**Blood Error:**
```
error[E0425]: cannot find value `x` in this scope
   --> src/main.blood:2:20
    |
  2 |     println!("{}", x);
    |                    ^ not found in this scope
    |
    = note: `x` is declared on line 3, but used here on line 2
    = help: move the `let x = 5;` declaration before this line
```

### Rust

```rust
fn main() {
    println!("{}", x);
    let x = 5;
}
```

**Rust Error:**
```
error[E0425]: cannot find value `x` in this scope
 --> src/main.rs:2:20
  |
2 |     println!("{}", x);
  |                    ^ not found in this scope
  |
help: a local variable with a similar name exists
  |
2 |     println!("{}", x);
  |                    ~
```

### Go

```go
func main() {
    fmt.Println(x)
    x := 5
}
```

**Go Error:**
```
./main.go:6:14: undefined: x
```

### Comparison

| Aspect | Blood | Rust | Go |
|--------|-------|------|-----|
| Explains cause | Yes | Partial | No |
| Shows declaration | Yes | No | No |
| Suggests fix | Yes | Partial | No |

**Winner**: Blood - explicitly notes the declaration line

---

## 3. Missing Return Value

### Blood

```blood
fn calculate(x: i32) -> i32 {
    let y = x * 2;
    // missing return
}
```

**Blood Error:**
```
error[E0308]: mismatched types
   --> src/main.blood:1:25
    |
  1 | fn calculate(x: i32) -> i32 {
    |    ---------            ^^^ expected `i32`, found `()`
    |    |
    |    implicitly returns `()` as its body has no tail expression
  2 |     let y = x * 2;
  3 | }
    | - help: consider returning the local variable: `y`
    |
    = note: expected type `i32`
               found type `()`
```

### Rust

```rust
fn calculate(x: i32) -> i32 {
    let y = x * 2;
}
```

**Rust Error:**
```
error[E0308]: mismatched types
 --> src/main.rs:1:25
  |
1 | fn calculate(x: i32) -> i32 {
  |    ---------            ^^^ expected `i32`, found `()`
  |    |
  |    implicitly returns `()` as its body has no tail expression
2 |     let y = x * 2;
  |                  - help: remove this semicolon to return this value
```

### Go

```go
func calculate(x int) int {
    y := x * 2
}
```

**Go Error:**
```
./main.go:3:1: missing return
```

### Comparison

| Aspect | Blood | Rust | Go |
|--------|-------|------|-----|
| Identifies issue | Clear | Clear | Minimal |
| Suggests fix | Yes (return variable) | Yes (remove semicolon) | No |
| Shows relevant code | Yes | Yes | No |

**Winner**: Blood and Rust (tied) - both suggest fixes

---

## 4. Effect Not Handled

### Blood (Unique)

```blood
effect Log {
    fn log(msg: String) -> ();
}

fn process() with Log {
    do Log.log("Processing...");
}

fn main() {
    process();  // Error: Log effect not handled
}
```

**Blood Error:**
```
error[E0601]: unhandled effect `Log`
   --> src/main.blood:10:5
    |
 10 |     process();
    |     ^^^^^^^^^ requires effect `Log` which is not provided
    |
    = note: `process` has effect signature: `fn process() with Log`
    = help: handle the effect with a handler:
    |
    |     try {
    |         process()
    |     } with LogHandler
    |
    = note: available handlers for `Log`: ConsoleLogger, FileLogger
```

### Rust (No equivalent - uses traits)

```rust
trait Log {
    fn log(&self, msg: &str);
}

fn process<L: Log>(logger: &L) {
    logger.log("Processing...");
}

fn main() {
    process(???);  // Error: need to provide logger
}
```

**Rust Error:**
```
error[E0282]: type annotations needed
 --> src/main.rs:10:5
  |
10|     process(???);
  |     ^^^^^^^ cannot infer type of the type parameter `L`
```

### Go (No equivalent - uses interfaces)

```go
type Logger interface {
    Log(msg string)
}

func process(logger Logger) {
    logger.Log("Processing...")
}

func main() {
    process(nil)  // Compiles but panics at runtime
}
```

**Go**: No compile-time error; panics at runtime with `nil pointer dereference`

### Comparison

Blood's effect system provides unique error messages that don't exist in languages without algebraic effects. The error:
- Names the unhandled effect
- Shows the effect signature
- Suggests how to handle it
- Lists available handlers

---

## 5. Ownership/Borrow Errors

### Blood

```blood
fn main() {
    let data = vec![1, 2, 3];
    let reference = &data;

    consume(data);      // Move data
    println!("{:?}", reference);  // Error: data was moved
}

fn consume(v: Vec<i32>) { }
```

**Blood Error:**
```
error[E0505]: cannot use `reference` because `data` was moved
   --> src/main.blood:6:22
    |
  3 |     let reference = &data;
    |                     ----- borrow of `data` occurs here
  4 |
  5 |     consume(data);
    |             ---- `data` moved here
  6 |     println!("{:?}", reference);
    |                      ^^^^^^^^^ use of borrowed value after move
    |
    = note: move occurs because `data` has type `Vec<i32>`, which does not implement `Copy`
    = help: consider cloning `data` before moving: `consume(data.clone())`
    = help: or consider borrowing: `consume(&data)` if `consume` can accept a reference
```

### Rust

```rust
fn main() {
    let data = vec![1, 2, 3];
    let reference = &data;

    consume(data);
    println!("{:?}", reference);
}

fn consume(v: Vec<i32>) { }
```

**Rust Error:**
```
error[E0505]: cannot move out of `data` because it is borrowed
 --> src/main.rs:5:13
  |
3 |     let reference = &data;
  |                     ----- borrow of `data` occurs here
4 |
5 |     consume(data);
  |             ^^^^ move out of `data` occurs here
6 |     println!("{:?}", reference);
  |                      --------- borrow later used here

For more information about this error, try `rustc --explain E0505`.
```

### Go

Go uses garbage collection, so this pattern doesn't produce an error:

```go
func main() {
    data := []int{1, 2, 3}
    reference := &data

    consume(data)
    fmt.Println(*reference)  // Works (GC keeps data alive)
}
```

### Comparison

| Aspect | Blood | Rust | Go |
|--------|-------|------|-----|
| Detects issue | Yes | Yes | N/A (GC) |
| Shows borrow location | Yes | Yes | N/A |
| Shows move location | Yes | Yes | N/A |
| Suggests fixes | Yes (multiple) | No | N/A |

**Winner**: Blood - provides multiple fix suggestions

---

## 6. Pattern Match Exhaustiveness

### Blood

```blood
enum Status {
    Pending,
    Active,
    Completed,
    Cancelled,
}

fn describe(status: Status) -> String {
    match status {
        Status::Pending => "Waiting".to_string(),
        Status::Active => "In progress".to_string(),
    }
}
```

**Blood Error:**
```
error[E0004]: non-exhaustive patterns: `Completed` and `Cancelled` not covered
   --> src/main.blood:9:11
    |
  1 | enum Status {
    | ----------- `Status` defined here
...
  9 |     match status {
    |           ^^^^^^ patterns `Status::Completed` and `Status::Cancelled` not covered
    |
    = note: the matched value is of type `Status`
    = help: ensure all variants are matched, or add a wildcard pattern:
    |
    |         Status::Completed => todo!(),
    |         Status::Cancelled => todo!(),
    |
    = help: or add a catch-all pattern:
    |
    |         _ => todo!(),
```

### Rust

```rust
enum Status {
    Pending,
    Active,
    Completed,
    Cancelled,
}

fn describe(status: Status) -> String {
    match status {
        Status::Pending => "Waiting".to_string(),
        Status::Active => "In progress".to_string(),
    }
}
```

**Rust Error:**
```
error[E0004]: non-exhaustive patterns: `Completed` and `Cancelled` not covered
 --> src/main.rs:10:11
  |
10|     match status {
  |           ^^^^^^ patterns `Status::Completed` and `Status::Cancelled` not covered
  |
note: `Status` defined here
 --> src/main.rs:1:6
  |
1 | enum Status {
  |      ^^^^^^
...
4 |     Completed,
  |     --------- not covered
5 |     Cancelled,
  |     --------- not covered
  = note: the matched value is of type `Status`
help: ensure that all possible cases are being handled by adding a match arm with a wildcard pattern, a match arm with multiple or-patterns as shown, or multiple match arms
  |
12~         Status::Active => "In progress".to_string(),
13+         Status::Completed | Status::Cancelled => todo!(),
  |
```

### Go

Go doesn't have algebraic data types or exhaustiveness checking:

```go
type Status int

const (
    Pending Status = iota
    Active
    Completed
    Cancelled
)

func describe(status Status) string {
    switch status {
    case Pending:
        return "Waiting"
    case Active:
        return "In progress"
    }
    return ""  // No warning about missing cases
}
```

**Go**: No compile-time error

### Comparison

| Aspect | Blood | Rust | Go |
|--------|-------|------|-----|
| Detects missing cases | Yes | Yes | No |
| Lists missing variants | Yes | Yes | N/A |
| Shows definition location | Yes | Yes | N/A |
| Suggests fixes | Yes (two options) | Yes | N/A |

**Winner**: Blood and Rust (tied)

---

## 7. Generational Reference Errors (Blood-specific)

### Blood (Unique)

```blood
fn main() {
    let mut container = vec![1, 2, 3];
    let reference = &container[0];

    container.clear();  // Invalidates reference

    println!("{}", *reference);  // Error at runtime
}
```

**Blood Runtime Error:**
```
error: use after free detected
   --> src/main.blood:7:20
    |
  3 |     let reference = &container[0];
    |                     ^^^^^^^^^^^^^ reference created here (generation 42)
  4 |
  5 |     container.clear();
    |     ----------------- invalidated here (generation now 43)
  6 |
  7 |     println!("{}", *reference);
    |                    ^^^^^^^^^^ attempted access with stale generation
    |
    = note: reference holds generation 42, but current generation is 43
    = note: this indicates a use-after-free bug that was caught at runtime
    = help: ensure references do not outlive the data they point to

Backtrace:
  main() at src/main.blood:7
```

### Rust

Rust catches this at compile time:

```rust
fn main() {
    let mut container = vec![1, 2, 3];
    let reference = &container[0];

    container.clear();  // Compile error

    println!("{}", *reference);
}
```

**Rust Compile Error:**
```
error[E0502]: cannot borrow `container` as mutable because it is also borrowed as immutable
 --> src/main.rs:5:5
  |
3 |     let reference = &container[0];
  |                      --------- immutable borrow occurs here
4 |
5 |     container.clear();
  |     ^^^^^^^^^^^^^^^^^ mutable borrow occurs here
6 |
7 |     println!("{}", *reference);
  |                    ---------- immutable borrow later used here
```

### Comparison

| Aspect | Blood | Rust |
|--------|-------|------|
| When caught | Runtime | Compile time |
| Shows creation point | Yes | Yes |
| Shows invalidation point | Yes | Yes |
| Explains mechanism | Yes (generations) | Yes (borrows) |

**Trade-off**: Rust catches this at compile time (safer), but Blood allows more flexible patterns and catches at runtime with helpful diagnostics.

---

## 8. Syntax Errors

### Blood

```blood
fn main() {
    let x = 5
    let y = 10;
}
```

**Blood Error:**
```
error: expected `;`, found `let`
   --> src/main.blood:3:5
    |
  2 |     let x = 5
    |              - help: add `;` here
  3 |     let y = 10;
    |     ^^^ unexpected token
    |
    = note: statements in Blood must end with a semicolon
```

### Rust

```rust
fn main() {
    let x = 5
    let y = 10;
}
```

**Rust Error:**
```
error: expected `;`, found `let`
 --> src/main.rs:3:5
  |
2 |     let x = 5
  |              - help: add `;` here
3 |     let y = 10;
  |     ^^^
```

### Go

```go
func main() {
    x := 5
    y := 10
}
```

Go uses automatic semicolon insertion, so this compiles successfully.

### Comparison

Both Blood and Rust provide excellent syntax error messages with suggestions.

---

## Summary: Error Message Quality Scores

| Category | Blood | Rust | Go |
|----------|-------|------|-----|
| Type mismatches | 5/5 | 5/5 | 3/5 |
| Undefined variables | 5/5 | 4/5 | 2/5 |
| Missing returns | 5/5 | 5/5 | 2/5 |
| Effect handling | 5/5 | N/A | N/A |
| Ownership/borrowing | 5/5 | 4/5 | N/A |
| Pattern exhaustiveness | 5/5 | 5/5 | N/A |
| Runtime safety | 5/5 | N/A | N/A |
| Syntax errors | 5/5 | 5/5 | 3/5 |
| **Average** | **5.0/5** | **4.7/5** | **2.5/5** |

---

## Blood's Error Message Philosophy

Blood's error messages follow these principles:

1. **Show, Don't Just Tell**: Display the relevant code, not just describe the problem
2. **Be Specific**: Point to exact locations with precise spans
3. **Suggest Fixes**: Provide actionable suggestions when possible
4. **Explain Why**: Help developers understand the underlying issue
5. **Use Plain Language**: Avoid jargon where possible
6. **Be Consistent**: Use the same style and terminology throughout

---

## Conclusion

Blood's error messages are designed to be best-in-class:

- **On par with Rust**: Both languages excel at error message quality
- **Better than Go**: Significantly more helpful and detailed
- **Unique for effects**: Error messages for effect handling don't exist elsewhere

The effect system introduces error categories that don't exist in other languages, and Blood handles these with the same quality as traditional errors.

---

*Error messages are a key part of developer experience. Blood aims to make errors helpful, not frustrating.*
