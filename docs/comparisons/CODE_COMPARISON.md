# Code Comparison: Blood vs Rust vs Go

**Purpose**: Demonstrate ergonomic differences through identical programs implemented in each language.

This document shows the same non-trivial programs implemented in Blood, Rust, and Go to illustrate how each language approaches common patterns.

---

## Example 1: Word Frequency Counter

A program that reads a text file, counts word frequencies, and reports the top N most common words. This demonstrates:
- File I/O and error handling
- String processing
- Collection usage (hash maps)
- Sorting and iteration

### Blood Implementation

```blood
// word_frequency.blood - Word frequency counter with effects

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, BufRead};

effect IO {
    fn read_file(path: String) -> String;
    fn println(msg: String) -> ();
}

effect Error {
    fn fail(msg: String) -> !;
}

struct WordCount {
    word: String,
    count: u64,
}

fn count_words(text: String) -> HashMap<String, u64> {
    let mut counts = HashMap::new();

    for word in text.split_whitespace() {
        // Normalize: lowercase, strip punctuation
        let normalized = word
            .to_lowercase()
            .trim_matches(|c: char| !c.is_alphanumeric());

        if !normalized.is_empty() {
            counts.entry(normalized)
                  .and_modify(|c| *c += 1)
                  .or_insert(1);
        }
    }

    counts
}

fn top_n(counts: HashMap<String, u64>, n: usize) -> Vec<WordCount> {
    let mut entries: Vec<WordCount> = counts
        .into_iter()
        .map(|(word, count)| WordCount { word, count })
        .collect();

    // Sort by count descending
    entries.sort_by(|a, b| b.count.cmp(&a.count));
    entries.truncate(n);
    entries
}

fn main() with IO, Error {
    let args = std::env::args();

    if args.len() < 2 {
        do Error.fail("Usage: word_frequency <file> [top_n]");
    }

    let filename = args[1].clone();
    let n: usize = args.get(2)
        .map(|s| s.parse().unwrap_or(10))
        .unwrap_or(10);

    let text = do IO.read_file(filename.clone());
    let counts = count_words(text);
    let top = top_n(counts, n);

    do IO.println(format!("Top {} words in '{}':", n, filename));
    do IO.println("".to_string());

    for (i, wc) in top.iter().enumerate() {
        do IO.println(format!("{:3}. {:20} {}", i + 1, wc.word, wc.count));
    }
}

// Handler for real I/O
handler RealIO: IO {
    fn read_file(path: String) -> String {
        match std::fs::read_to_string(&path) {
            Ok(content) => resume(content),
            Err(e) => do Error.fail(format!("Failed to read '{}': {}", path, e)),
        }
    }

    fn println(msg: String) -> () {
        println!("{}", msg);
        resume(())
    }
}

// Entry point with handler
fn run() {
    try {
        with RealIO {
            try {
                main()
            } with ErrorHandler
        }
    }
}

handler ErrorHandler: Error {
    fn fail(msg: String) -> ! {
        eprintln!("Error: {}", msg);
        std::process::exit(1);
    }
}
```

**Blood Characteristics:**
- Effects separate "what" (IO operations) from "how" (handlers)
- Error handling through effect system, not Result types
- Clean control flow without `?` operator chains
- Handlers can be swapped for testing (mock IO)
- No lifetime annotations needed

### Rust Implementation

```rust
// word_frequency.rs - Word frequency counter

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};

struct WordCount {
    word: String,
    count: u64,
}

fn count_words(text: &str) -> HashMap<String, u64> {
    let mut counts = HashMap::new();

    for word in text.split_whitespace() {
        // Normalize: lowercase, strip punctuation
        let normalized: String = word
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();

        if !normalized.is_empty() {
            *counts.entry(normalized).or_insert(0) += 1;
        }
    }

    counts
}

fn top_n(counts: HashMap<String, u64>, n: usize) -> Vec<WordCount> {
    let mut entries: Vec<WordCount> = counts
        .into_iter()
        .map(|(word, count)| WordCount { word, count })
        .collect();

    // Sort by count descending
    entries.sort_by(|a, b| b.count.cmp(&a.count));
    entries.truncate(n);
    entries
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: word_frequency <file> [top_n]");
        std::process::exit(1);
    }

    let filename = &args[1];
    let n: usize = args.get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let text = fs::read_to_string(filename)?;
    let counts = count_words(&text);
    let top = top_n(counts, n);

    println!("Top {} words in '{}':", n, filename);
    println!();

    for (i, wc) in top.iter().enumerate() {
        println!("{:3}. {:20} {}", i + 1, wc.word, wc.count);
    }

    Ok(())
}
```

