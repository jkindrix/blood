# Blood Surface Syntax Grammar

**Version**: 0.4.0
**Status**: Specification target
**Last Updated**: 2026-02-28

**Revision 0.4.0 Changes**:
- Replaced `::` with `.` as universal path separator; removed `::` from the language
- Removed turbofish syntax; Blood uses type ascription instead of call-site type arguments
- Added `ModDecl`, `BridgeDecl`, `MacroDecl` to Declaration grammar
- Added `if let`, `while let`, `try/with`, postfix `?`, `default`, macro call expressions
- Added `forall` (higher-rank) and `dyn Trait` (trait object) types
- Added const generic parameters (`const N: usize` in TypeParam)
- Fixed `RegionExpr` to use `Lifetime` (not `Ident`), removed `Box::new` from `AllocExpr`
- Added `pub use` and allowed imports anywhere among declarations
- Added variadic parameter syntax in bridge FFI declarations
- Keyword reclassification: three-tier system (strict, contextual, reserved)
- Fixed pipe/assign operator precedence (pipe now binds tighter than assignment)
- Added `isize`/`usize` integer suffixes, byte string literals, doc comments
- Added raw identifier syntax (`r#keyword`)
- Added `@` prefix design rule documentation
- Added path disambiguation rule (Appendix B.2), replacing turbofish section
- Added comparison chaining design note

**Revision 0.3.0 Changes**:
- Added cross-references to FORMAL_SEMANTICS.md for effect syntax (§4.2, §8)
- Added notation alignment notes
- Added implementation status

This document provides the complete grammar for Blood's surface syntax, including operator precedence, associativity, and lexical rules.

---

## Table of Contents

