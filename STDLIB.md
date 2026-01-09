# Blood Standard Library Specification

**Version**: 0.1.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

This document specifies Blood's standard library, including core types, traits, effects, and handlers.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Primitive Types](#2-primitive-types)
3. [Core Types](#3-core-types)
4. [Collections](#4-collections)
5. [Core Traits](#5-core-traits)
6. [Standard Effects](#6-standard-effects)
7. [Standard Handlers](#7-standard-handlers)
8. [Prelude](#8-prelude)

---

## 1. Overview

### 1.1 Library Organization

```
std/
├── prelude.blood      # Automatically imported
├── primitive/         # Primitive type methods
├── core/              # Option, Result, Box, etc.
├── collections/       # Vec, HashMap, etc.
├── traits/            # Core trait definitions
├── effects/           # Effect definitions
├── handlers/          # Standard handlers
├── io/                # IO operations
├── mem/               # Memory operations
├── sync/              # Synchronization primitives
├── iter/              # Iterator traits and adapters
└── ops/               # Operator traits
```

### 1.2 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — Box<T> pointer semantics
- [GRAMMAR.md](./GRAMMAR.md) — Type and effect syntax
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — Effect typing rules
- [CONCURRENCY.md](./CONCURRENCY.md) — Fiber and Channel effects
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — Standard error types

### 1.3 Design Principles

1. **Effect-aware**: All side-effectful operations declare their effects
2. **Zero-cost abstractions**: No overhead for unused features
3. **Minimal prelude**: Only essential items auto-imported
4. **Orthogonal design**: Features compose without special cases

---

## 2. Primitive Types

### 2.1 Integer Types

| Type | Size | Range | Notes |
|------|------|-------|-------|
| `i8` | 8 bits | -128 to 127 | Signed |
| `i16` | 16 bits | -32,768 to 32,767 | Signed |
| `i32` | 32 bits | -2³¹ to 2³¹-1 | Default integer |
| `i64` | 64 bits | -2⁶³ to 2⁶³-1 | Signed |
| `i128` | 128 bits | -2¹²⁷ to 2¹²⁷-1 | Signed |
| `u8` | 8 bits | 0 to 255 | Unsigned |
| `u16` | 16 bits | 0 to 65,535 | Unsigned |
| `u32` | 32 bits | 0 to 2³²-1 | Unsigned |
| `u64` | 64 bits | 0 to 2⁶⁴-1 | Unsigned |
| `u128` | 128 bits | 0 to 2¹²⁸-1 | Unsigned |
| `isize` | ptr size | Platform dependent | Signed pointer size |
| `usize` | ptr size | Platform dependent | Unsigned pointer size |

### 2.2 Floating Point Types

| Type | Size | Precision | Standard |
|------|------|-----------|----------|
| `f32` | 32 bits | ~7 decimal digits | IEEE 754 single |
| `f64` | 64 bits | ~15 decimal digits | IEEE 754 double |

### 2.3 Other Primitives

```blood
type bool = true | false
type char = <Unicode scalar value>  // 4 bytes
type unit = ()
type never = !  // The type with no values (diverges)
```

### 2.4 Primitive Methods

All primitives have methods via implicit trait implementations:

```blood
impl i32 {
    const MIN: i32 = -2147483648
    const MAX: i32 = 2147483647

    fn abs(self) -> i32 / pure
    fn pow(self, exp: u32) -> i32 / pure
    fn checked_add(self, other: i32) -> Option<i32> / pure
    fn saturating_add(self, other: i32) -> i32 / pure
    fn wrapping_add(self, other: i32) -> i32 / pure
    fn to_string(self) -> String / pure
}
```

---

## 3. Core Types

### 3.1 Option<T>

Represents an optional value.

```blood
enum Option<T> {
    None,
    Some(T),
}

impl<T> Option<T> {
    fn is_some(&self) -> bool / pure
    fn is_none(&self) -> bool / pure

    fn unwrap(self) -> T / {Panic}
    fn unwrap_or(self, default: T) -> T / pure
    fn unwrap_or_else<F: fn() -> T>(self, f: F) -> T / pure

    fn map<U, F: fn(T) -> U>(self, f: F) -> Option<U> / pure
    fn and_then<U, F: fn(T) -> Option<U>>(self, f: F) -> Option<U> / pure
    fn or(self, other: Option<T>) -> Option<T> / pure
    fn or_else<F: fn() -> Option<T>>(self, f: F) -> Option<T> / pure

    fn ok_or<E>(self, err: E) -> Result<T, E> / pure
    fn ok_or_else<E, F: fn() -> E>(self, f: F) -> Result<T, E> / pure

    fn filter<P: fn(&T) -> bool>(self, predicate: P) -> Option<T> / pure
    fn flatten(self) -> Option<T> where T: Option<U> / pure
}
```

### 3.2 Result<T, E>

Represents success or failure.

```blood
enum Result<T, E> {
    Ok(T),
    Err(E),
}

impl<T, E> Result<T, E> {
    fn is_ok(&self) -> bool / pure
    fn is_err(&self) -> bool / pure

    fn ok(self) -> Option<T> / pure
    fn err(self) -> Option<E> / pure

    fn unwrap(self) -> T / {Panic}
    fn unwrap_err(self) -> E / {Panic}
    fn unwrap_or(self, default: T) -> T / pure
    fn unwrap_or_else<F: fn(E) -> T>(self, f: F) -> T / pure

    fn map<U, F: fn(T) -> U>(self, f: F) -> Result<U, E> / pure
    fn map_err<F, O: fn(E) -> F>(self, f: O) -> Result<T, F> / pure
    fn and_then<U, F: fn(T) -> Result<U, E>>(self, f: F) -> Result<U, E> / pure

    // Convert to effect-based error handling
    fn into_effect(self) -> T / {Error<E>}
}
```

### 3.3 Box<T>

Heap-allocated value with generational safety.

```blood
struct Box<T> {
    /// Internal 128-bit generational pointer layout:
    /// - Bits 127-64: Memory address (64 bits)
    /// - Bits 63-32:  Generation counter (32 bits)
    /// - Bits 31-0:   Metadata (tier, flags, type fingerprint)
    ///
    /// See MEMORY_MODEL.md Section 2.1 for complete pointer layout.
    ptr: GenPtr<T>,
}

impl<T> Box<T> {
    /// Allocates on the heap (Tier 1: Region) and returns a boxed value.
    /// The generation counter is initialized to the current generation at that address.
    fn new(value: T) -> Box<T> / {Allocate}

    /// Consumes the box and returns the inner value.
    /// Does NOT increment generation (ownership transferred, not freed).
    fn into_inner(self) -> T / pure

    /// Leaks the box, returning a static reference.
    /// The memory will never be freed (promoted to Tier 2).
    fn leak(self) -> &'static mut T / pure
}

impl<T> Deref for Box<T> {
    type Target = T

    /// Dereferences the box, performing generation check.
    ///
    /// # Generation Safety
    /// Compares the generation stored in `self.ptr` against the current
    /// generation at the memory address. If they differ, raises `StaleReference`.
    ///
    /// ```blood
    /// // Pseudocode for generation check:
    /// // if self.ptr.generation != memory[self.ptr.addr].current_gen {
    /// //     perform StaleReference.stale_access(...)
    /// // }
    /// ```
    fn deref(&self) -> &T / {StaleReference}
}

impl<T> DerefMut for Box<T> {
    /// Mutable dereference with generation check.
    /// Same safety guarantees as `deref`, plus exclusive access verification.
    fn deref_mut(&mut self) -> &mut T / {StaleReference}
}

impl<T> Drop for Box<T> {
    /// Drops the box, incrementing the generation counter at the memory address.
    ///
    /// After this call, any existing references to this memory will have
    /// stale generation counters and will raise `StaleReference` on dereference.
    ///
    /// # Generation Lifecycle
    /// 1. Memory allocated with generation N
    /// 2. Box created pointing to address with generation N
    /// 3. Drop increments generation to N+1
    /// 4. Old Box's generation (N) now mismatches (N+1) → stale
    fn drop(&mut self) / pure
}
```

### 3.4 String

UTF-8 encoded string.

```blood
struct String {
    buf: Vec<u8>,
}

impl String {
    fn new() -> String / pure
    fn with_capacity(cap: usize) -> String / {Allocate}
    fn from_utf8(bytes: Vec<u8>) -> Result<String, Utf8Error> / pure
    fn from_utf8_lossy(bytes: &[u8]) -> String / {Allocate}

    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure
    fn capacity(&self) -> usize / pure

    fn push(&mut self, c: char) / {Allocate}
    fn push_str(&mut self, s: &str) / {Allocate}
    fn pop(&mut self) -> Option<char> / pure

    fn as_str(&self) -> &str / pure
    fn as_bytes(&self) -> &[u8] / pure
    fn into_bytes(self) -> Vec<u8> / pure

    fn concat(&self, other: &str) -> String / {Allocate}
    fn chars(&self) -> impl Iterator<Item = char> / pure
    fn lines(&self) -> impl Iterator<Item = &str> / pure

    fn trim(&self) -> &str / pure
    fn to_lowercase(&self) -> String / {Allocate}
    fn to_uppercase(&self) -> String / {Allocate}
    fn contains(&self, pattern: &str) -> bool / pure
    fn replace(&self, from: &str, to: &str) -> String / {Allocate}
    fn split(&self, pattern: &str) -> impl Iterator<Item = &str> / pure
}

// String slice
type str = [u8]  // UTF-8 validated slice
```

### 3.5 Frozen<T>

Deeply immutable, safely shareable across fibers.

```blood
struct Frozen<T> {
    // Internal: deep-frozen value with FROZEN flag
}

impl<T: Freeze> Frozen<T> {
    fn new(value: T) -> Frozen<T> / {Allocate}
}

impl<T> Deref for Frozen<T> {
    type Target = T
    fn deref(&self) -> &T / pure
}

// Cannot DerefMut - frozen values are immutable
```

---

## 4. Collections

### 4.1 Vec<T>

Growable array.

```blood
struct Vec<T> {
    buf: RawVec<T>,
    len: usize,
}

impl<T> Vec<T> {
    fn new() -> Vec<T> / pure
    fn with_capacity(cap: usize) -> Vec<T> / {Allocate}

    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure
    fn capacity(&self) -> usize / pure

    fn push(&mut self, value: T) / {Allocate}
    fn pop(&mut self) -> Option<T> / pure
    fn insert(&mut self, index: usize, value: T) / {Allocate, Panic}
    fn remove(&mut self, index: usize) -> T / {Panic}
    fn swap_remove(&mut self, index: usize) -> T / {Panic}

    fn clear(&mut self) / pure
    fn truncate(&mut self, len: usize) / pure
    fn reserve(&mut self, additional: usize) / {Allocate}
    fn shrink_to_fit(&mut self) / {Allocate}

    fn get(&self, index: usize) -> Option<&T> / pure
    fn get_mut(&mut self, index: usize) -> Option<&mut T> / pure
    fn first(&self) -> Option<&T> / pure
    fn last(&self) -> Option<&T> / pure

    fn iter(&self) -> impl Iterator<Item = &T> / pure
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> / pure
    fn into_iter(self) -> impl Iterator<Item = T> / pure

    fn as_slice(&self) -> &[T] / pure
    fn as_mut_slice(&mut self) -> &mut [T] / pure

    fn sort(&mut self) where T: Ord / pure
    fn sort_by<F: fn(&T, &T) -> Ordering>(&mut self, f: F) / pure
    fn binary_search(&self, x: &T) -> Result<usize, usize> where T: Ord / pure

    fn retain<F: fn(&T) -> bool>(&mut self, f: F) / pure
    fn dedup(&mut self) where T: Eq / pure
    fn reverse(&mut self) / pure
}
```

### 4.2 HashMap<K, V>

Hash map with generational safety.

```blood
struct HashMap<K, V, S = DefaultHasher> {
    // Implementation details
}

impl<K: Hash + Eq, V> HashMap<K, V> {
    fn new() -> HashMap<K, V> / pure
    fn with_capacity(cap: usize) -> HashMap<K, V> / {Allocate}

    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure
    fn capacity(&self) -> usize / pure

    fn insert(&mut self, k: K, v: V) -> Option<V> / {Allocate}
    fn remove(&mut self, k: &K) -> Option<V> / pure
    fn get(&self, k: &K) -> Option<&V> / pure
    fn get_mut(&mut self, k: &K) -> Option<&mut V> / pure
    fn contains_key(&self, k: &K) -> bool / pure

    fn entry(&mut self, k: K) -> Entry<K, V> / pure

    fn keys(&self) -> impl Iterator<Item = &K> / pure
    fn values(&self) -> impl Iterator<Item = &V> / pure
    fn iter(&self) -> impl Iterator<Item = (&K, &V)> / pure
    fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> / pure

    fn clear(&mut self) / pure
    fn reserve(&mut self, additional: usize) / {Allocate}
}
```

### 4.3 HashSet<T>

Hash set.

```blood
struct HashSet<T, S = DefaultHasher> {
    map: HashMap<T, (), S>,
}

impl<T: Hash + Eq> HashSet<T> {
    fn new() -> HashSet<T> / pure
    fn with_capacity(cap: usize) -> HashSet<T> / {Allocate}

    fn insert(&mut self, value: T) -> bool / {Allocate}
    fn remove(&mut self, value: &T) -> bool / pure
    fn contains(&self, value: &T) -> bool / pure

    fn union(&self, other: &HashSet<T>) -> HashSet<T> / {Allocate}
    fn intersection(&self, other: &HashSet<T>) -> HashSet<T> / {Allocate}
    fn difference(&self, other: &HashSet<T>) -> HashSet<T> / {Allocate}

    fn is_subset(&self, other: &HashSet<T>) -> bool / pure
    fn is_superset(&self, other: &HashSet<T>) -> bool / pure
    fn is_disjoint(&self, other: &HashSet<T>) -> bool / pure
}
```

### 4.4 VecDeque<T>

Double-ended queue.

```blood
struct VecDeque<T> {
    // Ring buffer implementation
}

impl<T> VecDeque<T> {
    fn new() -> VecDeque<T> / pure
    fn with_capacity(cap: usize) -> VecDeque<T> / {Allocate}

    fn push_front(&mut self, value: T) / {Allocate}
    fn push_back(&mut self, value: T) / {Allocate}
    fn pop_front(&mut self) -> Option<T> / pure
    fn pop_back(&mut self) -> Option<T> / pure

    fn front(&self) -> Option<&T> / pure
    fn back(&self) -> Option<&T> / pure
}
```

---

## 5. Core Traits

### 5.1 Comparison Traits

```blood
trait Eq {
    fn eq(&self, other: &Self) -> bool / pure
    fn ne(&self, other: &Self) -> bool / pure { !self.eq(other) }
}

trait PartialEq<Rhs = Self> {
    fn eq(&self, other: &Rhs) -> bool / pure
    fn ne(&self, other: &Rhs) -> bool / pure { !self.eq(other) }
}

trait Ord: Eq {
    fn cmp(&self, other: &Self) -> Ordering / pure
    fn max(self, other: Self) -> Self / pure
    fn min(self, other: Self) -> Self / pure
    fn clamp(self, min: Self, max: Self) -> Self / pure
}

trait PartialOrd<Rhs = Self>: PartialEq<Rhs> {
    fn partial_cmp(&self, other: &Rhs) -> Option<Ordering> / pure
    fn lt(&self, other: &Rhs) -> bool / pure
    fn le(&self, other: &Rhs) -> bool / pure
    fn gt(&self, other: &Rhs) -> bool / pure
    fn ge(&self, other: &Rhs) -> bool / pure
}

enum Ordering {
    Less,
    Equal,
    Greater,
}
```

### 5.2 Conversion Traits

```blood
trait From<T> {
    fn from(value: T) -> Self / pure
}

trait Into<T> {
    fn into(self) -> T / pure
}

// Blanket implementation
impl<T, U> Into<U> for T where U: From<T> {
    fn into(self) -> U / pure { U::from(self) }
}

trait TryFrom<T> {
    type Error
    fn try_from(value: T) -> Result<Self, Self::Error> / pure
}

trait TryInto<T> {
    type Error
    fn try_into(self) -> Result<T, Self::Error> / pure
}

trait AsRef<T> {
    fn as_ref(&self) -> &T / pure
}

trait AsMut<T> {
    fn as_mut(&mut self) -> &mut T / pure
}
```

### 5.3 Operator Traits

```blood
// Arithmetic
trait Add<Rhs = Self> {
    type Output
    fn add(self, rhs: Rhs) -> Self::Output / pure
}

trait Sub<Rhs = Self> {
    type Output
    fn sub(self, rhs: Rhs) -> Self::Output / pure
}

trait Mul<Rhs = Self> {
    type Output
    fn mul(self, rhs: Rhs) -> Self::Output / pure
}

trait Div<Rhs = Self> {
    type Output
    fn div(self, rhs: Rhs) -> Self::Output / pure
}

trait Rem<Rhs = Self> {
    type Output
    fn rem(self, rhs: Rhs) -> Self::Output / pure
}

trait Neg {
    type Output
    fn neg(self) -> Self::Output / pure
}

// Index
trait Index<Idx> {
    type Output
    fn index(&self, index: Idx) -> &Self::Output / pure
}

trait IndexMut<Idx>: Index<Idx> {
    fn index_mut(&mut self, index: Idx) -> &mut Self::Output / pure
}

// Dereference
trait Deref {
    type Target
    fn deref(&self) -> &Self::Target / pure
}

trait DerefMut: Deref {
    fn deref_mut(&mut self) -> &mut Self::Target / pure
}
```

### 5.4 Iterator Traits

```blood
trait Iterator {
    type Item

    fn next(&mut self) -> Option<Self::Item> / pure

    // Provided methods (selection)
    fn map<B, F: fn(Self::Item) -> B>(self, f: F) -> Map<Self, F> / pure
    fn filter<P: fn(&Self::Item) -> bool>(self, p: P) -> Filter<Self, P> / pure
    fn fold<B, F: fn(B, Self::Item) -> B>(self, init: B, f: F) -> B / pure
    fn collect<B: FromIterator<Self::Item>>(self) -> B / pure
    fn count(self) -> usize / pure
    fn sum<S: Sum<Self::Item>>(self) -> S / pure
    fn product<P: Product<Self::Item>>(self) -> P / pure
    fn any<F: fn(Self::Item) -> bool>(&mut self, f: F) -> bool / pure
    fn all<F: fn(Self::Item) -> bool>(&mut self, f: F) -> bool / pure
    fn find<P: fn(&Self::Item) -> bool>(&mut self, p: P) -> Option<Self::Item> / pure
    fn position<P: fn(Self::Item) -> bool>(&mut self, p: P) -> Option<usize> / pure
    fn take(self, n: usize) -> Take<Self> / pure
    fn skip(self, n: usize) -> Skip<Self> / pure
    fn chain<U: Iterator<Item = Self::Item>>(self, other: U) -> Chain<Self, U> / pure
    fn zip<U: Iterator>(self, other: U) -> Zip<Self, U> / pure
    fn enumerate(self) -> Enumerate<Self> / pure
}

trait IntoIterator {
    type Item
    type IntoIter: Iterator<Item = Self::Item>

    fn into_iter(self) -> Self::IntoIter / pure
}

trait FromIterator<A> {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self / pure
}
```

### 5.5 Ownership Traits

```blood
trait Clone {
    fn clone(&self) -> Self / pure
}

trait Copy: Clone {
    // Marker trait - bitwise copy is sufficient
}

trait Drop {
    fn drop(&mut self) / pure
}

trait Default {
    fn default() -> Self / pure
}

// Marker traits for ownership modalities
trait Linear {
    // Must be used exactly once
}

trait Affine {
    // May be used at most once
}

trait Freeze {
    // Can be deeply frozen for cross-fiber sharing
}
```

### 5.6 Hash Trait

```blood
trait Hash {
    fn hash<H: Hasher>(&self, state: &mut H) / pure
}

trait Hasher {
    fn finish(&self) -> u64 / pure
    fn write(&mut self, bytes: &[u8]) / pure
}

struct DefaultHasher {
    // SipHash-1-3 by default
}
```

---

## 6. Standard Effects

### 6.0 Error Handling Philosophy: Panic vs Error

Blood distinguishes between two categories of failures:

| Category | Mechanism | Recoverable | When to Use |
|----------|-----------|-------------|-------------|
| **Errors** | `Error<E>` effect | Yes | Expected failures, external inputs, IO |
| **Panics** | `Panic` effect | No* | Bugs, invariant violations, impossible states |

*Panics can be caught at coarse boundaries but should not be used for control flow.

#### 6.0.1 When to Use Error<E>

Use `Error<E>` for **expected failures** that callers should handle:

```blood
// Good: IO operations can fail
fn read_config(path: &Path) -> Config / {IO, Error<ConfigError>} { ... }

// Good: Parsing untrusted input
fn parse_json(input: &str) -> Json / {Error<ParseError>} { ... }

// Good: Network operations
fn fetch_url(url: &Url) -> Response / {Async, Error<HttpError>} { ... }
```

#### 6.0.2 When to Use Panic

Use `Panic` for **bugs and invariant violations** that indicate programmer error:

```blood
// Good: Index out of bounds (programmer error)
fn get(slice: &[T], index: usize) -> &T / {Panic} {
    if index >= slice.len() {
        panic!("index {} out of bounds for slice of length {}", index, slice.len())
    }
    &slice[index]
}

// Good: Unreachable code
fn process(variant: MyEnum) -> i32 / {Panic} {
    match variant {
        MyEnum::A => 1,
        MyEnum::B => 2,
        _ => panic!("unreachable: all variants handled"),
    }
}
```

#### 6.0.3 The `#[no_panic]` Attribute

For safety-critical code, Blood provides compile-time panic prevention:

```blood
#[no_panic]
fn critical_path(data: &SensorData) -> ControlSignal / {Error<SensorError>} {
    // Compiler ERROR if any code path can panic
    // Must use Error<E> for all failure cases
    let value = data.get_checked(0)?  // Returns Error, not panic
    compute_signal(value)
}

// Functions called from #[no_panic] context must also be #[no_panic]
#[no_panic]
fn compute_signal(value: f64) -> ControlSignal / pure {
    // No operations that can panic allowed here
}
```

#### 6.0.4 Converting Between Panic and Error

```blood
// Panic → Error: Catch panics at boundaries
fn resilient_parse(input: &str) -> Result<Json, ParseError> / pure {
    with CatchPanic handle {
        // If parse panics, caught and converted to Err
        Ok(parse_json_unchecked(input))
    }
}

// Error → Panic: When errors indicate bugs
fn expect<T, E: Display>(result: Result<T, E>, msg: &str) -> T / {Panic} {
    match result {
        Ok(v) => v,
        Err(e) => panic!("{}: {}", msg, e),
    }
}
```

#### 6.0.5 Guidelines by Domain

| Domain | Preferred Approach |
|--------|-------------------|
| Safety-critical (avionics, medical) | `#[no_panic]` + `Error<E>` everywhere |
| Server applications | `Error<E>` with panic boundaries per request |
| CLI tools | `Panic` acceptable for startup, `Error<E>` for user input |
| Libraries | `Error<E>` for public API, `Panic` for internal bugs |

### 6.1 Error<E>

Recoverable errors.

```blood
effect Error<E> {
    op raise(err: E) -> never;
}

// Usage
fn parse_int(s: &str) -> i32 / {Error<ParseError>} {
    match s.parse() {
        Ok(n) => n,
        Err(e) => raise(ParseError::InvalidDigit(e)),
    }
}
```

### 6.2 State<S>

Mutable state threading.

```blood
effect State<S> {
    op get() -> S;
    op put(s: S) -> unit;
    op modify(f: fn(S) -> S) -> unit;
}

// Usage
fn increment() -> i32 / {State<i32>} {
    let current = get()
    put(current + 1)
    current
}
```

### 6.3 IO

Input/output operations. IO is a **compound effect** that subsumes several simpler effects.

#### 6.3.1 Effect Hierarchy and Subsumption

```blood
/// IO subsumes Error<IoError>, Log, and implicitly State for internal buffers
/// This is declared via effect extension:
effect IO extends Log {
    op read(fd: Fd, buf: &mut [u8]) -> Result<usize, IoError>;
    op write(fd: Fd, buf: &[u8]) -> Result<usize, IoError>;
    op open(path: &Path, opts: OpenOptions) -> Result<Fd, IoError>;
    op close(fd: Fd) -> Result<unit, IoError>;
    op flush(fd: Fd) -> Result<unit, IoError>;

    // Environment access
    op get_env(key: &str) -> Option<String>;
    op set_env(key: &str, value: &str) -> unit;

    // Time
    op now() -> Instant;
    op sleep(duration: Duration) -> unit;
}
```

#### 6.3.2 Subsumption Semantics

When an effect `E1 extends E2`, handlers for `E1` automatically handle `E2` operations:

```blood
// IO handler provides Log handling implicitly
deep handler RealIO for IO {
    // Log operations are forwarded to stderr by default
    op log(level, msg, fields) {
        let formatted = format_log(level, msg, fields)
        write(STDERR, formatted.as_bytes())
        resume(())
    }

    // IO-specific operations
    op read(fd, buf) { resume(@unsafe { syscall::read(fd, buf) }) }
    op write(fd, buf) { resume(@unsafe { syscall::write(fd, buf) }) }
    // ... other IO operations
}

// This works: Log operations handled by IO handler
fn example() / {IO} {
    info!("Starting operation")  // Log effect, handled by IO
    let data = read_file("input.txt")  // IO effect
    info!("Read {} bytes", data.len())  // Log effect again
}
```

#### 6.3.3 Effect Hierarchy Rules

The standard effect subsumption hierarchy:

```
pure                    (no effects)
  │
  ├── Error<E>          (can fail with E)
  ├── State<S>          (can read/write state S)
  ├── Log               (can emit log messages)
  ├── Random            (can generate random values)
  │
  └── IO ───────────────(subsumes Log; can do file/network/env/time)
        │
        └── Async       (subsumes IO; can spawn/await tasks)
```

Functions with narrower effects can be called from contexts with broader effects:

```blood
fn pure_compute(x: i32) -> i32 / pure { x * 2 }

fn io_context() / {IO} {
    let y = pure_compute(21)  // OK: pure ⊆ IO
    println(y.to_string())
}
```

#### 6.3.4 Convenience Functions

```blood
// Convenience functions
fn print(s: &str) -> unit / {IO} {
    write(STDOUT, s.as_bytes()).unwrap()
}

fn println(s: &str) -> unit / {IO} {
    print(s)
    print("\n")
}

fn read_line() -> String / {IO, Error<IoError>} {
    let mut buf = String::new()
    let mut byte = [0u8; 1]
    loop {
        match read(STDIN, &mut byte)? {
            0 => break,
            _ if byte[0] == b'\n' => break,
            _ => buf.push(byte[0] as char),
        }
    }
    buf
}

fn read_file(path: &Path) -> Bytes / {IO, Error<IoError>} {
    let fd = open(path, OpenOptions::read())?
    let mut buf = Vec::new()
    let mut chunk = [0u8; 4096]
    loop {
        match read(fd, &mut chunk)? {
            0 => break,
            n => buf.extend_from_slice(&chunk[..n]),
        }
    }
    close(fd)?
    buf
}

fn write_file(path: &Path, data: &[u8]) -> unit / {IO, Error<IoError>} {
    let fd = open(path, OpenOptions::write().create(true).truncate(true))?
    write(fd, data)?
    flush(fd)?
    close(fd)?
}
```

### 6.4 Async

Asynchronous operations.

```blood
effect Async {
    op await<T>(future: Future<T>) -> T;
    op spawn<T>(task: fn() -> T / Async) -> TaskHandle<T>;
    op yield_now() -> unit;
}

// Future type
struct Future<T> {
    // Internal state machine
}

impl<T> Future<T> {
    fn pending() -> Future<T> / pure;
    fn ready(value: T) -> Future<T> / pure;
    fn map<U, F: fn(T) -> U>(self, f: F) -> Future<U> / pure;
}

// Task handle
struct TaskHandle<T> {
    // Handle to spawned task
}

impl<T> TaskHandle<T> {
    fn join(self) -> T / {Async}
    fn abort(self) -> unit / pure
}
```

### 6.5 Yield<T>

Generator/coroutine support.

```blood
effect Yield<T> {
    op yield(value: T) -> unit;
}

// Generator type alias
type Generator<T, R> = fn() -> R / {Yield<T>}

// Usage
fn fibonacci() -> impl Iterator<Item = u64> / pure {
    gen {
        let (mut a, mut b) = (0, 1)
        loop {
            yield(a)
            (a, b) = (b, a + b)
        }
    }
}
```

### 6.6 NonDet

Non-determinism (for backtracking search).

```blood
effect NonDet {
    op choose<T>(options: Vec<T>) -> T;
    op fail() -> never;
}

// Usage: SAT solver, constraint satisfaction
fn solve_puzzle() -> Solution / {NonDet} {
    let x = choose(vec![1, 2, 3, 4, 5])
    let y = choose(vec![1, 2, 3, 4, 5])
    if x + y != 7 { fail() }
    (x, y)
}
```

### 6.7 Allocate

Memory allocation (for tracking).

```blood
effect Allocate {
    op alloc(layout: Layout) -> *mut u8;
    op dealloc(ptr: *mut u8, layout: Layout) -> unit;
    op realloc(ptr: *mut u8, old: Layout, new: Layout) -> *mut u8;
}

// This effect is typically handled by the runtime
// User code rarely handles it directly
```

### 6.8 Panic

Unrecoverable errors.

```blood
effect Panic {
    op panic(msg: &str) -> never;
}

// panic! macro expands to:
fn panic(msg: &str) -> ! / {Panic} {
    perform Panic.panic(msg)
}
```

### 6.9 Log

Structured logging.

```blood
effect Log {
    op log(level: LogLevel, msg: &str, fields: &[(&str, &dyn Display)]) -> unit;
}

enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// Convenience macros expand to Log.log calls
// trace!("message")
// debug!("message")
// info!("message")
// warn!("message")
// error!("message")
```

### 6.10 StaleReference

Blood's novel effect for handling generational reference invalidation.

```blood
effect StaleReference {
    /// Raised when a generational reference is dereferenced after
    /// the target object's generation has been incremented (freed/reused).
    op stale_access(
        addr: Address,           // Memory address accessed
        expected_gen: u32,       // Generation stored in reference
        actual_gen: u32,         // Current generation at address
        access_type: AccessType, // Read or Write
    ) -> never;
}

enum AccessType {
    Read,
    Write,
}

/// Metadata about a stale reference for debugging
struct StaleReferenceInfo {
    addr: Address,
    expected_gen: u32,
    actual_gen: u32,
    access_type: AccessType,
    /// Stack trace where the reference was created (debug builds)
    creation_trace: Option<StackTrace>,
    /// Stack trace where the object was freed (debug builds)
    free_trace: Option<StackTrace>,
}
```

#### 6.10.1 When StaleReference is Raised

This effect is raised by the runtime in these scenarios:

1. **Direct Dereference**: Accessing a generational reference whose generation doesn't match
2. **Effect Resume**: When an effect handler resumes a continuation containing stale references (via Generation Snapshot validation)
3. **Cross-Fiber Access**: When a Frozen<T> contains stale interior references

```blood
// Example: Direct dereference causing StaleReference
fn example() / {StaleReference} {
    let boxed = Box::new(42)
    let ptr = &*boxed      // Get reference, gen = N
    drop(boxed)            // Increments generation to N+1
    let _ = *ptr           // RAISES: StaleReference (expected N, actual N+1)
}
```

#### 6.10.2 Integration with Generation Snapshots

When an effect handler resumes a continuation, the Generation Snapshot is validated:

```blood
// Continuation capture stores snapshot of referenced generations
fn example() / {State<i32>, StaleReference} {
    let data = Box::new(vec![1, 2, 3])
    let slice = &data[..]  // Reference captured in snapshot

    modify(|s| {
        drop(data)  // Free the data while suspended
        s + 1
    })

    // When continuation resumes, snapshot validation detects
    // that 'slice' now points to freed memory
    // RAISES: StaleReference
    println!("{:?}", slice)
}
```

### 6.11 Resource<R>

Linear resource acquisition and release tracking.

```blood
effect Resource<R: Linear> {
    /// Acquire a linear resource
    op acquire() -> R;

    /// Release a linear resource (required exactly once)
    op release(resource: R) -> unit;
}

// Usage pattern with bracket semantics
fn with_file<T>(
    path: &Path,
    mode: FileMode,
    f: fn(File) -> T / E
) -> T / {E, Error<IoError>} {
    let file = acquire()  // Opens file
    let result = f(file)
    release(file)         // Closes file (required by linearity)
    result
}
```

### 6.12 Random

Randomness as an effect (for reproducibility and testing).

```blood
effect Random {
    /// Generate random bytes
    op random_bytes(buf: &mut [u8]) -> unit;

    /// Generate random integer in range [0, max)
    op random_range(max: u64) -> u64;

    /// Generate random float in [0.0, 1.0)
    op random_float() -> f64;
}

// Convenience functions
fn random<T: Uniform>() -> T / {Random} {
    T::sample_uniform()
}

fn shuffle<T>(slice: &mut [T]) / {Random} {
    // Fisher-Yates shuffle using Random effect
    for i in (1..slice.len()).rev() {
        let j = random_range((i + 1) as u64) as usize
        slice.swap(i, j)
    }
}
```

---

## 7. Standard Handlers

### 7.1 Error Handlers

```blood
// Propagate errors as Result
deep handler PropagateError<E> for Error<E> {
    return(x) { Ok(x) }
    op raise(err) { Err(err) }
}

// Recover with default value
deep handler RecoverWith<E, T> for Error<E> {
    let default: T

    return(x) { x }
    op raise(_) { default }
}

// Log and reraise
deep handler LogErrors<E: Display> for Error<E> {
    return(x) { x }
    op raise(err) {
        eprintln!("Error: {}", err)
        raise(err)
    }
}
```

### 7.2 State Handlers

```blood
// In-memory state
deep handler LocalState<S> for State<S> {
    let mut state: S

    return(x) { (x, state) }
    op get() { resume(state) }
    op put(s) { state = s; resume(()) }
    op modify(f) { state = f(state); resume(()) }
}

// Read-only state (ignores puts)
deep handler ReadOnlyState<S: Clone> for State<S> {
    let state: S

    return(x) { x }
    op get() { resume(state.clone()) }
    op put(_) { resume(()) }  // Ignored
    op modify(_) { resume(()) }  // Ignored
}
```

#### 7.2.1 Generation Snapshots in State Handlers

When a handler calls `resume(value)`, the runtime captures a **Generation Snapshot** of all generational references in the suspended continuation. This snapshot is validated when the continuation actually resumes.

```blood
deep handler StateWithSnapshot<S> for State<S> {
    let mut state: S

    return(x) { x }

    op get() {
        // When resume(state) is called:
        // 1. Runtime captures GenerationSnapshot of continuation's environment
        // 2. Snapshot records: [(addr₁, gen₁), (addr₂, gen₂), ...]
        // 3. On actual resume, each (addr, gen) is validated
        // 4. If any generation mismatches, StaleReference is raised
        resume(state)
    }

    op put(s) {
        state = s
        resume(())
    }
}

// Example showing snapshot validation
fn example() / {State<i32>, StaleReference} {
    let data = Box::new(vec![1, 2, 3])
    let slice = &data[..]  // Reference to heap data

    // When modify() suspends, snapshot captures slice's generation
    modify(|s| {
        drop(data)  // Increment generation while continuation suspended
        s + 1
    })

    // When continuation resumes after modify returns:
    // - Snapshot validation checks slice's generation
    // - Generation mismatch detected → StaleReference raised
    println!("{:?}", slice)  // Would access freed memory without snapshot!
}
```

See FORMAL_SEMANTICS.md Section 5 for the formal operational semantics of Generation Snapshots.

### 7.3 IO Handlers

```blood
// Real IO (production)
deep handler RealIO for IO {
    return(x) { x }
    op read(fd, buf) { resume(@unsafe { syscall::read(fd, buf) }) }
    op write(fd, buf) { resume(@unsafe { syscall::write(fd, buf) }) }
    op open(path, opts) { resume(@unsafe { syscall::open(path, opts) }) }
    op close(fd) { resume(@unsafe { syscall::close(fd) }) }
    op flush(fd) { resume(@unsafe { syscall::fsync(fd) }) }
}

// Mock IO (testing)
deep handler MockIO for IO {
    let filesystem: MockFilesystem

    return(x) { x }
    op read(fd, buf) {
        let data = filesystem.read(fd)
        buf[..data.len()].copy_from_slice(&data)
        resume(Ok(data.len()))
    }
    // ... other operations
}
```

### 7.4 Async Handlers

```blood
// Single-threaded executor
deep handler SingleThreadExecutor for Async {
    let mut ready_queue: VecDeque<Task>
    let mut waiting: HashMap<TaskId, (Future, Continuation)>

    return(x) { x }

    op await(future) {
        if future.is_ready() {
            resume(future.get())
        } else {
            let id = current_task_id()
            waiting.insert(id, (future, resume))
            run_next()
        }
    }

    op spawn(task) {
        let id = next_task_id()
        ready_queue.push_back(Task::new(id, task))
        resume(TaskHandle { id })
    }

    op yield_now() {
        ready_queue.push_back(current_task())
        run_next()
    }
}
```

### 7.5 Yield Handlers

```blood
// Collect all yielded values
deep handler Collect<T> for Yield<T> {
    let mut items: Vec<T> = vec![]

    return(_) { items }
    op yield(value) {
        items.push(value)
        resume(())
    }
}

// Lazy iterator (returns on first yield)
shallow handler LazyIter<T> for Yield<T> {
    return(_) { GeneratorState::Complete }
    op yield(value) {
        GeneratorState::Yielded(value, resume)
    }
}

enum GeneratorState<T, R> {
    Yielded(T, fn(unit) -> GeneratorState<T, R>),
    Complete(R),
}
```

### 7.6 NonDet Handlers

```blood
// Find first solution
deep handler FirstSolution for NonDet {
    return(x) { Some(x) }

    op choose(options) {
        for opt in options {
            match resume(opt) {
                Some(result) => return Some(result),
                None => continue,
            }
        }
        None
    }

    op fail() { None }
}

// Enumerate all solutions
deep handler AllSolutions for NonDet {
    return(x) { vec![x] }

    op choose(options) {
        options.flat_map(|opt| resume(opt)).collect()
    }

    op fail() { vec![] }
}

// Probabilistic sampling (for Monte Carlo, MCMC, etc.)
deep handler MonteCarloSampler for NonDet {
    return(x) { Some(x) }

    op choose(options) {
        if options.is_empty() {
            None
        } else {
            // Uniform random selection
            let idx = random_range(options.len() as u64) as usize
            resume(options[idx])
        }
    }

    op fail() { None }
}

// Weighted probabilistic sampling
deep handler WeightedSampler for NonDet {
    let weights: fn(&T) -> f64

    return(x) { Some(x) }

    op choose(options) {
        if options.is_empty() {
            None
        } else {
            let total: f64 = options.iter().map(weights).sum()
            let mut threshold = random_float() * total
            for opt in options {
                threshold -= weights(&opt)
                if threshold <= 0.0 {
                    return resume(opt)
                }
            }
            resume(options.last().unwrap())
        }
    }

    op fail() { None }
}

// Bounded backtracking (limit exploration depth)
deep handler BoundedSearch for NonDet {
    let max_depth: usize
    let mut current_depth: usize = 0

    return(x) { Some(x) }

    op choose(options) {
        if current_depth >= max_depth {
            return None  // Cut off search
        }
        current_depth += 1
        for opt in options {
            match resume(opt) {
                Some(result) => return Some(result),
                None => continue,
            }
        }
        current_depth -= 1
        None
    }

    op fail() { None }
}
```

### 7.7 StaleReference Handlers

```blood
// Panic on stale reference (default for safety-critical code)
deep handler PanicOnStale for StaleReference {
    return(x) { x }

    op stale_access(addr, expected, actual, access_type) {
        panic!("Stale reference detected: address {:x}, expected gen {}, actual gen {}, {:?}",
               addr, expected, actual, access_type)
    }
}

// Convert to Result (for recoverable scenarios)
deep handler StaleToResult<T> for StaleReference {
    return(x) { Ok(x) }

    op stale_access(addr, expected, actual, access_type) {
        Err(StaleReferenceInfo {
            addr,
            expected_gen: expected,
            actual_gen: actual,
            access_type,
            creation_trace: None,
            free_trace: None,
        })
    }
}

// Log and continue with default (for resilient systems)
deep handler LogStaleAndRecover<T: Default> for StaleReference {
    return(x) { x }

    op stale_access(addr, expected, actual, access_type) {
        error!("Stale reference: {:x} (gen {} vs {}), recovering with default",
               addr, expected, actual)
        // Note: This handler cannot resume because op returns never
        // Must provide alternative value through handler return type
        T::default()
    }
}
```

### 7.8 Resource Handlers

```blood
// Bracket pattern for file resources
deep handler FileResource for Resource<File> {
    let path: Path
    let mode: FileMode

    return(x) { x }

    op acquire() {
        match open_file(&path, mode) {
            Ok(file) => resume(file),
            Err(e) => raise(e),  // Propagate to Error handler
        }
    }

    op release(file) {
        // Close is called even if computation fails (bracket semantics)
        let _ = file.close()  // Ignore close errors
        resume(())
    }
}

// Generic bracket handler with cleanup guarantee
deep handler Bracket<R: Linear, E> for Resource<R> {
    let acquire_fn: fn() -> Result<R, E>
    let release_fn: fn(R) -> unit

    return(x) { x }

    op acquire() {
        match acquire_fn() {
            Ok(resource) => resume(resource),
            Err(e) => raise(e),
        }
    }

    op release(resource) {
        release_fn(resource)
        resume(())
    }
}

// Pool-based resource handler
deep handler ResourcePool<R: Linear> for Resource<R> {
    let pool: &mut Pool<R>

    return(x) { x }

    op acquire() {
        match pool.checkout() {
            Some(resource) => resume(resource),
            None => raise(PoolExhaustedError),
        }
    }

    op release(resource) {
        pool.checkin(resource)
        resume(())
    }
}
```

### 7.9 Random Handlers

```blood
// Cryptographically secure random (production)
deep handler SecureRandom for Random {
    return(x) { x }

    op random_bytes(buf) {
        @unsafe { getrandom(buf) }
        resume(())
    }

    op random_range(max) {
        let mut bytes = [0u8; 8]
        @unsafe { getrandom(&mut bytes) }
        resume(u64::from_le_bytes(bytes) % max)
    }

    op random_float() {
        let mut bytes = [0u8; 8]
        @unsafe { getrandom(&mut bytes) }
        resume((u64::from_le_bytes(bytes) as f64) / (u64::MAX as f64))
    }
}

// Seeded PRNG (for reproducibility and testing)
deep handler SeededRandom for Random {
    let mut rng: Xoshiro256PlusPlus

    return(x) { x }

    op random_bytes(buf) {
        rng.fill_bytes(buf)
        resume(())
    }

    op random_range(max) {
        resume(rng.next_u64() % max)
    }

    op random_float() {
        resume(rng.next_f64())
    }
}

// Deterministic "random" for testing
deep handler DeterministicRandom for Random {
    let sequence: Vec<u64>
    let mut index: usize = 0

    return(x) { x }

    op random_bytes(buf) {
        for b in buf.iter_mut() {
            *b = (sequence[index % sequence.len()] & 0xFF) as u8
            index += 1
        }
        resume(())
    }

    op random_range(max) {
        let val = sequence[index % sequence.len()] % max
        index += 1
        resume(val)
    }

    op random_float() {
        let val = (sequence[index % sequence.len()] as f64) / (u64::MAX as f64)
        index += 1
        resume(val)
    }
}
```

---

## 8. Prelude

### 8.1 Auto-Imported Items

The prelude is automatically imported into every module:

```blood
// std/prelude.blood
pub use std::primitive::*;
pub use std::core::{Option, Option::*, Result, Result::*, Box};
pub use std::string::{String, str};
pub use std::collections::Vec;
pub use std::traits::{
    Clone, Copy, Default, Drop,
    Eq, PartialEq, Ord, PartialOrd,
    Iterator, IntoIterator,
    From, Into, AsRef, AsMut,
    Deref, DerefMut,
    Add, Sub, Mul, Div, Neg,
    Index, IndexMut,
};
pub use std::effects::{Error, IO, Panic};
pub use std::handlers::{PropagateError};
```

### 8.2 Excluding Prelude

To exclude the prelude:

```blood
#![no_prelude]

// Must explicitly import everything needed
use std::core::Option;
```

---

## Appendix A: Effect Hierarchy

```
                    ┌─────────┐
                    │  pure   │
                    └────┬────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
    ┌────▼────┐    ┌────▼────┐    ┌────▼────┐
    │  Error  │    │  State  │    │   Log   │
    └────┬────┘    └────┬────┘    └────┬────┘
         │               │               │
         └───────────────┼───────────────┘
                         │
                    ┌────▼────┐
                    │   IO    │
                    └────┬────┘
                         │
                    ┌────▼────┐
                    │  Async  │
                    └─────────┘
```

Effects lower in the hierarchy subsume those above:
- `IO` implies `Error`, `State`, `Log` capabilities
- `Async` implies `IO` capabilities

---

*Last updated: 2026-01-09*
