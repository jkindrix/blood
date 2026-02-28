# Design Document: Comparison Chaining

**Status:** Research / Under Evaluation
**Feature:** `a < b < c` meaning `a < b && b < c`
**Referenced by:** `docs/spec/GRAMMAR.md` (Section 7.1, precedence table design note)

---

## 1. Executive Summary

This document evaluates comparison chaining as a potential future feature for Blood. Comparison chaining allows mathematical-style compound comparisons like `a < b < c` to be interpreted as `a < b && b < c`, with intermediate expressions evaluated exactly once.

Blood currently marks comparison operators as **non-associative** in its precedence table, making `a < b < c` a parse error. This was a deliberate design choice that preserves the option to add chaining later. The grammar already documents this intent:

> Comparison chaining (Python-style, where `a < b < c` means `a < b && b < c`) is a candidate future feature pending evaluation of interaction with linear/affine types and algebraic effects.

This document evaluates whether chaining should be adopted, and if so, under what constraints.

**Recommendation:** Do not implement comparison chaining at this time. The interaction with linear/affine types creates a fundamental tension that has no clean resolution. Blood should instead invest in range containment patterns (`x in lo..hi`) as an idiomatic alternative. Comparison chaining may be revisited if the language adopts by-reference comparison semantics universally, which would eliminate the linear type conflict.

---

## 2. Survey: Comparison Chaining Across Languages

### 2.1 Languages WITH Chaining

#### Python

**Semantics:** `a op1 b op2 c ... y opN z` is equivalent to `a op1 b and b op2 c and ... y opN z`, except that each expression is evaluated at most once.

**Implementation:** At the bytecode level, Python uses `DUP_TOP` and `ROT_THREE` stack instructions. After loading `a` and `b`, `DUP_TOP` duplicates `b` on the stack, then `ROT_THREE` rearranges the stack for the first comparison while preserving `b` for reuse. The `JUMP_IF_FALSE_OR_POP` instruction implements short-circuiting: when a comparison yields `False`, execution skips subsequent comparisons entirely. The bytecode generation occurs in `Python/compile.c` within `compiler_compare()`.

**Key properties:**
- All comparison operators can chain (`<`, `<=`, `>`, `>=`, `==`, `!=`, `is`, `is not`, `in`, `not in`)
- Mixed operators allowed: `a < b >= c` is legal (though confusing)
- Short-circuit evaluation: if `a < b` is false, `c` is never evaluated
- No ownership concerns (garbage collected, no linear types)

**Limitations encountered:** PEP 535 (status: Deferred) identified a problem with NumPy arrays. When comparisons return non-boolean values (element-wise boolean arrays), the `and` in the desugaring triggers a `ValueError` ("The truth value of an array with more than one element is ambiguous"). This demonstrates that chaining is fragile when comparison operators return non-`bool` types.

#### Julia

**Semantics:** Same as Python in principle: `a < b < c` means `a < b && b < c` with `b` evaluated once.

