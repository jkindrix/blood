# Getting Started with Blood

This tutorial walks you through writing your first Blood programs. You'll learn the core language features that make Blood unique: algebraic effects, generational memory safety, multiple dispatch, and linear types.

**Prerequisites:** LLVM 18 installed on your system (`llc-18`, `clang-18`).

## Building and Installing

```bash
cd src/selfhost
./build_selfhost.sh build first_gen    # ~2 minutes
./build_selfhost.sh install            # install to ~/.blood/bin/blood
```

After install, add `~/.blood/bin` to your PATH and use `blood` directly:

```bash
export PATH="$HOME/.blood/bin:$PATH"
blood run hello.blood
blood check hello.blood
blood build hello.blood -o hello
```

Or use the build directory directly: `build/first_gen run hello.blood`.

## Hello, World

```blood
fn main() {
    println_str("Hello, World!");
}
```

Save this as `hello.blood` and run it:

```bash
build/first_gen run hello.blood
```

Output: `Hello, World!`

## Functions and Types

Blood is a statically typed language with type inference. Every function declares its return type:

```blood
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let sum = add(10, 20);
    println_int(sum);  // 30
}
```

### Structs

```blood
struct Point {
    x: i32,
    y: i32,
}

fn distance_squared(p: Point) -> i32 {
    p.x * p.x + p.y * p.y
}

fn main() {
    let p = Point { x: 3, y: 4 };
    println_int(distance_squared(p));  // 25
}
```

### Enums and Pattern Matching

```blood
enum Shape {
    Circle(i32),
    Rectangle(i32, i32),
}

fn area(s: Shape) -> i32 {
    match s {
        Shape.Circle(r) => r * r * 3,  // approximate
        Shape.Rectangle(w, h) => w * h,
    }
}

fn main() {
    let c = Shape.Circle(5);
    let r = Shape.Rectangle(3, 4);
    println_int(area(c));  // 75
    println_int(area(r));  // 12
}
```

Note: Blood uses `.` for enum variant access (`Shape.Circle`), not `::`.

## Generics

```blood
struct Pair<A, B> {
    first: A,
    second: B,
}

fn swap<A, B>(p: Pair<A, B>) -> Pair<B, A> {
    Pair { first: p.second, second: p.first }
}

fn main() {
    let p = Pair { first: 1, second: true };
    let swapped = swap(p);
    println_int(swapped.second);  // 1
}
```

## Algebraic Effects

This is Blood's most distinctive feature. Effects let you declare *what* side effects a computation can perform, separate from *how* those effects are handled.

### Declaring an Effect

```blood
effect Logger {
    op log(msg: i32) -> ();
}
```

This declares a `Logger` effect with one operation: `log` takes an integer and returns unit.

### Performing Effects

```blood
fn do_work() / {Logger} {
    perform Logger.log(1);
    perform Logger.log(2);
    perform Logger.log(3);
}
```

The `/ {Logger}` annotation declares that `do_work` performs the `Logger` effect. The `perform` keyword invokes an effect operation.

### Handling Effects

```blood
deep handler PrintLogger for Logger {
    return(x) { x }
    op log(msg) {
        println_int(msg);
        resume(())
    }
}

fn main() {
    with PrintLogger {} handle {
        do_work()
    };
}
```

Output:
```
1
2
3
```

The handler intercepts each `perform Logger.log(msg)` call, prints the message, and `resume(())` continues the computation.

### Stateful Handlers

Handlers can carry mutable state:

```blood
effect Counter {
    op increment() -> ();
    op get_count() -> i32;
}

deep handler CounterImpl for Counter {
    let mut count: i32

    return(x) { x }
    op increment() {
        self.count += 1;
        resume(())
    }
    op get_count() {
        resume(self.count)
    }
}

fn count_things() / {Counter} {
    perform Counter.increment();
    perform Counter.increment();
    perform Counter.increment();
    let n = perform Counter.get_count();
    println_int(n);  // 3
}

fn main() {
    with CounterImpl { count: 0 } handle {
        count_things()
    };
}
```

## Memory Safety: Regions and Generational References

Blood uses generational references for memory safety without a garbage collector. Every reference carries a generation counter that's checked on dereference.

### Regions

Regions are scoped allocation pools. All memory allocated in a region is freed when the region exits:

```blood
fn main() -> i32 {
    let mut result: i32 = 0;
    region {
        let data = Point { x: 10, y: 20 };
        let r: &Point = &data;
        result = (*r).x + (*r).y;
        // r is valid here — region is still alive
    }
    // data and r are now invalid — region destroyed
    println_int(result);  // 30
    0
}
```

### Stale Reference Detection

If you hold a reference past the lifetime of its data, Blood detects it at runtime:

```blood
fn main() -> i32 {
    let mut s = String.new();
    s.push_str("hello");
    let view: &str = s.as_str();

    // This push_str may reallocate the buffer, invalidating 'view'
    s.push_str(" world! this is enough text to trigger reallocation");

    // Using 'view' now triggers: panic: stale reference detected
    print(view);
    0
}
```

This is Blood's answer to use-after-free: not a compile-time borrow checker, but runtime generation checking that catches every stale dereference.

## Multiple Dispatch

Blood dispatches function calls based on the types of *all* arguments, not just the receiver:

```blood
impl format(x: i32) -> String {
    let mut s = String.new();
    s.push_str("int:");
    // ... format integer
    s
}

impl format(x: bool) -> String {
    if x { String.from("true") } else { String.from("false") }
}

impl format(x: &str) -> String {
    let mut s = String.new();
    s.push_str("str:");
    s.push_str(x);
    s
}
```

The compiler selects the most specific overload at compile time based on argument types.

## Linear Types

Linear types enforce that a value is used exactly once — critical for resource management:

```blood
fn consume(linear handle: i32) {
    println_int(handle);
    // handle is consumed here
}

fn main() {
    let linear h: i32 = 42;
    consume(h);
    // Using h again would be a compile error:
    // "linear value used more than once"
}
```

## Closures

```blood
fn apply(f: fn(i32) -> i32, x: i32) -> i32 {
    f(x)
}

fn main() {
    let double = |x: i32| -> i32 { x * 2 };
    let result = apply(double, 21);
    println_int(result);  // 42
}
```

## For Loops

```blood
fn main() {
    let numbers = vec![1, 2, 3, 4, 5];
    let mut sum: i32 = 0;
    for n in &numbers {
        sum += *n;
    }
    println_int(sum);  // 15

    // Range loops
    for i in 0..10 {
        print_int(i);
        print_str(" ");
    }
    println_str("");
}
```

## User-Defined Macros

Blood supports declarative macros within a single file:

```blood
macro repeat {
    ($body:expr, $n:expr) => {
        for _i in 0..$n {
            $body;
        }
    };
}

fn main() -> i32 {
    repeat!(println_str("hello"), 3);
    0
}
```

Output:
```
hello
hello
hello
```

## Compiling vs. Running

```bash
# Type-check only (fast)
build/first_gen check myfile.blood

# Compile and run
build/first_gen run myfile.blood

# Compile to binary
build/first_gen build myfile.blood -o myprogram
./myprogram
```

## What's Next

- Browse the `examples/` directory for 68 complete programs
- Read `docs/spec/SPECIFICATION.md` for the full language specification
- See `docs/KNOWN_LIMITATIONS.md` for honest status of each feature
- Try the [proving ground programs](../tests/proving/) for complex multi-feature examples
