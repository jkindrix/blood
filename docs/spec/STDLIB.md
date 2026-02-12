# Blood Standard Library Specification

**Version**: 0.3.0
**Status**: Specified
**Last Updated**: 2026-01-10

This document specifies Blood's standard library, including core types, traits, effects, and handlers.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Primitive Types](#2-primitive-types)
3. [Core Types](#3-core-types)
   - 3.1 [Option\<T\>](#31-optiont)
   - 3.2 [Result\<T, E\>](#32-resultt-e)
   - 3.3 [Box\<T\>](#33-boxt)
   - 3.4 [String](#34-string)
   - 3.5 [Frozen\<T\>](#35-frozent)
   - 3.6 [Range Types](#36-range-types)
4. [Collections](#4-collections)
   - 4.0 [Complexity Notation](#40-complexity-notation)
   - 4.1 [Vec\<T\>](#41-vect)
   - 4.2 [HashMap\<K, V\>](#42-hashmapk-v)
   - 4.3 [HashSet\<T\>](#43-hashsett)
   - 4.4 [VecDeque\<T\>](#44-vecdequt)
   - 4.5 [BTreeMap\<K, V\>](#45-btreemapk-v)
   - 4.6 [BTreeSet\<T\>](#46-btreesett)
   - 4.7 [BinaryHeap\<T\>](#47-binaryheapt)
   - 4.8 [Concurrent Collections](#48-concurrent-collections)
5. [Core Traits](#5-core-traits)
   - 5.1 [Comparison Traits](#51-comparison-traits)
   - 5.2 [Conversion Traits](#52-conversion-traits)
   - 5.3 [Operator Traits](#53-operator-traits)
   - 5.4 [Iterator Traits](#54-iterator-traits)
   - 5.5 [Ownership Traits](#55-ownership-traits)
   - 5.6 [Hash Trait](#56-hash-trait)
   - 5.7 [Display and Debug Traits](#57-display-and-debug-traits)
   - 5.8 [Numeric Traits](#58-numeric-traits)
   - 5.9 [Thread Safety Markers](#59-thread-safety-markers)
6. [Iterator Adapters](#6-iterator-adapters)
7. [IO Types](#7-io-types)
8. [Time Types](#8-time-types)
9. [Format Macros](#9-format-macros)
10. [Standard Effects](#10-standard-effects)
11. [Standard Handlers](#11-standard-handlers)
12. [Prelude](#12-prelude)
13. [Implementation Notes](#13-implementation-notes)

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

### 3.6 Range Types

Blood provides a family of range types for expressing intervals, used extensively with iterators and slicing operations.

```blood
/// A range from `start` to `end` (exclusive): `start..end`
struct Range<Idx> {
    start: Idx,
    end: Idx,
}

/// A range from `start` to `end` (inclusive): `start..=end`
struct RangeInclusive<Idx> {
    start: Idx,
    end: Idx,
    exhausted: bool,  // Tracks if iterator has completed
}

/// A range from `start` with no upper bound: `start..`
struct RangeFrom<Idx> {
    start: Idx,
}

/// A range up to `end` (exclusive): `..end`
struct RangeTo<Idx> {
    end: Idx,
}

/// A range up to `end` (inclusive): `..=end`
struct RangeToInclusive<Idx> {
    end: Idx,
}

/// The full range, unbounded: `..`
struct RangeFull;
```

#### 3.6.1 Range Trait

```blood
/// Trait for types that can be used as range bounds.
trait RangeBounds<T> {
    fn start_bound(&self) -> Bound<&T> / pure
    fn end_bound(&self) -> Bound<&T> / pure

    fn contains<U>(&self, item: &U) -> bool / pure
    where
        T: PartialOrd<U>,
        U: PartialOrd<T>
}

enum Bound<T> {
    Included(T),
    Excluded(T),
    Unbounded,
}
```

#### 3.6.2 Range Implementations

```blood
impl<Idx: PartialOrd> Range<Idx> {
    fn new(start: Idx, end: Idx) -> Range<Idx> / pure
    fn is_empty(&self) -> bool / pure { self.start >= self.end }
    fn contains<U>(&self, item: &U) -> bool / pure
    where
        Idx: PartialOrd<U>,
        U: PartialOrd<Idx>
    {
        self.start <= *item && *item < self.end
    }
}

impl<Idx: Clone + PartialOrd + Step> Iterator for Range<Idx> {
    type Item = Idx

    fn next(&mut self) -> Option<Idx> / pure {
        if self.start < self.end {
            let n = self.start.clone()
            self.start = Step::forward(self.start.clone(), 1)
            Some(n)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) / pure {
        let len = Step::steps_between(&self.start, &self.end).unwrap_or(usize::MAX)
        (len, Some(len))
    }
}

impl<Idx: Clone + PartialOrd + Step> ExactSizeIterator for Range<Idx> {}
impl<Idx: Clone + PartialOrd + Step> DoubleEndedIterator for Range<Idx> {
    fn next_back(&mut self) -> Option<Idx> / pure {
        if self.start < self.end {
            self.end = Step::backward(self.end.clone(), 1)
            Some(self.end.clone())
        } else {
            None
        }
    }
}

impl<Idx: Clone + PartialOrd + Step> RangeInclusive<Idx> {
    fn new(start: Idx, end: Idx) -> RangeInclusive<Idx> / pure
    fn start(&self) -> &Idx / pure { &self.start }
    fn end(&self) -> &Idx / pure { &self.end }
    fn is_empty(&self) -> bool / pure { self.exhausted || self.start > self.end }
}

impl<Idx> RangeBounds<Idx> for Range<Idx> {
    fn start_bound(&self) -> Bound<&Idx> / pure { Bound::Included(&self.start) }
    fn end_bound(&self) -> Bound<&Idx> / pure { Bound::Excluded(&self.end) }
}

impl<Idx> RangeBounds<Idx> for RangeInclusive<Idx> {
    fn start_bound(&self) -> Bound<&Idx> / pure { Bound::Included(&self.start) }
    fn end_bound(&self) -> Bound<&Idx> / pure { Bound::Included(&self.end) }
}

impl<Idx> RangeBounds<Idx> for RangeFrom<Idx> {
    fn start_bound(&self) -> Bound<&Idx> / pure { Bound::Included(&self.start) }
    fn end_bound(&self) -> Bound<&Idx> / pure { Bound::Unbounded }
}

impl<Idx> RangeBounds<Idx> for RangeTo<Idx> {
    fn start_bound(&self) -> Bound<&Idx> / pure { Bound::Unbounded }
    fn end_bound(&self) -> Bound<&Idx> / pure { Bound::Excluded(&self.end) }
}

impl<Idx> RangeBounds<Idx> for RangeFull {
    fn start_bound(&self) -> Bound<&Idx> / pure { Bound::Unbounded }
    fn end_bound(&self) -> Bound<&Idx> / pure { Bound::Unbounded }
}
```

#### 3.6.3 Step Trait

The `Step` trait enables iteration over ranges:

```blood
trait Step: Clone + PartialOrd {
    /// Returns the number of steps from `start` to `end`.
    fn steps_between(start: &Self, end: &Self) -> Option<usize> / pure

    /// Returns the value `count` steps forward from `start`.
    fn forward(start: Self, count: usize) -> Self / pure

    /// Returns the value `count` steps backward from `start`.
    fn backward(start: Self, count: usize) -> Self / pure

    /// Returns the value that would be obtained by taking one step forward.
    fn forward_checked(start: Self, count: usize) -> Option<Self> / pure

    /// Returns the value that would be obtained by taking one step backward.
    fn backward_checked(start: Self, count: usize) -> Option<Self> / pure
}

// Implemented for all integer types
impl Step for i32 {
    fn steps_between(start: &i32, end: &i32) -> Option<usize> / pure {
        if *end >= *start {
            Some((*end - *start) as usize)
        } else {
            None
        }
    }

    fn forward(start: i32, count: usize) -> i32 / pure {
        start + count as i32
    }

    fn backward(start: i32, count: usize) -> i32 / pure {
        start - count as i32
    }

    fn forward_checked(start: i32, count: usize) -> Option<i32> / pure {
        start.checked_add(count as i32)
    }

    fn backward_checked(start: i32, count: usize) -> Option<i32> / pure {
        start.checked_sub(count as i32)
    }
}

// Also implemented for: i8, i16, i64, i128, isize, u8, u16, u32, u64, u128, usize, char
```

#### 3.6.4 Range Syntax Sugar

```blood
// Range syntax maps to constructors:
1..10           // Range::new(1, 10)
1..=10          // RangeInclusive::new(1, 10)
1..             // RangeFrom { start: 1 }
..10            // RangeTo { end: 10 }
..=10           // RangeToInclusive { end: 10 }
..              // RangeFull

// Common patterns:
for i in 0..n { ... }           // Iterate [0, n)
for i in 0..=n { ... }          // Iterate [0, n]
let slice = &arr[1..4]          // Slice elements 1, 2, 3
let tail = &arr[2..]            // Slice from index 2 to end
let head = &arr[..3]            // Slice first 3 elements
let copy = &arr[..]             // Slice entire array
```

---

## 4. Collections

### 4.0 Complexity Notation

All complexity bounds use standard big-O notation:
- `n` = number of elements in the collection
- `m` = number of elements in another collection (for set operations)
- Amortized bounds marked with †

### 4.1 Vec<T>

Growable contiguous array, similar to Rust's `Vec` or C++'s `std::vector`.

**Memory Layout**: Contiguous heap allocation with `(pointer, capacity, length)` header.

**Growth Strategy**: Geometric growth (2x) for amortized O(1) push.

```blood
struct Vec<T> {
    buf: RawVec<T>,   // Pointer + capacity (Tier 1 or 2 depending on escape)
    len: usize,        // Current element count
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

**Complexity Bounds**:

| Operation | Time | Space | Notes |
|-----------|------|-------|-------|
| `new()` | O(1) | O(1) | No allocation |
| `with_capacity(n)` | O(1) | O(n) | Single allocation |
| `push(x)` | O(1)† | O(1)† | Amortized, may reallocate |
| `pop()` | O(1) | O(1) | |
| `insert(i, x)` | O(n) | O(1)† | Shifts elements |
| `remove(i)` | O(n) | O(1) | Shifts elements |
| `swap_remove(i)` | O(1) | O(1) | Swaps with last |
| `get(i)` | O(1) | O(1) | Bounds check + index |
| `sort()` | O(n log n) | O(log n) | Introsort variant |
| `binary_search(x)` | O(log n) | O(1) | Requires sorted |
| `retain(f)` | O(n) | O(1) | In-place filter |
| `reverse()` | O(n) | O(1) | In-place |
| `clear()` | O(n) | O(1) | Drops elements |
| `iter()` | O(1) | O(1) | Creates iterator |

### 4.2 HashMap<K, V>

Hash map with generational safety, using SwissTable-style layout (similar to Rust's `hashbrown`).

**Memory Layout**: Flat array of control bytes + key-value pairs for cache efficiency.

**Hash Algorithm**: SipHash-1-3 default (DoS-resistant), configurable via `S` parameter.

```blood
struct HashMap<K, V, S = DefaultHasher> {
    ctrl: *mut u8,       // Control bytes (EMPTY/DELETED/hash bits)
    data: *mut (K, V),   // Key-value pairs
    len: usize,
    capacity: usize,
    hasher: S,
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

**Complexity Bounds** (expected case with good hash function):

| Operation | Time (expected) | Time (worst) | Notes |
|-----------|-----------------|--------------|-------|
| `new()` | O(1) | O(1) | No allocation |
| `with_capacity(n)` | O(n) | O(n) | Allocates control + data |
| `insert(k, v)` | O(1)† | O(n) | Amortized, may resize |
| `remove(k)` | O(1) | O(n) | Tombstone deletion |
| `get(k)` | O(1) | O(n) | Probe sequence |
| `contains_key(k)` | O(1) | O(n) | Probe sequence |
| `entry(k)` | O(1) | O(n) | Probe + possible insert |
| `clear()` | O(n) | O(n) | Drops all entries |
| `iter()` | O(1) | O(1) | Creates iterator |
| `reserve(n)` | O(n) | O(n) | May resize |

**Load Factor**: 87.5% maximum (Swiss Table standard).

### 4.3 HashSet<T>

Hash set implemented as `HashMap<T, ()>`, inheriting SwissTable performance characteristics.

**Memory Layout**: Same as HashMap, with zero-sized unit value optimized away.

```blood
struct HashSet<T, S = DefaultHasher> {
    map: HashMap<T, (), S>,
}

impl<T: Hash + Eq> HashSet<T> {
    fn new() -> HashSet<T> / pure
    fn with_capacity(cap: usize) -> HashSet<T> / {Allocate}

    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure
    fn capacity(&self) -> usize / pure

    fn insert(&mut self, value: T) -> bool / {Allocate}
    fn remove(&mut self, value: &T) -> bool / pure
    fn contains(&self, value: &T) -> bool / pure
    fn take(&mut self, value: &T) -> Option<T> / pure

    fn union(&self, other: &HashSet<T>) -> HashSet<T> / {Allocate}
    fn intersection(&self, other: &HashSet<T>) -> HashSet<T> / {Allocate}
    fn difference(&self, other: &HashSet<T>) -> HashSet<T> / {Allocate}
    fn symmetric_difference(&self, other: &HashSet<T>) -> HashSet<T> / {Allocate}

    fn is_subset(&self, other: &HashSet<T>) -> bool / pure
    fn is_superset(&self, other: &HashSet<T>) -> bool / pure
    fn is_disjoint(&self, other: &HashSet<T>) -> bool / pure

    fn iter(&self) -> impl Iterator<Item = &T> / pure
    fn drain(&mut self) -> impl Iterator<Item = T> / pure
    fn clear(&mut self) / pure
    fn reserve(&mut self, additional: usize) / {Allocate}
}
```

**Complexity Bounds** (expected case with good hash function):

| Operation | Time (expected) | Time (worst) | Space | Notes |
|-----------|-----------------|--------------|-------|-------|
| `new()` | O(1) | O(1) | O(1) | No allocation |
| `with_capacity(n)` | O(n) | O(n) | O(n) | Allocates control bytes |
| `insert(v)` | O(1)† | O(n) | O(1)† | Amortized, may resize |
| `remove(v)` | O(1) | O(n) | O(1) | Tombstone deletion |
| `contains(v)` | O(1) | O(n) | O(1) | Probe sequence |
| `union(other)` | O(n + m) | O((n + m)²) | O(n + m) | Creates new set |
| `intersection(other)` | O(min(n, m)) | O(n × m) | O(min(n, m)) | Iterates smaller |
| `difference(other)` | O(n) | O(n × m) | O(n) | Checks each element |
| `is_subset(other)` | O(n) | O(n × m) | O(1) | Early exit on mismatch |
| `is_disjoint(other)` | O(min(n, m)) | O(n × m) | O(1) | Early exit on match |

Where `n` = `self.len()`, `m` = `other.len()`.

### 4.4 VecDeque<T>

Double-ended queue implemented as a growable ring buffer.

**Memory Layout**: Contiguous allocation with `(pointer, head, len, capacity)` header. Elements stored in circular fashion; `head` tracks the start position.

**Growth Strategy**: Same as Vec (2x geometric growth).

```blood
struct VecDeque<T> {
    buf: RawVec<T>,    // Pointer + capacity
    head: usize,        // Index of first element
    len: usize,         // Element count
}

impl<T> VecDeque<T> {
    fn new() -> VecDeque<T> / pure
    fn with_capacity(cap: usize) -> VecDeque<T> / {Allocate}

    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure
    fn capacity(&self) -> usize / pure

    fn push_front(&mut self, value: T) / {Allocate}
    fn push_back(&mut self, value: T) / {Allocate}
    fn pop_front(&mut self) -> Option<T> / pure
    fn pop_back(&mut self) -> Option<T> / pure

    fn front(&self) -> Option<&T> / pure
    fn front_mut(&mut self) -> Option<&mut T> / pure
    fn back(&self) -> Option<&T> / pure
    fn back_mut(&mut self) -> Option<&mut T> / pure

    fn get(&self, index: usize) -> Option<&T> / pure
    fn get_mut(&mut self, index: usize) -> Option<&mut T> / pure

    fn insert(&mut self, index: usize, value: T) / {Allocate, Panic}
    fn remove(&mut self, index: usize) -> Option<T> / pure
    fn swap_remove_front(&mut self, index: usize) -> Option<T> / pure
    fn swap_remove_back(&mut self, index: usize) -> Option<T> / pure

    fn make_contiguous(&mut self) -> &mut [T] / pure
    fn as_slices(&self) -> (&[T], &[T]) / pure
    fn as_mut_slices(&mut self) -> (&mut [T], &mut [T]) / pure

    fn rotate_left(&mut self, n: usize) / pure
    fn rotate_right(&mut self, n: usize) / pure

    fn iter(&self) -> impl Iterator<Item = &T> / pure
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> / pure
    fn drain(&mut self, range: Range<usize>) -> impl Iterator<Item = T> / pure

    fn clear(&mut self) / pure
    fn truncate(&mut self, len: usize) / pure
    fn reserve(&mut self, additional: usize) / {Allocate}
    fn shrink_to_fit(&mut self) / {Allocate}
}
```

**Complexity Bounds**:

| Operation | Time | Space | Notes |
|-----------|------|-------|-------|
| `new()` | O(1) | O(1) | No allocation |
| `with_capacity(n)` | O(1) | O(n) | Single allocation |
| `push_front(x)` | O(1)† | O(1)† | Amortized, may reallocate |
| `push_back(x)` | O(1)† | O(1)† | Amortized, may reallocate |
| `pop_front()` | O(1) | O(1) | Adjusts head pointer |
| `pop_back()` | O(1) | O(1) | Adjusts length |
| `get(i)` | O(1) | O(1) | Index + modular arithmetic |
| `insert(i, x)` | O(min(i, n-i)) | O(1)† | Shifts toward closer end |
| `remove(i)` | O(min(i, n-i)) | O(1) | Shifts toward closer end |
| `rotate_left(k)` | O(min(k, n-k)) | O(1) | In-place rotation |
| `make_contiguous()` | O(n) | O(1) | May rotate elements |
| `as_slices()` | O(1) | O(1) | Returns two views |

**Ring Buffer Advantage**: Unlike Vec, VecDeque provides O(1) push/pop at both ends, making it ideal for queues, work-stealing deques, and sliding window algorithms.

### 4.5 BTreeMap<K, V>

Ordered map implemented as a B-Tree.

**Memory Layout**: Tree of nodes, each containing up to `B-1` keys and `B` children (B = branching factor, typically 6-12).

```blood
struct BTreeMap<K, V> {
    root: Option<Box<Node<K, V>>>,
    len: usize,
}

impl<K: Ord, V> BTreeMap<K, V> {
    fn new() -> BTreeMap<K, V> / pure

    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure

    fn insert(&mut self, k: K, v: V) -> Option<V> / {Allocate}
    fn remove(&mut self, k: &K) -> Option<V> / pure
    fn get(&self, k: &K) -> Option<&V> / pure
    fn get_mut(&mut self, k: &K) -> Option<&mut V> / pure
    fn contains_key(&self, k: &K) -> bool / pure

    fn entry(&mut self, k: K) -> Entry<K, V> / pure

    fn first_key_value(&self) -> Option<(&K, &V)> / pure
    fn last_key_value(&self) -> Option<(&K, &V)> / pure
    fn pop_first(&mut self) -> Option<(K, V)> / pure
    fn pop_last(&mut self) -> Option<(K, V)> / pure

    fn range<R: RangeBounds<K>>(&self, range: R) -> impl Iterator<Item = (&K, &V)> / pure
    fn keys(&self) -> impl Iterator<Item = &K> / pure
    fn values(&self) -> impl Iterator<Item = &V> / pure
    fn iter(&self) -> impl Iterator<Item = (&K, &V)> / pure

    fn clear(&mut self) / pure
}
```

**Complexity Bounds**:

| Operation | Time | Space | Notes |
|-----------|------|-------|-------|
| `new()` | O(1) | O(1) | No allocation |
| `insert(k, v)` | O(log n) | O(log n)† | May split nodes |
| `remove(k)` | O(log n) | O(1) | May merge nodes |
| `get(k)` | O(log n) | O(1) | Binary search in nodes |
| `contains_key(k)` | O(log n) | O(1) | Binary search |
| `first_key_value()` | O(log n) | O(1) | Leftmost traversal |
| `last_key_value()` | O(log n) | O(1) | Rightmost traversal |
| `range(r)` | O(log n + k) | O(1) | k = items yielded |
| `iter()` | O(1) | O(log n) | Iterator creation + stack |

**B-Tree Advantage**: Better cache locality than binary search trees due to node packing; ordered iteration; no hash function required.

### 4.6 BTreeSet<T>

Ordered set implemented as `BTreeMap<T, ()>`.

```blood
struct BTreeSet<T> {
    map: BTreeMap<T, ()>,
}

impl<T: Ord> BTreeSet<T> {
    fn new() -> BTreeSet<T> / pure

    fn insert(&mut self, value: T) -> bool / {Allocate}
    fn remove(&mut self, value: &T) -> bool / pure
    fn contains(&self, value: &T) -> bool / pure

    fn first(&self) -> Option<&T> / pure
    fn last(&self) -> Option<&T> / pure
    fn pop_first(&mut self) -> Option<T> / pure
    fn pop_last(&mut self) -> Option<T> / pure

    fn range<R: RangeBounds<T>>(&self, range: R) -> impl Iterator<Item = &T> / pure
    fn iter(&self) -> impl Iterator<Item = &T> / pure

    // Set operations (all return ordered results)
    fn union(&self, other: &BTreeSet<T>) -> impl Iterator<Item = &T> / pure
    fn intersection(&self, other: &BTreeSet<T>) -> impl Iterator<Item = &T> / pure
    fn difference(&self, other: &BTreeSet<T>) -> impl Iterator<Item = &T> / pure
    fn symmetric_difference(&self, other: &BTreeSet<T>) -> impl Iterator<Item = &T> / pure
}
```

**Complexity Bounds**: Same as BTreeMap for single-element operations. Set operations return lazy iterators that merge sorted sequences in O(n + m) time.

### 4.7 BinaryHeap<T>

Priority queue implemented as a binary max-heap.

**Memory Layout**: Dense array representation where parent of node `i` is at `(i-1)/2`, left child at `2i+1`, right child at `2i+2`.

**Heap Property**: Every parent node is greater than or equal to its children (max-heap). For min-heap behavior, use `Reverse<T>` wrapper.

```blood
struct BinaryHeap<T> {
    data: Vec<T>,
}

impl<T: Ord> BinaryHeap<T> {
    fn new() -> BinaryHeap<T> / pure
    fn with_capacity(cap: usize) -> BinaryHeap<T> / {Allocate}

    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure
    fn capacity(&self) -> usize / pure

    /// Pushes an element onto the heap.
    fn push(&mut self, item: T) / {Allocate}

    /// Removes and returns the maximum element.
    fn pop(&mut self) -> Option<T> / pure

    /// Returns a reference to the maximum element.
    fn peek(&self) -> Option<&T> / pure

    /// Returns a mutable reference to the maximum element.
    /// Note: Caller must not modify the element in a way that violates heap property.
    fn peek_mut(&mut self) -> Option<PeekMut<T>> / pure

    /// Pushes an item and pops the largest (more efficient than separate push + pop).
    fn push_pop(&mut self, item: T) -> T / pure

    /// Pops the largest and pushes an item (more efficient than separate pop + push).
    fn replace(&mut self, item: T) -> Option<T> / pure

    /// Consumes the heap and returns a vector in sorted (ascending) order.
    fn into_sorted_vec(self) -> Vec<T> / pure

    /// Converts a vector into a heap (heapify).
    fn from_vec(vec: Vec<T>) -> BinaryHeap<T> / pure

    /// Moves all elements from `other` into `self`.
    fn append(&mut self, other: &mut BinaryHeap<T>) / {Allocate}

    fn clear(&mut self) / pure
    fn reserve(&mut self, additional: usize) / {Allocate}
    fn shrink_to_fit(&mut self) / {Allocate}

    /// Returns an iterator visiting all values in arbitrary order.
    fn iter(&self) -> impl Iterator<Item = &T> / pure

    /// Returns an iterator visiting all values in heap order (largest first).
    fn drain(&mut self) -> impl Iterator<Item = T> / pure

    /// Drops all items in arbitrary order.
    fn drain_sorted(&mut self) -> impl Iterator<Item = T> / pure

    /// Consumes the heap, returning the underlying vector in arbitrary order.
    fn into_vec(self) -> Vec<T> / pure

    /// Retains only the elements specified by the predicate.
    /// Note: Does not preserve heap ordering during iteration.
    fn retain<F: fn(&T) -> bool>(&mut self, f: F) / pure
}

/// Wrapper for mutable peek access.
struct PeekMut<'a, T: Ord> {
    heap: &'a mut BinaryHeap<T>,
    sift: bool,
}

impl<'a, T: Ord> PeekMut<'a, T> {
    /// Removes the peeked value from the heap.
    fn pop(self) -> T / pure
}

impl<'a, T: Ord> Deref for PeekMut<'a, T> {
    type Target = T
    fn deref(&self) -> &T / pure
}

impl<'a, T: Ord> DerefMut for PeekMut<'a, T> {
    fn deref_mut(&mut self) -> &mut T / pure
}

impl<'a, T: Ord> Drop for PeekMut<'a, T> {
    fn drop(&mut self) / pure {
        // Sift down if element was modified
        if self.sift {
            self.heap.sift_down(0)
        }
    }
}

/// Wrapper for creating min-heaps from max-heap implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Reverse<T>(T);

impl<T: Ord> Ord for Reverse<T> {
    fn cmp(&self, other: &Self) -> Ordering / pure {
        other.0.cmp(&self.0)  // Reversed comparison
    }
}

impl<T: PartialOrd> PartialOrd for Reverse<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> / pure {
        other.0.partial_cmp(&self.0)
    }
}
```

**Complexity Bounds**:

| Operation | Time | Space | Notes |
|-----------|------|-------|-------|
| `new()` | O(1) | O(1) | No allocation |
| `with_capacity(n)` | O(1) | O(n) | Single allocation |
| `push(x)` | O(log n)† | O(1)† | Amortized, sift up |
| `pop()` | O(log n) | O(1) | Sift down |
| `peek()` | O(1) | O(1) | Read root |
| `push_pop(x)` | O(log n) | O(1) | Combined operation |
| `replace(x)` | O(log n) | O(1) | Combined operation |
| `from_vec(v)` | O(n) | O(1) | Heapify in-place |
| `into_sorted_vec()` | O(n log n) | O(1) | Heap sort |
| `append(other)` | O(n + m) | O(1)† | Merge + heapify |
| `drain_sorted()` | O(n log n) | O(1) | Yields in order |
| `iter()` | O(1) | O(1) | Arbitrary order |

**Use Cases**:
- Priority queues (task scheduling, event systems)
- Dijkstra's shortest path algorithm
- K-largest/smallest elements (streaming)
- Huffman coding (with min-heap via `Reverse<T>`)
- Median maintenance (two heaps)

**Example Usage**:

```blood
// Max-heap (default)
let mut max_heap = BinaryHeap::new()
max_heap.push(3)
max_heap.push(1)
max_heap.push(4)
assert_eq!(max_heap.pop(), Some(4))
assert_eq!(max_heap.pop(), Some(3))

// Min-heap using Reverse wrapper
let mut min_heap: BinaryHeap<Reverse<i32>> = BinaryHeap::new()
min_heap.push(Reverse(3))
min_heap.push(Reverse(1))
min_heap.push(Reverse(4))
assert_eq!(min_heap.pop(), Some(Reverse(1)))
assert_eq!(min_heap.pop(), Some(Reverse(3)))

// Priority queue for tasks
struct Task {
    priority: u32,
    name: String,
}

impl Ord for Task {
    fn cmp(&self, other: &Self) -> Ordering / pure {
        self.priority.cmp(&other.priority)
    }
}

let mut task_queue = BinaryHeap::new()
task_queue.push(Task { priority: 1, name: "low".into() })
task_queue.push(Task { priority: 10, name: "high".into() })
// Highest priority task is processed first
```

### 4.8 Concurrent Collections

Blood provides thread-safe collections for use across fibers and effect handlers.

#### 4.8.1 Channel<T>

Multi-producer, multi-consumer bounded channel for inter-fiber communication.

```blood
/// Bounded MPMC channel.
struct Channel<T> {
    // Internal: lock-free ring buffer with generation counters
}

/// Sender handle for a channel.
struct Sender<T> {
    channel: Arc<Channel<T>>,
}

/// Receiver handle for a channel.
struct Receiver<T> {
    channel: Arc<Channel<T>>,
}

impl<T: Send> Channel<T> {
    /// Creates a bounded channel with the specified capacity.
    fn bounded(capacity: usize) -> (Sender<T>, Receiver<T>) / {Allocate}

    /// Creates an unbounded channel (use with caution - memory can grow).
    fn unbounded() -> (Sender<T>, Receiver<T>) / {Allocate}
}

impl<T: Send> Sender<T> {
    /// Sends a value, blocking if the channel is full.
    fn send(&self, value: T) -> Result<(), SendError<T>> / {Async}

    /// Attempts to send without blocking.
    fn try_send(&self, value: T) -> Result<(), TrySendError<T>> / pure

    /// Sends a value with a timeout.
    fn send_timeout(&self, value: T, timeout: Duration) -> Result<(), SendTimeoutError<T>> / {Async}

    /// Returns true if the channel is full.
    fn is_full(&self) -> bool / pure

    /// Returns the number of messages in the channel.
    fn len(&self) -> usize / pure

    /// Returns the channel's capacity.
    fn capacity(&self) -> Option<usize> / pure

    /// Closes the sender, preventing further sends.
    fn close(&self) / pure
}

impl<T: Send> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> / pure
}

impl<T: Send> Receiver<T> {
    /// Receives a value, blocking if the channel is empty.
    fn recv(&self) -> Result<T, RecvError> / {Async}

    /// Attempts to receive without blocking.
    fn try_recv(&self) -> Result<T, TryRecvError> / pure

    /// Receives a value with a timeout.
    fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> / {Async}

    /// Returns true if the channel is empty.
    fn is_empty(&self) -> bool / pure

    /// Returns the number of messages in the channel.
    fn len(&self) -> usize / pure

    /// Creates an iterator that receives messages until the channel is closed.
    fn iter(&self) -> impl Iterator<Item = T> / {Async}

    /// Closes the receiver, preventing further receives.
    fn close(&self) / pure
}

impl<T: Send> Clone for Receiver<T> {
    fn clone(&self) -> Receiver<T> / pure
}

/// Error types for channel operations.
enum SendError<T> {
    Disconnected(T),
}

enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

enum SendTimeoutError<T> {
    Timeout(T),
    Disconnected(T),
}

enum RecvError {
    Disconnected,
}

enum TryRecvError {
    Empty,
    Disconnected,
}

enum RecvTimeoutError {
    Timeout,
    Disconnected,
}
```

**Complexity Bounds** (for bounded channels):

| Operation | Time | Notes |
|-----------|------|-------|
| `bounded(n)` | O(n) | Allocates ring buffer |
| `send(v)` | O(1) | May block if full |
| `try_send(v)` | O(1) | Never blocks |
| `recv()` | O(1) | May block if empty |
| `try_recv()` | O(1) | Never blocks |
| `len()` | O(1) | Atomic read |
| `clone()` | O(1) | Reference count increment |

#### 4.8.2 OnceCell<T>

Thread-safe cell that can be written to exactly once.

```blood
struct OnceCell<T> {
    // Internal: atomic state + value storage
}

impl<T> OnceCell<T> {
    /// Creates an empty OnceCell.
    const fn new() -> OnceCell<T>

    /// Gets the value, or initializes it with `f` if empty.
    fn get_or_init<F: fn() -> T>(&self, f: F) -> &T / pure

    /// Gets the value, or initializes it with fallible `f` if empty.
    fn get_or_try_init<F: fn() -> Result<T, E>, E>(&self, f: F) -> Result<&T, E> / pure

    /// Gets the value if initialized.
    fn get(&self) -> Option<&T> / pure

    /// Gets the value mutably if initialized.
    fn get_mut(&mut self) -> Option<&mut T> / pure

    /// Sets the value if empty, returning Err(value) if already set.
    fn set(&self, value: T) -> Result<(), T> / pure

    /// Takes the value, leaving the cell empty.
    fn take(&mut self) -> Option<T> / pure

    /// Consumes the cell and returns the value if initialized.
    fn into_inner(self) -> Option<T> / pure
}

impl<T: Clone> Clone for OnceCell<T> {
    fn clone(&self) -> OnceCell<T> / pure
}
```

#### 4.8.3 RwLock<T>

Reader-writer lock allowing multiple readers or single writer.

```blood
struct RwLock<T> {
    // Internal: state machine + value
}

/// Read guard with shared access.
struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

/// Write guard with exclusive access.
struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> RwLock<T> {
    /// Creates a new RwLock.
    fn new(value: T) -> RwLock<T> / pure

    /// Acquires shared read access, blocking if a writer holds the lock.
    fn read(&self) -> RwLockReadGuard<T> / {Async}

    /// Attempts to acquire read access without blocking.
    fn try_read(&self) -> Option<RwLockReadGuard<T>> / pure

    /// Acquires exclusive write access, blocking until no readers or writers.
    fn write(&self) -> RwLockWriteGuard<T> / {Async}

    /// Attempts to acquire write access without blocking.
    fn try_write(&self) -> Option<RwLockWriteGuard<T>> / pure

    /// Returns a mutable reference to the underlying data.
    /// Requires exclusive ownership of the lock.
    fn get_mut(&mut self) -> &mut T / pure

    /// Consumes the lock and returns the inner value.
    fn into_inner(self) -> T / pure
}

impl<'a, T> Deref for RwLockReadGuard<'a, T> {
    type Target = T
    fn deref(&self) -> &T / pure
}

impl<'a, T> Deref for RwLockWriteGuard<'a, T> {
    type Target = T
    fn deref(&self) -> &T / pure
}

impl<'a, T> DerefMut for RwLockWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T / pure
}
```

#### 4.8.4 Mutex<T>

Mutual exclusion lock for exclusive access.

```blood
struct Mutex<T> {
    // Internal: atomic state + value
}

struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    /// Creates a new Mutex.
    fn new(value: T) -> Mutex<T> / pure

    /// Acquires the lock, blocking until available.
    fn lock(&self) -> MutexGuard<T> / {Async}

    /// Attempts to acquire the lock without blocking.
    fn try_lock(&self) -> Option<MutexGuard<T>> / pure

    /// Returns a mutable reference to the underlying data.
    fn get_mut(&mut self) -> &mut T / pure

    /// Consumes the mutex and returns the inner value.
    fn into_inner(self) -> T / pure
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T
    fn deref(&self) -> &T / pure
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T / pure
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) / pure {
        // Releases the lock
    }
}
```

#### 4.8.5 Atomic Types

Lock-free atomic operations for primitive types.

```blood
struct AtomicBool { /* internal */ }
struct AtomicI8 { /* internal */ }
struct AtomicI16 { /* internal */ }
struct AtomicI32 { /* internal */ }
struct AtomicI64 { /* internal */ }
struct AtomicIsize { /* internal */ }
struct AtomicU8 { /* internal */ }
struct AtomicU16 { /* internal */ }
struct AtomicU32 { /* internal */ }
struct AtomicU64 { /* internal */ }
struct AtomicUsize { /* internal */ }
struct AtomicPtr<T> { /* internal */ }

/// Memory ordering for atomic operations.
enum Ordering {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

impl AtomicI32 {
    const fn new(value: i32) -> AtomicI32
    fn load(&self, order: Ordering) -> i32 / pure
    fn store(&self, value: i32, order: Ordering) / pure
    fn swap(&self, value: i32, order: Ordering) -> i32 / pure
    fn compare_exchange(
        &self,
        current: i32,
        new: i32,
        success: Ordering,
        failure: Ordering
    ) -> Result<i32, i32> / pure
    fn compare_exchange_weak(
        &self,
        current: i32,
        new: i32,
        success: Ordering,
        failure: Ordering
    ) -> Result<i32, i32> / pure
    fn fetch_add(&self, value: i32, order: Ordering) -> i32 / pure
    fn fetch_sub(&self, value: i32, order: Ordering) -> i32 / pure
    fn fetch_and(&self, value: i32, order: Ordering) -> i32 / pure
    fn fetch_or(&self, value: i32, order: Ordering) -> i32 / pure
    fn fetch_xor(&self, value: i32, order: Ordering) -> i32 / pure
    fn fetch_max(&self, value: i32, order: Ordering) -> i32 / pure
    fn fetch_min(&self, value: i32, order: Ordering) -> i32 / pure
    fn get_mut(&mut self) -> &mut i32 / pure
    fn into_inner(self) -> i32 / pure
}

// Similar implementations for other atomic types
```

**Ordering Guidelines**:

| Ordering | Use When |
|----------|----------|
| `Relaxed` | No synchronization needed (just atomicity) |
| `Acquire` | Reading shared data (pairs with Release) |
| `Release` | Writing shared data (pairs with Acquire) |
| `AcqRel` | Both reading and writing (RMW operations) |
| `SeqCst` | Need global total ordering (rare, expensive) |

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

### 5.7 Display and Debug Traits

Traits for human-readable and programmer-oriented formatting.

```blood
/// Trait for user-facing display output.
trait Display {
    /// Formats the value for display.
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Trait for programmer-oriented debug output.
trait Debug {
    /// Formats the value for debugging.
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Write adapter for formatting operations.
struct Formatter<'a> {
    // Internal write buffer and formatting options
}

impl<'a> Formatter<'a> {
    /// Writes a string slice to the output.
    fn write_str(&mut self, s: &str) -> Result<(), FormatError> / pure

    /// Writes a char to the output.
    fn write_char(&mut self, c: char) -> Result<(), FormatError> / pure

    /// Writes formatted arguments to the output.
    fn write_fmt(&mut self, args: Arguments) -> Result<(), FormatError> / pure

    /// Returns the fill character.
    fn fill(&self) -> char / pure

    /// Returns the alignment setting.
    fn align(&self) -> Option<Alignment> / pure

    /// Returns the width setting.
    fn width(&self) -> Option<usize> / pure

    /// Returns the precision setting.
    fn precision(&self) -> Option<usize> / pure

    /// Returns whether the sign should always be displayed.
    fn sign_plus(&self) -> bool / pure

    /// Returns whether the sign should be displayed for negative only.
    fn sign_minus(&self) -> bool / pure

    /// Returns whether the alternate flag is set.
    fn alternate(&self) -> bool / pure

    /// Returns whether zero-padding is requested.
    fn sign_aware_zero_pad(&self) -> bool / pure

    /// Creates a debug struct builder.
    fn debug_struct(&mut self, name: &str) -> DebugStruct / pure

    /// Creates a debug tuple builder.
    fn debug_tuple(&mut self, name: &str) -> DebugTuple / pure

    /// Creates a debug list builder.
    fn debug_list(&mut self) -> DebugList / pure

    /// Creates a debug set builder.
    fn debug_set(&mut self) -> DebugSet / pure

    /// Creates a debug map builder.
    fn debug_map(&mut self) -> DebugMap / pure
}

/// Alignment options for formatting.
enum Alignment {
    Left,
    Right,
    Center,
}

/// Error type for formatting operations.
struct FormatError;
```

#### 5.7.1 Debug Helpers

Helper types for building complex debug output:

```blood
/// Helper for formatting structs in debug output.
struct DebugStruct<'a, 'b> {
    fmt: &'a mut Formatter<'b>,
}

impl<'a, 'b> DebugStruct<'a, 'b> {
    /// Adds a field to the struct output.
    fn field(&mut self, name: &str, value: &dyn Debug) -> &mut Self / pure

    /// Marks that there are additional unshown fields.
    fn finish_non_exhaustive(&mut self) -> Result<(), FormatError> / pure

    /// Finishes formatting the struct.
    fn finish(&mut self) -> Result<(), FormatError> / pure
}

/// Helper for formatting tuples in debug output.
struct DebugTuple<'a, 'b> {
    fmt: &'a mut Formatter<'b>,
}

impl<'a, 'b> DebugTuple<'a, 'b> {
    /// Adds a field to the tuple output.
    fn field(&mut self, value: &dyn Debug) -> &mut Self / pure

    /// Finishes formatting the tuple.
    fn finish(&mut self) -> Result<(), FormatError> / pure
}

/// Helper for formatting lists in debug output.
struct DebugList<'a, 'b> {
    fmt: &'a mut Formatter<'b>,
}

impl<'a, 'b> DebugList<'a, 'b> {
    /// Adds an entry to the list.
    fn entry(&mut self, value: &dyn Debug) -> &mut Self / pure

    /// Adds multiple entries to the list.
    fn entries<I: Iterator<Item = D>, D: Debug>(&mut self, entries: I) -> &mut Self / pure

    /// Finishes formatting the list.
    fn finish(&mut self) -> Result<(), FormatError> / pure
}

/// Helper for formatting sets in debug output.
struct DebugSet<'a, 'b> {
    fmt: &'a mut Formatter<'b>,
}

impl<'a, 'b> DebugSet<'a, 'b> {
    /// Adds an entry to the set.
    fn entry(&mut self, value: &dyn Debug) -> &mut Self / pure

    /// Adds multiple entries to the set.
    fn entries<I: Iterator<Item = D>, D: Debug>(&mut self, entries: I) -> &mut Self / pure

    /// Finishes formatting the set.
    fn finish(&mut self) -> Result<(), FormatError> / pure
}

/// Helper for formatting maps in debug output.
struct DebugMap<'a, 'b> {
    fmt: &'a mut Formatter<'b>,
}

impl<'a, 'b> DebugMap<'a, 'b> {
    /// Adds a key to the map (must be followed by value).
    fn key(&mut self, key: &dyn Debug) -> &mut Self / pure

    /// Adds a value to the map (must follow key).
    fn value(&mut self, value: &dyn Debug) -> &mut Self / pure

    /// Adds a key-value pair to the map.
    fn entry(&mut self, key: &dyn Debug, value: &dyn Debug) -> &mut Self / pure

    /// Adds multiple entries to the map.
    fn entries<K: Debug, V: Debug, I: Iterator<Item = (K, V)>>(&mut self, entries: I) -> &mut Self / pure

    /// Finishes formatting the map.
    fn finish(&mut self) -> Result<(), FormatError> / pure
}
```

#### 5.7.2 Example Implementations

```blood
// Manual Display implementation
impl Display for Point {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure {
        write!(f, "({}, {})", self.x, self.y)
    }
}

// Manual Debug implementation
impl Debug for Point {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure {
        f.debug_struct("Point")
            .field("x", &self.x)
            .field("y", &self.y)
            .finish()
    }
}

// Derive macro (preferred for most types)
#[derive(Debug)]
struct Rectangle {
    top_left: Point,
    bottom_right: Point,
}

// Debug with alternate mode
fn example() / {IO} {
    let rect = Rectangle { ... }
    println!("{:?}", rect)    // Single line
    println!("{:#?}", rect)   // Pretty-printed with indentation
}
```

### 5.8 Numeric Traits

Traits for generic numeric programming.

```blood
/// Marker trait for numeric types.
trait Numeric: Add + Sub + Mul + Div + PartialOrd + Clone {}

/// Types that have an additive identity (zero).
trait Zero: Numeric {
    /// Returns the additive identity.
    fn zero() -> Self / pure

    /// Returns true if this value is zero.
    fn is_zero(&self) -> bool / pure
}

/// Types that have a multiplicative identity (one).
trait One: Numeric {
    /// Returns the multiplicative identity.
    fn one() -> Self / pure

    /// Returns true if this value is one.
    fn is_one(&self) -> bool / pure
}

/// Signed numeric types.
trait Signed: Numeric + Neg<Output = Self> {
    /// Returns the absolute value.
    fn abs(&self) -> Self / pure

    /// Returns the sign as -1, 0, or 1.
    fn signum(&self) -> Self / pure

    /// Returns true if this value is positive.
    fn is_positive(&self) -> bool / pure

    /// Returns true if this value is negative.
    fn is_negative(&self) -> bool / pure
}

/// Unsigned numeric types.
trait Unsigned: Numeric {
    // Marker trait - all values are non-negative
}

/// Integer types.
trait Integer: Numeric + Ord + Eq {
    /// Returns the quotient of floor division.
    fn div_floor(&self, other: &Self) -> Self / pure

    /// Returns the remainder of floor division.
    fn mod_floor(&self, other: &Self) -> Self / pure

    /// Returns both quotient and remainder.
    fn div_rem(&self, other: &Self) -> (Self, Self) / pure

    /// Returns the greatest common divisor.
    fn gcd(&self, other: &Self) -> Self / pure

    /// Returns the least common multiple.
    fn lcm(&self, other: &Self) -> Self / pure

    /// Returns true if this value is even.
    fn is_even(&self) -> bool / pure

    /// Returns true if this value is odd.
    fn is_odd(&self) -> bool / pure
}

/// Floating-point types.
trait Float: Numeric + Signed + PartialEq {
    /// Returns the infinity value.
    fn infinity() -> Self / pure

    /// Returns the negative infinity value.
    fn neg_infinity() -> Self / pure

    /// Returns NaN.
    fn nan() -> Self / pure

    /// Returns true if this value is NaN.
    fn is_nan(&self) -> bool / pure

    /// Returns true if this value is infinite.
    fn is_infinite(&self) -> bool / pure

    /// Returns true if this value is finite.
    fn is_finite(&self) -> bool / pure

    /// Returns true if this value is normal (not zero, subnormal, infinite, or NaN).
    fn is_normal(&self) -> bool / pure

    /// Returns the floor of this value.
    fn floor(&self) -> Self / pure

    /// Returns the ceiling of this value.
    fn ceil(&self) -> Self / pure

    /// Returns the rounded value.
    fn round(&self) -> Self / pure

    /// Returns the truncated value.
    fn trunc(&self) -> Self / pure

    /// Returns the fractional part.
    fn fract(&self) -> Self / pure

    /// Returns the square root.
    fn sqrt(&self) -> Self / pure

    /// Returns e raised to this power.
    fn exp(&self) -> Self / pure

    /// Returns the natural logarithm.
    fn ln(&self) -> Self / pure

    /// Returns the logarithm base 10.
    fn log10(&self) -> Self / pure

    /// Returns the logarithm base 2.
    fn log2(&self) -> Self / pure

    /// Returns the sine.
    fn sin(&self) -> Self / pure

    /// Returns the cosine.
    fn cos(&self) -> Self / pure

    /// Returns the tangent.
    fn tan(&self) -> Self / pure

    /// Returns this value raised to a power.
    fn powf(&self, n: Self) -> Self / pure

    /// Returns this value raised to an integer power.
    fn powi(&self, n: i32) -> Self / pure
}

/// Bounded types with minimum and maximum values.
trait Bounded {
    /// Returns the smallest value.
    fn min_value() -> Self / pure

    /// Returns the largest value.
    fn max_value() -> Self / pure
}
```

#### 5.8.1 Numeric Constants

```blood
impl Bounded for i32 {
    fn min_value() -> i32 / pure { -2147483648 }
    fn max_value() -> i32 / pure { 2147483647 }
}

impl Zero for i32 {
    fn zero() -> i32 / pure { 0 }
    fn is_zero(&self) -> bool / pure { *self == 0 }
}

impl One for i32 {
    fn one() -> i32 / pure { 1 }
    fn is_one(&self) -> bool / pure { *self == 1 }
}

// Mathematical constants for floats
impl f64 {
    const PI: f64 = 3.14159265358979323846
    const E: f64 = 2.71828182845904523536
    const TAU: f64 = 6.28318530717958647692
    const SQRT_2: f64 = 1.41421356237309504880
    const LN_2: f64 = 0.693147180559945309417
    const LN_10: f64 = 2.30258509299404568402

    // Special values
    const INFINITY: f64 = f64::infinity()
    const NEG_INFINITY: f64 = f64::neg_infinity()
    const NAN: f64 = f64::nan()
    const EPSILON: f64 = 2.2204460492503131e-16
    const MIN: f64 = -1.7976931348623157e+308
    const MAX: f64 = 1.7976931348623157e+308
    const MIN_POSITIVE: f64 = 2.2250738585072014e-308
}
```

#### 5.8.2 Checked Arithmetic

```blood
/// Trait for types that support checked arithmetic.
trait CheckedAdd: Sized {
    /// Returns Some(result) or None on overflow.
    fn checked_add(&self, other: &Self) -> Option<Self> / pure
}

trait CheckedSub: Sized {
    fn checked_sub(&self, other: &Self) -> Option<Self> / pure
}

trait CheckedMul: Sized {
    fn checked_mul(&self, other: &Self) -> Option<Self> / pure
}

trait CheckedDiv: Sized {
    fn checked_div(&self, other: &Self) -> Option<Self> / pure
}

trait CheckedRem: Sized {
    fn checked_rem(&self, other: &Self) -> Option<Self> / pure
}

/// Trait for saturating arithmetic.
trait Saturating: Sized {
    /// Returns result clamped to min/max on overflow.
    fn saturating_add(&self, other: Self) -> Self / pure
    fn saturating_sub(&self, other: Self) -> Self / pure
    fn saturating_mul(&self, other: Self) -> Self / pure
}

/// Trait for wrapping arithmetic.
trait Wrapping: Sized {
    /// Returns result with wrapping on overflow.
    fn wrapping_add(&self, other: Self) -> Self / pure
    fn wrapping_sub(&self, other: Self) -> Self / pure
    fn wrapping_mul(&self, other: Self) -> Self / pure
}

/// Trait for overflowing arithmetic.
trait Overflowing: Sized {
    /// Returns (result, overflowed).
    fn overflowing_add(&self, other: Self) -> (Self, bool) / pure
    fn overflowing_sub(&self, other: Self) -> (Self, bool) / pure
    fn overflowing_mul(&self, other: Self) -> (Self, bool) / pure
}
```

### 5.9 Thread Safety Markers

Marker traits for thread-safe data sharing.

```blood
/// Types that can be transferred across fiber boundaries.
///
/// A type is `Send` if it is safe to send it to another fiber.
/// This property is checked by the compiler and cannot be implemented manually.
///
/// # Auto-implementation
/// Types are automatically `Send` if all of their components are `Send`.
///
/// # Non-Send Types
/// - `*const T`, `*mut T` (raw pointers)
/// - `Rc<T>` (non-atomic reference counting)
/// - Types containing non-Send fields
///
/// # Example
/// ```blood
/// fn spawn_work<T: Send>(data: T) / {Async} {
///     spawn(|| {
///         // data can be safely used in the new fiber
///         process(data)
///     })
/// }
/// ```
#[lang = "send"]
unsafe trait Send {
    // Marker trait - compiler verifies automatically
}

/// Types that can be safely shared between fibers via shared references.
///
/// A type is `Sync` if `&T` is `Send`. This means multiple fibers can
/// hold references to the same value simultaneously.
///
/// # Auto-implementation
/// Types are automatically `Sync` if all of their components are `Sync`.
///
/// # Non-Sync Types
/// - `Cell<T>`, `RefCell<T>` (interior mutability without synchronization)
/// - `Rc<T>` (non-atomic reference counting)
/// - Types containing non-Sync fields
///
/// # Relationship with Send
/// - `T: Sync` implies `&T: Send`
/// - `T: Send + Sync` means the type can be both moved and shared
///
/// # Example
/// ```blood
/// fn share_data<T: Sync>(data: &T) / {Async} {
///     let data1 = data
///     let data2 = data
///     spawn(|| use_data(data1))
///     spawn(|| use_data(data2))
/// }
/// ```
#[lang = "sync"]
unsafe trait Sync {
    // Marker trait - compiler verifies automatically
}

// Standard implementations

// All primitive types are Send + Sync
unsafe impl Send for bool {}
unsafe impl Sync for bool {}
unsafe impl Send for i8 {}
unsafe impl Sync for i8 {}
// ... (all primitive types)

// Box<T> is Send if T is Send
unsafe impl<T: Send> Send for Box<T> {}
// Box<T> is Sync if T is Sync
unsafe impl<T: Sync> Sync for Box<T> {}

// Vec<T> is Send if T is Send
unsafe impl<T: Send> Send for Vec<T> {}
// Vec<T> is Sync if T is Sync
unsafe impl<T: Sync> Sync for Vec<T> {}

// Arc<T> is Send if T is Send + Sync
unsafe impl<T: Send + Sync> Send for Arc<T> {}
// Arc<T> is Sync if T is Send + Sync
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

// Mutex<T> provides Sync even if T is not Sync (but requires T: Send)
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

// RwLock<T> provides Sync if T is Send + Sync
unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

// Channel endpoints are Send if T is Send
unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Send for Receiver<T> {}

// Atomic types are always Send + Sync
unsafe impl Send for AtomicI32 {}
unsafe impl Sync for AtomicI32 {}
// ... (all atomic types)

// Frozen<T> is always Sync (immutable after creation)
unsafe impl<T: Freeze> Send for Frozen<T> {}
unsafe impl<T: Freeze> Sync for Frozen<T> {}
```

#### 5.9.1 Negative Implementations

Some types explicitly opt out of Send/Sync:

```blood
// Raw pointers are not Send or Sync
impl<T> !Send for *const T {}
impl<T> !Sync for *const T {}
impl<T> !Send for *mut T {}
impl<T> !Sync for *mut T {}

// Rc is not Send or Sync (use Arc for thread-safe sharing)
impl<T> !Send for Rc<T> {}
impl<T> !Sync for Rc<T> {}

// Cell and RefCell are not Sync (interior mutability without sync)
impl<T> !Sync for Cell<T> {}
impl<T> !Sync for RefCell<T> {}

// MutexGuard is not Send (must be released on same fiber)
impl<T> !Send for MutexGuard<'_, T> {}

// PhantomData markers
struct PhantomNotSend(PhantomData<*const ()>);
struct PhantomNotSync(PhantomData<Cell<()>>);
```

#### 5.9.2 Thread Safety Guidelines

| Type | Send | Sync | Notes |
|------|------|------|-------|
| Primitives (i32, bool, etc.) | Yes | Yes | No interior mutability |
| Box<T> | If T: Send | If T: Sync | Owned heap data |
| Vec<T> | If T: Send | If T: Sync | Owned collection |
| Arc<T> | If T: Send+Sync | If T: Send+Sync | Shared ownership |
| Rc<T> | No | No | Use Arc instead |
| Mutex<T> | If T: Send | If T: Send | Provides Sync |
| RwLock<T> | If T: Send | If T: Send+Sync | Multiple readers |
| Cell<T> | If T: Send | No | Single-threaded interior mut |
| RefCell<T> | If T: Send | No | Single-threaded borrow checking |
| AtomicT | Yes | Yes | Lock-free operations |
| Channel<T> | If T: Send | If T: Send | Inter-fiber communication |
| Frozen<T> | If T: Freeze | Yes | Deeply immutable |

---

## 6. Iterator Adapters

Blood provides a rich set of iterator adapters for functional-style data processing.

### 6.1 Adapter Types

```blood
/// Maps each element using a function.
struct Map<I, F> {
    iter: I,
    f: F,
}

impl<B, I: Iterator, F: fn(I::Item) -> B> Iterator for Map<I, F> {
    type Item = B

    fn next(&mut self) -> Option<B> / pure {
        self.iter.next().map(self.f)
    }

    fn size_hint(&self) -> (usize, Option<usize>) / pure {
        self.iter.size_hint()
    }
}

/// Filters elements using a predicate.
struct Filter<I, P> {
    iter: I,
    predicate: P,
}

impl<I: Iterator, P: fn(&I::Item) -> bool> Iterator for Filter<I, P> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        loop {
            match self.iter.next() {
                Some(item) if (self.predicate)(&item) => return Some(item),
                Some(_) => continue,
                None => return None,
            }
        }
    }
}

/// Maps and filters in one pass.
struct FilterMap<I, F> {
    iter: I,
    f: F,
}

impl<B, I: Iterator, F: fn(I::Item) -> Option<B>> Iterator for FilterMap<I, F> {
    type Item = B

    fn next(&mut self) -> Option<B> / pure {
        loop {
            match self.iter.next() {
                Some(item) => {
                    if let Some(b) = (self.f)(item) {
                        return Some(b)
                    }
                }
                None => return None,
            }
        }
    }
}

/// Chains two iterators together.
struct Chain<A, B> {
    a: Option<A>,
    b: B,
}

impl<T, A: Iterator<Item = T>, B: Iterator<Item = T>> Iterator for Chain<A, B> {
    type Item = T

    fn next(&mut self) -> Option<T> / pure {
        match &mut self.a {
            Some(a) => match a.next() {
                Some(item) => Some(item),
                None => {
                    self.a = None
                    self.b.next()
                }
            },
            None => self.b.next(),
        }
    }
}

/// Zips two iterators into pairs.
struct Zip<A, B> {
    a: A,
    b: B,
}

impl<A: Iterator, B: Iterator> Iterator for Zip<A, B> {
    type Item = (A::Item, B::Item)

    fn next(&mut self) -> Option<(A::Item, B::Item)> / pure {
        let a = self.a.next()?
        let b = self.b.next()?
        Some((a, b))
    }

    fn size_hint(&self) -> (usize, Option<usize>) / pure {
        let (a_min, a_max) = self.a.size_hint()
        let (b_min, b_max) = self.b.size_hint()
        let min = a_min.min(b_min)
        let max = match (a_max, b_max) {
            (Some(a), Some(b)) => Some(a.min(b)),
            _ => None,
        }
        (min, max)
    }
}

/// Adds indices to iterator elements.
struct Enumerate<I> {
    iter: I,
    count: usize,
}

impl<I: Iterator> Iterator for Enumerate<I> {
    type Item = (usize, I::Item)

    fn next(&mut self) -> Option<(usize, I::Item)> / pure {
        let item = self.iter.next()?
        let i = self.count
        self.count += 1
        Some((i, item))
    }

    fn size_hint(&self) -> (usize, Option<usize>) / pure {
        self.iter.size_hint()
    }
}

/// Takes first n elements.
struct Take<I> {
    iter: I,
    remaining: usize,
}

impl<I: Iterator> Iterator for Take<I> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        if self.remaining > 0 {
            self.remaining -= 1
            self.iter.next()
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) / pure {
        let (min, max) = self.iter.size_hint()
        let min = min.min(self.remaining)
        let max = max.map(|m| m.min(self.remaining)).or(Some(self.remaining))
        (min, max)
    }
}

/// Skips first n elements.
struct Skip<I> {
    iter: I,
    n: usize,
}

impl<I: Iterator> Iterator for Skip<I> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        while self.n > 0 {
            self.iter.next()?
            self.n -= 1
        }
        self.iter.next()
    }
}

/// Takes while predicate is true.
struct TakeWhile<I, P> {
    iter: I,
    predicate: P,
    done: bool,
}

impl<I: Iterator, P: fn(&I::Item) -> bool> Iterator for TakeWhile<I, P> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        if self.done {
            return None
        }
        match self.iter.next() {
            Some(item) if (self.predicate)(&item) => Some(item),
            _ => {
                self.done = true
                None
            }
        }
    }
}

/// Skips while predicate is true.
struct SkipWhile<I, P> {
    iter: I,
    predicate: P,
    done: bool,
}

impl<I: Iterator, P: fn(&I::Item) -> bool> Iterator for SkipWhile<I, P> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        if !self.done {
            loop {
                match self.iter.next() {
                    Some(item) if (self.predicate)(&item) => continue,
                    Some(item) => {
                        self.done = true
                        return Some(item)
                    }
                    None => return None,
                }
            }
        }
        self.iter.next()
    }
}

/// Flattens nested iterators.
struct Flatten<I: Iterator>
where
    I::Item: IntoIterator,
{
    outer: I,
    inner: Option<<I::Item as IntoIterator>::IntoIter>,
}

impl<I: Iterator> Iterator for Flatten<I>
where
    I::Item: IntoIterator,
{
    type Item = <I::Item as IntoIterator>::Item

    fn next(&mut self) -> Option<Self::Item> / pure {
        loop {
            if let Some(ref mut inner) = self.inner {
                if let Some(item) = inner.next() {
                    return Some(item)
                }
            }
            match self.outer.next() {
                Some(inner) => self.inner = Some(inner.into_iter()),
                None => return None,
            }
        }
    }
}

/// Maps then flattens.
struct FlatMap<I, F, U: IntoIterator> {
    inner: Flatten<Map<I, F>>,
}

/// Peekable iterator that can look ahead.
struct Peekable<I: Iterator> {
    iter: I,
    peeked: Option<Option<I::Item>>,
}

impl<I: Iterator> Peekable<I> {
    /// Returns a reference to the next element without consuming it.
    fn peek(&mut self) -> Option<&I::Item> / pure {
        if self.peeked.is_none() {
            self.peeked = Some(self.iter.next())
        }
        self.peeked.as_ref().unwrap().as_ref()
    }

    /// Returns a mutable reference to the next element.
    fn peek_mut(&mut self) -> Option<&mut I::Item> / pure {
        if self.peeked.is_none() {
            self.peeked = Some(self.iter.next())
        }
        self.peeked.as_mut().unwrap().as_mut()
    }

    /// Consumes the next element if it matches the predicate.
    fn next_if<P: fn(&I::Item) -> bool>(&mut self, predicate: P) -> Option<I::Item> / pure {
        match self.peek() {
            Some(item) if predicate(item) => self.next(),
            _ => None,
        }
    }
}

impl<I: Iterator> Iterator for Peekable<I> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        match self.peeked.take() {
            Some(v) => v,
            None => self.iter.next(),
        }
    }
}

/// Fuses an iterator after it returns None once.
struct Fuse<I> {
    iter: Option<I>,
}

impl<I: Iterator> Iterator for Fuse<I> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        match &mut self.iter {
            Some(iter) => match iter.next() {
                Some(item) => Some(item),
                None => {
                    self.iter = None
                    None
                }
            },
            None => None,
        }
    }
}

/// Calls a closure on each element.
struct Inspect<I, F> {
    iter: I,
    f: F,
}

impl<I: Iterator, F: fn(&I::Item)> Iterator for Inspect<I, F> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        let item = self.iter.next()?
        (self.f)(&item)
        Some(item)
    }
}

/// Interleaves elements from two iterators.
struct Interleave<I, J> {
    a: I,
    b: J,
    flag: bool,
}

impl<T, I: Iterator<Item = T>, J: Iterator<Item = T>> Iterator for Interleave<I, J> {
    type Item = T

    fn next(&mut self) -> Option<T> / pure {
        self.flag = !self.flag
        if self.flag {
            self.a.next().or_else(|| self.b.next())
        } else {
            self.b.next().or_else(|| self.a.next())
        }
    }
}

/// Cycles through an iterator infinitely.
struct Cycle<I: Clone + Iterator> {
    orig: I,
    iter: I,
}

impl<I: Clone + Iterator> Iterator for Cycle<I> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        match self.iter.next() {
            Some(item) => Some(item),
            None => {
                self.iter = self.orig.clone()
                self.iter.next()
            }
        }
    }
}

/// Repeats an element infinitely.
struct Repeat<T> {
    element: T,
}

impl<T: Clone> Iterator for Repeat<T> {
    type Item = T

    fn next(&mut self) -> Option<T> / pure {
        Some(self.element.clone())
    }
}

/// Generates elements from a closure.
struct FromFn<F> {
    f: F,
}

impl<T, F: fn() -> Option<T>> Iterator for FromFn<F> {
    type Item = T

    fn next(&mut self) -> Option<T> / pure {
        (self.f)()
    }
}

/// Generates elements with state.
struct Unfold<St, F> {
    state: St,
    f: F,
}

impl<St, T, F: fn(&mut St) -> Option<T>> Iterator for Unfold<St, F> {
    type Item = T

    fn next(&mut self) -> Option<T> / pure {
        (self.f)(&mut self.state)
    }
}

/// Iterates in reverse.
struct Rev<I> {
    iter: I,
}

impl<I: DoubleEndedIterator> Iterator for Rev<I> {
    type Item = I::Item

    fn next(&mut self) -> Option<I::Item> / pure {
        self.iter.next_back()
    }
}
```

### 6.2 Iterator Extension Methods

The full set of methods available on `Iterator`:

```blood
trait Iterator {
    type Item

    // Core method (must be implemented)
    fn next(&mut self) -> Option<Self::Item> / pure

    // Size hints
    fn size_hint(&self) -> (usize, Option<usize>) / pure { (0, None) }
    fn count(self) -> usize / pure
    fn last(self) -> Option<Self::Item> / pure
    fn nth(&mut self, n: usize) -> Option<Self::Item> / pure

    // Adapters (return new iterators)
    fn map<B, F: fn(Self::Item) -> B>(self, f: F) -> Map<Self, F> / pure
    fn filter<P: fn(&Self::Item) -> bool>(self, predicate: P) -> Filter<Self, P> / pure
    fn filter_map<B, F: fn(Self::Item) -> Option<B>>(self, f: F) -> FilterMap<Self, F> / pure
    fn chain<U: Iterator<Item = Self::Item>>(self, other: U) -> Chain<Self, U> / pure
    fn zip<U: Iterator>(self, other: U) -> Zip<Self, U> / pure
    fn enumerate(self) -> Enumerate<Self> / pure
    fn take(self, n: usize) -> Take<Self> / pure
    fn skip(self, n: usize) -> Skip<Self> / pure
    fn take_while<P: fn(&Self::Item) -> bool>(self, predicate: P) -> TakeWhile<Self, P> / pure
    fn skip_while<P: fn(&Self::Item) -> bool>(self, predicate: P) -> SkipWhile<Self, P> / pure
    fn flatten(self) -> Flatten<Self> / pure where Self::Item: IntoIterator
    fn flat_map<U: IntoIterator, F: fn(Self::Item) -> U>(self, f: F) -> FlatMap<Self, F, U> / pure
    fn peekable(self) -> Peekable<Self> / pure
    fn fuse(self) -> Fuse<Self> / pure
    fn inspect<F: fn(&Self::Item)>(self, f: F) -> Inspect<Self, F> / pure
    fn interleave<J: Iterator<Item = Self::Item>>(self, other: J) -> Interleave<Self, J> / pure
    fn cycle(self) -> Cycle<Self> / pure where Self: Clone
    fn rev(self) -> Rev<Self> / pure where Self: DoubleEndedIterator
    fn step_by(self, step: usize) -> StepBy<Self> / pure

    // Consumers (collect/reduce to a value)
    fn collect<B: FromIterator<Self::Item>>(self) -> B / pure
    fn fold<B, F: fn(B, Self::Item) -> B>(self, init: B, f: F) -> B / pure
    fn reduce<F: fn(Self::Item, Self::Item) -> Self::Item>(self, f: F) -> Option<Self::Item> / pure
    fn sum<S: Sum<Self::Item>>(self) -> S / pure
    fn product<P: Product<Self::Item>>(self) -> P / pure
    fn min(self) -> Option<Self::Item> / pure where Self::Item: Ord
    fn max(self) -> Option<Self::Item> / pure where Self::Item: Ord
    fn min_by<F: fn(&Self::Item, &Self::Item) -> Ordering>(self, compare: F) -> Option<Self::Item> / pure
    fn max_by<F: fn(&Self::Item, &Self::Item) -> Ordering>(self, compare: F) -> Option<Self::Item> / pure
    fn min_by_key<B: Ord, F: fn(&Self::Item) -> B>(self, f: F) -> Option<Self::Item> / pure
    fn max_by_key<B: Ord, F: fn(&Self::Item) -> B>(self, f: F) -> Option<Self::Item> / pure

    // Searching
    fn find<P: fn(&Self::Item) -> bool>(&mut self, predicate: P) -> Option<Self::Item> / pure
    fn find_map<B, F: fn(Self::Item) -> Option<B>>(&mut self, f: F) -> Option<B> / pure
    fn position<P: fn(Self::Item) -> bool>(&mut self, predicate: P) -> Option<usize> / pure
    fn rposition<P: fn(Self::Item) -> bool>(&mut self, predicate: P) -> Option<usize> / pure
        where Self: DoubleEndedIterator + ExactSizeIterator

    // Boolean operations
    fn any<F: fn(Self::Item) -> bool>(&mut self, f: F) -> bool / pure
    fn all<F: fn(Self::Item) -> bool>(&mut self, f: F) -> bool / pure

    // Comparison
    fn eq<I: Iterator<Item = Self::Item>>(self, other: I) -> bool / pure where Self::Item: PartialEq
    fn ne<I: Iterator<Item = Self::Item>>(self, other: I) -> bool / pure where Self::Item: PartialEq
    fn lt<I: Iterator<Item = Self::Item>>(self, other: I) -> bool / pure where Self::Item: PartialOrd
    fn le<I: Iterator<Item = Self::Item>>(self, other: I) -> bool / pure where Self::Item: PartialOrd
    fn gt<I: Iterator<Item = Self::Item>>(self, other: I) -> bool / pure where Self::Item: PartialOrd
    fn ge<I: Iterator<Item = Self::Item>>(self, other: I) -> bool / pure where Self::Item: PartialOrd
    fn cmp<I: Iterator<Item = Self::Item>>(self, other: I) -> Ordering / pure where Self::Item: Ord

    // Partitioning
    fn partition<B: Default + Extend<Self::Item>, F: fn(&Self::Item) -> bool>(self, f: F) -> (B, B) / pure
    fn unzip<A, B, FromA: Default + Extend<A>, FromB: Default + Extend<B>>(self) -> (FromA, FromB) / pure
        where Self: Iterator<Item = (A, B)>

    // Side effects
    fn for_each<F: fn(Self::Item)>(self, f: F) / pure
}

/// Double-ended iterators can iterate from both ends.
trait DoubleEndedIterator: Iterator {
    fn next_back(&mut self) -> Option<Self::Item> / pure

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> / pure
    fn rfold<B, F: fn(B, Self::Item) -> B>(self, init: B, f: F) -> B / pure
    fn rfind<P: fn(&Self::Item) -> bool>(&mut self, predicate: P) -> Option<Self::Item> / pure
}

/// Iterators that know their exact length.
trait ExactSizeIterator: Iterator {
    fn len(&self) -> usize / pure
    fn is_empty(&self) -> bool / pure { self.len() == 0 }
}

/// Iterators that can be trusted for exact size (used for optimization).
unsafe trait TrustedLen: Iterator {}
```

---

## 7. IO Types

> **Implementation Status**: The types and signatures in this section are **specified but not yet implemented**. Path manipulation is partially implemented (`blood-std/std/path.blood` ~150 lines). File system operations have type definitions only (`blood-std/std/fs/mod.blood` ~50 lines, no syscall implementations). Networking types are defined but socket operations are not implemented (`blood-std/std/net/mod.blood` ~210 lines, types only).

Blood provides types for file system and I/O operations.

### 7.1 Path and PathBuf

```blood
/// Borrowed path slice.
struct Path {
    inner: [u8],  // OsStr-like representation
}

impl Path {
    /// Creates a Path from a string slice.
    fn new<S: AsRef<str>>(s: &S) -> &Path / pure

    /// Returns the path as a string slice (may fail for non-UTF8).
    fn to_str(&self) -> Option<&str> / pure

    /// Returns the path as a byte slice.
    fn as_bytes(&self) -> &[u8] / pure

    /// Converts to an owned PathBuf.
    fn to_path_buf(&self) -> PathBuf / {Allocate}

    /// Returns the parent directory.
    fn parent(&self) -> Option<&Path> / pure

    /// Returns the final component (file name).
    fn file_name(&self) -> Option<&str> / pure

    /// Returns the file stem (name without extension).
    fn file_stem(&self) -> Option<&str> / pure

    /// Returns the file extension.
    fn extension(&self) -> Option<&str> / pure

    /// Joins this path with another path.
    fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf / {Allocate}

    /// Returns true if the path is absolute.
    fn is_absolute(&self) -> bool / pure

    /// Returns true if the path is relative.
    fn is_relative(&self) -> bool / pure

    /// Returns true if the path has a root.
    fn has_root(&self) -> bool / pure

    /// Returns an iterator over the components.
    fn components(&self) -> Components / pure

    /// Returns an iterator over the ancestors.
    fn ancestors(&self) -> Ancestors / pure

    /// Strips a prefix from the path.
    fn strip_prefix<P: AsRef<Path>>(&self, base: P) -> Result<&Path, StripPrefixError> / pure

    /// Returns true if the path starts with the given prefix.
    fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool / pure

    /// Returns true if the path ends with the given suffix.
    fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool / pure

    /// Queries file system metadata.
    fn metadata(&self) -> Result<Metadata, IoError> / {IO}

    /// Returns true if the path exists.
    fn exists(&self) -> bool / {IO}

    /// Returns true if the path is a file.
    fn is_file(&self) -> bool / {IO}

    /// Returns true if the path is a directory.
    fn is_dir(&self) -> bool / {IO}

    /// Returns true if the path is a symbolic link.
    fn is_symlink(&self) -> bool / {IO}

    /// Reads the entire file as bytes.
    fn read(&self) -> Result<Vec<u8>, IoError> / {IO}

    /// Reads the entire file as a string.
    fn read_to_string(&self) -> Result<String, IoError> / {IO}

    /// Reads directory entries.
    fn read_dir(&self) -> Result<ReadDir, IoError> / {IO}
}

/// Owned path buffer.
struct PathBuf {
    inner: Vec<u8>,
}

impl PathBuf {
    /// Creates an empty PathBuf.
    fn new() -> PathBuf / pure

    /// Creates a PathBuf with the given capacity.
    fn with_capacity(capacity: usize) -> PathBuf / {Allocate}

    /// Returns a borrowed Path.
    fn as_path(&self) -> &Path / pure

    /// Pushes a component onto the path.
    fn push<P: AsRef<Path>>(&mut self, path: P) / {Allocate}

    /// Pops the last component.
    fn pop(&mut self) -> bool / pure

    /// Sets the file name.
    fn set_file_name<S: AsRef<str>>(&mut self, file_name: S) / {Allocate}

    /// Sets the file extension.
    fn set_extension<S: AsRef<str>>(&mut self, extension: S) -> bool / {Allocate}

    /// Clears the path.
    fn clear(&mut self) / pure

    /// Returns the capacity.
    fn capacity(&self) -> usize / pure

    /// Reserves capacity for at least `additional` more bytes.
    fn reserve(&mut self, additional: usize) / {Allocate}

    /// Shrinks capacity to fit the current length.
    fn shrink_to_fit(&mut self) / {Allocate}
}

/// Path component.
enum Component<'a> {
    Prefix(PrefixComponent<'a>),  // Windows drive letters
    RootDir,                       // `/` or `\`
    CurDir,                        // `.`
    ParentDir,                     // `..`
    Normal(&'a str),               // Regular component
}

impl Deref for PathBuf {
    type Target = Path
    fn deref(&self) -> &Path / pure
}
```

### 7.2 File and OpenOptions

```blood
/// File handle for reading and writing.
struct File {
    fd: Fd,
}

impl File {
    /// Opens a file in read-only mode.
    fn open<P: AsRef<Path>>(path: P) -> Result<File, IoError> / {IO}

    /// Creates a new file or truncates an existing one.
    fn create<P: AsRef<Path>>(path: P) -> Result<File, IoError> / {IO}

    /// Opens a file with the given options.
    fn open_with<P: AsRef<Path>>(path: P, options: &OpenOptions) -> Result<File, IoError> / {IO}

    /// Reads bytes into the buffer, returning bytes read.
    fn read(&self, buf: &mut [u8]) -> Result<usize, IoError> / {IO}

    /// Reads the entire file into a vector.
    fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize, IoError> / {IO}

    /// Reads the entire file into a string.
    fn read_to_string(&self, buf: &mut String) -> Result<usize, IoError> / {IO}

    /// Writes bytes from the buffer, returning bytes written.
    fn write(&self, buf: &[u8]) -> Result<usize, IoError> / {IO}

    /// Writes all bytes from the buffer.
    fn write_all(&self, buf: &[u8]) -> Result<(), IoError> / {IO}

    /// Flushes any buffered data.
    fn flush(&self) -> Result<(), IoError> / {IO}

    /// Seeks to a position.
    fn seek(&self, pos: SeekFrom) -> Result<u64, IoError> / {IO}

    /// Returns the current position.
    fn stream_position(&self) -> Result<u64, IoError> / {IO}

    /// Truncates or extends the file to the given length.
    fn set_len(&self, size: u64) -> Result<(), IoError> / {IO}

    /// Returns file metadata.
    fn metadata(&self) -> Result<Metadata, IoError> / {IO}

    /// Attempts to clone the file handle.
    fn try_clone(&self) -> Result<File, IoError> / {IO}

    /// Syncs all data and metadata to disk.
    fn sync_all(&self) -> Result<(), IoError> / {IO}

    /// Syncs data (but not necessarily metadata) to disk.
    fn sync_data(&self) -> Result<(), IoError> / {IO}
}

impl Drop for File {
    fn drop(&mut self) / pure {
        // Closes the file descriptor
    }
}

/// Options for opening files.
struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    mode: u32,
}

impl OpenOptions {
    /// Creates a new set of options (all false).
    fn new() -> OpenOptions / pure

    /// Sets read access.
    fn read(&mut self, read: bool) -> &mut Self / pure

    /// Sets write access.
    fn write(&mut self, write: bool) -> &mut Self / pure

    /// Sets append mode.
    fn append(&mut self, append: bool) -> &mut Self / pure

    /// Sets truncate mode.
    fn truncate(&mut self, truncate: bool) -> &mut Self / pure

    /// Sets create mode.
    fn create(&mut self, create: bool) -> &mut Self / pure

    /// Sets create_new mode (fails if file exists).
    fn create_new(&mut self, create_new: bool) -> &mut Self / pure

    /// Sets the file mode (Unix permissions).
    fn mode(&mut self, mode: u32) -> &mut Self / pure

    /// Opens the file with these options.
    fn open<P: AsRef<Path>>(&self, path: P) -> Result<File, IoError> / {IO}
}

/// Seek position.
enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}

/// File metadata.
struct Metadata {
    // Internal: stat-like data
}

impl Metadata {
    fn is_file(&self) -> bool / pure
    fn is_dir(&self) -> bool / pure
    fn is_symlink(&self) -> bool / pure
    fn len(&self) -> u64 / pure
    fn permissions(&self) -> Permissions / pure
    fn modified(&self) -> Result<SystemTime, IoError> / pure
    fn accessed(&self) -> Result<SystemTime, IoError> / pure
    fn created(&self) -> Result<SystemTime, IoError> / pure
}

/// File permissions.
struct Permissions {
    mode: u32,
}

impl Permissions {
    fn readonly(&self) -> bool / pure
    fn set_readonly(&mut self, readonly: bool) / pure
    fn mode(&self) -> u32 / pure  // Unix only
    fn set_mode(&mut self, mode: u32) / pure  // Unix only
}
```

### 7.3 File Descriptor

```blood
/// Raw file descriptor.
struct Fd(i32);

impl Fd {
    const STDIN: Fd = Fd(0)
    const STDOUT: Fd = Fd(1)
    const STDERR: Fd = Fd(2)

    /// Creates from a raw descriptor.
    fn from_raw(fd: i32) -> Fd / pure

    /// Returns the raw descriptor.
    fn as_raw(&self) -> i32 / pure
}
```

### 7.4 IO Error

```blood
/// I/O error type.
struct IoError {
    kind: IoErrorKind,
    message: Option<String>,
}

impl IoError {
    fn new(kind: IoErrorKind, msg: &str) -> IoError / {Allocate}
    fn kind(&self) -> IoErrorKind / pure
    fn raw_os_error(&self) -> Option<i32> / pure
}

impl Display for IoError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Categories of I/O errors.
enum IoErrorKind {
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    BrokenPipe,
    AlreadyExists,
    WouldBlock,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    Interrupted,
    UnexpectedEof,
    Other,
}
```

---

## 8. Time Types

Blood provides types for time representation and measurement.

### 8.1 Duration

```blood
/// A span of time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Duration {
    secs: u64,
    nanos: u32,  // 0 <= nanos < 1_000_000_000
}

impl Duration {
    /// Zero duration.
    const ZERO: Duration = Duration { secs: 0, nanos: 0 }

    /// Maximum duration.
    const MAX: Duration = Duration { secs: u64::MAX, nanos: 999_999_999 }

    /// One second.
    const SECOND: Duration = Duration { secs: 1, nanos: 0 }

    /// One millisecond.
    const MILLISECOND: Duration = Duration { secs: 0, nanos: 1_000_000 }

    /// One microsecond.
    const MICROSECOND: Duration = Duration { secs: 0, nanos: 1_000 }

    /// One nanosecond.
    const NANOSECOND: Duration = Duration { secs: 0, nanos: 1 }

    /// Creates a duration from seconds.
    fn from_secs(secs: u64) -> Duration / pure

    /// Creates a duration from milliseconds.
    fn from_millis(millis: u64) -> Duration / pure

    /// Creates a duration from microseconds.
    fn from_micros(micros: u64) -> Duration / pure

    /// Creates a duration from nanoseconds.
    fn from_nanos(nanos: u64) -> Duration / pure

    /// Creates a duration from seconds as f64.
    fn from_secs_f64(secs: f64) -> Duration / pure

    /// Creates a duration from seconds as f32.
    fn from_secs_f32(secs: f32) -> Duration / pure

    /// Creates a new duration from seconds and nanoseconds.
    fn new(secs: u64, nanos: u32) -> Duration / pure

    /// Returns true if this duration is zero.
    fn is_zero(&self) -> bool / pure

    /// Returns the number of whole seconds.
    fn as_secs(&self) -> u64 / pure

    /// Returns the fractional part in milliseconds.
    fn subsec_millis(&self) -> u32 / pure

    /// Returns the fractional part in microseconds.
    fn subsec_micros(&self) -> u32 / pure

    /// Returns the fractional part in nanoseconds.
    fn subsec_nanos(&self) -> u32 / pure

    /// Returns the total number of milliseconds.
    fn as_millis(&self) -> u128 / pure

    /// Returns the total number of microseconds.
    fn as_micros(&self) -> u128 / pure

    /// Returns the total number of nanoseconds.
    fn as_nanos(&self) -> u128 / pure

    /// Returns the duration as seconds (f64).
    fn as_secs_f64(&self) -> f64 / pure

    /// Returns the duration as seconds (f32).
    fn as_secs_f32(&self) -> f32 / pure

    /// Checked addition.
    fn checked_add(&self, rhs: Duration) -> Option<Duration> / pure

    /// Saturating addition.
    fn saturating_add(&self, rhs: Duration) -> Duration / pure

    /// Checked subtraction.
    fn checked_sub(&self, rhs: Duration) -> Option<Duration> / pure

    /// Saturating subtraction.
    fn saturating_sub(&self, rhs: Duration) -> Duration / pure

    /// Checked multiplication.
    fn checked_mul(&self, rhs: u32) -> Option<Duration> / pure

    /// Saturating multiplication.
    fn saturating_mul(&self, rhs: u32) -> Duration / pure

    /// Checked division.
    fn checked_div(&self, rhs: u32) -> Option<Duration> / pure

    /// Multiplies by a float.
    fn mul_f64(&self, rhs: f64) -> Duration / pure

    /// Divides by a float.
    fn div_f64(&self, rhs: f64) -> Duration / pure

    /// Returns the absolute difference between two durations.
    fn abs_diff(&self, other: Duration) -> Duration / pure
}

impl Add for Duration {
    type Output = Duration
    fn add(self, rhs: Duration) -> Duration / pure
}

impl Sub for Duration {
    type Output = Duration
    fn sub(self, rhs: Duration) -> Duration / pure
}

impl Mul<u32> for Duration {
    type Output = Duration
    fn mul(self, rhs: u32) -> Duration / pure
}

impl Div<u32> for Duration {
    type Output = Duration
    fn div(self, rhs: u32) -> Duration / pure
}
```

### 8.2 Instant

```blood
/// A monotonically non-decreasing clock.
/// Used for measuring elapsed time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Instant {
    // Internal: platform-specific representation
}

impl Instant {
    /// Returns the current instant.
    fn now() -> Instant / {IO}

    /// Returns the elapsed time since this instant.
    fn elapsed(&self) -> Duration / {IO}

    /// Returns the duration since an earlier instant.
    fn duration_since(&self, earlier: Instant) -> Duration / pure

    /// Checked duration since an earlier instant.
    fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> / pure

    /// Saturating duration since an earlier instant.
    fn saturating_duration_since(&self, earlier: Instant) -> Duration / pure

    /// Checked addition.
    fn checked_add(&self, duration: Duration) -> Option<Instant> / pure

    /// Checked subtraction.
    fn checked_sub(&self, duration: Duration) -> Option<Instant> / pure
}

impl Add<Duration> for Instant {
    type Output = Instant
    fn add(self, duration: Duration) -> Instant / pure
}

impl Sub<Duration> for Instant {
    type Output = Instant
    fn sub(self, duration: Duration) -> Instant / pure
}

impl Sub<Instant> for Instant {
    type Output = Duration
    fn sub(self, other: Instant) -> Duration / pure
}
```

### 8.3 SystemTime

```blood
/// Wall clock time (can go backwards due to clock adjustments).
/// Use Instant for measuring elapsed time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct SystemTime {
    // Internal: platform-specific representation
}

impl SystemTime {
    /// The Unix epoch (1970-01-01 00:00:00 UTC).
    const UNIX_EPOCH: SystemTime = SystemTime { /* ... */ }

    /// Returns the current system time.
    fn now() -> SystemTime / {IO}

    /// Returns the duration since the given time.
    fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SystemTimeError> / pure

    /// Returns the elapsed time since this time.
    fn elapsed(&self) -> Result<Duration, SystemTimeError> / {IO}

    /// Checked addition.
    fn checked_add(&self, duration: Duration) -> Option<SystemTime> / pure

    /// Checked subtraction.
    fn checked_sub(&self, duration: Duration) -> Option<SystemTime> / pure
}

impl Add<Duration> for SystemTime {
    type Output = SystemTime
    fn add(self, duration: Duration) -> SystemTime / pure
}

impl Sub<Duration> for SystemTime {
    type Output = SystemTime
    fn sub(self, duration: Duration) -> SystemTime / pure
}

/// Error when calculating duration between system times.
struct SystemTimeError {
    duration: Duration,  // The absolute difference
}

impl SystemTimeError {
    /// Returns the positive duration difference.
    fn duration(&self) -> Duration / pure
}
```

### 8.4 Time Usage Examples

```blood
// Measuring elapsed time
fn benchmark<T, F: fn() -> T>(f: F) -> (T, Duration) / {IO} {
    let start = Instant::now()
    let result = f()
    let elapsed = start.elapsed()
    (result, elapsed)
}

// Timeouts
fn with_timeout<T, F: fn() -> T>(
    f: F,
    timeout: Duration
) -> Result<T, TimeoutError> / {Async} {
    // Implementation uses async runtime
}

// Delays
fn sleep(duration: Duration) / {IO} {
    // Platform-specific sleep
}

// Rate limiting
struct RateLimiter {
    last: Instant,
    interval: Duration,
}

impl RateLimiter {
    fn new(interval: Duration) -> RateLimiter / {IO}

    fn wait(&mut self) / {IO} {
        let elapsed = self.last.elapsed()
        if elapsed < self.interval {
            sleep(self.interval - elapsed)
        }
        self.last = Instant::now()
    }
}
```

---

## 9. Format Macros

Blood provides compile-time format string processing.

### 9.1 Format Syntax

```blood
// Basic formatting
format!("Hello, {}!", name)          // Display formatting
format!("Debug: {:?}", value)        // Debug formatting
format!("Pretty: {:#?}", value)      // Pretty-print debug

// Positional arguments
format!("{0} {} {0}", a, b)          // Explicit and implicit positions
format!("{1} {0}", a, b)             // Reversed order

// Named arguments
format!("{name} is {age} years old", name="Alice", age=30)

// Width and alignment
format!("{:10}", s)                  // Right-align in 10 chars
format!("{:<10}", s)                 // Left-align
format!("{:^10}", s)                 // Center-align
format!("{:>10}", s)                 // Right-align (explicit)
format!("{:*^10}", s)                // Center with '*' fill

// Precision
format!("{:.3}", 3.14159)            // 3 decimal places
format!("{:.3}", "hello")            // First 3 chars
format!("{:10.3}", 3.14159)          // Width 10, 3 decimals

// Sign
format!("{:+}", 42)                  // Always show sign
format!("{:-}", -42)                 // Negative only (default)

// Number bases
format!("{:b}", 42)                  // Binary
format!("{:o}", 42)                  // Octal
format!("{:x}", 42)                  // Lowercase hex
format!("{:X}", 42)                  // Uppercase hex
format!("{:#x}", 42)                 // Hex with 0x prefix
format!("{:#b}", 42)                 // Binary with 0b prefix

// Pointer
format!("{:p}", &value)              // Pointer address

// Escape braces
format!("{{literal braces}}")        // Prints: {literal braces}
```

### 9.2 Format Macros

```blood
/// Formats arguments into a String.
macro format!($fmt:literal $(, $args:expr)*) -> String / {Allocate}

/// Writes formatted string to a writer.
macro write!($dst:expr, $fmt:literal $(, $args:expr)*) -> Result<(), FormatError> / pure

/// Writes formatted string with newline to a writer.
macro writeln!($dst:expr, $fmt:literal $(, $args:expr)*) -> Result<(), FormatError> / pure

/// Prints to stdout.
macro print!($fmt:literal $(, $args:expr)*) / {IO}

/// Prints to stdout with newline.
macro println!($fmt:literal $(, $args:expr)*) / {IO}

/// Prints to stderr.
macro eprint!($fmt:literal $(, $args:expr)*) / {IO}

/// Prints to stderr with newline.
macro eprintln!($fmt:literal $(, $args:expr)*) / {IO}

/// Panics with formatted message.
macro panic!($fmt:literal $(, $args:expr)*) -> ! / {Panic}

/// Debug print (includes source location).
macro dbg!($val:expr) -> T / {IO}

/// Compile-time format string for later use.
macro format_args!($fmt:literal $(, $args:expr)*) -> Arguments / pure
```

### 9.3 Implementation Details

```blood
/// Compiled format arguments.
struct Arguments<'a> {
    pieces: &'a [&'a str],
    fmt: Option<&'a [Argument]>,
    args: &'a [ArgumentV1<'a>],
}

/// Format argument metadata.
struct Argument {
    position: usize,
    format: FormatSpec,
}

/// Format specification.
struct FormatSpec {
    fill: char,
    align: Alignment,
    flags: u32,
    precision: Count,
    width: Count,
}

enum Count {
    Is(usize),
    Param(usize),
    Implied,
}

/// Type-erased format argument.
struct ArgumentV1<'a> {
    value: &'a dyn Display,  // Or Debug, depending on format spec
    formatter: fn(&dyn Display, &mut Formatter) -> Result<(), FormatError>,
}

impl<'a> Arguments<'a> {
    /// Returns estimated output length.
    fn estimated_capacity(&self) -> usize / pure
}
```

### 9.4 Custom Formatting

```blood
// Implementing Display for a custom type
impl Display for IpAddress {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure {
        write!(f, "{}.{}.{}.{}", self.0, self.1, self.2, self.3)
    }
}

// Implementing Debug with helpers
impl Debug for HashMap<K, V>
where
    K: Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure {
        f.debug_map()
            .entries(self.iter())
            .finish()
    }
}

// Custom formatter for special needs
impl LowerHex for MyType {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure {
        write!(f, "{:x}", self.inner)
    }
}
```

### 9.5 Formatting Traits

```blood
/// Format with `{}`.
trait Display {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:?}`.
trait Debug {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:o}`.
trait Octal {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:x}`.
trait LowerHex {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:X}`.
trait UpperHex {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:b}`.
trait Binary {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:e}`.
trait LowerExp {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:E}`.
trait UpperExp {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}

/// Format with `{:p}`.
trait Pointer {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> / pure
}
```

---

## 10. Standard Effects

### 10.0 Error Handling Philosophy: Panic vs Error

Blood distinguishes between two categories of failures:

| Category | Mechanism | Recoverable | When to Use |
|----------|-----------|-------------|-------------|
| **Errors** | `Error<E>` effect | Yes | Expected failures, external inputs, IO |
| **Panics** | `Panic` effect | No* | Bugs, invariant violations, impossible states |

*Panics can be caught at coarse boundaries but should not be used for control flow.

#### 10.0.1 When to Use Error<E>

Use `Error<E>` for **expected failures** that callers should handle:

```blood
// Good: IO operations can fail
fn read_config(path: &Path) -> Config / {IO, Error<ConfigError>} { ... }

// Good: Parsing untrusted input
fn parse_json(input: &str) -> Json / {Error<ParseError>} { ... }

// Good: Network operations
fn fetch_url(url: &Url) -> Response / {Async, Error<HttpError>} { ... }
```

#### 10.0.2 When to Use Panic

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

#### 10.0.3 The `#[no_panic]` Attribute

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

#### 10.0.4 Converting Between Panic and Error

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

#### 10.0.5 Guidelines by Domain

| Domain | Preferred Approach |
|--------|-------------------|
| Safety-critical (avionics, medical) | `#[no_panic]` + `Error<E>` everywhere |
| Server applications | `Error<E>` with panic boundaries per request |
| CLI tools | `Panic` acceptable for startup, `Error<E>` for user input |
| Libraries | `Error<E>` for public API, `Panic` for internal bugs |

### 10.1 Error<E>

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

### 10.2 State<S>

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

### 10.3 IO

Input/output operations. IO is a **compound effect** that subsumes several simpler effects.

#### 10.3.1 Effect Hierarchy and Subsumption

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

#### 10.3.2 Subsumption Semantics

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

#### 10.3.3 Effect Hierarchy Rules

The standard effect subsumption hierarchy:

```
pure                    (no effects)
  │
  ├── Error<E>          (can fail with E)
  ├── State<S>          (can read/write state S)
  ├── Log               (can emit log messages)
  ├── Random            (can generate random values)
  │
  └── IO ───────────────(subsumes Log; can do file/network*/env/time)
        │
        └── Async       (subsumes IO; can spawn/await tasks)
```

*\*Network operations are specified but not yet implemented. See §7 implementation status note.*

Functions with narrower effects can be called from contexts with broader effects:

```blood
fn pure_compute(x: i32) -> i32 / pure { x * 2 }

fn io_context() / {IO} {
    let y = pure_compute(21)  // OK: pure ⊆ IO
    println(y.to_string())
}
```

#### 10.3.4 Convenience Functions

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

### 10.4 Async

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

### 10.5 Yield<T>

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

### 10.6 NonDet

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

### 10.7 Allocate

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

### 10.8 Panic

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

### 10.9 Log

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

### 10.10 StaleReference

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

#### 10.10.1 When StaleReference is Raised

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

#### 10.10.2 Integration with Generation Snapshots

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

### 10.11 Resource<R>

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

### 10.12 Random

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

## 11. Standard Handlers

### 11.1 Error Handlers

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

### 11.2 State Handlers

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

#### 11.2.1 Generation Snapshots in State Handlers

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

### 11.3 IO Handlers

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

### 11.4 Async Handlers

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

### 11.5 Yield Handlers

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

### 11.6 NonDet Handlers

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

### 11.7 StaleReference Handlers

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

### 11.8 Resource Handlers

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

### 11.9 Random Handlers

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

## 12. Prelude

### 12.1 Auto-Imported Items

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

### 12.2 Excluding Prelude

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

## 13. Implementation Notes

This section provides implementation guidance for compiler writers and advanced users.

### 13.1 Effect Compilation Strategy

Blood effects are compiled using **evidence passing**, similar to Koka's approach. This transforms effect operations into explicit dictionary parameters.

#### 13.1.1 Monomorphization

For monomorphic effect usage, the compiler eliminates effect overhead entirely:

```blood
// Source code
fn increment() -> i32 / {State<i32>} {
    let x = get()
    put(x + 1)
    x
}

// After monomorphization (conceptual)
fn increment(state: &mut i32) -> i32 {
    let x = *state;
    *state = x + 1;
    x
}
```

#### 13.1.2 Polymorphic Effects

For polymorphic effect rows, evidence dictionaries are passed:

```blood
// Source with effect polymorphism
fn map_with_effect<T, U, E>(
    items: &[T],
    f: fn(T) -> U / {E}
) -> Vec<U> / {E} {
    items.iter().map(f).collect()
}

// After evidence passing (conceptual)
fn map_with_effect<T, U>(
    items: &[T],
    f: fn(T, &Evidence_E) -> U,
    evidence: &Evidence_E
) -> Vec<U> {
    items.iter().map(|t| f(t, evidence)).collect()
}
```

#### 13.1.3 Handler Compilation

Handlers compile to different implementations based on their characteristics:

| Handler Type | Resume Behavior | Implementation |
|-------------|-----------------|----------------|
| Deep, single-shot | `resume` called once | Direct continuation |
| Deep, multi-shot | `resume` may be called multiple times | Continuation cloning |
| Shallow | `resume` returns new handler | CPS transformation |
| Tail-resumptive | `resume` is last operation | Loop optimization |

**Tail-Resumptive Optimization**: When a handler always resumes as its final action, it compiles to a loop rather than continuation capture:

```blood
// Tail-resumptive handler
deep handler CountOps for Log {
    let mut count: usize = 0
    return(x) { (x, count) }
    op log(level, msg, _) {
        count += 1
        resume(())  // Tail position
    }
}

// Compiles to (conceptual)
fn count_ops<T>(computation: fn() -> T) -> (T, usize) {
    let mut count = 0;
    loop {
        match computation.run_until_effect() {
            Complete(x) => return (x, count),
            Effect::Log(_, _, _, k) => {
                count += 1;
                computation = k(());  // Continue without stack growth
            }
        }
    }
}
```

### 13.2 Memory Management Implementation

#### 13.2.1 Generational Reference Layout

The 128-bit generational pointer format:

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│ Bit 127                                                                   Bit 0 │
├────────────────────────────────────────────┬──────────────────┬─────────────────┤
│            Address (64 bits)               │  Generation (32) │  Metadata (32)  │
│                                            │                  │                 │
│  Virtual memory address                    │  Monotonic       │  Tier (2 bits)  │
│  (NULL = 0x0000000000000000)              │  counter         │  Flags (14 bits)│
│                                            │                  │  Type FP (16b)  │
└────────────────────────────────────────────┴──────────────────┴─────────────────┘
```

**Generation Check Cost**: On modern x86-64, the generation check adds approximately 2-3 cycles per dereference (single cache line, branch prediction favorable).

#### 13.2.2 Slot Allocator Design

Each memory tier uses optimized allocators:

| Tier | Allocator | Generation Increment | Deallocation |
|------|-----------|---------------------|--------------|
| Tier 0 (Stack) | Stack pointer | N/A (no references) | Automatic (scope exit) |
| Tier 1 (Region) | Bump allocator per region | Per-slot | Region release |
| Tier 2 (Persistent) | System allocator + RC | Per-slot | RC zero |

**Slot Structure**:

```rust
struct Slot {
    generation: AtomicU32,  // Current generation
    data: MaybeUninit<T>,   // Payload
}
```

#### 13.2.3 Collection Memory Patterns

**Vec Growth**: Geometric (2x) growth with configurable initial capacity:

```
Capacity: 0 → 4 → 8 → 16 → 32 → 64 → 128 → ...
```

**HashMap Probe Sequence**: Quadratic probing with SIMD-accelerated control byte scanning:

```
probe(i) = (h + i * (i + 1) / 2) mod capacity
```

**BTreeMap Node Size**: Targeting cache-line alignment (6-12 keys per node on 64-byte cache lines).

### 13.3 Effect Safety Invariants

#### 13.3.1 Generation Snapshot Invariants

The runtime maintains these invariants for generation snapshots:

1. **Capture Completeness**: Snapshot includes all generational references in continuation
2. **Validation Atomicity**: All generations checked before any continuation code runs
3. **Monotonicity**: Generations only increase; snapshot gen ≤ current gen always

#### 13.3.2 Linear Type Enforcement

Linear values have additional compile-time tracking:

```blood
// Linear resource must be used exactly once
fn process_file(file: File @linear) / {IO, Error<IoError>} {
    // Compiler tracks: file must be consumed before function returns
    let data = file.read_all()?
    file.close()  // Consumes file - this is required
    data
}
```

**Compilation Strategy**: Linear values tracked through abstract interpretation; error if any path drops without use or uses more than once.

### 13.4 Standard Handler Implementation Patterns

#### 13.4.1 State Handler Memory

The `LocalState<S>` handler stores state in handler activation:

```rust
struct LocalStateHandler<S> {
    state: S,
    // No heap allocation for small S (stack-allocated)
}
```

For large state, consider `Box<S>` or passing by reference.

#### 13.4.2 Async Handler Internals

The async runtime uses:

- **Task Queue**: Lock-free MPSC queue for ready tasks
- **Wait Set**: HashMap<TaskId, (Future, Continuation)> for blocked tasks
- **Executor**: Single-threaded event loop (multi-threaded optional)

**Task State Machine**:

```
Created → Ready ⟷ Running → Completed
              ↘ Waiting ↗
```

#### 13.4.3 NonDet Handler Search Strategies

| Handler | Strategy | Memory | Use Case |
|---------|----------|--------|----------|
| `FirstSolution` | DFS | O(depth) stack | Find any solution |
| `AllSolutions` | DFS + collection | O(solutions) | Enumerate all |
| `BoundedSearch` | DFS + depth limit | O(max_depth) | Prevent infinite |
| `MonteCarloSampler` | Random | O(1) | Probabilistic |

### 13.5 FFI Integration

For C interop, effects must be explicitly handled at the boundary:

```blood
#[ffi]
extern "C" fn callback_wrapper(
    user_fn: fn(i32) -> i32 / {Error<E>},
    arg: i32
) -> i32 {
    // Must handle Error effect before crossing FFI boundary
    with PropagateError handle {
        match user_fn(arg) {
            Ok(v) => v,
            Err(_) => -1,  // Error code for C
        }
    }
}
```

See [FFI.md](./FFI.md) for complete FFI specification.

### 13.6 Performance Guidelines

#### 13.6.1 Effect Overhead Guidelines

| Scenario | Overhead | Mitigation |
|----------|----------|------------|
| Monomorphic effects | ~0% | Full specialization |
| Polymorphic (known at compile) | ~5-10% | Evidence inlining |
| Polymorphic (unknown) | ~15-30% | Evidence dictionary |
| Multi-shot handlers | O(continuation size) | Avoid in hot paths |

#### 13.6.2 Collection Selection Guide

| Need | Use | Rationale |
|------|-----|-----------|
| Random access | `Vec<T>` | O(1) indexing, cache-friendly |
| Key-value lookup | `HashMap<K, V>` | O(1) expected |
| Ordered iteration | `BTreeMap<K, V>` | O(log n) ops, sorted keys |
| FIFO queue | `VecDeque<T>` | O(1) both ends |
| Priority queue | `BinaryHeap<T>` | O(log n) push/pop |
| Set membership | `HashSet<T>` | O(1) expected |
| Ordered set ops | `BTreeSet<T>` | Efficient range queries |

#### 13.6.3 Memory Tier Selection

```
┌─────────────────────────────────────────────────────────────────┐
│ Does value escape current scope?                                │
│   NO → Tier 0 (Stack)                                           │
│   YES ↓                                                         │
├─────────────────────────────────────────────────────────────────┤
│ Does value escape current region?                               │
│   NO → Tier 1 (Region) with generational references            │
│   YES ↓                                                         │
├─────────────────────────────────────────────────────────────────┤
│ Is value shared across fibers/threads?                          │
│   NO → Tier 1 (Region) with promotion                          │
│   YES → Tier 2 (Persistent) with reference counting            │
└─────────────────────────────────────────────────────────────────┘
```

---

## Appendix B: Collection Selection Quick Reference

| If you need... | Use | Not |
|----------------|-----|-----|
| A growable array | `Vec<T>` | Array |
| A stack | `Vec<T>` with `push`/`pop` | VecDeque |
| A queue | `VecDeque<T>` | Vec |
| A set | `HashSet<T>` (unordered) or `BTreeSet<T>` (ordered) | Vec with dedup |
| A map | `HashMap<K, V>` (unordered) or `BTreeMap<K, V>` (ordered) | Vec of tuples |
| Sorted data with range queries | `BTreeMap`/`BTreeSet` | HashMap/HashSet |
| Maximum/minimum tracking | `BinaryHeap<T>` | Sorted Vec |
| Double-ended operations | `VecDeque<T>` | Vec |

---

*Last updated: 2026-01-09*