**Rust Characteristics:**
- Explicit `Result` types for error handling
- `?` operator for error propagation
- References (`&str`, `&args[1]`) require lifetime thinking
- `Box<dyn std::error::Error>` for generic error handling
- Explicit `Ok(())` return
- No built-in way to swap I/O for testing without traits

### Go Implementation

```go
// word_frequency.go - Word frequency counter

package main

import (
	"fmt"
	"os"
	"sort"
	"strconv"
	"strings"
	"unicode"
)

type WordCount struct {
	Word  string
	Count int
}

func countWords(text string) map[string]int {
	counts := make(map[string]int)

	for _, word := range strings.Fields(text) {
		// Normalize: lowercase, strip punctuation
		normalized := strings.Map(func(r rune) rune {
			if unicode.IsLetter(r) || unicode.IsDigit(r) {
				return unicode.ToLower(r)
			}
			return -1
		}, word)

		if normalized != "" {
			counts[normalized]++
		}
	}

	return counts
}

func topN(counts map[string]int, n int) []WordCount {
	entries := make([]WordCount, 0, len(counts))
	for word, count := range counts {
		entries = append(entries, WordCount{word, count})
	}

	// Sort by count descending
	sort.Slice(entries, func(i, j int) bool {
		return entries[i].Count > entries[j].Count
	})

	if n < len(entries) {
		entries = entries[:n]
	}
	return entries
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "Usage: word_frequency <file> [top_n]")
		os.Exit(1)
	}

	filename := os.Args[1]
	n := 10
	if len(os.Args) > 2 {
		if parsed, err := strconv.Atoi(os.Args[2]); err == nil {
			n = parsed
		}
	}

	text, err := os.ReadFile(filename)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error reading '%s': %v\n", filename, err)
		os.Exit(1)
	}

	counts := countWords(string(text))
	top := topN(counts, n)

	fmt.Printf("Top %d words in '%s':\n\n", n, filename)

	for i, wc := range top {
		fmt.Printf("%3d. %-20s %d\n", i+1, wc.Word, wc.Count)
	}
}
```

**Go Characteristics:**
- Explicit `if err != nil` error checking
- No generics needed for this example (but limited when needed)
- Simple, straightforward imperative code
- Must convert `[]byte` to `string` explicitly
- No algebraic data types (would use interface{} for sum types)
- Error handling verbosity

---

## Example 2: Concurrent HTTP Fetcher

A program that fetches multiple URLs concurrently and reports results. Demonstrates:
- Concurrency model
- Error handling in async contexts
- Channel/communication patterns

### Blood Implementation

```blood
// concurrent_fetch.blood - Concurrent HTTP fetcher with effects

use std::net::http::{Client, Response};
use std::time::Duration;

effect Async {
    fn spawn<T>(task: fn() -> T) -> Future<T>;
    fn await<T>(future: Future<T>) -> T;
}

effect Http {
    fn get(url: String) -> Response;
}

effect Log {
    fn info(msg: String) -> ();
    fn error(msg: String) -> ();
}

struct FetchResult {
    url: String,
    status: u16,
    size: usize,
    elapsed_ms: u64,
}

fn fetch_url(url: String) -> FetchResult with Http, Log {
    let start = std::time::Instant::now();

    let response = do Http.get(url.clone());
    let elapsed = start.elapsed().as_millis() as u64;

    do Log.info(format!("Fetched {} in {}ms", url, elapsed));

    FetchResult {
        url,
        status: response.status(),
        size: response.body().len(),
        elapsed_ms: elapsed,
    }
}

fn fetch_all(urls: Vec<String>) -> Vec<FetchResult> with Async, Http, Log {
    // Spawn all fetches concurrently
    let futures: Vec<Future<FetchResult>> = urls
        .into_iter()
        .map(|url| do Async.spawn(|| fetch_url(url)))
        .collect();

    // Await all results
    futures
        .into_iter()
        .map(|f| do Async.await(f))
        .collect()
}

fn main() with Async, Http, Log {
    let urls = vec![
        "https://example.com".to_string(),
        "https://httpbin.org/get".to_string(),
        "https://api.github.com".to_string(),
    ];

    do Log.info(format!("Fetching {} URLs concurrently...", urls.len()));

    let results = fetch_all(urls);

    do Log.info("Results:".to_string());
    for result in results {
        do Log.info(format!(
            "  {} - {} ({} bytes in {}ms)",
            result.url, result.status, result.size, result.elapsed_ms
        ));
    }
}

// Handlers compose naturally
handler AsyncRuntime: Async {
    fn spawn<T>(task: fn() -> T) -> Future<T> {
        // Implementation uses fibers/green threads
        resume(runtime::spawn(task))
    }

    fn await<T>(future: Future<T>) -> T {
        resume(runtime::block_on(future))
    }
}

handler HttpClient: Http {
    fn get(url: String) -> Response {
        match Client::new().get(&url).send() {
            Ok(resp) => resume(resp),
            Err(e) => do Log.error(format!("HTTP error for {}: {}", url, e)),
        }
    }
}

handler ConsoleLog: Log {
    fn info(msg: String) -> () {
        println!("[INFO] {}", msg);
        resume(())
    }

    fn error(msg: String) -> () {
        eprintln!("[ERROR] {}", msg);
        resume(())
    }
}
```

