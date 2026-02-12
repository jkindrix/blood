# Getting Started with Blood

Blood is a systems programming language with algebraic effects, generational memory safety, and multiple dispatch. This guide walks you through the basics.

## Installation

### Prerequisites

- Rust toolchain (1.77+)
- LLVM 17

### Building from Source

```bash
git clone https://github.com/blood-lang/blood
cd blood
cargo build --release
```

The compiler binary will be at `target/release/bloodc`.

## Your First Blood Program

Create a file called `hello.blood`:

```blood
fn main() {
    println("Hello, Blood!");
}
```

Compile and run:

```bash
bloodc hello.blood -o hello
./hello
```

## Basic Syntax

### Variables and Types

```blood
// Immutable binding
let x: i32 = 42;

// Mutable binding
let mut y = 10;
y = y + 1;

// Type inference
let name = "Alice";  // String inferred
let pi = 3.14159;    // f64 inferred
```

### Functions

```blood
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn greet(name: String) {
    println("Hello, " + name + "!");
}
```

### Control Flow

```blood
fn classify(n: i32) -> String {
    if n < 0 {
        "negative"
    } else if n == 0 {
        "zero"
    } else {
        "positive"
    }
}

fn describe(value: Option<i32>) -> String {
    match value {
        Some(n) => "has value: " + n.to_string(),
        None => "no value",
    }
}
```

### Loops

```blood
fn sum_to(n: i32) -> i32 {
    let mut total = 0;
    for i in 0..n {
        total = total + i;
    }
    total
}

fn find_first_even(items: [i32]) -> Option<i32> {
    for item in items {
        if item % 2 == 0 {
            return Some(item);
        }
    }
    None
}
```

## Structs and Enums

### Defining Structs

```blood
struct Point {
    x: f64,
    y: f64,
}

struct Person {
    name: String,
    age: u32,
}

fn main() {
    let p = Point { x: 1.0, y: 2.0 };
    let alice = Person { name: "Alice", age: 30 };

    println(alice.name);
}
```

### Methods with impl

```blood
impl Point {
    fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn distance_from_origin(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    fn translate(&mut self, dx: f64, dy: f64) {
        self.x = self.x + dx;
        self.y = self.y + dy;
    }
}
```

### Enums

```blood
enum Color {
    Red,
    Green,
    Blue,
    Rgb(u8, u8, u8),
}

fn color_name(c: Color) -> String {
    match c {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue",
        Color::Rgb(r, g, b) => "rgb(" + r.to_string() + ")",
    }
}
```

## Algebraic Effects

Blood's most distinctive feature is its effect system. Effects let you define and handle side effects in a composable way.

### Defining Effects

```blood
effect Console {
    op print(msg: String);
    op read_line() -> String;
}

effect State<T> {
    op get() -> T;
    op set(value: T);
}
```

### Using Effects

```blood
fn greet_user() / {Console} {
    perform Console::print("What is your name? ");
    let name = perform Console::read_line();
    perform Console::print("Hello, " + name + "!");
}
```

The `/ {Console}` annotation declares that this function performs the `Console` effect.

### Handling Effects

```blood
handler RealConsole for Console {
    op print(msg: String) {
        // Actual I/O implementation
        libc::printf(msg);
        resume(())
    }

    op read_line() -> String {
        let line = libc::readline();
        resume(line)
    }
}

fn main() {
    with RealConsole handle {
        greet_user()
    }
}
```

### State Effect Example

```blood
fn increment_twice() / {State<i32>} {
    let x = perform State::get();
    perform State::set(x + 1);
    let y = perform State::get();
    perform State::set(y + 1);
}

handler LocalState for State<i32> {
    state: i32 = 0,

    op get() -> i32 {
        resume(self.state)
    }

    op set(value: i32) {
        self.state = value;
        resume(())
    }
}

fn main() {
    let result = with LocalState { state: 10 } handle {
        increment_twice();
        perform State::get()
    };
    // result is 12
}
```

## Generational Memory Safety

Blood provides memory safety through generational references, without a garbage collector or borrow checker.

### Stack Allocation (Tier 0)

