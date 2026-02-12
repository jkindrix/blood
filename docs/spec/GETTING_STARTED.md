# Getting Started with Blood

A quick guide to writing and running your first Blood programs.

## Prerequisites

- **Rust**: Install via [rustup](https://rustup.rs/) (1.75+)
- **LLVM**: Version 14+ (`llvm-config --version`)
- **C compiler**: GCC or Clang for runtime linking

## Building the Compiler

```bash
# Clone the repository
git clone https://github.com/blood-lang/blood.git
cd blood

# Build in release mode
cargo build --release

# The compiler binary is at target/release/blood
```

## Your First Program

### Hello World

Create a file `hello.blood`:

```blood
fn main() {
    println_str("Hello, World!");
}
```

Compile and run:

```bash
# Compile to executable
cargo run -- build examples/hello.blood -o hello

# Run the program
./hello
# Output: Hello, World!
```

### FizzBuzz

A more complete example (`fizzbuzz.blood`):

```blood
fn main() {
    let mut i: i32 = 1;
    while i <= 15 {
        if i % 15 == 0 {
            println_str("FizzBuzz");
        } else if i % 3 == 0 {
            println_str("Fizz");
        } else if i % 5 == 0 {
            println_str("Buzz");
        } else {
            println_int(i);
        }
        i = i + 1;
    }
}
```

Compile and run:

```bash
cargo run -- build fizzbuzz.blood -o fizzbuzz
./fizzbuzz
```

Output:
```
1
2
Fizz
4
Buzz
Fizz
7
8
Fizz
Buzz
11
Fizz
13
14
FizzBuzz
```

## Command Reference

```bash
# Compile a Blood file
blood build <file.blood> [-o <output>]

# Compile and run immediately
blood run <file.blood>

# Show help
blood --help
```

## Language Basics

### Variables

```blood
// Immutable binding
let x: i32 = 42;

// Mutable binding
let mut y: i32 = 0;
y = y + 1;

// Type inference
let z = 100;  // inferred as i32
```

### Functions

```blood
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn greet(name: &str) {
    print_str("Hello, ");
    println_str(name);
}
```

### Control Flow

```blood
// If-else
if condition {
    // ...
} else if other_condition {
    // ...
} else {
    // ...
}

// While loop
while i < 10 {
    i = i + 1;
}

// Loop with break
loop {
    if done {
        break;
    }
}
```

### Structs

```blood
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 10, y: 20 };
}
```

### Enums

```blood
enum Option<T> {
    Some(T),
    None,
}

enum Direction {
    North,
    South,
    East,
    West,
}
```

## Effects (Advanced)

Blood uses algebraic effects for I/O, state, and error handling:

```blood
// Effect declaration
effect State<T> {
    fn get() -> T;
    fn put(value: T);
}

// Function with effect
fn increment() / {State<i32>} {
    let x = perform State.get();
    perform State.put(x + 1);
}

// Handler
handle increment() with {
    State {
        get => resume(state),
        put(v) => { state = v; resume(()) },
    }
} where state = 0;
```

## More Examples

### Working with Collections

#### Vectors

```blood
use std::collections::Vec;

fn main() {
    // Create a vector
    let mut numbers: Vec<i32> = Vec::new();

    // Add elements
    numbers.push(1);
    numbers.push(2);
    numbers.push(3);

    // Or use the vec! macro
    let colors = vec!["red", "green", "blue"];

    // Iterate
    for n in &numbers {
        println_int(*n);
    }

    // Access by index
    let first = numbers[0];  // 1

    // Length check
    if !numbers.is_empty() {
        println_str("Vector has elements");
    }

    // Pop last element
    if let Some(last) = numbers.pop() {
        println_int(last);  // 3
    }
}
```

#### HashMaps

```blood
use std::collections::HashMap;

fn main() {
    // Create a hash map
    let mut scores: HashMap<str, i32> = HashMap::new();

    // Insert key-value pairs
    scores.insert("Alice", 100);
    scores.insert("Bob", 85);
    scores.insert("Carol", 92);

    // Get a value
    if let Some(score) = scores.get(&"Alice") {
        print_str("Alice's score: ");
        println_int(*score);
    }

    // Update using entry API
    *scores.entry("Bob").or_insert(0) += 10;

    // Iterate over key-value pairs
    for (name, score) in scores.iter() {
        print_str(name);
        print_str(": ");
        println_int(*score);
    }

    // Check if key exists
    if scores.contains_key(&"Dave") {
        println_str("Found Dave");
    } else {
        println_str("Dave not found");
    }
}
```

### Pattern Matching

```blood
enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
    Triangle { base: f64, height: f64 },
}

fn area(shape: Shape) -> f64 {
    match shape {
        Shape::Circle { radius } => 3.14159 * radius * radius,
        Shape::Rectangle { width, height } => width * height,
        Shape::Triangle { base, height } => 0.5 * base * height,
    }
}

fn describe_number(n: i32) -> str {
    match n {
        0 => "zero",
        1 => "one",
        2..=9 => "single digit",
        10..=99 => "double digit",
        _ => "big number",
    }
}

fn main() {
    let circle = Shape::Circle { radius: 5.0 };
    let rect = Shape::Rectangle { width: 4.0, height: 3.0 };

    println_float(area(circle));     // ~78.54
    println_float(area(rect));       // 12.0

    println_str(describe_number(42)); // "double digit"
}
```

### Option and Result Types

```blood
use std::core::Option::{Some, None};
use std::core::Result::{Ok, Err};

// Option for nullable values
fn divide(a: i32, b: i32) -> Option<i32> {
    if b == 0 {
        None
    } else {
        Some(a / b)
    }
}

// Result for operations that can fail
fn parse_digit(c: char) -> Result<i32, str> {
    match c {
        '0' => Ok(0),
        '1' => Ok(1),
        '2' => Ok(2),
        '3' => Ok(3),
        '4' => Ok(4),
        '5' => Ok(5),
        '6' => Ok(6),
        '7' => Ok(7),
        '8' => Ok(8),
        '9' => Ok(9),
        _ => Err("not a digit"),
    }
}

fn main() {
    // Using Option
    match divide(10, 3) {
        Some(result) => println_int(result),
        None => println_str("Cannot divide by zero"),
    }

    // Using unwrap_or for defaults
    let result = divide(10, 0).unwrap_or(0);

    // Using Result
    match parse_digit('7') {
        Ok(n) => println_int(n),
        Err(msg) => println_str(msg),
    }

    // Chaining operations
    let doubled = divide(20, 4)
        .map(|x| x * 2)
        .unwrap_or(0);
    println_int(doubled);  // 10
}
```

### Generic Functions

```blood
// Generic identity function
fn identity<T>(x: T) -> T {
    x
}

// Generic swap
fn swap<T>(a: &mut T, b: &mut T) {
    let temp = *a;
    *a = *b;
    *b = temp;
}

// Generic max with trait bound
fn max<T: Ord>(a: T, b: T) -> T {
    if a > b { a } else { b }
}

// Generic pair struct
struct Pair<A, B> {
    first: A,
    second: B,
}

impl<A, B> Pair<A, B> {
    fn new(first: A, second: B) -> Self {
        Pair { first, second }
    }

    fn swap(self) -> Pair<B, A> {
        Pair { first: self.second, second: self.first }
    }
}

fn main() {
    // Using generics
    let x = identity(42);           // i32
    let y = identity("hello");      // &str

    let mut a = 1;
    let mut b = 2;
    swap(&mut a, &mut b);
    println_int(a);  // 2
    println_int(b);  // 1

    let m = max(10, 20);
    println_int(m);  // 20

    let pair = Pair::new("key", 42);
    println_str(pair.first);   // "key"
    println_int(pair.second);  // 42
}
```

### Traits and Implementations

```blood
// Define a trait
trait Printable {
    fn print(&self);
}

// Implement for custom types
struct Person {
    name: str,
    age: i32,
}

impl Printable for Person {
    fn print(&self) {
        print_str("Person: ");
        print_str(self.name);
        print_str(", age ");
        println_int(self.age);
    }
}

impl Printable for i32 {
    fn print(&self) {
        println_int(*self);
    }
}

// Generic function using trait bound
fn print_twice<T: Printable>(item: &T) {
    item.print();
    item.print();
}

fn main() {
    let alice = Person { name: "Alice", age: 30 };
    alice.print();

    let number = 42;
    print_twice(&number);
}
```

### Error Handling with Effects

```blood
// Define an Error effect
effect Error<E> {
    fn throw(error: E) -> never;
}

// Function that can fail
fn parse_positive(s: str) -> i32 / {Error<str>} {
    let n = parse_int(s)?;  // assume this exists
    if n <= 0 {
        perform Error.throw("must be positive");
    }
    n
}

// Handler that converts to Result
fn main() {
    let result: Result<i32, str> = handle {
        let value = parse_positive("42");
        Ok(value)
    } with {
        Error {
            throw(e) => Err(e)
        }
    };

    match result {
        Ok(n) => {
            print_str("Parsed: ");
            println_int(n);
        }
        Err(msg) => {
            print_str("Error: ");
            println_str(msg);
        }
    }
}
```

### State Effect Example

```blood
// Counter using state effect
effect State<T> {
    fn get() -> T;
    fn put(value: T);
}

fn increment() / {State<i32>} {
    let current = perform State.get();
    perform State.put(current + 1);
}

fn add(n: i32) / {State<i32>} {
    let current = perform State.get();
    perform State.put(current + n);
}

fn multiply(n: i32) / {State<i32>} {
    let current = perform State.get();
    perform State.put(current * n);
}

fn main() {
    // Run a computation with state
    let final_value = handle {
        increment();      // state = 1
        increment();      // state = 2
        add(10);          // state = 12
        multiply(2);      // state = 24
        perform State.get()
    } with {
        State {
            get => resume(state),
            put(v) => { state = v; resume(()) }
        }
    } where state = 0;

    print_str("Final value: ");
    println_int(final_value);  // 24
}
```

### Recursive Data Structures

```blood
// Binary tree
enum Tree<T> {
    Empty,
    Node {
        value: T,
        left: Box<Tree<T>>,
        right: Box<Tree<T>>,
    },
}

impl<T> Tree<T> {
    fn leaf(value: T) -> Self {
        Tree::Node {
            value,
            left: Box::new(Tree::Empty),
            right: Box::new(Tree::Empty),
        }
    }

    fn node(value: T, left: Tree<T>, right: Tree<T>) -> Self {
        Tree::Node {
            value,
            left: Box::new(left),
            right: Box::new(right),
        }
    }
}

fn sum_tree(tree: &Tree<i32>) -> i32 {
    match tree {
        Tree::Empty => 0,
        Tree::Node { value, left, right } => {
            *value + sum_tree(left) + sum_tree(right)
        }
    }
}

fn count_nodes<T>(tree: &Tree<T>) -> i32 {
    match tree {
        Tree::Empty => 0,
        Tree::Node { left, right, .. } => {
            1 + count_nodes(left) + count_nodes(right)
        }
    }
}

fn main() {
    //       5
    //      / \
    //     3   7
    //    / \
    //   1   4
    let tree = Tree::node(
        5,
        Tree::node(3, Tree::leaf(1), Tree::leaf(4)),
        Tree::leaf(7)
    );

    print_str("Sum: ");
    println_int(sum_tree(&tree));    // 20

    print_str("Nodes: ");
    println_int(count_nodes(&tree)); // 5
}
```

### Linked List

```blood
// Singly linked list
enum List<T> {
    Nil,
    Cons(T, Box<List<T>>),
}

impl<T> List<T> {
    fn new() -> Self {
        List::Nil
    }

    fn prepend(self, value: T) -> Self {
        List::Cons(value, Box::new(self))
    }

    fn len(&self) -> i32 {
        match self {
            List::Nil => 0,
            List::Cons(_, tail) => 1 + tail.len(),
        }
    }
}

fn main() {
    let list = List::new()
        .prepend(3)
        .prepend(2)
        .prepend(1);

    print_str("Length: ");
    println_int(list.len());  // 3
}
```

### Iterators

```blood
fn main() {
    let numbers = vec![1, 2, 3, 4, 5];

    // Map - transform each element
    let doubled: Vec<i32> = numbers.iter()
        .map(|x| *x * 2)
        .collect();
    // doubled = [2, 4, 6, 8, 10]

    // Filter - keep matching elements
    let evens: Vec<i32> = numbers.iter()
        .filter(|x| *x % 2 == 0)
        .cloned()
        .collect();
    // evens = [2, 4]

    // Fold - reduce to single value
    let sum: i32 = numbers.iter().fold(0, |acc, x| acc + x);
    // sum = 15

    // Chaining operations
    let result: i32 = numbers.iter()
        .filter(|x| *x > 2)
        .map(|x| *x * 10)
        .sum();
    // result = 30 + 40 + 50 = 120

    println_int(result);
}
```

## Next Steps

- Read the [Language Specification](SPECIFICATION.md)
- Explore [Example Programs](../../examples/)
- Learn about [Effects](SPECIFICATION.md#4-effect-system)
- Understand the [Memory Model](MEMORY_MODEL.md)
- See the [Effects Tutorial](EFFECTS_TUTORIAL.md) for in-depth effect usage
- Check the [Performance Guide](PERFORMANCE_GUIDE.md) for optimization tips

## Troubleshooting

### LLVM Not Found

```bash
# Ubuntu/Debian
sudo apt install llvm-14-dev

# macOS
brew install llvm@14

# Set LLVM path
export LLVM_SYS_140_PREFIX=/usr/lib/llvm-14
```

### Linker Errors

The compiler generates object files that must be linked with the Blood runtime:

```bash
# Generate and compile the minimal C runtime
blood build --emit-runtime > runtime.c
cc -c runtime.c -o runtime.o

# Link your program
cc your_program.o runtime.o -o your_program
```

### Common Errors

| Error | Solution |
|-------|----------|
| "Unknown type `i32`" | Use lowercase types: `i32`, `bool`, `f64` |
| "Expected `;`" | Add semicolon after statements |
| "Undefined function" | Declare functions before use |
| "Type mismatch" | Check argument and return types |

## Getting Help

- [GitHub Issues](https://github.com/blood-lang/blood/issues)
- [Language Specification](SPECIFICATION.md)
- [Implementation Status](IMPLEMENTATION_STATUS.md)