**Blood's Async Approach:**
- Effects make concurrency explicit in type signatures
- Handlers can implement different concurrency strategies
- No colored functions (async/await syntax pollution)
- Composable: Log + Http + Async all work together
- Testable: swap handlers for deterministic testing

### Rust Implementation

```rust
// concurrent_fetch.rs - Concurrent HTTP fetcher with tokio

use std::time::Instant;
use tokio;
use reqwest;

struct FetchResult {
    url: String,
    status: u16,
    size: usize,
    elapsed_ms: u64,
}

async fn fetch_url(client: &reqwest::Client, url: &str) -> Result<FetchResult, reqwest::Error> {
    let start = Instant::now();

    let response = client.get(url).send().await?;
    let status = response.status().as_u16();
    let body = response.bytes().await?;
    let elapsed = start.elapsed().as_millis() as u64;

    println!("[INFO] Fetched {} in {}ms", url, elapsed);

    Ok(FetchResult {
        url: url.to_string(),
        status,
        size: body.len(),
        elapsed_ms: elapsed,
    })
}

async fn fetch_all(urls: Vec<&str>) -> Vec<Result<FetchResult, reqwest::Error>> {
    let client = reqwest::Client::new();

    // Spawn all fetches concurrently
    let futures: Vec<_> = urls
        .iter()
        .map(|url| {
            let client = client.clone();
            let url = url.to_string();
            tokio::spawn(async move {
                fetch_url(&client, &url).await
            })
        })
        .collect();

    // Await all results
    let mut results = Vec::new();
    for future in futures {
        match future.await {
            Ok(result) => results.push(result),
            Err(e) => eprintln!("[ERROR] Task panicked: {}", e),
        }
    }
    results
}

#[tokio::main]
async fn main() {
    let urls = vec![
        "https://example.com",
        "https://httpbin.org/get",
        "https://api.github.com",
    ];

    println!("[INFO] Fetching {} URLs concurrently...", urls.len());

    let results = fetch_all(urls).await;

    println!("[INFO] Results:");
    for result in results {
        match result {
            Ok(r) => println!(
                "  {} - {} ({} bytes in {}ms)",
                r.url, r.status, r.size, r.elapsed_ms
            ),
            Err(e) => eprintln!("  Error: {}", e),
        }
    }
}
```

**Rust's Async Approach:**
- `async`/`await` keywords required throughout
- `tokio::spawn` requires `'static` + `Send` bounds
- Must clone client for each task
- `Result` types nest with `JoinHandle` errors
- Error handling becomes verbose in async contexts
- "Function coloring" - async infects call stack

### Go Implementation