```blood
fn stack_example() {
    let x = 42;           // Stack allocated
    let arr = [1, 2, 3];  // Stack allocated array
}
```

### Arena Allocation (Tier 1)

```blood
fn arena_example() {
    region arena {
        let items = Vec::new();    // Allocated in arena
        items.push(1);
        items.push(2);
        // All arena allocations freed at end of region
    }
}
```

### Persistent Allocation (Tier 2)

```blood
fn persistent_example() {
    let data = Box::new_persistent([0u8; 1024]);
    // Data persists until explicitly dropped or refcount reaches 0
}
```

### Detecting Stale References

```blood
fn stale_example() {
    let ptr = region arena {
        let temp = Box::new(42);
        temp.as_ptr()  // WARNING: returns pointer to arena-allocated value
    };
    // Accessing ptr here would be detected as a stale reference
}
```

## Multiple Dispatch

Blood supports multiple dispatch - method selection based on runtime types of all arguments.

```blood
// Define a trait for drawable objects
trait Drawable {
    fn draw(&self);
}

// Multiple dispatch on shape types
fn collide(a: impl Shape, b: impl Shape) -> bool {
    // Dispatched based on both a and b's types
    ...
}

impl Collision for (Circle, Circle) {
    fn collide(a: Circle, b: Circle) -> bool {
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        let dist = (dx*dx + dy*dy).sqrt();
        dist < a.radius + b.radius
    }
}

impl Collision for (Circle, Rectangle) {
    fn collide(a: Circle, b: Rectangle) -> bool {
        // Circle-rectangle collision
        ...
    }
}
```

## Traits

```blood
trait Printable {
    fn to_string(&self) -> String;
}

impl Printable for i32 {
    fn to_string(&self) -> String {
        // Built-in integer to string
        format!("{}", self)
    }
}

impl Printable for Point {
    fn to_string(&self) -> String {
        "(" + self.x.to_string() + ", " + self.y.to_string() + ")"
    }
}
```

## Generics

```blood
fn identity<T>(x: T) -> T {
    x
}

fn swap<T>(pair: (T, T)) -> (T, T) {
    (pair.1, pair.0)
}

struct Container<T> {
    value: T,
}

impl<T> Container<T> {
    fn new(value: T) -> Container<T> {
        Container { value }
    }

    fn get(&self) -> &T {
        &self.value
    }
}
```

### Generic Constraints

```blood
fn print_all<T: Printable>(items: [T]) {
    for item in items {
        println(item.to_string());
    }
}

fn compare<T: Ord + Clone>(a: T, b: T) -> Ordering {
    a.clone().cmp(&b)
}
```

## FFI (Foreign Function Interface)

Blood can call C functions directly:

```blood
extern "C" {
    fn printf(format: *const u8, ...) -> i32;
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
}

fn main() {
    unsafe {
        printf(c"Hello from C!\n".as_ptr());
    }
}
```

## Error Handling

### Using Result

```blood
fn parse_int(s: String) -> Result<i32, ParseError> {
    // ...
}

fn main() {
    match parse_int("42") {
        Ok(n) => println("Parsed: " + n.to_string()),
        Err(e) => println("Error: " + e.message),
    }
}
```

### Using Option

```blood
fn find<T>(items: [T], predicate: fn(T) -> bool) -> Option<T> {
    for item in items {
        if predicate(item) {
            return Some(item);
        }
    }
    None
}
```

## Next Steps

- Explore the [examples/](../examples/) directory for more code samples
- Read [SPECIFICATION.md](../SPECIFICATION.md) for the full language specification
- Check [EFFECTS.md](../EFFECTS.md) for detailed effect system documentation
- See [MEMORY_MODEL.md](../MEMORY_MODEL.md) for memory safety details

## Building Larger Projects

Create a `Blood.toml` for project configuration:

```toml
[package]
name = "my_project"
version = "0.1.0"

[dependencies]
# Dependencies listed here

[build]
opt_level = 2
```

Build with:

```bash
blood build
blood run
blood test
```

## Getting Help

- File issues at: https://github.com/blood-lang/blood/issues
- Run `bloodc --help` for compiler options
- Run `blood help` for CLI tool usage