**Critical difference:** Julia changed from short-circuit to **eager** evaluation in v0.7 (Issue #16088). The expression `a < b < c` is now lowered to `(a < b) & (b < c)` using bitwise AND, evaluating all comparisons regardless. The rationale was that short-circuiting made the lowering harder to reason about, and most chained comparisons (like `0 < i <= n`) are safe to evaluate eagerly.

**Side effects warning:** Julia's documentation explicitly states: "the order of evaluations in a chained comparison is undefined" and "it is strongly recommended not to use expressions with side effects in chained comparisons." This is a significant red flag for a language with algebraic effects.

#### Raku (Perl 6)

**Semantics:** Chaining operators have special "C" (chaining) associativity. `a < b < c` expands to `(a < b) and (b < c)` with `b` evaluated once.

**Chainable operators:** `!= == < <= > >= eq ne lt le gt ge ~~ === eqv !eqv`

**Properties:**
- Short-circuit evaluation via `and` semantics
- Single evaluation of intermediate operands guaranteed
- Explicit warning about side effects: "multiple references to a single mutating object in the same expression may result in undefined behavior"

#### Clojure

**Semantics:** Clojure's prefix notation makes chaining natural. `(< a b c)` checks `(< a b)` AND `(< b c)`. This works because comparison operators are variadic functions that accept any number of arguments.

**Key insight:** The prefix notation eliminates the parsing ambiguity entirely. There is no `a < b < c` infix syntax to confuse with `(a < b) < c`. This suggests chaining is most natural in languages designed around it from the start.

### 2.2 Languages WITHOUT Chaining

#### Rust

**Current state:** `a < b < c` is a **parse error** since RFC 558 (pre-1.0). This was deliberate to preserve the option for future chaining.

**RFC 2083 (Allow chaining of comparisons):** Open but dormant. Key discussion points:
- Readability benefit for range checks: `lower <= x < upper`
- Concern about language complexity: "adding unnecessary complexity for a pretty much optional feature"
- `bool` implements `PartialOrd`, so `(1 < 0) < true` is already valid code. Changing semantics would break this.
- `Range::contains` and `is_sorted()` proposed as alternatives
- **Crucially:** Rust comparisons take `&self` and `&Rhs` (shared borrows), so the temporary binding issue is manageable. The comparison does not consume its operands.

**Rust internals discussion (Python-like chained comparison operators):** The community debated eager vs. lazy desugaring strategies. Opponents argued: "So far all the comparison operators have been eager. Making them conditionally lazy requires such a mental shift that I would consider it breaking." No consensus was reached.

#### Zig

**Proposal (Issue #370):** Rejected by Andrew Kelley. Rationale: "I read that as `(a == b) == c`, which doesn't even make sense according to types, if a,b,c are integers." The core argument was that chaining adds cognitive overhead without sufficient benefit.

**Hidden short-circuiting concern:** The proposal noted that `a() == b() == c()` would call all three functions when it sometimes only needs to call two, but this behavior would be invisible to the reader.

#### D Language

D does not support chained comparisons. The standard approach is explicit conjunction: `a < b && b < c`.

#### Kotlin

Kotlin does not support chained comparisons. Comparison operators are desugared to `compareTo()` calls, which return `Int`. Chaining would require special-casing.

#### C++

**P0893R0 (Chaining Comparisons):** Proposed but not adopted. Key findings:
- Analysis of open-source codebases found **zero intentional uses** of `a < b < c` with C's current semantics (where it means `(a < b) < c`)
- **Thousands of instances** where developers explicitly wrote `a < b && b < c`
- Proposed restricting chaining to transitive families (all ascending, all descending, or all equality)
- Parentheses would disable chaining, preserving backward compatibility
- Lifetime/temporary concerns addressed via lambda-like implementation
- Ultimately not adopted into C++20 (only the spaceship operator `<=>` was adopted)

#### C# (Alternative Approach)

C# 9.0 took a different path with **relational pattern matching**:
```csharp
if (x is > 0 and < 100) { ... }
```
This avoids the chaining problem entirely by treating range checks as pattern matches on a single expression. The `and`/`or`/`not` combinators are explicit, and the subject expression `x` is evaluated once.

### 2.3 Summary Table

| Language | Chaining? | Short-circuit? | Eval once? | Side effect safe? | Ownership-aware? |
|----------|-----------|----------------|------------|-------------------|------------------|
| Python | Yes, all ops | Yes | Yes | No (warning) | N/A (GC) |
| Julia | Yes, all ops | No (eager) | Yes | No (undefined order) | N/A (GC) |
| Raku | Yes, all ops | Yes | Yes | No (warning) | N/A (GC) |
| Clojure | Yes (prefix) | Yes | Yes | N/A (pure-ish) | N/A (GC) |
| Rust | No (error) | N/A | N/A | N/A | Borrows for cmp |
| Zig | No (rejected) | N/A | N/A | N/A | N/A |
| D | No | N/A | N/A | N/A | N/A |
| Kotlin | No | N/A | N/A | N/A | N/A |
| C++ | No (proposed) | N/A | N/A | N/A | N/A |
| C# | Alternative | N/A | Yes | Yes | N/A (GC) |

**Pattern:** Every language that supports chaining is garbage-collected with no ownership semantics. No systems language with ownership or linear/affine types has adopted comparison chaining.

---

## 3. The "Evaluate Once" Problem

### 3.1 Basic Desugaring

The canonical desugaring of `a < b < c` is:

```
{ let __tmp = b; (a < __tmp) && (__tmp < c) }
```

This introduces a temporary binding `__tmp` that is:
1. Initialized once (from `b`)
2. Used twice (once in each comparison)
3. Implicitly dropped after the block

For longer chains, `a < b < c < d` becomes:

```
{
    let __tmp1 = b;
    let __tmp2 = c;
    (a < __tmp1) && (__tmp1 < __tmp2) && (__tmp2 < d)
}
```

### 3.2 Side Effect Ordering

If `b` is a function call with side effects, the desugaring must preserve:
1. **Evaluation order:** `a` is evaluated first, then `b`, then (conditionally) `c`
2. **Single evaluation:** `b` is evaluated exactly once
3. **Effect ordering:** Effects from evaluating `a` happen before effects from `b`, which happen before effects from `c`

With short-circuit evaluation, `c` may or may not be evaluated depending on `a < b`. With eager evaluation (Julia's approach), all operands are always evaluated.

### 3.3 Interaction with Algebraic Effects

Blood's algebraic effects introduce a deeper concern. Consider:

```blood
fn read_sensor() -> i32 / {IO} {
    perform IO.read()
}

// With chaining:
if lo < read_sensor() < hi { ... }

// Desugars to:
{
    let __tmp = read_sensor();  // performs IO effect
    (lo < __tmp) && (__tmp < hi)
}
```

**With short-circuit evaluation:** If `lo < __tmp` is false, `hi` is never evaluated. This is fine if `hi` is pure, but if `hi` also performs effects:

```blood
if read_sensor() < read_sensor() < read_sensor() { ... }
```

The desugaring becomes:

```blood
{
    let __tmp1 = read_sensor();  // Effect 1
    let __tmp2 = read_sensor();  // Effect 2
    (__tmp1 < __tmp2) && (__tmp2 < read_sensor())  // Effect 3 (conditional)
}
```

Effect 3 may or may not execute depending on the result of the first comparison. This is the same behavior as explicit `&&`, so it is consistent. However, the user sees three `read_sensor()` calls and might expect three effects, not two-or-three.

**Julia's response to this problem:** Make evaluation order undefined and warn against side effects in chained comparisons. This is unacceptable for Blood, where effects are first-class and their ordering is semantically significant.

**Possible Blood response:** Require eager evaluation of all operands before any comparison, preserving effect ordering. But this eliminates the short-circuit benefit and changes the semantics from `&&` (which Blood programmers expect to short-circuit).

### 3.4 Effect Handler Interaction

A more subtle issue arises with effect handlers. Consider:

```blood
handle {
    if lo < effectful_expr() < hi { ... }
} with MyHandler {
    SomeEffect.op(x) => { resume(default_value) }
}
```

If `effectful_expr()` performs `SomeEffect.op(x)`, the handler resumes with `default_value`, which becomes `__tmp`. The comparison then uses `__tmp` twice. If the handler is multi-shot (resumes multiple times), `__tmp` would need to be duplicated, which is fine for `Copy` types but problematic for linear types (see Section 4).

---

## 4. The Linear/Affine Type Conflict

This is the core technical obstacle for comparison chaining in Blood.

### 4.1 The Fundamental Problem

A chained comparison `a < b < c` desugars to a form where `b` (or its temporary) is **used twice**: once in `a < b` and once in `b < c`. For a linear type (which must be used exactly once), this is a direct violation.

```blood
fn get_resource() -> linear FileHandle { ... }

// This MUST be rejected:
if lo < get_resource() < hi { ... }

// Because it desugars to:
{
    let __tmp: linear FileHandle = get_resource();
    (lo < __tmp) && (__tmp < hi)
    //     ^^^^        ^^^^
    //     used twice: VIOLATION
}
```

For affine types (used at most once), the same violation occurs: the temporary is consumed by the first comparison and then illegally reused in the second.

### 4.2 Can Comparisons Take References?

One solution is to make comparison operators take `&T` (shared references) instead of `T`:

```blood
// If comparison signature is:
fn lt(self: &T, other: &T) -> bool

// Then desugaring becomes:
{
    let __tmp = b;
    (&a < &__tmp) && (&__tmp < &c)
    //   ^^^^^^       ^^^^^^
    //   borrows, not moves: OK for linear types
}
```

This is how Rust handles it: `PartialEq::eq(&self, other: &Rhs) -> bool` and `PartialOrd::partial_cmp(&self, other: &Self) -> Option<Ordering>` both take shared references. The comparison never consumes its operands.

**Blood's situation:** Blood does not currently have a formalized comparison trait system like Rust's `PartialOrd`/`PartialEq`. The comparison operators work on built-in types. When Blood formalizes operator overloading for user-defined types, the by-reference-or-by-value question becomes critical.

**If Blood adopts by-reference comparison semantics:**
- Chaining works for all types, including linear/affine types
- The temporary is borrowed, not consumed
- The temporary is dropped at the end of the block (satisfying linearity)
- This is the clean solution

**If Blood allows by-value comparison semantics:**
- Chaining is fundamentally incompatible with linear types
- The compiler would need to:
  - Reject chaining for linear/affine types (inconsistent syntax)
  - Or reject by-value comparison operators entirely (restrictive)
  - Or perform some kind of implicit clone (violates zero-cost abstractions)

### 4.3 Analysis of Possible Resolutions

#### Option A: Restrict chaining to Copy/Clone types

```blood
// OK: i32 is Copy
if 0 < x < 100 { ... }

// ERROR: FileHandle is linear, chaining not allowed
if lo < get_handle() < hi { ... }
```

**Pros:** Simple rule, covers the common case (numeric range checks).
**Cons:** Inconsistent syntax. Users learn `a < b < c` works for numbers but not for custom types. Surprising error messages when types change.

#### Option B: Require by-reference comparison operators

Make all comparison operators take shared references. This is the Rust model.

**Pros:** Chaining works uniformly. No linear type conflict.
**Cons:** Forces a specific operator signature. May not work for types that need to consume arguments during comparison (rare but possible). Requires formalizing a trait system for comparisons.

#### Option C: Reject chaining entirely

Keep the current behavior: `a < b < c` is a parse error.

**Pros:** No complexity. No linear type conflict. No effect ordering surprises. Current users are not affected.
**Cons:** Misses the readability benefit for numeric range checks.

#### Option D: Implicit temporary borrow

The compiler automatically borrows the intermediate expression:

```blood
// User writes:
a < b < c

// Compiler generates:
{ let __tmp = b; a < &__tmp && &__tmp < c }
```

**Pros:** Works for all types. User does not see the borrow.
**Cons:** Implicit borrows are surprising. The user wrote `a < b` (by value) but the compiler inserts `a < &__tmp` (by reference). This requires the comparison operator to accept references, which may not exist for all types. Mismatch between explicit and chained comparison behavior.

#### Option E: Eager evaluation + single-use decomposition

For linear types, decompose the chaining differently:

```blood
// User writes:
a < b < c

// For linear b, compiler generates:
{
    let __tmp = b;
    let __result1 = a < &__tmp;
    let __result2 = &__tmp < c;
    drop(__tmp);  // linear value consumed exactly once via drop
    __result1 && __result2
}
```

**Pros:** Linear value is consumed exactly once (via drop). Both comparisons use borrows.
**Cons:** Still requires by-reference comparison operators. Eager evaluation changes semantics (no short-circuiting). The `drop` is hidden from the user.

### 4.4 The Verdict on Linear Types

The interaction between comparison chaining and linear/affine types has no clean resolution unless Blood commits to by-reference comparison operators universally. Every other approach introduces either:
- Inconsistent syntax (works for some types, not others)
- Hidden borrows (implicit behavior the user did not write)
- Forced eager evaluation (changes from `&&` semantics)
- Implicit drops (hidden resource cleanup)

By-reference comparison is the prerequisite for chaining in a language with linear types.

---

## 5. Desugaring Strategies in Detail

### 5.1 Short-Circuit (Python Model)

```
a < b < c < d
=>
{ let t1 = b; let t2 = c; (a < t1) && (t1 < t2) && (t2 < d) }
```

**Properties:**
- Operands evaluated left-to-right
- If any comparison fails, subsequent operands are NOT evaluated
- Consistent with `&&` semantics that Blood users already know
- Effect ordering: effects of `a` before `b` before (conditionally) `c` before (conditionally) `d`
- **Problem:** Multiple temporaries alive simultaneously. With linear types and by-reference comparisons, all temporaries must be dropped at block end.

### 5.2 Eager (Julia Model)

```
a < b < c < d
=>
{ let t1 = a; let t2 = b; let t3 = c; let t4 = d;
  (t1 < t2) & (t2 < t3) & (t3 < t4) }
```

**Properties:**
- All operands evaluated unconditionally, left-to-right
- No short-circuiting
- Predictable effect ordering (all effects happen)
- Simpler codegen (no conditional jumps for intermediate results)
- **Problem:** Evaluates expressions the user may not expect. `a < expensive() < c` always calls `expensive()` even if `a < expensive()` would be false.

### 5.3 Hybrid (Proposed for Blood, if adopted)

```
a < b < c < d
=>
{
    let t1 = b;
    if !(a < t1) { false }
    else {
        let t2 = c;
        if !(t1 < t2) { false }
        else { t2 < d }
    }
}
```

**Properties:**
- Short-circuit evaluation preserved
- Each temporary scoped to its usage block
- Temporaries dropped as soon as they are no longer needed
- Effect ordering preserved (left-to-right, conditional)
- Linear types handled correctly if comparisons are by-reference (temporary dropped at end of innermost block)

---

## 6. Alternatives to Chaining

### 6.1 Range Containment (Recommended for Blood)

Blood already has range syntax (`..` and `..=`). A natural extension would be:

```blood
// Half-open range: lo <= x < hi
if x in lo..hi { ... }

// Closed range: lo <= x <= hi
if x in lo..=hi { ... }
```

**Advantages over chaining:**
- No hidden temporaries
- No linear type conflict (only `x` is tested, `lo` and `hi` are range bounds)
- No effect ordering ambiguity
- Single syntax for the most common use case (bounds checking)
- Works with pattern matching: `match x { in 0..100 => ..., _ => ... }`
- C# 9.0 validates this approach (relational patterns rather than chaining)

**Disadvantages:**
- Does not cover asymmetric comparisons like `lo < x <= hi` (strict on one side, inclusive on the other). Blood's `lo..=hi` is inclusive on both sides; `lo..hi` is exclusive on the high end. The pattern `lo < x <= hi` would need `(lo+1)..=hi` for integers, which is unnatural.
- Does not cover descending chains like `a > b > c`
- Limited to range membership, not arbitrary comparison chains

### 6.2 Explicit Conjunction (Current Blood)

```blood
if lo < x && x < hi { ... }
```

**Advantages:**
- No hidden behavior
- Linear types work naturally (each comparison is independent)
- Effects are ordered exactly as written
- Universally understood

**Disadvantages:**
- `x` is written twice (DRY violation for complex expressions)
- Slightly less readable for mathematical inequalities

### 6.3 Named Functions

```blood
fn in_range(x: i32, lo: i32, hi: i32) -> bool {
    lo <= x && x < hi
}

if in_range(sensor_reading, 0, 100) { ... }
```

**Advantages:**
- Explicit, self-documenting
- Customizable bounds (open, closed, half-open)
- No syntax changes needed

**Disadvantages:**
- Verbose
- Not generic without trait system
- Not idiomatic for simple comparisons

### 6.4 Let Binding (for complex expressions)

```blood
let reading = read_sensor();
if lo < reading && reading < hi { ... }
```

**Advantages:**
- Explicit. No hidden behavior.
- Works with linear types (reading is used twice in comparisons, but if comparisons borrow, that's fine)
- Names the intermediate value, improving readability

**Disadvantages:**
- Extra line. Slightly more verbose.

### 6.5 Comparison Matrix

| Approach | Readability | Linear-safe | Effect-safe | Parser change | Use case |
|----------|-------------|-------------|-------------|---------------|----------|
| Chaining | Best for math | Problematic | Problematic | Yes | Numeric ranges |
| Range `in` | Good | Safe | Safe | Minor | Bounds checks |
| Explicit `&&` | Adequate | Safe | Safe | None | Universal |
| Named fn | Good | Safe | Safe | None | Domain-specific |
| Let binding | Adequate | Safe | Safe | None | Complex exprs |

---

## 7. Parser and Grammar Impact

### 7.1 Current Grammar

```ebnf
CmpOp ::= '==' | '!=' | '<' | '>' | '<=' | '>='
```

Comparison operators are non-associative at precedence level 6. Attempting to chain them is a parse error.

### 7.2 Grammar Change Required for Chaining

If chaining were adopted, the parser would need to recognize a sequence of comparisons as a single expression node rather than a binary operator application:

```ebnf
(* New: comparison chain expression *)
ChainedCmpExpr ::= Expr (CmpOp Expr)+
```

This requires changing the parser from treating `<` as a binary operator to treating a sequence of comparisons as a single n-ary expression. The AST representation would change:

```
// Current: Binary expression
BinaryExpr { op: Lt, lhs: a, rhs: b }

// New: Chained comparison expression
ChainedCmp { operands: [a, b, c], operators: [Lt, Lt] }
```

### 7.3 Which Operators Can Chain?

Following the C++ P0893 proposal's analysis, only "transitive families" should be chainable:

| Family | Operators | Example | Meaning |
|--------|-----------|---------|---------|
| Ascending | `<`, `<=` | `a < b <= c` | `a < b && b <= c` |
| Descending | `>`, `>=` | `a > b >= c` | `a > b && b >= c` |
| Equality | `==` | `a == b == c` | `a == b && b == c` |

**Operators that should NOT chain:**
- `!=` (not transitive: `a != b && b != c` does NOT imply `a != c`)
- Mixed families: `a < b > c` (no clear mathematical meaning)
- Mixed equality/relational: `a == b < c` (confusing)

### 7.4 Ambiguity with Type Arguments

Blood already addresses the `<` ambiguity between comparison and type arguments:

> In type positions, `<` after a type name is always a type argument list. In expression positions, `<` is always the less-than operator. Use type ascription on bindings when the compiler needs explicit type information.

Chaining does not introduce new ambiguities because it only applies in expression position where `<` is already the less-than operator.

---

## 8. Arguments For Chaining

1. **Mathematical notation:** `a < b < c` is universally understood in mathematics. Blood targets systems programming but values readability.

2. **Readability for numeric code:** Range checks like `0 <= index < length` are clearer than `0 <= index && index < length`, especially when the variable name is long.

3. **DRY principle:** In `lo < complex_expression && complex_expression < hi`, the expression is written twice. Chaining eliminates this duplication.

4. **Precedent:** Python's success with chaining demonstrates real-world utility. The C++ P0893 analysis found thousands of hand-written `a < b && b < c` patterns in major codebases (clang, LLVM, Boost).

5. **Forward compatibility:** Blood already reserves the syntax space by making `a < b < c` a parse error (following Rust RFC 558). Adding chaining later is non-breaking.

6. **Error prevention:** In C-family languages, `a < b < c` silently compiles as `(a < b) < c`, comparing a boolean to an integer. Chaining would give the expression its mathematically intended meaning.

---

## 9. Arguments Against Chaining

1. **Linear/affine type conflict:** The desugaring requires the intermediate expression to be used twice. This is fundamentally incompatible with linear types unless all comparisons are by-reference. See Section 4.

2. **Hidden temporaries:** The desugaring introduces implicit temporary variables that the user did not write. In a language with explicit ownership, hidden bindings are a source of confusion.

3. **Effect ordering concerns:** Blood's algebraic effects make evaluation order semantically significant. Short-circuit evaluation means some effects may or may not execute. Julia's response (undefined evaluation order) is unacceptable for Blood. See Section 3.3.

4. **Parser complexity:** The grammar change from binary operators to n-ary comparison chains adds complexity to the parser, AST representation, and every downstream pass (HIR lowering, type checking, MIR lowering).

5. **Limited use case:** Chaining is primarily useful for numeric range checks. Blood's existing range syntax (`..`, `..=`) and potential `in` operator cover this use case without the complications.

6. **Inconsistency risk:** If chaining is restricted to `Copy` types (Option A in Section 4.3), users encounter different syntax rules for different types. This is a source of confusion and a barrier to learning.

7. **Cognitive load:** As Zig's Andrew Kelley noted, `a == b == c` is naturally read as `(a == b) == c` by programmers trained on C-family languages. Changing this meaning requires retraining intuition.

8. **Short-circuit surprise:** `a < f() < g()` with short-circuiting means `g()` may not be called. If `g()` has side effects, this is a silent behavioral change from the "obvious" reading where all expressions are evaluated.

9. **Multi-shot handler interaction:** If a comparison chain occurs inside an effect handler that resumes multiple times, the temporaries may need to be duplicated, which conflicts with linear types. See Section 3.4.

10. **Minimal benefit:** The problem is well-solved by `let` bindings, range containment, or explicit `&&`. The syntactic saving is small. The implementation cost and conceptual weight are not.

---

## 10. Recommendation

### 10.1 Do Not Implement Chaining Now

The interaction with linear/affine types and algebraic effects creates fundamental tensions that have no clean resolution in Blood's current type system. Every language that successfully implements chaining is garbage-collected with no ownership semantics. No systems language with ownership or linear types has adopted it.

### 10.2 Invest in Range Containment Instead

Blood should develop the `in` operator for range containment as the idiomatic alternative:

```blood
// Proposed syntax:
if x in 0..100 { ... }     // 0 <= x < 100
if x in 0..=100 { ... }    // 0 <= x <= 100
```

This covers the primary use case (bounds checking) without hidden temporaries, linear type conflicts, or effect ordering surprises. The `in` operator would also integrate with pattern matching.

### 10.3 Revisit If Prerequisites Are Met

Comparison chaining could be reconsidered if both of the following are established:

1. **Blood formalizes comparison traits** with by-reference signatures (like Rust's `PartialOrd`), guaranteeing that comparisons never consume their operands.

2. **The community identifies patterns** where explicit `&&` or range `in` are genuinely insufficient, with concrete examples from Blood codebases.

### 10.4 Preserve the Syntax Space

Continue to make `a < b < c` a parse error (non-associative comparison operators). This preserves the ability to add chaining later without breaking existing code. The current grammar design is correct.

---

## 11. References

### Language Documentation
- [Python Expressions Reference (Section 6.10)](https://docs.python.org/3/reference/expressions.html) -- formal specification of comparison chaining semantics
- [Python Chained Operators Internals](https://arpitbhayani.me/blogs/chained-operators-python/) -- bytecode-level implementation details
- [PEP 535: Rich Comparison Chaining](https://peps.python.org/pep-0535/) -- complications with non-boolean comparison results (Deferred)
- [Julia Mathematical Operations](https://docs.julialang.org/en/v1/manual/mathematical-operations/) -- chained comparisons with undefined evaluation order
- [Julia Issue #16088: Don't short-circuit chained comparisons](https://github.com/JuliaLang/julia/issues/16088) -- switch from short-circuit to eager evaluation
- [Raku Operator Design Documents](https://github.com/Raku/old-design-docs/blob/master/S03-operators.pod) -- chaining associativity and side effect warnings

### Language Design Proposals
- [C++ P0893R0: Chaining Comparisons](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2018/p0893r0.html) -- comprehensive proposal with codebase analysis (not adopted)
- [Rust RFC 558: Require Parentheses for Chained Comparisons](https://rust-lang.github.io/rfcs/0558-require-parentheses-for-chained-comparisons.html) -- preserving syntax space for future chaining
- [Rust RFC Issue 2083: Allow Chaining of Comparisons](https://github.com/rust-lang/rfcs/issues/2083) -- open discussion
- [Rust Internals: Python-like Chained Comparison Operators](https://internals.rust-lang.org/t/python-like-chained-comparison-operators/13191) -- community discussion of desugaring strategies
- [Zig Issue #370: Chainable Comparison Operators](https://github.com/ziglang/zig/issues/370) -- rejected proposal
- [C# Relational Pattern Matching](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/operators/patterns) -- alternative approach via pattern combinators

### Type Theory and Ownership
- [CMU Lecture Notes on Linear Types](https://www.cs.cmu.edu/~fp/courses/15814-f20/lectures/23-linearity.pdf) -- formal definition of linear type usage constraints
- [Substructural Type Systems (Wikipedia)](https://en.wikipedia.org/wiki/Substructural_type_system) -- linear vs. affine type distinction
- [Rust PartialEq Trait](https://doc.rust-lang.org/std/cmp/trait.PartialEq.html) -- by-reference comparison operator signatures
- [Rust PartialOrd Trait](https://doc.rust-lang.org/std/cmp/trait.PartialOrd.html) -- by-reference ordering operator signatures

### Blood Language Specifications
- `docs/spec/GRAMMAR.md` Section 7.1 -- precedence table, non-associative comparison design note
- `docs/spec/FORMAL_SEMANTICS.md` Section 8 -- linear types and effects interaction rules