```go
// concurrent_fetch.go - Concurrent HTTP fetcher with goroutines

package main

import (
	"fmt"
	"io"
	"net/http"
	"sync"
	"time"
)

type FetchResult struct {
	URL       string
	Status    int
	Size      int
	ElapsedMs int64
	Err       error
}

func fetchURL(url string) FetchResult {
	start := time.Now()

	resp, err := http.Get(url)
	if err != nil {
		return FetchResult{URL: url, Err: err}
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	elapsed := time.Since(start).Milliseconds()

	if err != nil {
		return FetchResult{URL: url, Err: err}
	}

	fmt.Printf("[INFO] Fetched %s in %dms\n", url, elapsed)

	return FetchResult{
		URL:       url,
		Status:    resp.StatusCode,
		Size:      len(body),
		ElapsedMs: elapsed,
	}
}

func fetchAll(urls []string) []FetchResult {
	results := make([]FetchResult, len(urls))
	var wg sync.WaitGroup

	for i, url := range urls {
		wg.Add(1)
		go func(i int, url string) {
			defer wg.Done()
			results[i] = fetchURL(url)
		}(i, url)
	}

	wg.Wait()
	return results
}

func main() {
	urls := []string{
		"https://example.com",
		"https://httpbin.org/get",
		"https://api.github.com",
	}

	fmt.Printf("[INFO] Fetching %d URLs concurrently...\n", len(urls))

	results := fetchAll(urls)

	fmt.Println("[INFO] Results:")
	for _, r := range results {
		if r.Err != nil {
			fmt.Printf("  %s - Error: %v\n", r.URL, r.Err)
		} else {
			fmt.Printf("  %s - %d (%d bytes in %dms)\n",
				r.URL, r.Status, r.Size, r.ElapsedMs)
		}
	}
}
```

**Go's Concurrency Approach:**
- Goroutines are lightweight and easy to spawn
- `sync.WaitGroup` for coordination
- Must be careful with loop variable capture (fixed in Go 1.22)
- No async/await - all functions can be goroutines
- Error handling embedded in result struct
- Race conditions possible without careful synchronization

---

## Example 3: Generic Data Structure

A generic tree with map/fold operations. Demonstrates:
- Generic type parameters
- Algebraic data types
- Higher-order functions

### Blood Implementation

```blood
// tree.blood - Generic binary tree with effects

enum Tree<T> {
    Empty,
    Node {
        value: T,
        left: Box<Tree<T>>,
        right: Box<Tree<T>>,
    },
}

impl<T> Tree<T> {
    fn leaf(value: T) -> Tree<T> {
        Tree::Node {
            value,
            left: Box::new(Tree::Empty),
            right: Box::new(Tree::Empty),
        }
    }

    fn node(value: T, left: Tree<T>, right: Tree<T>) -> Tree<T> {
        Tree::Node {
            value,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn map<U>(self, f: fn(T) -> U) -> Tree<U> {
        match self {
            Tree::Empty => Tree::Empty,
            Tree::Node { value, left, right } => Tree::Node {
                value: f(value),
                left: Box::new(left.map(f)),
                right: Box::new(right.map(f)),
            },
        }
    }

    fn fold<A>(self, init: A, f: fn(A, T) -> A) -> A {
        match self {
            Tree::Empty => init,
            Tree::Node { value, left, right } => {
                let acc = left.fold(init, f);
                let acc = f(acc, value);
                right.fold(acc, f)
            }
        }
    }

    fn size(self) -> usize {
        self.fold(0, |acc, _| acc + 1)
    }

    fn sum(self) -> T where T: Add<Output = T> + Default {
        self.fold(T::default(), |acc, x| acc + x)
    }
}

// Traversal with effects
effect Visitor<T> {
    fn visit(value: T) -> ();
}

impl<T> Tree<T> {
    fn traverse(self) with Visitor<T> {
        match self {
            Tree::Empty => {},
            Tree::Node { value, left, right } => {
                left.traverse();
                do Visitor.visit(value);
                right.traverse();
            }
        }
    }
}

fn main() {
    let tree = Tree::node(
        5,
        Tree::node(3, Tree::leaf(1), Tree::leaf(4)),
        Tree::node(8, Tree::leaf(7), Tree::leaf(9)),
    );

    // Map: double all values
    let doubled = tree.clone().map(|x| x * 2);

    // Fold: sum all values
    let sum = tree.clone().sum();
    println!("Sum: {}", sum);  // 37

    // Traverse with printing handler
    try {
        tree.traverse()
    } with PrintVisitor;
}

handler PrintVisitor: Visitor<i32> {
    fn visit(value: i32) -> () {
        println!("Visited: {}", value);
        resume(())
    }
}
```

