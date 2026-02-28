# Blood Macro System Specification

**Version**: 0.1.0
**Status**: Design specification
**Last Updated**: 2026-02-28

## 1. Overview

Blood's macro system provides compile-time code generation through pattern-matching
and textual substitution. Macros expand before parsing, producing ordinary Blood
source code that is then subject to the full compilation pipeline (parsing, type
checking, effect inference, linearity checking, region inference, codegen).

### 1.1 Design Principles

1. **Transparency**: Macros are transparent to every downstream compiler phase.
   Expanded code is indistinguishable from hand-written code. The type checker,
   effect system, linearity checker, and region inference all operate on expanded
   code with no special macro awareness.

2. **Determinism**: Macro expansion is a pure function of (definition + invocation
   arguments). No I/O, no environment access, no randomness. This property is
   essential for content-addressed compilation: expanded source can be hashed and
   cached reliably.

3. **Least Power**: Use the simplest construct that solves the problem. Blood
   provides a tiered system (built-in < declarative < procedural) and users
   should prefer the least powerful tier that suffices.

4. **Explicit Invocation**: All macro invocations use the `!` sigil
   (`name!(...)`, `name![...]`, `name!{...}`), making them visually distinct
   from function calls. Code readers always know when code generation is
   happening.

### 1.2 Expansion Model

```
Source file
    |
    v
preprocess_macros()    -- collect declarations, expand invocations
    |
    v
Expanded source        -- ordinary Blood code, no macro syntax remains
    |
    v
parse()                -- standard parser
    |
    v
Full compilation pipeline (HIR, typeck, effects, linearity, MIR, codegen)
```

Macro expansion is a source-level preprocessing phase. After expansion, the
resulting text is valid Blood with no macro-specific constructs. Macro declarations
are removed from the source; macro invocations are replaced with their expansions.

### 1.3 Tiers

