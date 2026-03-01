# Blood Self-Hosting: Minimal Subset Analysis

**Document Version**: 1.0
**Status**: SELF-001 Completion
**Created**: 2026-01-13

---

## Overview

This document identifies the minimal subset of Blood language features required to implement a self-hosted compiler. Self-hosting is a critical milestone demonstrating language maturity and eating our own dog food.

---

## 1. Compiler Component Analysis

A compiler has four major phases, each with distinct language feature requirements:

### 1.1 Lexer Requirements

| Feature | Priority | Reason |
|---------|----------|--------|
| `char` type | P0 | Character-by-character tokenization |
| `str` type | P0 | Source input handling |
| `String` type | P0 | Token string building |
| Enums with variants | P0 | Token types (Ident, Number, Keyword, etc.) |
| Pattern matching | P0 | Character classification |
| `while` loops | P0 | Input consumption |
| Option<T> | P0 | Peek-ahead in lexer |
| Integer types (i32, usize) | P0 | Position tracking, line/column |
| Structs | P0 | Token struct with span, kind, value |
| Vec<Token> | P1 | Token stream output |

**Lexer complexity**: Low to Medium
- Mostly string/char manipulation
- Finite state machine patterns
- No complex type relationships

### 1.2 Parser Requirements

| Feature | Priority | Reason |
|---------|----------|--------|
| Algebraic Data Types | P0 | AST node types (Expr, Stmt, Type, etc.) |
| Recursive types | P0 | AST is inherently recursive |
| Box<T> | P0 | Heap allocation for recursive structures |
| Vec<T> | P0 | Lists of children (args, params, items) |
| Pattern matching (deep) | P0 | Match on token kinds, AST variants |
| Option<T> | P0 | Optional syntax elements |
| Result<T, E> | P0 | Parse errors |
| Generic types | P1 | Parameterized AST nodes |
| Trait methods | P1 | Display for AST nodes |

**Parser complexity**: Medium
- Recursive descent parsing
- Lookahead and backtracking
- Error recovery patterns

### 1.3 Type Checker Requirements

| Feature | Priority | Reason |
|---------|----------|--------|
| HashMap<K, V> | P0 | Symbol tables, type environments |
| Generics (polymorphism) | P0 | Type variables, instantiation |
| Associated types | P1 | Type families |
| Trait bounds | P1 | Generic constraints |
| Complex enums | P0 | Type representation (Fn, Tuple, Named, etc.) |
| Mutable references | P0 | Context mutation during type checking |
| Clone trait | P1 | Type copying during unification |
| Eq/PartialEq | P1 | Type comparison |
| Effect system | P0 | Track effect propagation |
| Row types | P2 | Effect row polymorphism |

**Type checker complexity**: High
- Most complex compiler phase
- Unification algorithm
- Constraint solving
- Effect inference

### 1.4 Code Generator Requirements

| Feature | Priority | Reason |
|---------|----------|--------|
| String building | P0 | Output code generation |
| File I/O | P0 | Write output files |
| HashMap | P1 | Symbol to register/offset mapping |
| Closures | P1 | Visitor patterns |
| Formatting traits | P2 | Debug output |

**Code generator complexity**: Medium to High
- Tree traversal
- Register allocation (if generating native code)
- String manipulation for output

---

## 2. Minimal Subset Definition

### 2.1 P0 Features (Absolutely Required)

These features MUST be working for any self-hosting attempt:

```blood
// === Primitive Types ===
i8, i16, i32, i64, isize
u8, u16, u32, u64, usize
f32, f64
bool
char
unit

// === String Types ===
str     // String slice
String  // Owned string

// === Composite Types ===
[T; N]           // Fixed arrays
[T]              // Slices
(T1, T2, ...)    // Tuples

// === User-Defined Types ===
struct Name { field: Type }
enum Name { Variant1, Variant2(T), ... }

// === Smart Pointers ===
Box<T>           // Heap allocation
Option<T>        // Optional values
Result<T, E>     // Error handling

// === Collections ===
Vec<T>           // Dynamic arrays
HashMap<K, V>    // Hash maps

// === References ===
&T               // Immutable borrow
&mut T           // Mutable borrow

// === Control Flow ===
if/else
match            // Pattern matching
while
for              // Iterator-based
loop
break/continue
return

// === Functions ===
fn name(params) -> ReturnType { body }
fn name<T>(x: T) -> T   // Generics

// === Effects ===
effect Error<E> { throw(e: E) -> never }
handle expr { ... }
perform Effect.op(args)
```