**Blood's Generics:**
- Clean enum syntax for ADTs
- Type parameters work naturally
- Effects can be parameterized
- `where` clauses for constraints
- No lifetime parameters needed

### Rust Implementation

```rust
// tree.rs - Generic binary tree

enum Tree<T> {
    Empty,
    Node {
        value: T,
        left: Box<Tree<T>>,
        right: Box<Tree<T>>,
    },
}

impl<T> Tree<T> {
    fn leaf(value: T) -> Tree<T> {
        Tree::Node {
            value,
            left: Box::new(Tree::Empty),
            right: Box::new(Tree::Empty),
        }
    }

    fn node(value: T, left: Tree<T>, right: Tree<T>) -> Tree<T> {
        Tree::Node {
            value,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn map<U, F>(self, f: F) -> Tree<U>
    where
        F: Fn(T) -> U + Clone,
    {
        match self {
            Tree::Empty => Tree::Empty,
            Tree::Node { value, left, right } => Tree::Node {
                value: f(value),
                left: Box::new(left.map(f.clone())),
                right: Box::new(right.map(f)),
            },
        }
    }

    fn fold<A, F>(self, init: A, f: F) -> A
    where
        F: Fn(A, T) -> A + Clone,
    {
        match self {
            Tree::Empty => init,
            Tree::Node { value, left, right } => {
                let acc = left.fold(init, f.clone());
                let acc = f(acc, value);
                right.fold(acc, f)
            }
        }
    }

    fn size(self) -> usize {
        self.fold(0, |acc, _| acc + 1)
    }
}

impl<T> Tree<T>
where
    T: std::ops::Add<Output = T> + Default,
{
    fn sum(self) -> T {
        self.fold(T::default(), |acc, x| acc + x)
    }
}

// Traversal requires trait for visitor pattern
trait Visitor<T> {
    fn visit(&mut self, value: &T);
}

impl<T> Tree<T> {
    fn traverse<V: Visitor<T>>(&self, visitor: &mut V) {
        match self {
            Tree::Empty => {},
            Tree::Node { value, left, right } => {
                left.traverse(visitor);
                visitor.visit(value);
                right.traverse(visitor);
            }
        }
    }
}

struct PrintVisitor;

impl Visitor<i32> for PrintVisitor {
    fn visit(&mut self, value: &i32) {
        println!("Visited: {}", value);
    }
}

fn main() {
    let tree = Tree::node(
        5,
        Tree::node(3, Tree::leaf(1), Tree::leaf(4)),
        Tree::node(8, Tree::leaf(7), Tree::leaf(9)),
    );

    // Must clone for multiple uses (or use references)
    let tree2 = tree.clone(); // Requires T: Clone
    let tree3 = tree.clone();

    // Map: double all values
    let _doubled = tree.map(|x| x * 2);

    // Fold: sum all values
    let sum = tree2.sum();
    println!("Sum: {}", sum);  // 37

    // Traverse with visitor
    let mut visitor = PrintVisitor;
    tree3.traverse(&mut visitor);
}

// Must derive Clone for the tree
impl<T: Clone> Clone for Tree<T> {
    fn clone(&self) -> Self {
        match self {
            Tree::Empty => Tree::Empty,
            Tree::Node { value, left, right } => Tree::Node {
                value: value.clone(),
                left: left.clone(),
                right: right.clone(),
            },
        }
    }
}
```

**Rust's Generics:**
- Must add `Clone` bounds for closures used recursively
- Trait bounds accumulate (`Clone + Fn + ...`)
- Visitor pattern requires mutable borrow
- Manual `Clone` implementation (or `#[derive(Clone)]`)
- Reference vs ownership decisions throughout
- More ceremony for same functionality

### Go Implementation