1. [Lexical Grammar](#1-lexical-grammar)
2. [Program Structure](#2-program-structure)
3. [Declarations](#3-declarations)
4. [Types](#4-types)
5. [Expressions](#5-expressions)
6. [Patterns](#6-patterns)
7. [Operators and Precedence](#7-operators-and-precedence)
8. [Effect Syntax](#8-effect-syntax)
9. [Reserved Words](#9-reserved-words)

### Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — Operational semantics for expressions; see §1.3 for effect row notation and §5.5 for row polymorphism rules
- [STDLIB.md](./STDLIB.md) — Standard library type signatures
- [FFI.md](./FFI.md) — Bridge block syntax
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — Parse error messages

### Notation Alignment

This document uses **surface syntax** notation. For **formal semantics** notation, see [FORMAL_SEMANTICS.md Appendix B](./FORMAL_SEMANTICS.md#appendix-b-notation-summary):

| This Document | FORMAL_SEMANTICS.md | Meaning |
|---------------|---------------------|---------|
| `/ {IO, Error<E>}` | `ε = {IO, Error<E>}` | Effect row |
| `/ {IO \| ε}` | `ε = {IO \| ρ}` | Open effect row with row variable |
| `/ pure` | `ε = {}` or `pure` | Empty effect row |
| `ε` (type parameter) | `ρ` | Row variable |

---

## 1. Lexical Grammar

### 1.1 Whitespace and Comments

```ebnf
Whitespace ::= ' ' | '\t' | '\n' | '\r'

Comment ::= LineComment | DocComment | BlockComment
LineComment ::= '//' [^\n]* '\n'
DocComment ::= '///' [^\n]* '\n'
BlockComment ::= '/*' (BlockComment | [^*] | '*' [^/])* '*/'
```

Comments nest (unlike C/Java). Doc comments (`///`) are distinct from regular line comments and are preserved for documentation tooling.

### 1.2 Identifiers

```ebnf
Ident ::= IdentStart IdentContinue*
IdentStart ::= [a-zA-Z_]
IdentContinue ::= [a-zA-Z0-9_]

TypeIdent ::= [A-Z] IdentContinue*        (* Types start with uppercase *)
ValueIdent ::= [a-z_] IdentContinue*      (* Values start with lowercase *)
LifetimeIdent ::= '\'' Ident              (* Lifetimes prefixed with ' *)
```

### 1.3 Literals

```ebnf
Literal ::= IntLiteral | FloatLiteral | StringLiteral | CharLiteral | BoolLiteral

IntLiteral ::= DecInt | HexInt | OctInt | BinInt
DecInt ::= [0-9] [0-9_]*
HexInt ::= '0x' [0-9a-fA-F_]+
OctInt ::= '0o' [0-7_]+
BinInt ::= '0b' [01_]+

(* Integer type suffixes *)
IntSuffix ::= 'i8' | 'i16' | 'i32' | 'i64' | 'i128' | 'isize'
            | 'u8' | 'u16' | 'u32' | 'u64' | 'u128' | 'usize'

FloatLiteral ::= DecInt '.' DecInt FloatExponent? FloatSuffix?
FloatExponent ::= [eE] [+-]? DecInt
FloatSuffix ::= 'f32' | 'f64'

StringLiteral ::= '"' StringChar* '"' | RawStringLiteral | ByteStringLiteral
ByteStringLiteral ::= 'b"' StringChar* '"'
StringChar ::= [^"\\] | EscapeSeq
EscapeSeq ::= '\\' ([nrt\\'"0] | 'x' HexDigit HexDigit | 'u{' HexDigit+ '}')

RawStringLiteral ::= 'r' RawStringBody
RawStringBody ::= '"' [^"]* '"' | '#' RawStringBody '#'

CharLiteral ::= '\'' (CharChar | EscapeSeq) '\''
CharChar ::= [^'\\]

BoolLiteral ::= 'true' | 'false'
```

### 1.4 Operators and Punctuation

```ebnf
(* Single-character *)
Punct1 ::= '(' | ')' | '{' | '}' | '[' | ']'
         | ',' | ';' | ':' | '.' | '@' | '#' | '!'

(* `!` serves dual roles: logical NOT (prefix operator, §7.1 level 14)
   and macro invocation sigil (`name!(args)`, see §5.6 MacroCallExpr). *)

(* Multi-character *)
Punct2 ::= '->' | '=>' | '..' | '..=' | '|>'

(* Operators - see Section 7 for precedence *)
```

### 1.5 Attributes

```ebnf
(* Outer attributes - apply to the following item *)
OuterAttribute ::= '#[' AttributeContent ']'

(* Inner attributes - apply to the enclosing item *)
InnerAttribute ::= '#![' AttributeContent ']'

AttributeContent ::= AttributePath AttributeInput?
AttributePath ::= Ident ('.' Ident)*
AttributeInput ::= '(' AttributeArgs ')'
                 | '=' Literal

AttributeArgs ::= (AttributeArg (',' AttributeArg)* ','?)?
AttributeArg ::= Ident ('=' Literal)?
               | Literal
```

#### 1.5.1 Standard Attributes

```blood
// Function attributes
#[inline]                          // Hint to inline
#[inline(always)]                  // Force inline
#[inline(never)]                   // Never inline
#[cold]                            // Unlikely to be called
#[no_panic]                        // Compile error if can panic
#[stable]                          // Assert type stability
#[unstable(reason = "...")]        // Opt-out of type stability
#[must_use]                        // Warn if return value unused
#[deprecated(since = "1.0", note = "use foo instead")]

// Type attributes
#[repr(C)]                         // C-compatible layout
#[repr(packed)]                    // No padding
#[repr(align(N))]                  // Minimum alignment
#[derive(Clone, Debug, Eq)]        // Auto-derive traits

// Module attributes
#![no_prelude]                     // Don't import prelude
#![feature(unstable_feature)]      // Enable unstable feature

// Test attributes
#[test]                            // Mark as test function
#[bench]                           // Mark as benchmark
#[ignore]                          // Skip test
#[should_panic]                    // Test expects panic

// Conditional compilation
#[cfg(target_os = "linux")]
#[cfg(feature = "async")]
#[cfg(debug_assertions)]
```

---

## 2. Program Structure

### 2.1 Compilation Unit

```ebnf
Program ::= ModuleDecl? Item*
Item    ::= Import | Declaration

ModuleDecl ::= 'module' ModulePath ';'
ModulePath ::= Ident ('.' Ident)*

Import ::= Visibility? 'use' ImportPath ('as' Ident)? ';'
         | Visibility? 'use' ImportPath '.{' ImportList '}' ';'
         | Visibility? 'use' ImportPath '.*' ';'
ImportPath ::= ModulePath ('.' Ident)?
ImportList ::= ImportItem (',' ImportItem)* ','?
ImportItem ::= Ident ('as' Ident)?
```

Imports can appear anywhere among declarations (not restricted to a file preamble). The `pub` visibility modifier enables re-exports.

### 2.2 Module System

```blood
module std.collections.vec;

use std.mem.allocate;
use std.iter.{Iterator, IntoIterator};
use std.ops.*;

// Re-exports
pub use std.collections.hashmap.HashMap;
pub use std.iter.*;

// Sub-module declarations
mod lexer;                     // loads lexer.blood from same directory
mod utils { fn helper() { } } // inline sub-module
```

**Path separator rule:** Blood uses `.` (dot) as the universal path separator — for module paths, qualified types, enum constructors, and imports. Blood does not use `::` anywhere.

**Disambiguation:** Types start with uppercase (`TypeIdent`), values start with lowercase (`ValueIdent`). This convention makes `collections.HashMap` (module-qualified type) visually distinct from `my_struct.field` (value field access) without needing a separate operator. See Appendix B.2.

---

## 3. Declarations

### 3.1 Declaration Grammar

```ebnf
Declaration ::=
    | FnDecl
    | TypeDecl
    | StructDecl
    | EnumDecl
    | EffectDecl
    | HandlerDecl
    | ConstDecl
    | StaticDecl
    | ImplBlock
    | TraitDecl
    | ModDecl
    | BridgeDecl
    | MacroDecl
```

### 3.2 Function Declaration

```ebnf
FnDecl ::= Visibility? FnQualifier* 'fn' Ident TypeParams? '(' Params ')'
           ('->' Type)? ('/' EffectRow)? SpecClause* WhereClause? (Block | ';')

SpecClause ::= 'requires' Expr
             | 'ensures' Expr
             | 'invariant' Expr
             | 'decreases' Expr

FnQualifier ::= 'const' | 'async' | '@unsafe'
(* `async fn foo()` is sugar for `fn foo() / {Async}` — see §8 for the Async effect *)
(* `unsafe` is a keyword but only valid with the `@` prefix. Bare `unsafe` is a compile error
   with a diagnostic suggesting `@unsafe`. See §9.5 for the `@` prefix design rule. *)

Visibility ::= 'pub' ('(' VisScope ')')?
VisScope ::= 'crate' | 'super' | 'self' | ModulePath

TypeParams ::= '<' TypeParam (',' TypeParam)* ','? '>'
TypeParam ::= Ident (':' TypeBound)?
            | 'const' Ident ':' Type        (* const generic parameter *)
TypeBound ::= Type ('+' Type)*

Params ::= (Param (',' Param)* ','?)?
Param ::= ParamQualifier? Pattern ':' Type
ParamQualifier ::= 'linear' | 'affine' | 'mut'

WhereClause ::= 'where' WherePredicate (',' WherePredicate)* ','?
WherePredicate ::= Type ':' TypeBound
                 | Lifetime ':' Lifetime
```

### 3.3 Type Declarations

```ebnf
TypeDecl ::= Visibility? 'type' Ident TypeParams? '=' Type ';'

StructDecl ::= Visibility? 'struct' Ident TypeParams? StructBody
StructBody ::= '{' StructFields '}' | '(' TupleFields ')' ';' | ';'
StructFields ::= (StructField (',' StructField)* ','?)?
StructField ::= Visibility? Ident ':' Type
TupleFields ::= (Type (',' Type)* ','?)?

EnumDecl ::= Visibility? 'enum' Ident TypeParams? '{' EnumVariants '}'
EnumVariants ::= (EnumVariant (',' EnumVariant)* ','?)?
EnumVariant ::= Ident StructBody?
```

### 3.4 Effect and Handler Declarations

```ebnf
EffectDecl ::= 'effect' Ident TypeParams? EffectExtends? '{' OperationDecl* '}'
EffectExtends ::= 'extends' TypePath (',' TypePath)*
OperationDecl ::= 'op' Ident TypeParams? '(' Params ')' '->' Type ';'

HandlerDecl ::= HandlerKind? 'handler' Ident TypeParams?
                'for' Type WhereClause? '{' HandlerBody '}'
HandlerKind ::= 'shallow' | 'deep'
HandlerBody ::= HandlerState* ReturnClause? OperationImpl*
HandlerState ::= 'let' 'mut'? Ident ':' Type ('=' Expr)?
ReturnClause ::= 'return' '(' Ident ')' Block
OperationImpl ::= 'op' Ident '(' Params ')' Block
```

#### 3.4.1 Effect Extension

Effects can extend other effects to form a hierarchy:

```blood
effect IO extends Log {
    op read(fd: Fd, buf: &mut [u8]) -> Result<usize, IoError>;
    // ...
}

effect Async extends IO {
    op spawn<T>(f: fn() -> T / Async) -> TaskHandle<T>;
    op await<T>(future: Future<T>) -> T;
}
```

#### 3.4.2 Handler State

Handlers can declare local state that persists across operation invocations:

```blood
deep handler LocalState<S> for State<S> {
    let mut state: S              // Mutable handler state
    let config: Config = default  // Immutable with default value

    return(x) { (x, state) }
    op get() { resume(state) }
    op put(s) { state = s; resume(()) }
}
```

### 3.5 Trait and Implementation

```ebnf
TraitDecl ::= Visibility? 'trait' Ident TypeParams? (':' TypeBound)?
              WhereClause? '{' TraitItem* '}'
TraitItem ::= FnDecl | TypeDecl | ConstDecl

ImplBlock ::= 'impl' TypeParams? Type ('for' Type)? WhereClause? '{' ImplItem* '}'
ImplItem ::= FnDecl | TypeDecl | ConstDecl
```

### 3.6 Constants and Statics

```ebnf
ConstDecl ::= Visibility? 'const' Ident ':' Type '=' Expr ';'
StaticDecl ::= Visibility? 'static' 'mut'? Ident ':' Type '=' Expr ';'
```

### 3.7 Module Declarations

```ebnf
ModDecl ::= Visibility? 'mod' Ident ';'                    (* external file *)
          | Visibility? 'mod' Ident '{' Item* '}'           (* inline module *)
```

External `mod` declarations load the corresponding file from the same directory:

```blood
mod lexer;          // loads ./lexer.blood
mod parser;         // loads ./parser.blood
mod utils {         // inline module
    pub fn helper() -> i32 { 42 }
}
```

### 3.8 Bridge FFI Declarations

```ebnf
BridgeDecl ::= 'bridge' StringLiteral Ident '{' BridgeItem* '}'
BridgeItem ::= BridgeFn | BridgeConst | BridgeTypeDecl | BridgeStruct

BridgeFn    ::= Attribute* 'fn' Ident '(' BridgeParams ')' ('->' Type)? ';'
BridgeConst ::= 'const' Ident ':' Type '=' Literal ';'
BridgeTypeDecl ::= 'type' Ident ';'
                 | 'type' Ident '=' Type ';'
BridgeStruct ::= Attribute* 'struct' Ident '{' StructFields '}'

BridgeParams ::= (BridgeParam (',' BridgeParam)* (',' '...')?)? (* variadic via ... *)
BridgeParam  ::= Ident ':' Type
```

```blood
bridge "C" libc {
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
    fn printf(format: *const u8, ...) -> i32;    // variadic

    const EOF: i32 = -1;

    type FILE;                                    // opaque type

    #[repr(C)]
    struct TimeSpec {
        tv_sec: i64,
        tv_nsec: i64,
    }
}
```

See [FFI.md](./FFI.md) for full FFI specification including callbacks, safety annotations, and ABI details.

### 3.9 Macro Declarations

```ebnf
MacroDecl ::= Visibility? 'macro' Ident MacroBody
```

**Design status:** The macro system syntax is under active design. See `docs/spec/MACROS.md` for the full macro system design, including definition syntax, capture kinds, hygiene model, and invocation rules.

---

## 4. Types

### 4.1 Type Grammar

```ebnf
Type ::= TypePath
       | ReferenceType
       | PointerType
       | ArrayType
       | SliceType
       | TupleType
       | FunctionType
       | RecordType
       | OwnershipType
       | ForallType
       | DynType
       | '!' (* never type *)
       | '_' (* inferred type *)
       | '(' Type ')'

TypePath ::= TypeIdent TypeArgs?
           | ModulePath '.' TypeIdent TypeArgs?
TypeArgs ::= '<' TypeArg (',' TypeArg)* ','? '>'
TypeArg ::= Type | Lifetime | Const
Const ::= Literal | '-' Literal | Ident | BlockExpr

ReferenceType ::= '&' Lifetime? 'mut'? Type
PointerType ::= '*' ('const' | 'mut') Type

ArrayType ::= '[' Type ';' Expr ']'
SliceType ::= '[' Type ']'

TupleType ::= '(' ')' | '(' Type ',' (Type ',')* Type? ')'

FunctionType ::= 'fn' '(' (Type (',' Type)*)? ')' '->' Type ('/' EffectRow)?

RecordType ::= '{' (RecordField (',' RecordField)*)? ('|' TypeVar)? '}'
RecordField ::= Ident ':' Type

OwnershipType ::= 'linear' Type | 'affine' Type

ForallType ::= 'forall' '<' TypeParam (',' TypeParam)* '>' '.' Type
DynType ::= 'dyn' TypeBound
```

**`forall` types** enable higher-rank polymorphism, which is needed for properly typing effect-polymorphic callbacks:

```blood
fn apply(f: forall<T>. fn(T) -> T, x: i32, y: bool) -> (i32, bool) {
    (f(x), f(y))
}
```

**`dyn Trait`** creates a trait object type for dynamic dispatch:

```blood
fn draw_all(shapes: &[&dyn Drawable]) / {IO} {
    for shape in shapes { shape.draw() }
}
```

> **Design note:** `impl Trait` in argument position is **rejected** — Blood's multiple dispatch already subsumes this use case. Return-position opaque types are **deferred** until real-world pain points are documented; if eventually needed, `opaque` type aliases are preferred over `impl Trait` syntax. See `docs/design/IMPL_TRAIT.md` for the full evaluation.

### 4.2 Effect Types

> **See Also**: [FORMAL_SEMANTICS.md §1.3](./FORMAL_SEMANTICS.md#13-syntax-definition) for formal effect row notation and [§5.5](./FORMAL_SEMANTICS.md#55-row-polymorphism-rules) for row polymorphism typing rules.

```ebnf
EffectRow ::= 'pure'
            | '{' '}'
            | '{' Effect (',' Effect)* ('|' TypeVar)? '}'

Effect ::= TypePath
```

#### 4.2.1 Effect Row Variables

Effect rows support **row polymorphism** via the optional `| TypeVar` suffix:

```blood
// Closed effect row (exact effects, no polymorphism)
fn precise() -> i32 / {IO, Error<E>} { ... }

// Open effect row (polymorphic, can have additional effects)
fn generic<ε>(f: fn() -> i32 / {IO | ε}) -> i32 / {IO | ε} {
    f()
}
```

**When to use row variables:**

| Syntax | Meaning | Use Case |
|--------|---------|----------|
| `/ pure` | No effects | Pure computation |
| `/ {}` | Empty effect row (same as pure) | Alternative pure syntax |
| `/ {IO}` | Exactly IO effect | Concrete signature |
| `/ {IO, Error<E>}` | Exactly IO and Error | Multiple concrete effects |
| `/ {IO \| ε}` | IO plus any other effects | Effect-polymorphic function |
| `/ ε` | Any effects (fully polymorphic) | Maximum flexibility |

**Row variable naming convention:**
- Use lowercase Greek letters: `ε`, `ρ`, `σ` (or ASCII: `e`, `r`, `s`)
- Convention: `ε` for effects, `ρ` for record rows

**Effect row subtyping:**
```blood
// A function with fewer effects can be used where more are expected
fn pure_fn() -> i32 / pure { 42 }
fn io_fn() -> i32 / {IO} { print("hi"); 42 }

fn takes_io(f: fn() -> i32 / {IO}) -> i32 { f() }

takes_io(pure_fn)  // OK: pure <: {IO}
takes_io(io_fn)    // OK: {IO} <: {IO}
```

### 4.3 Lifetimes

```ebnf
Lifetime ::= LifetimeIdent | '\'static' | '\'_'
```

---

## 5. Expressions

### 5.1 Expression Grammar

```ebnf
Expr ::= ExprWithBlock | ExprWithoutBlock

ExprWithBlock ::= BlockExpr
                | IfExpr
                | IfLetExpr
                | MatchExpr
                | LoopExpr
                | ForExpr
                | WhileExpr
                | WhileLetExpr
                | WithHandleExpr
                | TryWithExpr
                | UnsafeBlock
                | RegionExpr

ExprWithoutBlock ::= Literal
                   | PathExpr
                   | CallExpr
                   | MethodCallExpr
                   | FieldExpr
                   | IndexExpr
                   | TupleExpr
                   | ArrayExpr
                   | RecordExpr
                   | RangeExpr
                   | UnaryExpr
                   | BinaryExpr
                   | CastExpr
                   | TryExpr
                   | AssignExpr
                   | AllocExpr
                   | DefaultExpr
                   | ReturnExpr
                   | BreakExpr
                   | ContinueExpr
                   | ClosureExpr
                   | PerformExpr
                   | ResumeExpr
                   | MacroCallExpr
                   | '(' Expr ')'

PathExpr ::= Ident | ModulePath '.' Ident
```

**Note:** `BinaryExpr` and `UnaryExpr` are disambiguated by the precedence and associativity rules in §7. The parser uses precedence climbing (Pratt parsing). Path disambiguation relies on the uppercase/lowercase convention — see Appendix B.2.

### 5.2 Block and Control Flow

```ebnf
BlockExpr ::= '{' Statement* Expr? '}'

Statement ::= ';'
            | Item
            | LetStatement
            | ExprStatement

LetStatement ::= 'let' Pattern (':' Type)? ('=' Expr)? ';'
ExprStatement ::= ExprWithoutBlock ';' | ExprWithBlock ';'?

IfExpr    ::= 'if' Expr Block ('else' 'if' Expr Block)* ('else' Block)?
IfLetExpr ::= 'if' 'let' Pattern '=' Expr Block ('else' Block)?

MatchExpr ::= 'match' Expr '{' MatchArm* '}'
MatchArm ::= Pattern ('if' Expr)? '=>' Expr ','?

LoopExpr     ::= Label? 'loop' Block
ForExpr      ::= Label? 'for' Pattern 'in' Expr Block
WhileExpr    ::= Label? 'while' Expr Block
WhileLetExpr ::= Label? 'while' 'let' Pattern '=' Expr Block

Label ::= LifetimeIdent ':'
```

### 5.3 Effect Expressions

```ebnf
WithHandleExpr ::= 'with' Expr 'handle' Block

TryWithExpr ::= 'try' Block 'with' '{' TryWithArm* '}'
TryWithArm  ::= TypePath '.' Ident '(' Params ')' '=>' Block ','?

PerformExpr ::= 'perform' TypePath '.' Ident '(' Args ')'
              | 'perform' Ident '(' Args ')'  (* when unambiguous *)

ResumeExpr ::= 'resume' '(' Expr ')'
```

Blood provides two handler expression syntaxes:

- **`with handler_expr handle { body }`** — for reusable, named handlers
- **`try { body } with { arms }`** — for inline, one-off effect handling

```blood
// Named handler (reusable)
let result = with LocalState { state: 0 } handle {
    counter()
}

// Inline handler (one-off)
let result = try {
    let data = read_file("config.txt")
    parse(data)
} with {
    IO.read(path) => { resume(default_data) }
    Error.raise(e) => { log(e); resume(fallback) }
}
```

#### 5.3.1 Implicit Perform (Desugaring)

When a function's effect signature includes an effect, operation calls can omit `perform`:

```blood
fn counter() / {State<i32>} {
    let x = get()      // Desugars to: perform State.get()
    put(x + 1)         // Desugars to: perform State.put(x + 1)
    x
}
```

The compiler resolves bare operation names using:

1. **Current effect context**: Operations from effects in the function's effect row
2. **Lexical scope**: Nearest enclosing `with ... handle` block
3. **Unique match required**: If multiple effects define the same operation name, explicit qualification is required

```ebnf
(* Implicit perform resolution *)
ImplicitPerform ::= Ident '(' Args ')'

(* Resolved during type checking to: *)
(* perform EffectType.operation(args) *)
```

**Ambiguity resolution:**

```blood
// Both State<i32> and MyEffect define 'get'
fn ambiguous() / {State<i32>, MyEffect} {
    // get()                        // ERROR: ambiguous
    let s = perform State.get()     // OK: explicit
    let m = perform MyEffect.get()  // OK: explicit
}
```

### 5.4 Memory Expressions

```ebnf
RegionExpr ::= 'region' Lifetime? Block

UnsafeBlock ::= '@unsafe' Block

AllocExpr ::= '@heap' Expr
            | '@stack' Expr
```

See §9.5 for the `@` prefix design rule.

### 5.5 Closures

```ebnf
ClosureExpr ::= '|' ClosureParams '|' ('->' Type)? ('/' EffectRow)? ClosureBody
              | 'move' '|' ClosureParams '|' ClosureBody

ClosureParams ::= (ClosureParam (',' ClosureParam)*)?
ClosureParam ::= Pattern (':' Type)?

ClosureBody ::= Expr | Block
```

### 5.6 Operators and Calls

```ebnf
CallExpr ::= Expr '(' Args ')'
Args ::= (Arg (',' Arg)* ','?)?
Arg ::= (Ident ':')? Expr

MethodCallExpr ::= Expr '.' Ident TypeArgs? '(' Args ')'

FieldExpr ::= Expr '.' Ident | Expr '.' IntLiteral

IndexExpr ::= Expr '[' Expr ']'

UnaryExpr ::= UnaryOp Expr
BinaryExpr ::= Expr BinaryOp Expr

CastExpr ::= Expr 'as' Type

TryExpr ::= Expr '?'

AssignExpr ::= Expr '=' Expr
             | Expr AssignOp Expr

DefaultExpr ::= 'default'

MacroCallExpr ::= Ident '!' '(' Args? ')'
                | Ident '!' '[' Args? ']'
                | Ident '!' '{' Args? '}'
```

**`?` (try operator):** Propagates errors from `Result` types. `expr?` evaluates `expr`; if it is `Err(e)`, the enclosing function returns `Err(e)`. Use `?` for Result-based error propagation (especially FFI interop). Use algebraic effects for structured error handling with resumption.

**`default`:** Produces the default value for a type, inferred from context.

**Explicit type arguments at call sites:** Blood relies on type inference and type ascription rather than providing call-site type argument syntax. When inference cannot determine a type, annotate the binding:

```blood
// Preferred: type ascription on binding
let values: Vec<i32> = input.split(",").map(parse).collect();
let n: i32 = "42".parse();

// NOT supported: no turbofish or call-site type arguments
// let values = collect::<Vec<i32>>();  // ERROR
```

### 5.7 Data Construction

```ebnf
TupleExpr ::= '(' ')' | '(' Expr ',' (Expr ',')* Expr? ')'

ArrayExpr ::= '[' (Expr (',' Expr)* ','?)? ']'
            | '[' Expr ';' Expr ']'

RecordExpr ::= TypePath '{' RecordExprFields '}'
             | '{' RecordExprFields '}'
RecordExprFields ::= (RecordExprField (',' RecordExprField)* ','?)? RecordBase?
RecordBase ::= '..' Expr
RecordExprField ::= Ident (':' Expr)?
                  | Ident             (* Shorthand: x is same as x: x *)

RangeExpr ::= Expr? '..' Expr?
            | Expr? '..=' Expr
```

#### 5.7.1 Record Update Syntax

The `..base` syntax creates a new record with some fields updated:

```blood
struct Point { x: i32, y: i32, z: i32 }

let p1 = Point { x: 1, y: 2, z: 3 }

// Update specific fields, copy rest from base
let p2 = Point { x: 10, ..p1 }        // Point { x: 10, y: 2, z: 3 }
let p3 = Point { y: 20, z: 30, ..p1 } // Point { x: 1, y: 20, z: 30 }

// Shorthand field syntax
let x = 5
let y = 6
let p4 = Point { x, y, z: 7 }         // Point { x: 5, y: 6, z: 7 }
```

---

## 6. Patterns

### 6.1 Pattern Grammar

```ebnf
Pattern ::= LiteralPattern
          | IdentPattern
          | WildcardPattern
          | RestPattern
          | ReferencePattern
          | StructPattern
          | TupleStructPattern
          | TuplePattern
          | SlicePattern
          | OrPattern
          | RangePattern
          | '(' Pattern ')'

LiteralPattern ::= Literal | '-' IntLiteral | '-' FloatLiteral

IdentPattern ::= 'ref'? 'mut'? Ident ('@' Pattern)?

WildcardPattern ::= '_'

RestPattern ::= '..'

ReferencePattern ::= '&' 'mut'? Pattern

StructPattern ::= TypePath '{' StructPatternFields '}'
StructPatternFields ::= (StructPatternField (',' StructPatternField)* ','?)? RestPattern?
StructPatternField ::= Ident (':' Pattern)?

TupleStructPattern ::= TypePath '(' TuplePatternItems ')'

TuplePattern ::= '(' TuplePatternItems ')'
TuplePatternItems ::= (Pattern (',' Pattern)* ','?)? RestPattern?

SlicePattern ::= '[' (Pattern (',' Pattern)* ','?)? RestPattern? ']'

OrPattern ::= Pattern ('|' Pattern)+

RangePattern ::= RangePatternBound '..' RangePatternBound?
               | RangePatternBound '..=' RangePatternBound
RangePatternBound ::= Literal | '-' Literal | PathExpr
```

---

## 7. Operators and Precedence

### 7.1 Operator Precedence Table

From highest to lowest precedence:

| Precedence | Category | Operators | Associativity |
|------------|----------|-----------|---------------|
| 17 | Method call | `.method()` | Left |
| 16 | Field access | `.field` | Left |
| 15 | Postfix | `()` `[]` `?` | Left |
| 14 | Unary | `!` `-` `*` `&` `&mut` | Right |
| 13 | Cast | `as` | Left |
| 12 | Multiply | `*` `/` `%` | Left |
| 11 | Add | `+` `-` | Left |
| 10 | Shift | `<<` `>>` | Left |
| 9 | Bitwise AND | `&` | Left |
| 8 | Bitwise XOR | `^` | Left |
| 7 | Bitwise OR | `\|` | Left |
| 6 | Comparison | `==` `!=` `<` `>` `<=` `>=` | Non-assoc |
| 5 | Logical AND | `&&` | Left |
| 4 | Logical OR | `\|\|` | Left |
| 3 | Range | `..` `..=` | Non-assoc |
| 2 | Pipe | `\|>` | Left |
| 1 | Assign | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` | Right |
| 0 | Return/Break | `return` `break` `continue` | Right |

**Design notes:**

- **Pipe binds tighter than assignment** so `x = data |> transform |> collect` parses as `x = (data |> transform |> collect)`.
- **Comparison operators are non-associative.** `a < b < c` is a parse error. Comparison chaining is **not planned** — hidden temporaries conflict with linear types, and short-circuit evaluation creates ambiguous effect ordering. Range containment (`x in lo..hi`) is the recommended alternative for the dominant use case. See `docs/design/COMPARISON_CHAINING.md` for the full evaluation.
- **Postfix `?`** is the try operator for error propagation (see §5.6).
- **Path (`.`)** is handled during postfix parsing, not as a binary operator in the precedence table.

### 7.2 Unary Operators

```ebnf
UnaryOp ::= '!'    (* logical/bitwise NOT *)
          | '-'    (* negation *)
          | '*'    (* dereference *)
          | '&'    (* reference *)
          | '&mut' (* mutable reference *)
```

### 7.3 Binary Operators

```ebnf
(* Arithmetic *)
ArithOp ::= '+' | '-' | '*' | '/' | '%'

(* Comparison *)
CmpOp ::= '==' | '!=' | '<' | '>' | '<=' | '>='

(* Logical *)
LogicOp ::= '&&' | '||'

(* Bitwise *)
BitOp ::= '&' | '|' | '^' | '<<' | '>>'

(* Assignment *)
AssignOp ::= '+=' | '-=' | '*=' | '/=' | '%='
           | '&=' | '|=' | '^=' | '<<=' | '>>='

BinaryOp ::= ArithOp | CmpOp | LogicOp | BitOp
```

### 7.4 Pipe Operator

Blood includes a pipe operator for function chaining:

```blood
// These are equivalent:
let result = input |> step1 |> step2 |> step3;
let result = step3(step2(step1(input)));
```

---

## 8. Effect Syntax

> **See Also**: [FORMAL_SEMANTICS.md §3](./FORMAL_SEMANTICS.md#3-expression-typing) for effect typing rules, [§4](./FORMAL_SEMANTICS.md#4-evaluation-semantics) for operational semantics, and [§8](./FORMAL_SEMANTICS.md#8-linear-types-and-effects-interaction) for linear types and effects interaction.

### 8.1 Effect Declaration

```blood
effect State<S> {
    op get() -> S;
    op put(s: S) -> unit;
    op modify(f: fn(S) -> S) -> unit;
}
```

### 8.2 Handler Declaration

```blood
deep handler LocalState<S> for State<S> {
    let mut state: S

    return(x) { x }

    op get() {
        resume(state)
    }

    op put(s) {
        state = s
        resume(())
    }

    op modify(f) {
        state = f(state)
        resume(())
    }
}
```

### 8.3 Effect Usage

```blood
fn counter() -> i32 / {State<i32>} {
    let current = get()
    put(current + 1)
    current
}

fn main() / {IO} {
    let result = with LocalState { state: 0 } handle {
        counter()
        counter()
        counter()
    }
    println(result)  // prints: (2, 3)
}
```

---

## 9. Reserved Words

Blood uses a three-tier keyword system to balance language expressiveness with identifier availability.

### 9.1 Strict Keywords (Tier 1)

These words cannot be used as identifiers (except via raw identifiers, see §9.4).

```
as async await break const continue crate dyn effect else enum
extern false fn for forall if impl in let linear loop macro
match mod move mut op pub pure ref region return self Self
static struct super trait true try type unsafe use where while
```

**Note:** `unsafe` is only valid with the `@` prefix (`@unsafe`). Bare `unsafe` is a compile error with a diagnostic suggesting `@unsafe`. See §9.5.

### 9.2 Contextual Keywords (Tier 2)

These have special meaning only in specific syntactic positions. They can be used as identifiers elsewhere.

```
handler perform resume     (* effect/handler declarations and expressions *)
shallow deep               (* handler kind qualifiers *)
requires ensures           (* specification clauses — see §3.2 *)
invariant decreases        (* specification clauses — see §3.2 *)
extends                    (* trait/effect extension *)
bridge                     (* FFI declarations *)
with handle                (* handler expressions *)
affine                     (* ownership type qualifier *)
default                    (* impl blocks, default expressions *)
union                      (* type declarations *)
'static '_                 (* lifetimes *)
```

Examples of contextual keywords used as identifiers:

```blood
let handler = create_handler();      // OK — handler is a variable name
let resume = checkpoint.resume();    // OK — resume is a method name
let shallow = false;                 // OK — shallow is a variable name
fn perform(action: Action) { ... }   // OK — perform is a function name

// These same words become keywords in their respective contexts:
shallow handler MyHandler for MyEffect { ... }  // handler, shallow are keywords here
fn foo() requires x > 0 { ... }                 // requires is a keyword here
```

### 9.3 Reserved for Future Use (Tier 3)

These cannot be used as identifiers but have no current meaning. They are reserved to prevent identifier conflicts if Blood adopts these features later.

```
abstract become box catch defer do final finally
gen override priv raw select spawn throw typeof
unsized virtual yield
```

### 9.4 Raw Identifiers

Any strict or reserved keyword can be used as an identifier by prefixing with `r#`. This is primarily useful for FFI interop and serialization where field names collide with keywords.

```ebnf
RawIdent ::= 'r#' Ident
```

```blood
struct JsonPayload {
    r#type: String,          // field named "type"
    r#match: bool,           // field named "match"
}

bridge "C" lib {
    fn r#continue() -> i32;  // C function named "continue"
}
```

### 9.5 The `@` Prefix

The `@` prefix marks operations that alter the language's default safety, allocation, or execution model. These are operations a reviewer should be able to find with `grep @`.

| Construct | Meaning | Category |
|-----------|---------|----------|
| `@unsafe` | Disables safety checks within block or function | Safety relaxation |
| `@heap`   | Allocates on the heap (explicit placement) | Allocation control |
| `@stack`  | Allocates on the stack (explicit placement) | Allocation control |

**`@` is NOT used for:**

- **Optimization hints** — use attributes: `#[inline]`, `#[cold]`
- **Metadata/annotations** — use attributes: `#[deprecated]`, `#[test]`
- **Constraint qualifiers** — use bare keywords: `linear`, `affine`

**Design rationale:** Constraints like `linear` and `affine` *add* safety guarantees (the compiler checks more, not less). The `@` prefix marks *relaxations* of the default safety model. This distinction is deliberate — `@` means "caution: default guarantees weakened here," not "annotation."

---

## Appendix A: Complete Grammar (Consolidated)

For machine processing, see `grammar.ebnf` in the Blood repository.

---

## Appendix B: Grammar Disambiguation Rules

### B.0 Dangling Else Problem

**Blood's grammar is unambiguous with respect to the "dangling else" problem.**

The classic dangling else ambiguity:

```c
// C: Which `if` does the `else` belong to?
if (a) if (b) x(); else y();

// Interpretation 1: else binds to inner if
if (a) { if (b) x(); else y(); }

// Interpretation 2: else binds to outer if
if (a) { if (b) x(); } else y();
```

Blood **eliminates this ambiguity by requiring blocks** for all control flow:

```ebnf
IfExpr ::= 'if' Expr Block ('else' 'if' Expr Block)* ('else' Block)?
```

Note that `Block` is required, not optional. This means:

```blood
// Blood: Blocks are mandatory, no ambiguity possible
if a { if b { x() } else { y() } }  // OK: else belongs to inner if
if a { if b { x() } } else { y() }  // OK: else belongs to outer if

// These are INVALID in Blood:
// if a if b x() else y()           // ERROR: missing blocks
// if a { if b x() } else y()       // ERROR: missing blocks
```

This design choice follows Rust and Go, trading a small amount of verbosity for complete grammatical clarity.

### B.1 Expression vs Statement

A block `{ }` is a statement if followed by a semicolon, otherwise an expression.

```blood
let x = { compute() };  // Block is expression, value assigned to x
{ compute() };          // Block is statement (semicolon), value discarded
{ compute() }           // Block is expression (final expression in block)
```

### B.2 Path Disambiguation

Blood uses `.` as the universal path separator. Disambiguation between field access and module-qualified paths relies on the uppercase/lowercase naming convention:

```blood
// Uppercase left side → type/module qualified access
Option.Some(42)              // enum constructor (Option is uppercase = type)
collections.HashMap.new()   // module-qualified type constructor

// Lowercase left side → value field/method access
my_struct.field              // field access (my_struct is lowercase = value)
list.len()                   // method call

// In type positions, `.` is always a module path
let map: collections.HashMap<K, V> = ...;

// In import positions, `.` is always a module path
use std.collections.hashmap.HashMap;
```

**Scope-based resolution:** When the left side is lowercase and could be either a local variable or a module name, local variables take priority. If a module access is intended, use the full module path or rename the local variable.

```blood
let io = 42;
// io.read()  — field/method access on the variable `io`, NOT module access
```

**Generic type arguments:** In type positions, `<` after a type name is always a type argument list (no ambiguity with comparison). In expression positions, `<` is always the less-than operator. Use type ascription on bindings when the compiler needs explicit type information:

```blood
let x: Vec<i32> = items.collect();    // type ascription resolves the type
```

### B.3 Closure vs Or-Pattern

`|` after `match` arm pattern starts an or-pattern; `|` at statement start begins a closure.

---

*Last updated: 2026-02-28*
