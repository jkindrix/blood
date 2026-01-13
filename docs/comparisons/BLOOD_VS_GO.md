# Blood vs Go: Practical Systems Languages Compared

**Purpose**: Compare Blood and Go for developers choosing a practical systems language.

Go and Blood both aim to be practical, productive languages for building reliable software. Go achieves this through simplicity and garbage collection; Blood through effects and generational memory safety.

---

## Executive Summary

| Aspect | Blood | Go |
|--------|-------|-----|
| **Memory Model** | Generational references | Garbage collection |
| **Concurrency** | Effect handlers | Goroutines + channels |
| **Error Handling** | Effect system | Multiple return values |
| **Generics** | Full support | Basic (since 1.18) |
| **Compile Speed** | Fast | Very fast |
| **Simplicity** | Moderate | High |
| **Learning Curve** | Moderate | Low |
| **Ecosystem** | Early | Mature |

---

## 1. Design Philosophy

### Go: Simplicity Above All

Go was designed for simplicity at scale:

```go
// Go: Simple, explicit, readable
func ProcessItems(items []Item) ([]Result, error) {
    results := make([]Result, 0, len(items))
    for _, item := range items {
        result, err := process(item)
        if err != nil {
            return nil, fmt.Errorf("processing %s: %w", item.ID, err)
        }
        results = append(results, result)
    }
    return results, nil
}
```

**Go's Design Principles:**
- Less is more (few features)
- Explicit over implicit
- Composition over inheritance
- Readability at scale
- Fast compilation is non-negotiable

### Blood: Safety with Expressiveness

Blood balances safety, expressiveness, and performance:

```blood
// Blood: Effects make dependencies explicit
fn process_items(items: Vec<Item>) -> Vec<Result> with Process, Error {
    items.into_iter()
        .map(|item| {
            match do Process.process(item.clone()) {
                Ok(result) => result,
                Err(e) => do Error.fail(format!("processing {}: {}", item.id, e)),
            }
        })
        .collect()
}
```

**Blood's Design Principles:**
- Safety without garbage collection
- Effects for explicit control flow
- Performance for systems programming
- Expressiveness where it helps

---

## 2. Memory Management

### Go: Garbage Collection

Go uses a concurrent, tri-color mark-and-sweep collector:

```go
func main() {
    // Allocate freely - GC handles cleanup
    data := make([]byte, 1_000_000)
    process(data)
    // data is collected when unreachable
}

// Pointers work naturally
type Node struct {
    Value int
    Next  *Node  // GC handles cycles
}

func createCycle() *Node {
    a := &Node{Value: 1}
    b := &Node{Value: 2, Next: a}
    a.Next = b  // Cycle - no problem for GC
    return a
}
```

**Go GC Characteristics:**
- Sub-millisecond pause times (tuned for latency)
- No manual memory management
- Cycles handled automatically
- Some memory overhead (~30% typical)
- STW pauses can affect tail latency

### Blood: Generational References

Blood uses generation-based safety checking:

```blood
fn main() {
    // Allocation tracked by generation
    let data = vec![0u8; 1_000_000];
    process(&data);
    // data freed deterministically at scope end
}

// References include generation
struct Node {
    value: i32,
    next: Option<&Node>,
}

fn create_cycle() {
    let a = Node { value: 1, next: None };
    let b = Node { value: 2, next: Some(&a) };
    a.next = Some(&b);  // Cycle - no problem

    // When scope ends, both freed, references invalidated
}
```

**Blood Memory Characteristics:**
- Deterministic deallocation (no GC pauses)
- ~4 cycle overhead for checked accesses
- 128-bit pointers (memory overhead for pointer-heavy code)
- Stack allocation via escape analysis
- No STW pauses ever

### Memory Model Comparison

| Aspect | Go | Blood |
|--------|-----|-------|
| Pause times | Sub-ms (tunable) | Zero |
| Memory overhead | ~30% typical | ~10-15% (pointers) |
| Determinism | No (GC timing varies) | Yes |
| Cyclic structures | Handled by GC | Handled by generations |
| Embedded suitability | Limited (GC) | Good |
| Latency tail | Can spike | Predictable |

---

## 3. Concurrency

### Go: Goroutines and Channels

Go's concurrency is built on CSP:

```go
func fetchAll(urls []string) []Result {
    results := make(chan Result, len(urls))

    for _, url := range urls {
        go func(url string) {
            resp, err := http.Get(url)
            if err != nil {
                results <- Result{URL: url, Err: err}
                return
            }
            defer resp.Body.Close()
            body, _ := io.ReadAll(resp.Body)
            results <- Result{URL: url, Body: body}
        }(url)
    }

    var all []Result
    for range urls {
        all = append(all, <-results)
    }
    return all
}
```

**Go Concurrency Features:**
- Goroutines are lightweight (~2KB stack)
- Channels for communication
- `select` for multiplexing
- `sync` package for primitives
- No async/await coloring

### Blood: Effect-Based Concurrency

Blood uses effects for concurrency:

```blood
effect Async {
    fn spawn<T>(task: fn() -> T) -> Future<T>;
    fn await<T>(future: Future<T>) -> T;
}

effect Channel<T> {
    fn send(value: T) -> ();
    fn recv() -> T;
}

fn fetch_all(urls: Vec<String>) -> Vec<Result> with Async, Http {
    let futures: Vec<Future<Result>> = urls
        .into_iter()
        .map(|url| {
            do Async.spawn(|| {
                match do Http.get(url.clone()) {
                    Ok(resp) => Result { url, body: resp.body(), err: None },
                    Err(e) => Result { url, body: vec![], err: Some(e) },
                }
            })
        })
        .collect();

    futures
        .into_iter()
        .map(|f| do Async.await(f))
        .collect()
}
```

**Blood Concurrency Features:**
- Concurrency as effects
- Handlers control execution strategy
- Swappable for testing (deterministic concurrency)
- No colored functions
- Type-safe concurrent operations

### Concurrency Comparison

| Aspect | Go | Blood |
|--------|-----|-------|
| Model | Goroutines + channels | Effect handlers |
| Syntax | `go func()` + `<-chan` | `do Async.spawn()` |
| Testability | Requires mocking | Handler swapping |
| Race detection | Runtime flag | Type-level (effects) |
| Overhead | Very low | Low |
| Learning curve | Easy | Moderate |

---

## 4. Error Handling

### Go: Multiple Return Values

Go uses explicit error returns:

```go
func ReadConfig(path string) (*Config, error) {
    data, err := os.ReadFile(path)
    if err != nil {
        return nil, fmt.Errorf("reading config: %w", err)
    }

    var config Config
    if err := json.Unmarshal(data, &config); err != nil {
        return nil, fmt.Errorf("parsing config: %w", err)
    }

    if err := config.Validate(); err != nil {
        return nil, fmt.Errorf("invalid config: %w", err)
    }

    return &config, nil
}

// Caller must check
func main() {
    config, err := ReadConfig("config.json")
    if err != nil {
        log.Fatal(err)
    }
    // use config
}
```

**Go Error Characteristics:**
- Explicit `if err != nil` checks
- Error wrapping with `%w`
- No exceptions
- Can be verbose
- Easy to forget error checks

### Blood: Effect-Based Errors

Blood uses effects for errors:

```blood
effect Fail {
    fn fail(msg: String) -> !;
}

fn read_config(path: String) -> Config with IO, Fail {
    let data = match do IO.read_file(path.clone()) {
        Ok(d) => d,
        Err(e) => do Fail.fail(format!("reading config: {}", e)),
    };

    let config: Config = match json::parse(&data) {
        Ok(c) => c,
        Err(e) => do Fail.fail(format!("parsing config: {}", e)),
    };

    if let Err(e) = config.validate() {
        do Fail.fail(format!("invalid config: {}", e));
    }

    config
}

// Handler determines error behavior
fn main() {
    try {
        try {
            let config = read_config("config.json".to_string());
            // use config
        } with RealIO
    } with handler LogAndExit: Fail {
        fn fail(msg: String) -> ! {
            eprintln!("Error: {}", msg);
            std::process::exit(1);
        }
    }
}
```

**Blood Error Characteristics:**
- Errors propagate via effects
- Handler determines behavior
- No forgotten error checks (type system enforces)
- Less verbose than `if err != nil`
- Testable with different handlers

### Error Handling Comparison