```go
// tree.go - Generic binary tree (Go 1.18+)

package main

import (
	"fmt"
	"golang.org/x/exp/constraints"
)

type Tree[T any] struct {
	Value T
	Left  *Tree[T]
	Right *Tree[T]
	empty bool
}

func Empty[T any]() *Tree[T] {
	return &Tree[T]{empty: true}
}

func Leaf[T any](value T) *Tree[T] {
	return &Tree[T]{
		Value: value,
		Left:  Empty[T](),
		Right: Empty[T](),
	}
}

func Node[T any](value T, left, right *Tree[T]) *Tree[T] {
	return &Tree[T]{
		Value: value,
		Left:  left,
		Right: right,
	}
}

func Map[T, U any](t *Tree[T], f func(T) U) *Tree[U] {
	if t.empty {
		return Empty[U]()
	}
	return &Tree[U]{
		Value: f(t.Value),
		Left:  Map(t.Left, f),
		Right: Map(t.Right, f),
	}
}

func Fold[T, A any](t *Tree[T], init A, f func(A, T) A) A {
	if t.empty {
		return init
	}
	acc := Fold(t.Left, init, f)
	acc = f(acc, t.Value)
	return Fold(t.Right, acc, f)
}

func Size[T any](t *Tree[T]) int {
	return Fold(t, 0, func(acc int, _ T) int { return acc + 1 })
}

// Sum requires numeric constraint
func Sum[T constraints.Integer | constraints.Float](t *Tree[T]) T {
	var zero T
	return Fold(t, zero, func(acc, x T) T { return acc + x })
}

// Visitor - must use interface
type Visitor[T any] interface {
	Visit(T)
}

func Traverse[T any](t *Tree[T], v Visitor[T]) {
	if t.empty {
		return
	}
	Traverse(t.Left, v)
	v.Visit(t.Value)
	Traverse(t.Right, v)
}

type PrintVisitor struct{}

func (PrintVisitor) Visit(value int) {
	fmt.Println("Visited:", value)
}

func main() {
	tree := Node(5,
		Node(3, Leaf(1), Leaf(4)),
		Node(8, Leaf(7), Leaf(9)),
	)

	// Map: double all values
	_ = Map(tree, func(x int) int { return x * 2 })

	// Sum
	sum := Sum(tree)
	fmt.Println("Sum:", sum)  // 37

	// Traverse
	Traverse(tree, PrintVisitor{})
}
```

**Go's Generics:**
- Generics added in Go 1.18, still maturing
- No algebraic data types - must simulate with pointers + bool
- Methods cannot have type parameters (Map must be function)
- Constraint syntax less intuitive (`constraints.Integer | constraints.Float`)
- No pattern matching on enum variants
- Empty tree represented as flag (no true sum types)

---

## Comparison Summary

### Error Handling

| Language | Approach | Verbosity | Composability |
|----------|----------|-----------|---------------|
| **Blood** | Effect system | Low | Excellent - handlers compose |
| **Rust** | Result + ? operator | Medium | Good - traits help |
| **Go** | if err != nil | High | Poor - repetitive |

### Concurrency

| Language | Model | Ergonomics | Safety |
|----------|-------|------------|--------|
| **Blood** | Effects + handlers | Excellent - no coloring | Excellent |
| **Rust** | async/await + tokio | Medium - colored functions | Excellent |
| **Go** | Goroutines + channels | Good - simple model | Medium |

### Generics

| Language | Power | Verbosity | Constraints |
|----------|-------|-----------|-------------|
| **Blood** | Full ADTs + effects | Low | Clean where clauses |
| **Rust** | Full ADTs | Medium | Trait bounds accumulate |
| **Go** | Basic (since 1.18) | Medium | Limited, no ADTs |

### Memory Management

| Language | Model | Mental Overhead | Performance |
|----------|-------|-----------------|-------------|
| **Blood** | Generational | Low | Excellent |
| **Rust** | Ownership + borrowing | High | Excellent |
| **Go** | Garbage collection | Low | Good |

### Learning Curve

| Aspect | Blood | Rust | Go |
|--------|-------|------|-----|
| Basic syntax | Easy | Easy | Easy |
| Error handling | Medium (effects) | Medium (Results) | Easy |
| Memory model | Medium (generations) | Hard (lifetimes) | Easy |
| Concurrency | Medium (effect handlers) | Hard (async) | Easy |
| Advanced features | Medium | Hard | Limited |

---

## When to Choose Each

### Choose Blood When:
- Effect handling is central to your domain
- You want Rust-level safety without lifetime complexity
- Testing with mock handlers is important
- Building compilers, interpreters, or effect-heavy systems

### Choose Rust When:
- Maximum performance is required
- Ecosystem maturity is critical
- Memory layout control is needed
- Working with existing Rust codebases

### Choose Go When:
- Fast development iteration matters most
- Team includes developers new to systems programming
- Network services with simple concurrency
- Operational tooling and DevOps applications

---

*This document aims to be honest about trade-offs. Each language has strengths suited to different problems.*
