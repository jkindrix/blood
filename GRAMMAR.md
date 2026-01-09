# Blood Surface Syntax Grammar

**Version**: 0.1.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

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
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — Operational semantics for expressions
- [STDLIB.md](./STDLIB.md) — Standard library type signatures
- [FFI.md](./FFI.md) — Bridge block syntax
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — Parse error messages

---

## 1. Lexical Grammar

### 1.1 Whitespace and Comments

```ebnf
Whitespace ::= ' ' | '\t' | '\n' | '\r'

Comment ::= LineComment | BlockComment
LineComment ::= '//' [^\n]* '\n'
BlockComment ::= '/*' (BlockComment | [^*/])* '*/'
```

Comments nest (unlike C/Java).

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
IntSuffix ::= 'i8' | 'i16' | 'i32' | 'i64' | 'i128'
            | 'u8' | 'u16' | 'u32' | 'u64' | 'u128'

FloatLiteral ::= DecInt '.' DecInt FloatExponent? FloatSuffix?
FloatExponent ::= [eE] [+-]? DecInt
FloatSuffix ::= 'f32' | 'f64'

StringLiteral ::= '"' StringChar* '"'
StringChar ::= [^"\\] | EscapeSeq
EscapeSeq ::= '\\' ([nrt\\'"0] | 'x' HexDigit HexDigit | 'u{' HexDigit+ '}')

CharLiteral ::= '\'' (CharChar | EscapeSeq) '\''
CharChar ::= [^'\\]

BoolLiteral ::= 'true' | 'false'
```

### 1.4 Operators and Punctuation

```ebnf
(* Single-character *)
Punct1 ::= '(' | ')' | '{' | '}' | '[' | ']'
         | ',' | ';' | ':' | '.' | '@' | '#'

(* Multi-character *)
Punct2 ::= '->' | '=>' | '::' | '..' | '..=' | '|>'

(* Operators - see Section 7 for precedence *)
```

### 1.5 Attributes

```ebnf
(* Outer attributes - apply to the following item *)
OuterAttribute ::= '#[' AttributeContent ']'

(* Inner attributes - apply to the enclosing item *)
InnerAttribute ::= '#![' AttributeContent ']'

AttributeContent ::= AttributePath AttributeInput?
AttributePath ::= Ident ('::' Ident)*
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
Program ::= ModuleDecl? Import* Declaration*

ModuleDecl ::= 'module' ModulePath ';'
ModulePath ::= Ident ('.' Ident)*

Import ::= 'use' ImportPath ('as' Ident)? ';'
         | 'use' ImportPath '::' '{' ImportList '}' ';'
ImportPath ::= ModulePath ('::' '*')?
ImportList ::= Ident (',' Ident)* ','?
```

### 2.2 Module System

```blood
module std.collections.vec;

use std.mem.allocate;
use std.iter::{Iterator, IntoIterator};
use std.ops::*;
```

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
```

### 3.2 Function Declaration

```ebnf
FnDecl ::= Visibility? FnQualifier* 'fn' Ident TypeParams? '(' Params ')'
           ('->' Type)? ('/' EffectRow)? WhereClause? (Block | ';')

FnQualifier ::= 'const' | 'async' | 'unsafe'

Visibility ::= 'pub' ('(' VisScope ')')?
VisScope ::= 'crate' | 'super' | 'self' | ModulePath

TypeParams ::= '<' TypeParam (',' TypeParam)* ','? '>'
TypeParam ::= Ident (':' TypeBound)?
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
EffectExtends ::= 'extends' Type (',' Type)*
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
       | '!' (* never type *)
       | '_' (* inferred type *)
       | '(' Type ')'

TypePath ::= TypeIdent TypeArgs?
           | ModulePath '::' TypeIdent TypeArgs?
TypeArgs ::= '<' TypeArg (',' TypeArg)* ','? '>'
TypeArg ::= Type | Lifetime | Const

ReferenceType ::= '&' Lifetime? 'mut'? Type
PointerType ::= '*' ('const' | 'mut') Type

ArrayType ::= '[' Type ';' Expr ']'
SliceType ::= '[' Type ']'

TupleType ::= '(' ')' | '(' Type ',' (Type ',')* Type? ')'

FunctionType ::= 'fn' '(' (Type (',' Type)*)? ')' '->' Type ('/' EffectRow)?

RecordType ::= '{' (RecordField (',' RecordField)*)? ('|' TypeVar)? '}'
RecordField ::= Ident ':' Type

OwnershipType ::= 'linear' Type | 'affine' Type | 'Box' TypeArgs
```

### 4.2 Effect Types

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
                | MatchExpr
                | LoopExpr
                | ForExpr
                | WhileExpr
                | WithHandleExpr
                | UnsafeBlock
                | RegionExpr

ExprWithoutBlock ::= Literal
                   | PathExpr
                   | OperatorExpr
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
                   | AssignExpr
                   | ReturnExpr
                   | BreakExpr
                   | ContinueExpr
                   | ClosureExpr
                   | PerformExpr
                   | ResumeExpr
                   | '(' Expr ')'
```

### 5.2 Block and Control Flow

```ebnf
BlockExpr ::= '{' Statement* Expr? '}'

Statement ::= ';'
            | Item
            | LetStatement
            | ExprStatement

LetStatement ::= 'let' Pattern (':' Type)? ('=' Expr)? ';'
ExprStatement ::= ExprWithoutBlock ';' | ExprWithBlock ';'?

IfExpr ::= 'if' Expr Block ('else' 'if' Expr Block)* ('else' Block)?

MatchExpr ::= 'match' Expr '{' MatchArm* '}'
MatchArm ::= Pattern ('if' Expr)? '=>' Expr ','?

LoopExpr ::= Label? 'loop' Block
ForExpr ::= Label? 'for' Pattern 'in' Expr Block
WhileExpr ::= Label? 'while' Expr Block

Label ::= LifetimeIdent ':'
```

### 5.3 Effect Expressions

```ebnf
WithHandleExpr ::= 'with' Expr 'handle' Expr

PerformExpr ::= 'perform' TypePath '.' Ident '(' Args ')'
              | 'perform' Ident '(' Args ')'  (* when unambiguous *)

ResumeExpr ::= 'resume' '(' Expr ')'
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
RegionExpr ::= 'region' Ident? Block

(* @unsafe is the canonical syntax - distinctive and grep-able *)
UnsafeBlock ::= '@unsafe' Block

AllocExpr ::= '@heap' Expr
            | '@stack' Expr
            | 'Box' '::' 'new' '(' Expr ')'
```

**Note:** Blood uses `@unsafe` (with the `@` prefix) rather than bare `unsafe` to:
1. Make unsafe blocks visually distinctive and easy to audit
2. Enable simple text search (`grep @unsafe`) for security review
3. Distinguish from potential future `unsafe` type modifiers

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

AssignExpr ::= Expr '=' Expr
             | Expr AssignOp Expr
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
| 18 | Path | `::` | Left |
| 17 | Method call | `.method()` | Left |
| 16 | Field access | `.field` | Left |
| 15 | Function call | `()` | Left |
| 15 | Index | `[]` | Left |
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
| 2 | Assign | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` | Right |
| 1 | Pipe | `\|>` | Left |
| 0 | Return/Break | `return` `break` `continue` | Right |

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

### 9.1 Keywords

```
as async await break const continue crate
deep dyn effect else enum extends extern
false fn for handler if impl in
let linear loop match mod move mut
perform pub ref region resume return
self Self shallow static struct super
trait true type use where while
```

**Note:** `unsafe` is not a keyword in Blood. Use `@unsafe` block syntax instead (see Section 5.4).

### 9.2 Reserved for Future Use

```
abstract become box do final macro
override priv typeof unsized virtual yield
try catch finally throw
```

### 9.3 Contextual Keywords

These have special meaning only in specific contexts:

```
union     (* only in type declarations *)
default   (* only in impl blocks *)
'static   (* lifetime *)
'_        (* inferred lifetime *)
```

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

### B.2 Turbofish

When `<` is ambiguous between comparison and type arguments, use turbofish `::< >`:

```blood
// Ambiguous: is this less-than or type args?
foo(bar<T, U>(x))

// Clear: turbofish for type args
foo(bar::<T, U>(x))
```

### B.3 Closure vs Or-Pattern

`|` after `match` arm pattern starts an or-pattern; `|` at statement start begins a closure.

---

*Last updated: 2026-01-09*