| Aspect | Go | Blood |
|--------|-----|-------|
| Syntax | `if err != nil` | `do Effect.fail()` |
| Propagation | Manual | Automatic (effect) |
| Recovery | Manual | Handler-based |
| Verbosity | High | Low |
| Type safety | Partial (can ignore) | Full (effects tracked) |
| Testing | Mocking | Handler swapping |

---

## 5. Type System

### Go: Simple and Structural

Go has a deliberately simple type system:

```go
// No generics until 1.18, limited now
type Stack[T any] struct {
    items []T
}

func (s *Stack[T]) Push(item T) {
    s.items = append(s.items, item)
}

// No sum types - use interfaces
type Result interface {
    isResult()
}

type Success struct {
    Value int
}
func (Success) isResult() {}

type Failure struct {
    Error string
}
func (Failure) isResult() {}
```

**Go Type System:**
- Structural interfaces
- Basic generics (since 1.18)
- No sum types / enums (proposed)
- No pattern matching
- Type inference limited

### Blood: Expressive Types

Blood has a richer type system:

```blood
// Full generics
struct Stack<T> {
    items: Vec<T>,
}

impl<T> Stack<T> {
    fn push(&mut self, item: T) {
        self.items.push(item);
    }
}

// Sum types (enums)
enum Result<T, E> {
    Ok(T),
    Err(E),
}

// Pattern matching
fn handle(result: Result<i32, String>) -> i32 {
    match result {
        Result::Ok(value) => value,
        Result::Err(msg) => {
            eprintln!("Error: {}", msg);
            0
        }
    }
}
```

**Blood Type System:**
- Algebraic data types
- Full generics with constraints
- Pattern matching
- Effect types
- Trait-based polymorphism

### Type System Comparison

| Feature | Go | Blood |
|---------|-----|-------|
| Generics | Basic | Full |
| Sum types | No (interface workaround) | Yes |
| Pattern matching | No | Yes |
| Type inference | Limited | Extensive |
| Effect types | No | Yes |
| Null safety | No (nil) | Yes (Option) |

---

## 6. Compile Times

Both languages prioritize fast compilation:

### Go: Fastest in Class

```
$ time go build ./...
real    0m0.823s
```

Go achieves this through:
- No templates/monomorphization (until recently)
- Simple dependency analysis
- Parallel compilation
- No complex type inference

### Blood: Fast but More Work

```
$ time bloodc build
real    0m2.134s
```

Blood is slower due to:
- Generic monomorphization
- Effect type checking
- More complex type system

But still fast due to:
- Incremental compilation
- Content-addressed caching
- Simple effect semantics

### Compile Time Comparison

| Project Size | Go | Blood |
|--------------|-----|-------|
| Small (1K LOC) | <1s | ~2s |
| Medium (10K LOC) | 2s | 10s |
| Large (100K LOC) | 15s | 1-2m |

---

## 7. Standard Library

### Go: Batteries Included

Go's standard library is comprehensive:

```go
import (
    "net/http"       // HTTP client/server
    "encoding/json"  // JSON
    "database/sql"   // SQL databases
    "context"        // Cancellation
    "sync"           // Concurrency primitives
    "testing"        // Testing framework
)
```

### Blood: Growing

Blood's standard library is developing:

```blood
use std::net::http;      // HTTP (basic)
use std::json;           // JSON
use std::collections;    // Data structures
use std::fs;             // File system
use std::io;             // I/O traits
```

### Library Ecosystem

| Domain | Go | Blood |
|--------|-----|-------|
| HTTP | Excellent | Basic |
| JSON | Excellent | Good |
| Database | Excellent | Planned |
| Testing | Good | Good |
| Crypto | Excellent | Via FFI |
| Compression | Good | Basic |

---

## 8. Code Examples

### HTTP Server

**Go:**
```go
package main

import (
    "encoding/json"
    "net/http"
)

type Response struct {
    Message string `json:"message"`
}

func main() {
    http.HandleFunc("/hello", func(w http.ResponseWriter, r *http.Request) {
        resp := Response{Message: "Hello, World!"}
        json.NewEncoder(w).Encode(resp)
    })

    http.ListenAndServe(":8080", nil)
}
```