| Tier | Name | Status | Description |
|------|------|--------|-------------|
| 0 | Built-in macros | Complete | Compiler-intrinsic (`format!`, `vec!`, `assert!`, etc.) |
| 1 | Declarative macros | Complete | User-defined pattern-matching macros |
| 2 | Procedural macros | Future | Compile-time Blood functions (see [Section 9](#9-future-procedural-macros)) |

---

## 2. Declarative Macro Syntax

### 2.1 Definition

```ebnf
MacroDecl     ::= Visibility? 'macro' Ident '{' MacroRule (';' MacroRule)* ';'? '}'

MacroRule     ::= '(' MacroPattern ')' '=>' '{' MacroExpansion '}'
                | '(' MacroPattern ')' '=>' MacroExpansion

MacroPattern  ::= MacroPart*
MacroPart     ::= MacroCapture | MacroRepetition | MacroToken
MacroCapture  ::= '$' Ident ':' FragmentKind
MacroRepetition ::= '$(' MacroPart+ ')' Separator? RepetitionOp
RepetitionOp  ::= '+' | '*' | '?'
Separator     ::= ',' | ';' | Token

MacroExpansion ::= Token*

FragmentKind  ::= 'expr' | 'ty' | 'pat' | 'ident' | 'literal'
                | 'block' | 'stmt' | 'item' | 'tt'
```

### 2.2 Example

```blood
macro check_range {
    ($val:expr, $lo:expr, $hi:expr) => {
        if $val < $lo || $val > $hi {
            panic!("out of range");
        }
    };

    ($val:expr, $hi:expr) => {
        check_range!($val, 0, $hi)
    };
}
```

### 2.3 Invocation

```ebnf
MacroCall ::= Path '!' MacroDelimiter
MacroDelimiter ::= '(' TokenStream ')' | '[' TokenStream ']' | '{' TokenStream '}'
```

All three delimiter forms are equivalent. Convention:
- `name!(...)` for expression-like invocations
- `name![...]` for list-like invocations (e.g., `vec![1, 2, 3]`)
- `name!{...}` for block-like invocations

---

## 3. Fragment Kinds

Fragment kinds (also called fragment specifiers) determine what syntactic
constructs a capture variable can match.

| Kind | Matches | Examples |
|------|---------|---------|
| `expr` | Any expression | `1 + 2`, `f(x)`, `if c { a } else { b }` |
| `ty` | Any type | `i32`, `&str`, `Vec<T>`, `fn(i32) -> bool` |
| `pat` | Any pattern | `x`, `(a, b)`, `Some(v)`, `_` |
| `ident` | Single identifier | `foo`, `my_var` |
| `literal` | Literal value | `42`, `"hello"`, `true`, `3.14` |
| `block` | Block expression | `{ let x = 1; x + 1 }` |
| `stmt` | Statement | `let x = 1;`, `return 0;` |
| `item` | Top-level item | `fn foo() {}`, `struct X { ... }` |
| `tt` | Single token tree | Any single token, or balanced `(...)`, `[...]`, `{...}` |

### 3.1 Fragment Opacity

After capture, a fragment is an opaque unit. A captured `$x:expr` can only be
substituted as a complete expression — it cannot be decomposed or partially
matched by a downstream macro rule.

The exception is `tt`, which captures a single token or balanced group and can
be re-matched by subsequent macro rules.

### 3.2 Follow-Set Restrictions

To ensure unambiguous parsing, each fragment kind restricts which tokens may
follow it in a pattern:

| Kind | May be followed by |
|------|--------------------|
| `expr`, `stmt` | `=>`, `,`, `;`, `)`, `]`, `}` |
| `ty` | `=>`, `,`, `;`, `)`, `]`, `}`, `=`, `>`, `{` |
| `pat` | `=>`, `,`, `=`, `if`, `in` |
| `ident` | Any token |
| `literal` | Any token |
| `block` | Any token |
| `item` | Any token |
| `tt` | Any token |

These restrictions prevent ambiguities where the macro expander cannot determine
where a capture ends and the next pattern element begins.

---

## 4. Rule Matching

### 4.1 First-Match Semantics

When a macro is invoked, rules are tried in declaration order. The first rule
whose pattern matches the invocation arguments is selected. Remaining rules are
not considered.

**Consequence**: Place more specific rules before less specific ones.

```blood
macro log {
    // 3-arg rule must come first
    ($level:expr, $fmt:expr, $arg:expr) => {
        if log_enabled($level) {
            println(format!($fmt, $arg));
        }
    };

    // 2-arg rule (fallback)
    ($level:expr, $msg:expr) => {
        if log_enabled($level) {
            println($msg);
        }
    };
}
```

### 4.2 Pattern Matching Algorithm

For each rule, the expander attempts to match the invocation text against the
pattern parts left-to-right:

1. **Literal tokens**: Must match exactly (after whitespace normalization).
2. **Captures**: Consume input up to the next literal terminator, respecting
   balanced delimiters (`()`, `[]`, `{}`) and string/character literals.
3. **No terminator**: If a capture is the last pattern part, it consumes all
   remaining input.

### 4.3 Balanced Delimiter Handling

When scanning for a terminator (e.g., `,` between captures), the expander skips
over balanced delimiters and string/character literals:

```blood
macro pair {
    ($a:expr, $b:expr) => { ($a, $b) };
}

// The comma inside Vec::new() does not terminate $a
pair!(some_fn(1, 2), other_fn(3, 4))
// $a = "some_fn(1, 2)"
// $b = "other_fn(3, 4)"
```

---

## 5. Expansion

### 5.1 Substitution

Each `$name` in the expansion template is replaced with the text captured for
that name during pattern matching. Substitution is textual: the captured text
is inserted verbatim.

Boundary check: `$name` is recognized as a substitution only when `$` is
followed by the capture name and the character after the name is not an
identifier-continuation character (`[a-zA-Z0-9_]`).

### 5.2 Recursive Expansion

After substitution, the resulting text may contain further macro invocations.
The expander re-scans and expands iteratively until no macro invocations remain
or the expansion depth limit is reached.

```blood
macro double {
    ($x:expr) => { $x + $x };
}

macro quad {
    ($x:expr) => { double!(double!($x)) };
}

// quad!(5)
// Pass 1: double!(double!(5))
// Pass 2: double!(5 + 5)
// Pass 3: 5 + 5 + 5 + 5
```

### 5.3 Expansion Limits

| Limit | Value | Purpose |
|-------|-------|---------|
| Maximum expansion passes | 32 | Prevents infinite recursion in self-hosted compiler |
| Maximum expansion depth | 256 | Prevents infinite recursion in bootstrap compiler |

Exceeding the limit is a compile-time error.

### 5.4 Repetition Expansion

*Status: specified but not yet implemented.*

When a pattern uses repetition (`$($x:expr),*`), the corresponding expansion
uses matching repetition to expand once per captured element:

```blood
macro make_sum {
    ($($x:expr),*) => {
        0 $(+ $x)*
    };
}

// make_sum!(1, 2, 3) expands to: 0 + 1 + 2 + 3
```

Repetition variables:
- `$($capture:kind),*` — zero or more, comma-separated
- `$($capture:kind),+` — one or more, comma-separated
- `$($capture:kind)?` — zero or one (optional)

The separator (`,`, `;`, or any single token) appears between elements but not
after the last element.

---

## 6. Hygiene

### 6.1 Current Model

Blood's declarative macros currently use **unhygienic textual expansion**. The
self-hosted compiler performs direct string substitution with no scope tracking.
The bootstrap compiler maintains `HygieneId` annotations on tokens but applies
limited hygiene enforcement.

### 6.2 Target Model

Blood's target hygiene model is **definition-site binding with explicit escape**.

#### Rules

1. **Variables introduced by a macro expansion** are scoped to that expansion.
   They do not leak into the invoking scope and cannot capture identifiers from
   the invoking scope.

2. **Captures (`$name`)** retain the lexical context of the invocation site.
   When substituted into the expansion template, they resolve names as if
   written at the call site.

3. **Free identifiers in the expansion template** (not captures, not locally
   introduced) resolve at the macro's definition site.

4. **Explicit escape**: The `$crate` metavariable refers to the module where
   the macro is defined, enabling cross-module macros to reference their own
   definitions.

#### Example

```blood
macro with_temp {
    ($body:block) => {
        let temp = 0;       // 'temp' is hygienic — invisible to $body
        $body               // $body resolves names at the call site
    };
}

fn main() -> i32 {
    let temp = 42;
    with_temp!({
        temp                // Refers to the caller's 'temp' (42), not the macro's
    })
}
```

### 6.3 Implementation Roadmap

| Phase | Scope | Description |
|-------|-------|-------------|
| Phase 0 (current) | Unhygienic | Text substitution, no scope tracking |
| Phase 1 | Name freshening | `gensym` for macro-introduced bindings |
| Phase 2 | Definition-site resolution | Free identifiers resolve at definition site |
| Phase 3 | Full hygiene | Scope sets on all identifiers, `$crate` support |

Phase 0 is functional for the current macro use cases (simple substitution macros
where capture names do not collide with expansion-introduced names). Phases 1-3
are specified here for forward compatibility; implementation timeline is
unscheduled.

---

## 7. Built-in Macros

Built-in macros are compiler intrinsics. They use the same `name!(args)`
invocation syntax as user-defined macros but are expanded by dedicated compiler
logic rather than pattern matching.

### 7.1 `format!` — String Formatting

```blood
format!("template {} text", arg1)
format!("named: {name}", name = value)
```

Produces a `String`. Placeholders `{}` are replaced with string representations
of arguments in order. Named arguments use `name = value` syntax.

### 7.2 `vec!` — Collection Construction

```blood
vec![1, 2, 3]           // List form: elements
vec![0; 10]             // Repeat form: value; count
```

Produces a heap-allocated array. The repeat form requires a compile-time constant
count.

### 7.3 `assert!` — Runtime Assertion

```blood
assert!(condition)
assert!(condition, "message")
```

Panics at runtime if `condition` is `false`. The optional message is included in
the panic output.

### 7.4 `dbg!` — Debug Print

```blood
dbg!(expression)
```

Prints the expression text and its value to stderr, then returns the value.
Useful for debugging without modifying control flow.

### 7.5 `panic!` — Abort Execution

```blood
panic!("reason")
```

Immediately terminates the program with the given message.

### 7.6 `todo!` — Unfinished Code Marker

```blood
todo!()
```

Panics with a "not yet implemented" message. Marks code that is intentionally
incomplete. Distinct from `unreachable!` in intent: `todo!` means the code path
is valid but unfinished; `unreachable!` means the code path should never execute.

### 7.7 `unreachable!` — Impossible Code Path

```blood
unreachable!()
```

Panics with an "entered unreachable code" message. Asserts that a code path is
logically impossible. If reached, indicates a bug in the program.

### 7.8 `matches!` — Pattern Test

```blood
matches!(value, pattern)
```

Returns `true` if `value` matches `pattern`, `false` otherwise. Equivalent to
`match value { pattern => true, _ => false }`.

### 7.9 Adding New Built-in Macros

Built-in macros are added only when a macro requires compiler-internal
information that is unavailable to user-defined macros (e.g., type formatting
for `dbg!`, allocation for `vec!`). If the functionality can be expressed as a
declarative macro, it should be.

---

## 8. Interaction with Blood Language Features

### 8.1 Linear and Affine Types

Macros are transparent to linearity checking. The type checker enforces
linear/affine constraints on expanded code identically to hand-written code.

**Implication**: A macro that substitutes a capture variable multiple times will
produce expanded code that uses the value multiple times. If the value has a
linear type, the type checker will reject the expanded code.

```blood
macro use_twice {
    ($x:expr) => { consume($x); consume($x) };
}

let resource: linear File = open("test.txt");
use_twice!(resource);
// Expands to: consume(resource); consume(resource);
// Type error: 'resource' used after move (linear type violation)
```

This is the correct behavior. The macro system does not duplicate or circumvent
linearity — it generates code, and that code must satisfy all type constraints.

**Future consideration**: A `linear` fragment annotation could enable the macro
expander to reject definitions that substitute a capture more than once,
providing earlier and clearer error messages. This is not currently specified.

### 8.2 Algebraic Effects

Macros are transparent to effect inference. Effect rows are inferred on expanded
code, not on macro templates.

```blood
macro with_logging {
    ($body:block) => {
        perform Log("entering");
        let result = $body;
        perform Log("exiting");
        result
    };
}
```

The expanded code will be inferred to have the `Log` effect. The caller must
provide a handler or propagate the effect. No special annotation on the macro
is required.

### 8.3 Region-Based Memory

Macros are transparent to region inference. Allocations in macro-generated code
are assigned to regions by the standard region inference algorithm operating on
the expanded code.

### 8.4 Content-Addressed Compilation

Macro expansion is deterministic and pure. The expanded source text is suitable
as input to content hashing. Macro expansion results can be cached:

```
cache_key = hash(macro_definition + invocation_arguments)
cache_value = expanded_text
```

This property must be preserved by any future macro system evolution. Procedural
macros (Tier 2, Section 9) must maintain determinism through restricted I/O.

### 8.5 Multiple Dispatch

No special interaction. Macro-generated function definitions participate in
dispatch resolution identically to hand-written definitions.

---

## 9. Future: Procedural Macros

*Status: design sketch. Not yet implemented or scheduled.*

Procedural macros would allow compile-time Blood code to transform AST nodes.
This section establishes design constraints rather than specifying syntax.

### 9.1 Design Constraints

Any procedural macro system for Blood must satisfy:

1. **Determinism**: Procedural macros must be pure functions. No filesystem
   access, no network I/O, no randomness, no environment variable reads.
   This preserves content-addressed compilation.

2. **Separate compilation**: Procedural macros must be compiled before the
   crate that uses them, similar to Rust's proc-macro crate requirement.

3. **Sandboxing**: Procedural macro execution must be isolated from the
   host compiler's state.

4. **Type transparency**: Procedural macros output AST, which is then
   type-checked normally. The macro system does not bypass the type checker.

5. **Effect transparency**: Procedural macro output is subject to standard
   effect inference.

### 9.2 Potential Model

Inspired by Zig's `comptime` and Scala 3's `inline def`, a procedural macro
could be a function marked `comptime` that accepts and returns AST fragments:

```blood
// Hypothetical syntax — not finalized
comptime fn derive_debug(item: Ast.Item) -> Ast.Item {
    // Inspect fields, generate Debug implementation
    // ...
}
```

This model avoids token-stream manipulation in favor of typed AST operations,
reducing the class of bugs possible in macro implementations.

### 9.3 Decision: When to Specify

Procedural macros should be specified when:
- The declarative macro system proves insufficient for real-world use cases
- The AST representation is stable enough to expose as a public API
- The self-hosted compiler can compile and execute Blood code at compile time

Until then, the declarative system (Tier 1) is the complete macro facility.

---

## 10. Macro Scoping and Visibility

### 10.1 File Scope

Macro expansion is currently per-file. A macro defined in one file is available
only within that file. Macro definitions are collected from the file source
before expansion begins.

### 10.2 Module Scope (Target)

The target scoping model allows macros to be shared across modules:

```blood
// In utils.blood
pub macro assert_positive {
    ($x:expr) => {
        if $x <= 0 {
            panic!("expected positive value");
        }
    };
}

// In main.blood
mod utils;
use utils.assert_positive;

fn main() -> i32 {
    assert_positive!(42);
    0
}
```

Macro visibility follows the same rules as other items:
- No visibility modifier: private to the defining module
- `pub`: visible to importing modules

### 10.3 Expansion Order

Within a file, macros are collected in a first pass, then invocations are
expanded in a second pass. This means a macro can be invoked before its textual
definition:

```blood
fn main() -> i32 {
    double!(21)         // OK: macro is collected before expansion pass
}

macro double {
    ($x:expr) => { $x + $x };
}
```

---

## 11. Error Reporting

### 11.1 Diagnostic Codes

| Code | Condition |
|------|-----------|
| E0116 | Unsupported built-in macro name |
| E0118 | Unknown fragment specifier |
| E0119 | Macro expansion depth exceeded |
| E0120 | No matching rule for invocation |
| E0121 | Unbalanced delimiters in macro invocation |

### 11.2 Error Location

Errors in expanded code report the location in the expanded source. When
possible, the diagnostic includes the macro invocation site to help the user
trace the error to their code.

### 11.3 Expansion Tracing

For debugging, the compiler may support a `--dump-macros` flag that outputs
the fully expanded source before parsing. This allows users to inspect what
the macro system produced.

---

## 12. Implementation Reference

### 12.1 Self-Hosted Compiler

**File**: `src/selfhost/macro_expand.blood` (693 lines)

Entry point: `pub fn preprocess_macros(source: &String) -> String`

Phases:
1. `find_macro_declarations()` — scan for `macro ` keyword, parse rules
2. Remove declaration text from source
3. `expand_all_macros()` — iterative expansion (up to 32 passes)
4. Return expanded source

Key internal functions:
- `try_match_rule()` — match invocation against rule pattern
- `substitute_multi()` — replace `$name` with captured text
- `find_literal_balanced()` — scan for terminator with balanced delimiters

### 12.2 Bootstrap Compiler

**File**: `src/bootstrap/bloodc/src/macro_expand.rs` (1,162 lines)

Entry point: `MacroExpander::expand_program()`

Uses `TokenStream` representation with `HygieneId` tracking. More sophisticated
matching via `match_pattern()` and `capture_fragment()` functions. Supports all
9 fragment kinds with proper token-level validation.

### 12.3 AST Representation

**File**: `src/selfhost/ast.blood`

Key types: `MacroDecl`, `MacroRule`, `MacroPattern`, `MacroPatternPart`,
`MacroExpansion`, `MacroExpansionPart`, `FragmentKind`, `MacroDelimiter`,
`MacroCallKind`.

### 12.4 Test Coverage

| Test | Feature | Status |
|------|---------|--------|
| `t05_user_macro` | Single-rule user macro | Pass |
| `t05_format_macro` | format! built-in | Pass |
| `t05_vec_macro` | vec! built-in | Pass |
| `t05_dbg_expr` | dbg! built-in | Pass |
| `t05_panic_builtin` | panic! built-in | Pass |
| `t05_todo_unreachable` | todo!/unreachable! built-ins | Pass |
| `t09_multi_capture_macro` | Two-capture macro | Pass |
| `t09_multi_rule_macro` | Three-rule macro with arity dispatch | Pass |
| `t09_recursive_macro` | Macro expanding to macro invocation | Pass |
| `t06_err_invalid_macro_frag` | Invalid fragment (compile-fail) | Pass |
| `t06_err_unsupported_macro` | Unknown built-in (compile-fail) | Pass |

---

## Appendix A: Design Rationale

### A.1 Why Text-Level Expansion?

Blood chose source-level text preprocessing over token-stream or AST-level
macros for several reasons:

1. **Self-hosting simplicity**: The self-hosted compiler (written in Blood) can
   implement macro expansion with string operations, avoiding the need for a
   token-stream library or AST manipulation API at bootstrap time.

2. **Content-addressing compatibility**: Text-in, text-out is trivially
   deterministic and cacheable.

3. **Transparency**: Every downstream phase sees ordinary Blood source code.
   No phase needs macro-awareness.

4. **Sufficiency**: For declarative pattern-matching macros, text-level expansion
   produces correct results. The fragment-kind system and balanced-delimiter
   handling prevent the pitfalls of naive text substitution (like the C
   preprocessor).

The trade-off is limited hygiene and no type awareness at expansion time. These
limitations are acceptable for Tier 1 macros and will be addressed by Tier 2
(procedural macros) if needed.

### A.2 Why `!` Sigil?

The `!` sigil on macro invocations serves two purposes:

1. **Visual distinction**: Readers immediately know that code generation is
   happening. This is important in a language with effects (where `perform`
   already signals non-local behavior) — macros add another axis of
   "not what it looks like" that should be marked.

2. **Parsing unambiguity**: The parser can distinguish `name(args)` (function
   call) from `name!(args)` (macro invocation) without lookahead or context.

### A.3 Why Not Rust's `macro_rules!`?

Blood uses `macro name { ... }` instead of Rust's `macro_rules! name { ... }`:

1. `macro` is a keyword in Blood, not an invocation of a built-in macro.
2. The definition is an item declaration, consistent with `fn`, `struct`,
   `enum`, `trait`.
3. No `!` in the definition — `!` is for invocation only.

### A.4 Comparison with Other Systems

| Feature | Blood | Rust | Zig | Nim |
|---------|-------|------|-----|-----|
| Expansion level | Text | Token | Execution | AST |
| Hygiene | Target: definition-site | Mixed-site | N/A | Selective |
| Type awareness | None (transparent) | None (macro_rules!) | Full (comptime) | Optional (typed) |
| Invocation marker | `!` | `!` | `comptime` keyword | None |
| Deterministic | Yes (required) | No (proc macros) | Yes (required) | No |
| Fragment specifiers | 9 kinds | 13 kinds | N/A | N/A |
