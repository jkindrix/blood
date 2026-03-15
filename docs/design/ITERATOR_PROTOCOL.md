# DD-2: Iterator Protocol Design

**Status:** CLOSED — Hybrid approach (trait for consumption, effect for production)
**Date:** 2026-03-15
**Blocks:** stdlib Iterator, for-in desugaring, Vec.iter(), combinators, selfhost dog-fooding

## Problem

The selfhost has zero closures in 82K lines, with 1,031 manual indexed iterations. The root cause is missing Iterator infrastructure — no trait, no for-in desugaring, no map/filter/find. Blood needs an iterator protocol that works with its unique properties:

- Copy-by-default value semantics (no borrow checker)
- Algebraic effects (yield could be an effect)
- Linear/affine type qualifiers
- No garbage collector

## Research Summary

Five approaches evaluated (see research notes):

| Approach | Zero-cost | Early exit | Write ergonomics | Blood fit |
|----------|-----------|------------|-----------------|-----------|
| Trait-based (Rust) | Yes | Yes | Boilerplate | Good with MVS fixes |
| Effect-based (Koka) | TR-only | Yes | Excellent (yield) | Natural but slower |
| Lazy thunks (OCaml Seq) | No | Yes | Simple | Bad (no GC) |
| Internal (callback) | Yes | Needs break | Simple | Good |
| Hybrid (trait+effect) | Yes | Yes | Excellent | Best fit |

Key findings:
- Vale's gen-ref research shows ~11% overhead for safety checks — acceptable
- Koka's tail-resumptive effects compile to direct calls but multi-handler composition doesn't fuse like iterator adaptors
- Copy-by-default eliminates borrow checker complexity for iterators entirely

## Decision: Staged Hybrid

### Stage 1 (Now): Trait-based Iterator with MVS Adjustments

```blood
trait Iterator<T> {
    fn next(&mut self) -> Option<T>
}
```

**Key difference from Rust:** `next` returns `T` by copy, not `&T` by reference. This is correct for Blood's value semantics. For `Vec<i32>`, copying is free. For `Vec<LargeStruct>`, the copy cost is the price of value semantics — the same cost paid everywhere else in Blood.

**for-in desugaring:**
```blood
for x in expr { body }
// desugars to:
let mut __iter = expr;
loop {
    match __iter.next() {
        Option.Some(x) => { body }
        Option.None => { break; }
    }
}
```

**Combinators** as methods on Iterator with default implementations:
- `map<B>(self, f: fn(T) -> B) -> Map<Self, B>`
- `filter(self, f: fn(&T) -> bool) -> Filter<Self>`
- `find(self, f: fn(&T) -> bool) -> Option<T>`
- `any(self, f: fn(T) -> bool) -> bool`
- `all(self, f: fn(T) -> bool) -> bool`
- `fold<B>(self, init: B, f: fn(B, T) -> B) -> B`
- `enumerate(self) -> Enumerate<Self>`
- `collect(self) -> Vec<T>`

### Stage 2 (After closures are stable): Yield Effect

```blood
effect Yield<T> {
    op yield(value: T);
}
```

Generator functions perform `Yield`. A `to_iter` handler transforms a generator into an Iterator:

```blood
fn range(start: i32, end: i32) / {Yield<i32>} {
    let mut i = start;
    while i < end {
        perform Yield.yield(i);
        i += 1;
    }
}
```

This is syntactic sugar — the compiler could transform this into a state-machine struct implementing Iterator. But the unoptimized version (effect handler with tail-resumptive optimization) works correctly today.

### Stage 3 (Optimization): Generator-to-state-machine

Compile `fn gen() / {Yield<T>}` into an anonymous struct implementing `Iterator<T>`. This eliminates all effect overhead for generators, making them equivalent to hand-written iterator structs.

## Rationale

- **Trait is the consumption protocol** — what `for` uses, what adaptors compose over. Zero-cost, proven, well-understood.
- **Effects are the production mechanism** — how you write generators. Natural for Blood, no new language features.
- **Staged approach** means we can ship Stage 1 immediately and iterate.
- **Copy-by-default eliminates the hardest part** of Rust's iterator design (borrow checker interaction, lending iterators, etc.)

## Rejected Alternatives

**Pure effect-based (Koka-style):** Handler composition doesn't fuse into single loops. For tight loops over millions of elements (common in compilers), 2-5x slower than trait-based.

**Lazy thunks (OCaml Seq):** Allocates a closure per element. Without GC, this is prohibitively expensive.

**Internal iterator only:** Can't express zip, take, or interleave. Too limiting for a general-purpose language.
