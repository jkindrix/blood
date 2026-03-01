# Design Evaluation: `impl Trait` in Blood

**Version**: 0.1.0
**Status**: Design evaluation (pre-RFC)
**Last Updated**: 2026-02-28
**Referenced from**: `docs/spec/GRAMMAR.md` Section 4.1 design note

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [What `impl Trait` Does in Rust](#2-what-impl-trait-does-in-rust)
   - 2.1 [Argument Position (APIT)](#21-argument-position-impl-trait-apit)
   - 2.2 [Return Position (RPIT)](#22-return-position-impl-trait-rpit)
   - 2.3 [Return Position in Trait Definitions (RPITIT)](#23-return-position-impl-trait-in-trait-definitions-rpitit)
   - 2.4 [Type Alias Impl Trait (TAIT)](#24-type-alias-impl-trait-tait)
   - 2.5 [Summary of Positions and Semantics](#25-summary-of-positions-and-semantics)
3. [How Blood's Multiple Dispatch Changes the Equation](#3-how-bloods-multiple-dispatch-changes-the-equation)
   - 3.1 [Argument Position: Dispatch Subsumes APIT](#31-argument-position-dispatch-subsumes-apit)
   - 3.2 [Return Position: Dispatch Does Not Subsume RPIT](#32-return-position-dispatch-does-not-subsume-rpit)
   - 3.3 [Type Stability Constraint](#33-type-stability-constraint)
4. [`dyn Trait` vs `impl Trait`](#4-dyn-trait-vs-impl-trait)
   - 4.1 [Fundamental Difference](#41-fundamental-difference)
   - 4.2 [When `dyn Trait` Cannot Replace `impl Trait`](#42-when-dyn-trait-cannot-replace-impl-trait)
   - 4.3 [Swift's `some` vs `any` as Precedent](#43-swifts-some-vs-any-as-precedent)
5. [Alternative Approaches](#5-alternative-approaches)
   - 5.1 [Type Inference for the Argument-Position Case](#51-type-inference-for-the-argument-position-case)
   - 5.2 [Explicit Generics for the Return-Position Case](#52-explicit-generics-for-the-return-position-case)
   - 5.3 [Named Existential Types (Type Aliases)](#53-named-existential-types-type-aliases)
   - 5.4 [How Multiple-Dispatch Languages Handle This](#54-how-multiple-dispatch-languages-handle-this)
6. [Interaction with Blood's Effect System](#6-interaction-with-bloods-effect-system)
   - 6.1 [Effect-Polymorphic Return Types](#61-effect-polymorphic-return-types)
   - 6.2 [Existential Effects](#62-existential-effects)
   - 6.3 [`impl Effect` as a Concept](#63-impl-effect-as-a-concept)
   - 6.4 [Fiber Desugaring Analogy](#64-fiber-desugaring-analogy)
7. [The Case FOR Adding `impl Trait`](#7-the-case-for-adding-impl-trait)
8. [The Case AGAINST Adding `impl Trait`](#8-the-case-against-adding-impl-trait)
9. [Recommendation](#9-recommendation)
10. [If Blood Does Add It: Design Constraints](#10-if-blood-does-add-it-design-constraints)

---

## 1. Executive Summary

This document evaluates whether Blood needs `impl Trait` syntax, given that Blood already has multiple dispatch, `dyn Trait`, `forall` types, and an algebraic effect system with row polymorphism.

**Conclusion**: Blood does **not** need `impl Trait` in argument position. Blood **may** benefit from a return-position opaque type mechanism, but the need is significantly reduced compared to Rust, and the mechanism should be designed differently if adopted. The recommendation is to **defer** return-position opaque types until real-world usage reveals concrete pain points, and to **reject** argument-position `impl Trait` permanently.

**Key findings**:

| Position | Rust Motivation | Blood Alternative | Need? |
|----------|----------------|-------------------|-------|
| Argument (APIT) | Syntactic sugar for generics | Multiple dispatch + explicit generics | **No** |
| Return (RPIT) | Unnameable types (closures, iterators) | Explicit generics + type aliases | **Maybe later** |
| Return in traits (RPITIT) | Associated type ergonomics | Explicit associated types | **Maybe later** |
| Type alias (TAIT) | Named existential types | Type aliases with concrete types | **Maybe later** |

---

## 2. What `impl Trait` Does in Rust

### 2.1 Argument Position Impl Trait (APIT)

In argument position, `impl Trait` is **syntactic sugar for an anonymous generic type parameter**:

```rust
// These are equivalent in Rust:
fn print_it(x: impl Display) { println!("{}", x); }
fn print_it<T: Display>(x: T) { println!("{}", x); }
```

The caller chooses the concrete type. Each call site gets a monomorphized copy. This is **universal quantification** — "for any type T that implements Display."

**What problem it solves**: Ergonomics. Writing `impl Display` is shorter than `<T: Display>(x: T)`, especially when the type parameter is used only once and does not need to be named.

**Key behavioral difference from explicit generics**: When using `impl Trait`, the caller cannot explicitly specify the type parameter at the call site. With `<T: Display>`, the caller could write `print_it::<String>(s)`. This distinction is minor in practice.

**Important subtlety**: Two `impl Trait` parameters create two *independent* type parameters:

```rust
fn add(a: impl Numeric, b: impl Numeric) -> ???
// Desugars to: fn add<T1: Numeric, T2: Numeric>(a: T1, b: T2) -> ???
// NOT: fn add<T: Numeric>(a: T, b: T) -> ???
```

### 2.2 Return Position Impl Trait (RPIT)

In return position, `impl Trait` denotes an **opaque type** — the function chooses a single concrete type, but the caller sees only the trait bound:

```rust
fn make_counter() -> impl Iterator<Item = i32> {
    (0..).filter(|n| n % 2 == 0)
}
```

This is **existential quantification** — "there exists some type T implementing Iterator<Item = i32>, and the function returns that T." The compiler knows the concrete type, but the caller cannot name it or depend on it.

**What problems it solves**:

1. **Unnameable types**: Closures and iterator adaptor chains produce types that cannot be written in source code (e.g., `Filter<RangeFrom<i32>, [closure@src/lib.rs:2:25]>`). Without `impl Trait`, returning these requires `Box<dyn Iterator<Item = i32>>` — paying heap allocation and dynamic dispatch costs for what should be a zero-cost abstraction.

2. **API stability**: Library authors can change the concrete iterator chain without breaking callers, since callers only see `impl Iterator`.

3. **Static dispatch preservation**: Unlike `dyn Trait`, `impl Trait` in return position preserves monomorphization — the compiler knows the concrete type and can inline, optimize, and avoid vtable indirection.

### 2.3 Return Position Impl Trait in Trait Definitions (RPITIT)

RPITIT allows trait methods to use `impl Trait` in their return types:

```rust
trait IntoIterator {
    type Item;
    fn into_iter(self) -> impl Iterator<Item = Self::Item>;
}
```

This desugars to an **anonymous associated type**. Each impl provides its own concrete type, but the trait definition does not need to name it.

**What problem it solves**: Without RPITIT, every trait that returns an iterator, future, or closure must declare an explicit associated type:

```rust
// Without RPITIT — verbose and leaky
trait IntoIterator {
    type Item;
    type IntoIter: Iterator<Item = Self::Item>;
    fn into_iter(self) -> Self::IntoIter;
}
```

RPITIT is primarily an **ergonomic** improvement. The underlying mechanism (associated types) already exists.

### 2.4 Type Alias Impl Trait (TAIT)

TAIT allows naming opaque types via type aliases:

```rust
type Counter = impl Iterator<Item = i32>;

fn make_counter() -> Counter {
    (0..).filter(|n| n % 2 == 0)
}
```

This is still unstable in Rust as of 2025, with ongoing design debates about defining scopes and inference rules.

**What problem it solves**: RPIT types cannot be named or stored in struct fields. TAIT bridges that gap.

### 2.5 Summary of Positions and Semantics

| Position | Quantification | Who Chooses Type? | Dispatch | Rust Status |
|----------|---------------|-------------------|----------|-------------|
| Argument (APIT) | Universal (forall) | Caller | Static (monomorphized) | Stable |
| Return (RPIT) | Existential (exists) | Callee | Static (monomorphized) | Stable |
| Return in trait (RPITIT) | Existential per impl | Implementor | Static | Stable (1.75+) |
| Type alias (TAIT) | Existential (named) | Defining function | Static | Unstable |

---

## 3. How Blood's Multiple Dispatch Changes the Equation

### 3.1 Argument Position: Dispatch Subsumes APIT

In Rust, APIT is sugar for a generic. In Blood, **multiple dispatch already provides the mechanism** that APIT was designed to sugar over — and more.

Consider the Rust motivation:

```rust
// Rust: "I want a function that accepts anything Printable"
fn render(item: impl Printable) { item.print(); }
```

In Blood, this is expressed naturally through either generics or dispatch:

```blood
// Option A: Explicit generic (already supported)
fn render<T: Printable>(item: T) { item.print(); }

// Option B: Multiple dispatch (Blood's native mechanism)
fn render(item: String) { print(item); }
fn render(item: i32) { print(to_string(item)); }
fn render<T: Printable>(item: T) { item.print(); }
```

Multiple dispatch goes **further** than APIT because it allows different implementations per type, not just different monomorphizations of the same body. APIT hides the type parameter for convenience; multiple dispatch makes type-specific behavior a first-class design pattern.

**The APIT "independent parameters" problem vanishes**: In Rust, `fn f(a: impl T, b: impl T)` creates two independent type parameters, which surprises users who expect `a` and `b` to have the same type. In Blood, explicit generics make this clear: `fn f<T: Trait>(a: T, b: T)` vs `fn f<A: Trait, B: Trait>(a: A, b: B)`.

**Verdict**: APIT is unnecessary in Blood. Multiple dispatch and explicit generics cover all use cases with more clarity.

### 3.2 Return Position: Dispatch Does Not Subsume RPIT

Multiple dispatch selects *which function to call* based on argument types. It does not help with *hiding the return type* of a function. These are orthogonal concerns.

The core RPIT use case — returning a closure or iterator chain without naming the type — exists independently of dispatch:

```blood
// Problem: What is the return type of this function?
fn make_filter<T: Eq>(target: T) -> ??? {
    |x: T| -> bool { x == target }
}
```

Blood currently handles this with explicit types or `dyn Trait`:

```blood
// Option A: Explicit function pointer (loses closure state)
fn make_filter<T: Eq>(target: T) -> fn(T) -> bool { ... }

// Option B: dyn Trait (heap allocation + vtable)
fn make_filter<T: Eq>(target: T) -> &dyn Fn(T) -> bool { ... }
```

Neither option achieves the zero-cost abstraction that RPIT provides.

**Verdict**: Multiple dispatch does not address the return-position problem. RPIT-like functionality would need a different mechanism.

### 3.3 Type Stability Constraint

Blood's type stability requirement (DISPATCH.md Section 4) adds an important constraint: **the return type of a function must be fully determined by its parameter types at compile time.**

This is actually *aligned* with RPIT's semantics. An RPIT return type is a single concrete type determined by the function body — it satisfies type stability trivially, since the concrete type is fixed for each monomorphization.

However, type stability means that Blood **cannot** have a function that returns different `impl Trait` types from different branches:

```blood
// REJECTED by type stability: different concrete types per branch
fn make_iter(ascending: bool) -> impl Iterator<Item = i32> {
    if ascending { 0..100 }
    else { (0..100).rev() }  // Different concrete type!
}
```

This is already how RPIT works in Rust — the function must return the same concrete type from all branches. Blood's type stability rule is a natural fit.

---

## 4. `dyn Trait` vs `impl Trait`

### 4.1 Fundamental Difference

| Property | `dyn Trait` | `impl Trait` |
|----------|-------------|--------------|
| **Dispatch** | Dynamic (vtable) | Static (monomorphized) |
| **Memory** | Fat pointer (data + vtable) | Sized, stack-allocated |
| **Type erasure** | Full — concrete type unknown at compile time | Partial — compiler knows concrete type, caller does not |
| **Heterogeneous collections** | Yes — `Vec<&dyn Trait>` works | No — each element same concrete type |
| **Performance** | Vtable indirection, no inlining | Zero overhead, inlinable |
| **Object safety required** | Yes | No |

### 4.2 When `dyn Trait` Cannot Replace `impl Trait`

Blood already has `dyn Trait` in its grammar. But `dyn Trait` is not a substitute for `impl Trait` in several important cases:

**Case 1: Performance-sensitive return types**

```blood
// dyn Trait: heap allocation + vtable lookup on every call
fn make_hasher() -> &dyn Hasher { ... }

// impl Trait (hypothetical): zero-cost, inlineable
fn make_hasher() -> impl Hasher { ... }
```

In hot paths (iterators, numerical computations, codegen), the vtable overhead of `dyn Trait` is unacceptable. This is precisely why Rust added `impl Trait` — to avoid paying for dynamic dispatch when static dispatch suffices.

**Case 2: Non-object-safe traits**

Traits with associated types, generic methods, or `Self` in return position cannot be used as `dyn Trait`:

```blood
trait Serializer {
    type Output;
    fn serialize<T>(value: T) -> Self::Output;  // Generic method
}

// INVALID: dyn Serializer is not object-safe
fn get_serializer() -> &dyn Serializer { ... }

// With impl Trait (hypothetical): would work
fn get_serializer() -> impl Serializer { ... }
```

**Case 3: Sized return types**

`dyn Trait` is unsized — it must always be behind a pointer (`&dyn`, `Box<dyn>`). `impl Trait` is sized and can be returned by value, stored inline in structs, and passed without indirection.

### 4.3 Swift's `some` vs `any` as Precedent

Swift faced the same design question and introduced two keywords:
- `some Protocol` — opaque type (like `impl Trait`), static dispatch, compiler knows concrete type
- `any Protocol` — existential type (like `dyn Trait`), dynamic dispatch, type-erased

Swift's design guidance: "Write `some` by default, change to `any` when you need storage flexibility." This reflects the principle that static dispatch should be the default and dynamic dispatch an explicit opt-in.

Blood's `dyn Trait` corresponds to Swift's `any`. The question is whether Blood needs a `some`-equivalent.

---

## 5. Alternative Approaches

### 5.1 Type Inference for the Argument-Position Case

Blood's type inference already handles the argument-position case without special syntax:

```blood
// Blood: the compiler infers T from the call site
fn render<T: Printable>(item: T) { item.print(); }

render("hello");  // T = String
render(42);       // T = i32
```

No additional syntax is needed. The explicitness of naming `T` is a feature, not a bug — it makes the code clearer and avoids the APIT "independent parameters" surprise.

### 5.2 Explicit Generics for the Return-Position Case

For many RPIT use cases, explicit generics with associated types work:

```blood
trait Container {
    type Iter: Iterator
    fn iter(&self) -> Self.Iter
}
```

The associated type `Iter` is explicitly named, which is more verbose but also more transparent than an anonymous `impl Iterator`.

**Where this breaks down**: When the return type genuinely cannot be named — closures that capture local state, deeply nested iterator chains, or compiler-generated state machines.

### 5.3 Named Existential Types (Type Aliases)

An alternative to `impl Trait` is allowing type aliases to hide their definition:

```blood
// Hypothetical: opaque type alias
type MyIter = opaque Iterator<Item = i32>

fn make_iter() -> MyIter {
    (0..100).filter(|n| n % 2 == 0)
}
```

This is essentially TAIT (Rust's still-unstable feature) and avoids the syntactic overloading of `impl` in type position. It has the advantage of being explicit and nameable, while still providing type hiding.

### 5.4 How Multiple-Dispatch Languages Handle This

**Julia**: Does not have opaque return types or `impl Trait`. Julia relies on:
- Type inference to determine return types
- Abstract types for parameter constraints (but not return type abstraction)
- Type stability as a performance guideline, not a compiler-enforced rule

Julia's approach works because it is dynamically typed with JIT compilation — the JIT specializes at runtime based on observed types. Blood cannot rely on this because it is ahead-of-time compiled.

**Common Lisp (CLOS)**: Has no static type system. Multiple dispatch operates entirely at runtime. The question of opaque return types does not arise because there are no compile-time type signatures.

**Dylan**: Also dynamically typed with multiple dispatch. No concept of opaque return types. Dylan's type system is optional and primarily for documentation/optimization hints, not abstraction.

**Observation**: No existing multiple-dispatch language has `impl Trait` because:
1. They are dynamically typed (Julia, CLOS, Dylan) — the problem does not arise
2. They use JIT compilation — type specialization happens at runtime

Blood is unique in combining multiple dispatch with a static type system and AOT compilation, placing it in uncharted territory. The closest analogue is Rust (static types + traits + AOT), and Rust needed `impl Trait`.

---

## 6. Interaction with Blood's Effect System

### 6.1 Effect-Polymorphic Return Types

Blood's effect system introduces a dimension that Rust does not have. Consider:

```blood
fn transform<A, B, ε>(f: fn(A) -> B / ε, x: A) -> B / ε {
    f(x)
}
```

The effect row `ε` is already polymorphic — it is universally quantified over possible effect rows. This is analogous to `impl Trait` in argument position but for effects.

Blood handles this natively through row polymorphism without needing `impl`-style syntax.

### 6.2 Existential Effects

The research literature identifies a meaningful concept of **existential effects** — hiding which specific effect a computation uses. Biernacki et al. (POPL 2019) formalize this in the λHEL calculus, which provides:

- **Existential effects**: Hide the details of an effect from the caller
- **Local effects**: Guarantee that external code cannot interfere with an effect

In Blood's terms, this would look like:

```blood
// Hypothetical: existential effect — caller knows there IS a state effect,
// but cannot interact with it directly
fn with_hidden_state(f: fn() -> i32 / {exists S. State<S>}) -> i32 {
    with LocalState { state: 0 } handle { f() }
}
```

This is a genuinely novel capability that `impl Trait` could enable for effects. However:
- Blood already handles effect abstraction through handlers (the handler encapsulates the effect implementation)
- Row polymorphism with `| ε` already provides effect abstraction at the type level
- The λHEL-style existential effects are a research topic, not a proven production feature

### 6.3 `impl Effect` as a Concept

Could `impl Effect` be meaningful? Consider:

```blood
// Hypothetical: "returns a computation that performs some logging-like effect"
fn make_logger() -> fn(String) -> () / {impl Log} { ... }
```

This would mean "the returned function performs some effect that satisfies the Log interface, but the caller does not know which specific Log implementation." This is coherent in theory but of limited practical value because:

1. Effect handlers already provide this abstraction — the handler determines the effect's semantics
2. Row polymorphism (`/ {Log | ε}`) provides the flexibility to compose effects
3. The caller needs to *handle* the effect regardless, so hiding which effect it is creates more problems than it solves

**Verdict**: `impl Effect` is theoretically coherent but practically unnecessary given Blood's handler mechanism.

### 6.4 Fiber Desugaring Analogy

In Rust, `async fn` desugars to a function returning `impl Future`. This is the single most common use of RPIT:

```rust
// Rust: these are equivalent
async fn fetch(url: &str) -> Data { ... }
fn fetch(url: &str) -> impl Future<Output = Data> { ... }
```

Blood does not need this because it models concurrency as a Fiber effect:

```blood
// Blood: concurrency is a Fiber effect, not an opaque return type
fn fetch(url: &str) -> Data / {Fiber} { ... }
```

The `/ {Fiber}` effect annotation replaces what Rust needs `impl Future` for. The effect handler provides the runtime implementation (event loop, thread pool, etc.) without requiring an opaque type.

This is a significant advantage of Blood's design — it eliminates what is arguably the largest driver of `impl Trait` adoption in Rust.

---

## 7. The Case FOR Adding `impl Trait`

### 7.1 Unnameable Return Types

Some types genuinely cannot be written in source code:
- Closures with captured state
- Iterator adaptor chains
- Compiler-generated types from macros

Without `impl Trait`, these must use `dyn Trait` (runtime cost) or be avoided entirely (API limitation).

### 7.2 API Stability and Encapsulation

Library authors may want to hide concrete return types to preserve the freedom to change implementations:

```blood
// Without impl Trait: caller depends on concrete FilterMap<...> type
fn active_users(db: &DB) -> FilterMap<DBIter, fn(User) -> Option<User>> { ... }

// With impl Trait: caller only depends on Iterator bound
fn active_users(db: &DB) -> impl Iterator<Item = User> { ... }
```

### 7.3 Zero-Cost Abstraction Principle

Blood is a systems language committed to zero-cost abstractions. Forcing `dyn Trait` for all type-hidden returns violates this principle by imposing heap allocation and vtable indirection where none is needed.

### 7.4 Ecosystem Compatibility

If Blood aims to interoperate with Rust-ecosystem patterns (iterator chains, builder patterns, combinator-heavy APIs), lacking RPIT will force awkward workarounds.

### 7.5 Ergonomic Benefit for Closures

Closures are central to functional programming style. Without RPIT, returning closures requires boxing:

```blood
// Without impl Trait: heap allocation required
fn make_adder(n: i32) -> Box<dyn Fn(i32) -> i32> {
    Box::new(|x| x + n)
}

// With impl Trait: zero-cost
fn make_adder(n: i32) -> impl Fn(i32) -> i32 {
    |x| x + n
}
```

---

## 8. The Case AGAINST Adding `impl Trait`

### 8.1 Complexity Budget

Every syntax addition has a cost:
- Parser changes
- Type checker changes (opaque type inference, defining scopes)
- Codegen considerations
- Documentation burden
- Learning curve for users

Blood already has a rich type system (linear types, effects, regions, multiple dispatch, row polymorphism, `forall` types). Adding `impl Trait` increases the surface area users must learn, with diminishing returns compared to Rust where the type system is simpler.

### 8.2 Semantic Confusion with `dyn Trait`

Rust's `impl Trait` and `dyn Trait` look similar but have fundamentally different semantics. This is a well-documented source of confusion:
- `dyn Trait` = dynamic dispatch, type-erased, unsized
- `impl Trait` = static dispatch, compiler-known type, sized

Adding both to Blood duplicates this confusion. Blood could instead find a clearer naming convention if opaque types are needed.

### 8.3 `impl` Keyword Overloading

In Blood, `impl` already means "implement a trait for a type" (`impl Block`). Using `impl` in type position to mean "some type that implements" is a conceptual overload. This works in Rust because the language does not have other meanings for `impl` in type position, but it still generates confusion.

### 8.4 Multiple Dispatch Reduces the Need

As shown in Section 3.1, APIT is unnecessary in Blood. This eliminates roughly half of `impl Trait`'s use cases.

For the return-position case:
- Blood's Fiber is an effect, not `impl Future` — eliminating the largest RPIT driver
- Blood can use explicit associated types for trait return types
- Blood can use type aliases for named opaque types

### 8.5 TAIT Is Still Unstable in Rust

Rust has been designing `impl Trait` since 2016 and the type-alias form (TAIT) is still unstable after 8+ years. This suggests the design space is treacherous. Blood can learn from Rust's experience by waiting for the design to stabilize before adopting anything.

### 8.6 Effect System Reduces Closure Returns

Many Rust closure-return patterns exist because Rust lacks first-class effects:
- Callbacks (replaced by effect handlers in Blood)
- Async combinators (replaced by `/ {Fiber}` in Blood)
- Error handling closures (replaced by `/ {Error<E>}` in Blood)

With these use cases addressed by effects, the remaining need for closure-returning functions is smaller in Blood than in Rust.

### 8.7 Explicit is Better Than Implicit

Blood's design philosophy favors explicitness (zero shortcuts, exhaustive matches, explicit effect annotations). `impl Trait` moves in the opposite direction by hiding types. Named existential types (Section 5.3) would be more aligned with Blood's design if opaque types are needed.

---

## 9. Recommendation

### 9.1 Reject APIT Permanently

`impl Trait` in argument position should **never** be added to Blood. Multiple dispatch and explicit generics provide strictly more power with more clarity. APIT is syntactic sugar that Blood does not need.

### 9.2 Defer RPIT / Opaque Return Types

Return-position opaque types address a real problem (unnameable types), but Blood's effect system significantly reduces the need compared to Rust. The recommendation is to:

1. **Observe** real-world Blood code for patterns where `dyn Trait` is used only because the type cannot be named
2. **Measure** the performance impact of `dyn Trait` in those patterns
3. **Design** a Blood-native solution if the problem proves significant

### 9.3 Preferred Future Direction: `opaque` Type Aliases

If Blood eventually needs opaque return types, the recommended syntax is **not** `impl Trait` but rather a dedicated `opaque` type alias mechanism:

```blood
// Preferred: explicit opaque type alias
type MyIter = opaque Iterator<Item = i32>

fn make_iter() -> MyIter {
    (0..100).filter(|n| n % 2 == 0)
}
```

Advantages over `impl Trait`:
- No keyword overloading (`opaque` is a new keyword, not reusing `impl`)
- Types are nameable (can be stored in structs, referenced in other signatures)
- Explicit scope of opacity (the type alias declaration)
- Aligns with Blood's preference for explicitness
- Avoids the TAIT design quagmire by being the primary mechanism from the start

### 9.4 Decision Criteria for Future Revisit

Revisit this decision when **two or more** of the following are true:

| Criterion | Evidence Required |
|-----------|-------------------|
| Unnameable return types | 10+ instances in the self-hosted compiler or stdlib where `dyn Trait` is used only because the type cannot be named |
| Performance impact | Measurable performance regression from `dyn Trait` in hot paths |
| API ergonomics | Library authors report that explicit associated types are prohibitively verbose |
| Ecosystem pressure | Interop with Rust-style APIs requires opaque return types |
| Effect system gaps | Discovery of patterns where effects cannot replace `impl Trait`-style abstraction |

---

## 10. If Blood Does Add It: Design Constraints

If a future decision overrides this recommendation, the following constraints should apply:

### 10.1 Syntax

Do not use `impl Trait` in type position. Use `opaque` type aliases or a dedicated keyword to avoid confusion with `impl` blocks and `dyn Trait`:

```blood
// NOT this (Rust-style):
fn foo() -> impl Iterator<Item = i32> { ... }

// Instead, one of:
type Foo = opaque Iterator<Item = i32>
fn foo() -> Foo { ... }

// Or, if inline syntax is needed:
fn foo() -> some Iterator<Item = i32> { ... }  // Swift-style
```

### 10.2 No Argument Position

Never allow opaque types in argument position. Use generics or multiple dispatch.

### 10.3 Effect Interaction

Opaque return types must declare their effects explicitly:

```blood
// The effect row is visible even if the type is opaque
type FiberIter = opaque Iterator<Item = i32>
fn make_iter() -> FiberIter / {IO} { ... }
```

### 10.4 Type Stability

Opaque return types must satisfy Blood's type stability requirement: the concrete type is fixed for each monomorphization and determined by argument types.

### 10.5 Linear/Affine Compatibility

If the opaque type is linear or affine, this must be visible to the caller:

```blood
type Handle = opaque linear Resource
fn acquire() -> Handle / {IO} { ... }
// Caller knows Handle is linear and must be consumed exactly once
```

---

## References

### Rust Design Documents

- [RFC 1522: Conservative `impl Trait`](https://rust-lang.github.io/rfcs/1522-conservative-impl-trait.html) — Original RPIT/APIT RFC
- [RFC 3425: Return Position `impl Trait` in Traits](https://rust-lang.github.io/rfcs/3425-return-position-impl-trait-in-traits.html) — RPITIT design
- [RFC 2515: Type Alias Impl Trait](https://rust-lang.github.io/rfcs/2515-type_alias_impl_trait.html) — TAIT design
- [Impl Trait Initiative Explainer](https://rust-lang.github.io/impl-trait-initiative/explainer/rpit_trait.html) — Comprehensive overview
- [Changes to `impl Trait` in Rust 2024](https://blog.rust-lang.org/2024/09/05/impl-trait-capture-rules/) — Capture rules evolution
- [Rust Compiler Dev Guide: RPITIT](https://rustc-dev-guide.rust-lang.org/return-position-impl-trait-in-trait.html) — Implementation details

### Comparative Analysis

- [ncameron: Abstract Return Types, aka `impl Trait`](https://www.ncameron.org/blog/abstract-return-types-aka--impl-trait-/) — Design rationale
- [ncameron: `dyn Trait` and `impl Trait` in Rust](https://www.ncameron.org/blog/dyn-trait-and-impl-trait-in-rust/) — Comparison
- [quinedot: `dyn Trait` vs Alternatives](https://quinedot.github.io/rust-learning/dyn-trait-vs.html) — When to use each
- [varkor: Existential Types in Rust](https://varkor.github.io/blog/2018/07/03/existential-types-in-rust.html) — Theoretical foundation
- [Aaron Turon: Resurrecting `impl Trait`](https://aturon.github.io/blog/2015/09/28/impl-trait/) — Historical context
- [Swift: Opaque and Boxed Protocol Types](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/opaquetypes/) — Swift's `some`/`any` design

### Multiple Dispatch

- [Wikipedia: Multiple Dispatch](https://en.wikipedia.org/wiki/Multiple_dispatch) — Overview
- [Thomas Wiemann: Types and Multiple Dispatch in Julia](https://thomaswiemann.com/Types-and-Multiple-Dispatch-in-Julia) — Julia's approach
- [Eli Bendersky: A Polyglot's Guide to Multiple Dispatch](https://eli.thegreenplace.net/2016/a-polyglots-guide-to-multiple-dispatch/) — Cross-language comparison
- [Julia Documentation: Methods](https://docs.julialang.org/en/v1/manual/methods/) — Official Julia dispatch docs

### Effect Systems and Existential Types

- [Biernacki et al.: Abstracting Algebraic Effects (POPL 2019)](https://dl.acm.org/doi/10.1145/3290319) — Existential effects formalization
- [Koka Language](https://koka-lang.github.io/koka/doc/book.html) — Row-polymorphic effect types
- [Leijen: Koka: Programming with Row-polymorphic Effect Types](https://arxiv.org/pdf/1406.2061) — Koka's type system
- [INRIA Gallium: Safely Typing Algebraic Effects](http://gallium.inria.fr/blog/safely-typing-algebraic-effects/) — Effect typing overview
- [Effects in Rust (and Koka)](https://aloso.foo/blog/2025-10-10-effects/) — Rust-Koka comparison

### Blood Specifications

- [GRAMMAR.md Section 4.1](../spec/GRAMMAR.md#41-type-grammar) — Current type grammar (references this document)
- [DISPATCH.md](../spec/DISPATCH.md) — Multiple dispatch specification
- [FORMAL_SEMANTICS.md](../spec/FORMAL_SEMANTICS.md) — Formal type system