### 2.2 P1 Features (Highly Desirable)

These features significantly simplify compiler implementation:

```blood
// === Traits ===
trait Name {
    fn method(&self) -> T;
}
impl Name for Type { ... }

// === Closures ===
|x| x + 1
|x: i32| -> i32 { x + 1 }

// === Method Syntax ===
value.method(args)

// === Operators ===
impl Add for Type { ... }
impl Index for Type { ... }

// === Iteration ===
trait Iterator {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;
}

// === Derive Macros ===
#[derive(Clone, Debug, PartialEq)]
```

### 2.3 P2 Features (Nice to Have)

These can be implemented later or worked around:

```blood
// === Advanced Generics ===
where clauses
associated types
higher-kinded types

// === Effect Polymorphism ===
fn map<E>(f: fn(A) -> B / E) -> C / E

// === Macros ===
macro_rules!

// === Fiber/Concurrency ===
fiber fn
spawn
channels
```

---

## 3. Standard Library Requirements

The self-hosted compiler needs these stdlib modules:

### 3.1 Core (P0)

```
std::core::option     - Option<T>
std::core::result     - Result<T, E>
std::core::string     - String
std::core::vec        - Vec<T>
std::core::hash       - HashMap<K, V>
std::core::fmt        - Display trait
```

### 3.2 I/O (P0)

```
std::fs::read_to_string(path) -> Result<String, IoError>
std::fs::write(path, content) -> Result<(), IoError>
std::io::stdin/stdout
```

### 3.3 Collections (P1)

```
std::collections::HashSet<T>
std::collections::BTreeMap<K, V>  (for ordered iteration)
```

---

## 4. Feature Gap Analysis

### 4.1 Currently Implemented ✅

Based on SPECIFICATION.md and stdlib:

| Feature | Status | Location |
|---------|--------|----------|
| Primitive types | ✅ Implemented | Core language |
| Structs | ✅ Implemented | Core language |
| Enums | ✅ Implemented | Core language |
| Generics | ✅ Implemented | typeck/context |
| Pattern matching | ✅ Implemented | parser, typeck |
| Option<T> | ✅ Implemented | stdlib/core/option.blood |
| Result<T, E> | ✅ Implemented | stdlib/core/result.blood |
| Vec<T> | ✅ Implemented | stdlib/collections/vec.blood |
| HashMap<K, V> | ✅ Implemented | stdlib/collections/hash_map.blood |
| String | ✅ Implemented | stdlib/core/string.blood |
| Closures | ✅ Implemented | typeck/context/closure.rs |
| Effects | ✅ Implemented | effects/ module |
| Traits | ✅ Implemented | typeck/context/traits.rs |
| File I/O | ✅ Implemented | stdlib/fs/mod.blood |

### 4.2 Gaps to Address

| Feature | Status | Priority | Notes |
|---------|--------|----------|-------|
| Derive macros | 🔴 Missing | P2 | Can hand-implement for now |
| String interpolation | 🔴 Missing | P2 | Use format functions |
| Standard prelude | 🟡 Partial | P1 | Need auto-imports |
| Iterator adapters | 🟡 Partial | P1 | map, filter, collect |
| Debug formatting | 🟡 Partial | P1 | Debug trait |
| Comprehensive tests | 🟡 Partial | P1 | STD-006 addresses this |

### 4.3 Blood vs Rust Feature Comparison

| Rust Feature | Blood Equivalent | Status |
|--------------|------------------|--------|
| `String::new()` | `String::new()` | ✅ |
| `Vec::push()` | `Vec::push()` | ✅ |
| `HashMap::insert()` | `HashMap::insert()` | ✅ |
| `Option::map()` | `Option::map()` | ✅ |
| `Result::map_err()` | `Result::map_err()` | ✅ |
| `Box::new()` | `@heap expr` | ✅ Different syntax |
| `impl Trait for Type` | `impl Trait for Type` | ✅ |
| `#[derive(Debug)]` | Manual impl | 🟡 |
| `println!()` | `println_str()` | 🟡 No format macro |
| `format!()` | Manual building | 🟡 |
| `?` operator | Effect handlers | ✅ More powerful |
| `match` | `match` | ✅ |
| Iterators | Iterators | ✅ |