**Blood:**
```blood
use std::net::http::{Server, Request, Response};
use std::json;

struct JsonResponse {
    message: String,
}

fn main() with IO {
    let server = Server::new("0.0.0.0:8080".to_string());

    server.route("/hello", |_req: Request| -> Response {
        let resp = JsonResponse { message: "Hello, World!".to_string() };
        Response::json(&resp)
    });

    do IO.println("Server listening on :8080".to_string());
    server.run();
}
```

### Concurrent Processing

**Go:**
```go
func processItems(items []Item) []Result {
    var wg sync.WaitGroup
    results := make([]Result, len(items))

    for i, item := range items {
        wg.Add(1)
        go func(i int, item Item) {
            defer wg.Done()
            results[i] = process(item)
        }(i, item)
    }

    wg.Wait()
    return results
}
```

**Blood:**
```blood
fn process_items(items: Vec<Item>) -> Vec<Result> with Async {
    let futures: Vec<_> = items
        .into_iter()
        .map(|item| do Async.spawn(|| process(item)))
        .collect();

    futures
        .into_iter()
        .map(|f| do Async.await(f))
        .collect()
}
```

---

## 9. When to Choose Each

### Choose Go When:

1. **Team Productivity Matters Most**
   - Large teams with varying skill levels
   - Fast onboarding required

2. **Network Services**
   - HTTP servers, APIs
   - Microservices

3. **DevOps Tools**
   - CLI applications
   - Kubernetes ecosystem

4. **Fast Iteration**
   - Prototyping
   - Startups

5. **Mature Ecosystem Needed**
   - Production today
   - Well-established libraries

### Choose Blood When:

1. **Effect Handling is Central**
   - Compilers, interpreters
   - Complex control flow

2. **Deterministic Behavior Required**
   - Real-time systems
   - Safety-critical

3. **Testing is Critical**
   - Handler swapping for mocks
   - Deterministic concurrency tests

4. **Memory Control Needed**
   - Embedded systems
   - Latency-sensitive

5. **Type Safety Paramount**
   - Sum types for modeling
   - Effect tracking

---

## 10. Honest Assessment

### Go's Advantages Over Blood

1. **Simplicity**: Easier to learn, less cognitive load
2. **Ecosystem**: 15+ years of packages and tools
3. **Compile Speed**: Consistently fastest
4. **Production Proven**: Google, Uber, Dropbox scale
5. **Hiring**: Large talent pool familiar with Go
6. **Tooling**: `gofmt`, `go vet`, gopls are excellent

### Blood's Advantages Over Go

1. **Type Safety**: Sum types, effect tracking, no nil
2. **Determinism**: No GC pauses, predictable timing
3. **Testability**: Effect handlers for dependency injection
4. **Expressiveness**: Pattern matching, generics
5. **Error Handling**: Less verbose, cannot forget errors
6. **Memory Control**: Suitable for embedded/real-time

### Neither When:

- Need extensive ecosystem today → Go
- Need maximum performance with zero overhead → Rust
- Building simple scripts → Python
- GUI applications → Other platforms

---

## 11. Migration Considerations

### From Go to Blood

```go
// Go
func Process(data []byte) (Result, error) {
    if len(data) == 0 {
        return Result{}, errors.New("empty data")
    }
    // process
    return result, nil
}
```

```blood
// Blood
fn process(data: Vec<u8>) -> Result with Fail {
    if data.is_empty() {
        do Fail.fail("empty data".to_string());
    }
    // process
    result
}
```

**Key Changes:**
1. Replace `error` returns with `Fail` effect
2. Replace `nil` with `Option`
3. Use pattern matching instead of type assertions
4. Replace goroutines with `Async` effect

### From Blood to Go

**Key Changes:**
1. Replace effects with explicit error returns
2. Replace pattern matching with type switches
3. Replace sum types with interfaces
4. Replace effect handlers with dependency injection

---

## Conclusion

Go and Blood represent different philosophies:

- **Go**: Simplicity is the ultimate sophistication
- **Blood**: Safety and expressiveness can coexist

For teams that value simplicity and have mature ecosystem needs, Go is excellent. For teams building systems where effect handling, determinism, and type safety matter most, Blood offers compelling advantages.

Both are valid choices for systems programming. The decision depends on your specific constraints, team expertise, and domain requirements.

---

*This comparison aims to be fair to both languages. Each makes reasonable trade-offs for different goals.*