---

## 5. Bootstrap Strategy

### 5.1 Recommended Approach

**Phase 1: Blood-in-Rust Compiler (Current)**
- Complete Rust implementation
- Comprehensive test suite
- Stable language semantics

**Phase 2: Blood Lexer in Blood**
- Implement `src/selfhost/lexer.blood`
- Test against Rust lexer output
- SELF-002 milestone

**Phase 3: Blood Parser in Blood**
- Implement `src/selfhost/parser.blood`
- Depend on Blood lexer
- SELF-003 milestone

**Phase 4: Blood Type Checker in Blood**
- Most complex phase
- Implement `src/selfhost/typeck.blood`
- SELF-004 milestone

**Phase 5: Full Bootstrap**
- Compile Blood compiler with Blood compiler
- SELF-005 milestone
- Verify output matches Rust compiler output

### 5.2 Subset Expansion Path

```
Stage 1 (Lexer):
  - char, str, String
  - i32, bool
  - Option, Vec
  - Enums, structs
  - Pattern matching
  - Loops, conditionals

Stage 2 (Parser):
  - Box<T> (recursive types)
  - Result<T, E>
  - Generic types
  - Complex pattern matching

Stage 3 (Type Checker):
  - HashMap
  - Traits
  - Effect system
  - Generic bounds
  - Type inference

Stage 4 (Codegen):
  - File I/O
  - String formatting
  - Full effects
```

### 5.3 Testing Strategy

Each stage must:
1. Parse/check itself successfully
2. Match output of Rust implementation
3. Pass all existing test suites
4. Handle edge cases and errors gracefully

---

## 6. Estimated Complexity

### 6.1 Lines of Code Estimate

Based on current Rust implementation:

| Component | Rust LoC | Estimated Blood LoC | Ratio |
|-----------|----------|---------------------|-------|
| Lexer | ~1,000 | ~1,200 | 1.2x |
| Parser | ~3,500 | ~4,000 | 1.1x |
| AST | ~1,500 | ~1,800 | 1.2x |
| HIR | ~2,000 | ~2,400 | 1.2x |
| Type Checker | ~8,000 | ~10,000 | 1.25x |
| Codegen | ~5,000 | ~6,000 | 1.2x |
| **Total** | **~21,000** | **~25,400** | **1.2x** |

Note: Blood code may be slightly more verbose due to explicit effect annotations, but the effect system simplifies error handling.

### 6.2 Key Challenges

1. **Recursive Types**: AST nodes require heap allocation (Box<T>)
2. **Type Inference**: Complex algorithm implementation
3. **Pattern Matching**: Exhaustiveness checking
4. **Effect Inference**: Row polymorphism implementation
5. **Code Generation**: Output format decisions

---

## 7. Recommendations

### 7.1 Immediate Actions

1. **STD-006**: Add comprehensive stdlib tests (validates foundations)
2. **Test coverage**: Ensure Vec, HashMap, String, Option, Result are bulletproof
3. **Iterator completion**: Ensure map/filter/collect work properly

### 7.2 Before Starting Self-Hosting

- [ ] All P0 stdlib types have >90% test coverage
- [ ] File I/O operations fully working
- [ ] String manipulation comprehensive
- [ ] Pattern matching exhaustiveness verified
- [ ] Effect handlers working for error propagation

### 7.3 Success Criteria for SELF-005

The self-hosted compiler is complete when:
1. It can compile itself
2. The output is functionally equivalent to Rust compiler output
3. It passes the full test suite
4. Performance is within 2x of Rust implementation

---

## 8. Conclusion

Blood has implemented the core features needed for self-hosting. The main gaps are:

1. **Testing depth** (STD-006 addresses this)
2. **Standard prelude** (auto-imports)
3. **Derive macros** (can be worked around)

The recommended path is:
1. Complete STD-006 (stdlib tests)
2. Implement lexer in Blood (SELF-002)
3. Incrementally build up to full bootstrap

**Assessment**: Blood is approximately **80% ready** for self-hosting attempt, with the remaining 20% being test coverage and polish.

---

*This document fulfills ACTION_ITEM SELF-001: Identify minimal Blood subset for self-hosting.*
