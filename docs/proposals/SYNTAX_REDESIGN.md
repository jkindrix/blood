# Blood Syntax Redesign RFC

**Version:** 1.0
**Date:** 2026-02-27
**Status:** Active RFC
**Grammar Baseline:** GRAMMAR.md v0.3.0

---

## 1. Blood's Identity

Blood is an **AI-native, effects-first systems language**. Its syntax should optimize for:

1. **Effect clarity** — Effects are the primary differentiator. Every function's side effects are visible in its signature.
2. **AI readability** — Blood code should be maximally parseable by LLMs: explicit, unambiguous, low-noise.
3. **Systems performance** — Zero-cost abstractions, direct hardware access, no GC.
4. **Verification friendliness** — Specifications are first-class, not afterthoughts.
5. **Token efficiency** — Fewer tokens means more code fits in AI context windows.

### What Blood Is NOT

Blood is not Rust. It shares syntax heritage but diverges on:
- Module paths use dots (`std.collections.vec`), not `::`
- `::` is reserved for qualified expressions and grouped imports
- Effects replace lifetimes as the primary safety mechanism
- Specifications are language-level, not attributes

---

## 2. Current Pain Points

### 2.1 Quantitative Evidence

Audit of the 75,782-line self-hosted compiler source (71 files):

| Pattern | Count | Impact |
|---------|-------|--------|
| `while i < N { ... i = i + 1; }` loops | ~1,024 | Verbose, error-prone, off-by-one risk |
| `x = x + 1` manual increment | ~1,199 | 3 tokens where 1 suffices |
| `x = x - 1` manual decrement | ~84 | Same as above |
| `for` loops used | **0** | Feature exists but compiler predates it |
| `+=` compound assignment used | **0** | Feature exists but compiler predates it |
| `\|>` pipeline operator used | **0** | Feature exists, never adopted |
| Semicolons as noise tokens | ~35,000+ | ~2-3% of all tokens |

### 2.2 The Paradox

The self-hosted compiler **implements** `for` loops, `+=`, `|>`, `else if`, `break`/`continue`, and named arguments — but **uses none of them** in its own source. The compiler predates these features.

This creates a credibility gap: the language's flagship codebase doesn't use the language's own features.

### 2.3 Specific Patterns

**While-counter loops** (most common pattern, ~1,024 instances):
```blood
// Current: 4 lines, manual counter, off-by-one risk
let mut i: u32 = 0;
while i < items.len() {
    let item = items[i];
    // ... use item ...
    i = i + 1;
}

// Desired: 2 lines, automatic counter, no off-by-one
for i in 0..items.len() {
    let item = items[i];
    // ... use item ...
}
```

**Manual increment** (~1,283 instances):
```blood
// Current: 3 tokens
count = count + 1;

// Desired: 1 token equivalent
count += 1;
```

**Early-continue with pre-increment** (32 instances, 11 with pre-increment):
```blood
// Current: awkward pre-increment before continue
if should_skip(items[i]) {
    i = i + 1;
    continue;
}
// ... rest of loop ...
i = i + 1;

// Desired: for loop handles increment automatically
for i in 0..items.len() {
    if should_skip(items[i]) {
        continue;  // no manual increment needed
    }
    // ... rest of loop ...
}
```

**Nested function calls vs pipeline** (common in data transformation):
```blood
// Current: read inside-out
let result = serialize(transform(validate(parse(input))));

// Desired: read left-to-right
let result = input |> parse() |> validate() |> transform() |> serialize();
```

---

## 3. Grammar Spec vs. Implementation Audit

The grammar spec (GRAMMAR.md v0.3.0) defines features that the self-hosted compiler's source code doesn't use:

| Feature | In Grammar? | In Compiler? | Used in Compiler Source? | Used in Stdlib? |
|---------|-------------|-------------|--------------------------|-----------------|
| `for i in range` | Yes (§5) | Yes (implemented) | **No** | Yes (119 uses) |
| `+=`, `-=`, etc. | Yes (§7, prec 2) | Yes (implemented) | **No** | Yes (74 uses) |
| `\|>` pipeline | Yes (§7, prec 1) | Yes (implemented) | **No** | **No** |
| `else if` | Yes (§5) | Yes (implemented) | Yes (230 uses) | Yes (30 uses) |
| `break` | Yes (§5, §7 prec 0) | Yes (implemented) | Yes (plain only) | Yes |
| `continue` | Yes (§5, §7 prec 0) | Yes (implemented) | Yes (32 uses) | Yes (11 uses) |
| Named arguments | Yes (§5) | Yes (implemented) | **No** | **No** |
| `break VALUE` | Yes (§7) | Yes (implemented) | **No** | **No** |

### Conclusion

No new grammar is needed for **Category A** changes. The grammar already specifies these features; the compiler already implements them. Only the compiler's own source needs updating.

---

## 4. Proposed Changes

### Category A: Already Works (Grammar v0.3.0, Implemented, Not Adopted)

These require zero compiler changes — only mechanical source updates.

#### A.1 For Loops

**Replace** while-counter patterns with `for i in 0..N`:

```blood
// BEFORE (1,024 instances)
let mut i: u32 = 0;
while i < args.len() {
    process(args[i]);
    i = i + 1;
}

// AFTER
for i in 0..args.len() {
    process(args[i]);
}
```

**Conversion rules:**
- Simple `while i < N` with `i = i + 1` at end → `for i in 0..N`
- Loops with `continue` preceded by `i = i + 1` → `for i in 0..N` with bare `continue`
- Loops modifying the counter mid-body (e.g., `i = i + 2`) → keep as `while`
- Loops where counter is used after the loop → keep as `while` or bind result

**Estimated applicability:** ~80% of 1,024 loops (~820 conversions).

#### A.2 Compound Assignment

**Replace** `x = x + 1` with `x += 1` (and `-=`, `*=`, etc.):

```blood
// BEFORE (1,283 instances)
count = count + 1;
offset = offset + stride;
total = total - consumed;

// AFTER
count += 1;
offset += stride;
total -= consumed;
```

**Conversion rules:**
- `VAR = VAR op EXPR` where `op` is `+`, `-`, `*`, `/`, `%`, `&`, `|`, `^`, `<<`, `>>` → `VAR op= EXPR`
- Only when left-hand side is identical to first operand on right-hand side

**Estimated applicability:** ~95% of 1,283 instances (~1,220 conversions).

#### A.3 Pipeline Operator

**Adopt** `|>` for linear data transformations where readability improves:

```blood
// BEFORE
let name = intern_string(source_text(ctx, span));

// AFTER
let name = span |> source_text(ctx, _) |> intern_string(_);
```

**Adoption criteria:**
- 3+ nested function calls → strong candidate
- Data flows left-to-right naturally → candidate
- Single function call → not a candidate (no benefit)

**Estimated applicability:** Selective adoption, ~20-50 instances.

#### A.4 Continue in While Loops

Already used (32 instances). No change needed — will naturally be used more as while loops become for loops.

### Category B: New Grammar Additions

These require parser, AST, and potentially typeck/codegen changes.

#### B.1 Specification Annotations

**New keywords:** `requires`, `ensures`, `invariant`, `decreases`

**Grammar addition to GRAMMAR.md:**

```ebnf
FnDecl ::= Visibility? FnQualifier* 'fn' Ident TypeParams?
           '(' Params ')' ('->' Type)? ('/' EffectRow)?
           SpecClause*
           WhereClause? (Block | ';')

SpecClause ::= 'requires' Expr
             | 'ensures' Expr
             | 'invariant' Expr
             | 'decreases' Expr
```

**Examples:**

```blood
fn binary_search(haystack: &[i32], needle: i32) -> Option<usize>
    requires haystack.len() > 0
    ensures match result {
        Some(i) => haystack[i] == needle,
        None => true,
    }
{
    // ...
}

fn gcd(a: u32, b: u32) -> u32
    requires a > 0
    requires b > 0
    ensures result > 0
    decreases b
{
    if b == 0 { a } else { gcd(b, a % b) }
}
```

**Signature ordering (definitive):**
```blood
#[attributes]                         // 1. Attributes
pub fn name(params) -> ReturnType     // 2. Signature + return
    / {Effects}                       // 3. Effect row
    requires precondition             // 4. Spec clauses
    ensures postcondition
    decreases measure
    where T: Bound                    // 5. Where clause
{                                     // 6. Body
    // ...
}
```

**Implementation plan:**
1. Add `requires`, `ensures`, `invariant`, `decreases` as keywords in lexer
2. Parse spec clauses after effect row, before where clause
3. Store in AST `FnDecl` and HIR `FnDef`
4. Initially: ignore during type checking and codegen (parse-only)
5. Later: runtime assertion insertion (Phase 2), compile-time verification (Phase 3)

#### B.2 Safety Controls

**New syntax:** `#[unchecked(checks)]` attribute and `unchecked(checks) { }` block.

**Grammar addition:**

```ebnf
UncheckedAttr ::= '#[' 'unchecked' '(' SafetyCheckList ')' ']'

UncheckedBlock ::= 'unchecked' '(' SafetyCheckList ')' Block

SafetyCheckList ::= SafetyCheck (',' SafetyCheck)*
SafetyCheck ::= 'generation' | 'bounds' | 'overflow' | 'null' | 'alignment'
```

**Examples:**

```blood
// Function-level: skip generation checks for hot path
#[unchecked(generation)]
fn hot_inner_loop(data: &[f64]) -> f64 {
    let mut sum: f64 = 0.0;
    for i in 0..data.len() {
        sum += data[i];
    }
    sum
}

// Block-level: scoped unchecked region
fn process(data: &[u8]) -> u32 {
    let header = parse_header(data);  // checked

    unchecked(bounds, overflow) {
        // Performance-critical inner loop
        let mut hash: u32 = 0;
        for i in 0..data.len() {
            hash = hash * 31 + data[i] as u32;
        }
        hash
    }
}
```

**Available checks and their overhead:**

| Check | Keyword | Overhead | Risk if Disabled |
|-------|---------|----------|-----------------|
| Generation validation | `generation` | ~1-2 cycles/deref | Use-after-free |
| Bounds checking | `bounds` | ~2-5 cycles/access | Buffer overflow |
| Integer overflow | `overflow` | ~1 cycle/op | Wrapping arithmetic |
| Null pointer check | `null` | ~1 cycle/deref | Null dereference |
| Alignment check | `alignment` | ~1 cycle/access | Misaligned access |

**Implementation plan:**
1. Add `unchecked` as keyword in lexer
2. Parse `#[unchecked(...)]` as attribute with safety check list
3. Parse `unchecked(...) { }` as block expression in parser_expr
4. Store in AST/HIR as flags on function definitions or block expressions
5. In codegen: check flags and skip corresponding runtime checks
6. Add `--warn-unchecked` CLI flag to list all unchecked regions

#### B.3 Iterator Protocol (Future)

Not part of this RFC but mentioned for completeness. The `for` loop currently works with ranges (`0..N`). A future RFC will define the `Iterator` trait protocol enabling `for item in collection`.

### Category C: Philosophy Changes

Design decisions that don't change parsing but affect how the language is used.

#### C.1 Semicolon Policy

**Decision:** Semicolons remain the documented standard. Parser will accept missing semicolons.

**Grammar change:**

```ebnf
// Current
Statement ::= ExprStmt ';' | LetStmt ';' | ...

// Proposed (v0.4.0)
Statement ::= ExprStmt ';'? | LetStmt ';'? | ...
```

**Continuation rules** (when newline does NOT terminate a statement):
- Line ends with binary operator (`+`, `-`, `*`, `/`, `&&`, `||`, `|>`, etc.)
- Line ends with comma (`,`)
- Line ends with opening delimiter (`(`, `[`, `{`)
- Line ends with dot (`.`) — method chaining
- Inside parenthesized or bracketed expression

**Examples:**

```blood
// Both valid:
let x = 42;          // with semicolon (standard)
let x = 42           // without semicolon (accepted)

// Continuation:
let result = a
    + b               // continues from previous line (ends with +)
    + c

let list = [
    1,                // comma continues
    2,
    3
]
```

**Implementation plan:**
1. In parser: when expecting `;`, if next token is on a new line and is a valid statement start, treat as implicit semicolon
2. Check continuation rules before inserting implicit semicolon
3. Both styles compile identically — no semantic difference
4. Formatter (future) can enforce one style per project

#### C.2 Named Arguments (Gradual Adoption)

Named arguments are in the grammar and implemented. Adoption should be gradual:

```blood
// Positional (current, always valid):
connect("localhost", 5432, true, 30);

// Named (opt-in, clearer):
connect(host: "localhost", port: 5432, tls: true, timeout: 30);

// Mixed (positional first, then named):
connect("localhost", 5432, tls: true, timeout: 30);
```

**Adoption strategy:**
- No breaking changes — positional arguments always work
- Library APIs can document preferred calling convention
- Stdlib functions with 3+ parameters should prefer named arguments
- AI-generated code should prefer named arguments for clarity

#### C.3 Effect Syntax Refinement

Current effect syntax is already well-designed. Minor refinements:

```blood
// Current (good):
fn read_file(path: &str) -> String / {IO, Error<IoError>} { ... }

// Pure function (current):
fn add(a: i32, b: i32) -> i32 / {} { ... }

// Pure function (proposed shorthand, future RFC):
fn add(a: i32, b: i32) -> i32 / pure { ... }
```

The `/ pure` shorthand is deferred — `/ {}` is unambiguous and works today.

---

## 5. Before/After Comparisons

### 5.1 Typical Compiler Function (HIR Lowering)

**Before** (current style, ~20 lines):
```blood
fn lower_items(ctx: &mut LowerCtx, items: &Vec<AstItem>) {
    let mut i: u32 = 0;
    while i < items.len() {
        let item = items[i];
        match item.kind {
            AstItemKind::Fn(ref f) => {
                lower_fn(ctx, f);
            }
            AstItemKind::Struct(ref s) => {
                lower_struct(ctx, s);
            }
            AstItemKind::Enum(ref e) => {
                lower_enum(ctx, e);
            }
            _ => {
                ctx.error("unsupported item kind", item.span);
            }
        }
        i = i + 1;
    }
}
```

**After** (modernized, ~16 lines):
```blood
fn lower_items(ctx: &mut LowerCtx, items: &Vec<AstItem>) {
    for i in 0..items.len() {
        let item = items[i];
        match item.kind {
            AstItemKind::Fn(ref f) => {
                lower_fn(ctx, f);
            }
            AstItemKind::Struct(ref s) => {
                lower_struct(ctx, s);
            }
            AstItemKind::Enum(ref e) => {
                lower_enum(ctx, e);
            }
            _ => {
                ctx.error("unsupported item kind", item.span);
            }
        }
    }
}
```

**Savings:** 3 lines per loop (variable declaration, increment, and the loop is tighter). Across ~820 applicable loops: ~2,460 lines removed.

### 5.2 Counter-Heavy Function (Main Driver)

**Before** (current, from `main.blood`):
```blood
fn process_args(args: &Vec<String>) -> Options {
    let mut opts = Options::default();
    let mut i: u32 = 1;  // skip program name
    while i < args.len() {
        let arg = args[i];
        if str_eq(arg, "--help") {
            opts.show_help = true;
        } else if str_eq(arg, "--verbose") {
            opts.verbose = true;
        } else if str_eq(arg, "-o") {
            i = i + 1;
            if i < args.len() {
                opts.output = args[i];
            }
        } else {
            opts.input = arg;
        }
        i = i + 1;
    }
    opts
}
```

**After:**
```blood
fn process_args(args: &Vec<String>) -> Options {
    let mut opts = Options::default();
    let mut i: u32 = 1;  // skip program name; variable step, keep while
    while i < args.len() {
        let arg = args[i];
        if str_eq(arg, "--help") {
            opts.show_help = true;
        } else if str_eq(arg, "--verbose") {
            opts.verbose = true;
        } else if str_eq(arg, "-o") {
            i += 1;
            if i < args.len() {
                opts.output = args[i];
            }
        } else {
            opts.input = arg;
        }
        i += 1;
    }
    opts
}
```

**Note:** This loop has variable step (`i = i + 1` after `-o` flag), so it stays as `while`. Only the increments become `+=`.

### 5.3 Full Function with Specifications (Future)

**Current** (no specifications):
```blood
pub fn binary_search(sorted: &[i32], target: i32) -> i32 {
    let mut lo: u32 = 0;
    let mut hi: u32 = sorted.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if sorted[mid] == target {
            return mid as i32;
        } else if sorted[mid] < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    -1
}
```

**Modernized + Specifications:**
```blood
pub fn binary_search(sorted: &[i32], target: i32) -> i32
    requires sorted.len() > 0
    ensures result == -1 || sorted[result as u32] == target
    decreases hi - lo
{
    let mut lo: u32 = 0;
    let mut hi: u32 = sorted.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if sorted[mid] == target {
            return mid as i32;
        } else if sorted[mid] < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    -1
}
```

### 5.4 Safety-Critical Function (Future)

```blood
#[unchecked(generation, bounds)]
pub fn simd_dot_product(a: &[f64], b: &[f64]) -> f64
    requires a.len() == b.len()
    ensures result >= 0.0 || true  // dot product can be negative
{
    let mut sum: f64 = 0.0;
    for i in 0..a.len() {
        sum += a[i] * b[i];
    }
    sum
}
```

---

## 6. Token Efficiency Analysis

### 6.1 Per-Pattern Savings

| Pattern | Before (tokens) | After (tokens) | Savings | Instances | Total Saved |
|---------|-----------------|----------------|---------|-----------|-------------|
| `while` → `for` | ~12 | ~6 | 6/loop | ~820 | ~4,920 tokens |
| `x = x + 1` → `x += 1` | 5 | 3 | 2/instance | ~1,220 | ~2,440 tokens |
| `x = x - 1` → `x -= 1` | 5 | 3 | 2/instance | ~80 | ~160 tokens |
| Pipeline adoption | varies | varies | ~2-4/use | ~30 | ~90 tokens |

### 6.2 Aggregate Impact

| Metric | Before | After | Change |
|--------|--------|-------|--------|
| Estimated total tokens (selfhost) | ~380,000 | ~372,000 | -2.1% |
| Lines of code | ~75,782 | ~73,300 | -3.3% |
| While loops | ~1,024 | ~204 | -80% |
| Manual increments | ~1,283 | ~63 | -95% |

### 6.3 AI Context Window Impact

With an average context window of 128K tokens:
- **Before:** ~380K tokens → requires 3 context windows to see full compiler
- **After:** ~372K tokens → still 3 windows, but ~8K more room for instructions/chat
- **With semicolon flexibility:** Additional ~2-3% savings possible

The real win is readability and pattern recognition. AI systems trained on modern code will generate better Blood code when the compiler itself uses modern idioms.

---

## 7. Grammar Delta from GRAMMAR.md v0.3.0

### 7.1 New Reserved Keywords

Add to Section 9 (Reserved Words):

```
requires ensures invariant decreases unchecked
```

These move from "reserved for future use" to active keywords.

### 7.2 New Productions

#### Specification Clauses (add to Section 3)

```ebnf
FnDecl ::= Visibility? FnQualifier* 'fn' Ident TypeParams?
           '(' Params ')' ('->' Type)? ('/' EffectRow)?
           SpecClause*
           WhereClause? (Block | ';')

SpecClause ::= RequiresClause | EnsuresClause | InvariantClause | DecreasesClause

RequiresClause ::= 'requires' Expr
EnsuresClause ::= 'ensures' Expr
InvariantClause ::= 'invariant' Expr
DecreasesClause ::= 'decreases' Expr
```

**Note:** `result` is a magic identifier in `ensures` clauses, referring to the function return value.

#### Safety Controls (add to Section 1 and Section 5)

```ebnf
// Attribute form (Section 1)
UncheckedAttr ::= 'unchecked' '(' SafetyCheckList ')'
SafetyCheckList ::= SafetyCheck (',' SafetyCheck)*
SafetyCheck ::= 'generation' | 'bounds' | 'overflow' | 'null' | 'alignment'

// Block expression form (Section 5)
UncheckedBlock ::= 'unchecked' '(' SafetyCheckList ')' Block
```

`UncheckedBlock` is added to the `Expr` production at the same level as `@unsafe Block`.

#### Semicolon Flexibility (modify Section 2)

```ebnf
// Current
Statement ::= (LetStmt | ExprStmt | Item) ';'

// v0.4.0
Statement ::= (LetStmt | ExprStmt | Item) ';'?
// Where ';'? inserts an implicit semicolon when:
//   - Next token is on a new line AND
//   - Current line does NOT end with a continuation token AND
//   - Next token is a valid statement-start token

ContinuationToken ::= '+' | '-' | '*' | '/' | '%' | '&&' | '||'
                     | '|>' | '.' | ',' | '(' | '[' | '{'
                     | '&' | '|' | '^' | '<<' | '>>'
                     | '==' | '!=' | '<' | '>' | '<=' | '>='
                     | '=' | '+=' | '-=' | '*=' | '/='
```

### 7.3 Version Bump

Grammar version: **v0.3.0 → v0.4.0**

Changes summary:
- 5 new keywords: `requires`, `ensures`, `invariant`, `decreases`, `unchecked`
- 1 new expression form: `UncheckedBlock`
- 4 new clause types: `RequiresClause`, `EnsuresClause`, `InvariantClause`, `DecreasesClause`
- 1 modified rule: `Statement` with optional semicolon

---

## 8. Migration Plan for Self-Hosted Compiler

### 8.1 Phase 0: Mechanical Syntax Updates (No Grammar Changes)

**Batch 1: For loops** (~820 conversions across 65 files)

Process:
1. For each file, identify `while VAR < LIMIT` loops
2. Check if loop counter increments exactly once at end of body
3. Check if counter is not modified elsewhere in body
4. If both conditions met: convert to `for VAR in 0..LIMIT`
5. Remove `let mut VAR: TYPE = 0;` declaration
6. Remove `VAR = VAR + 1;` at end of body
7. If loop had `continue` preceded by `VAR = VAR + 1;`, remove the pre-increment

Verification after batch:
```bash
cd src/selfhost && ./build_selfhost.sh timings
./build_selfhost.sh ground-truth
./build_selfhost.sh rebuild
# Build third_gen from second_gen, verify byte-identical
```

**Batch 2: Compound assignment** (~1,220 conversions)

Process:
1. For each file, find `VAR = VAR + EXPR` patterns
2. Replace with `VAR += EXPR`
3. Same for `-`, `*`, `/`, `%`, `&`, `|`, `^`, `<<`, `>>`

Verification: same as Batch 1.

**Batch 3: Pipeline + misc** (selective adoption)

Process:
1. Identify deeply nested function calls
2. Replace with pipeline where readability improves
3. Verify each change individually

### 8.2 Phase 1: New Grammar Implementation

Order of implementation:
1. **Lexer:** Add `requires`, `ensures`, `invariant`, `decreases`, `unchecked` keywords
2. **AST:** Add `SpecClause` enum and `UncheckedBlock` node
3. **Parser:** Parse spec clauses after effect row; parse unchecked blocks
4. **HIR:** Store spec clauses in `FnDef`; store unchecked flags
5. **Codegen:** Check unchecked flags, skip corresponding checks
6. **Tests:** Add t09_spec_* and t09_unchecked_* ground-truth tests

### 8.3 Phase 2: Semicolon Flexibility

1. **Parser:** When expecting `;`, check if next token starts a new line
2. **Continuation detection:** Check if current token is a continuation token
3. **Implicit insert:** If new line and not continuation, treat as implicit `;`
4. **Tests:** Add t09_semicolon_optional ground-truth test
5. **Grammar update:** Bump GRAMMAR.md to v0.4.0

### 8.4 Bootstrap Verification Protocol

After every batch of changes:

```bash
# Step 1: Build first_gen from blood-rust
cd src/selfhost && ./build_selfhost.sh timings

# Step 2: Run all ground-truth tests
./build_selfhost.sh ground-truth
# Expected: 336/336 pass (or more with new tests)

# Step 3: Build second_gen from first_gen
./build_selfhost.sh rebuild

# Step 4: Build third_gen from second_gen
BLOOD_REF=build/second_gen ./build_selfhost.sh rebuild
# Rename output to third_gen, compare with second_gen

# Step 5: Verify byte-identical
diff build/second_gen build/third_gen
# Must be identical — stable bootstrap
```

---

## 9. Files Affected

### New Files
| File | Purpose |
|------|---------|
| `tests/ground-truth/t09_spec_requires.blood` | Specification requires clause test |
| `tests/ground-truth/t09_spec_ensures.blood` | Specification ensures clause test |
| `tests/ground-truth/t09_unchecked_bounds.blood` | Unchecked bounds attribute test |
| `tests/ground-truth/t09_semicolon_optional.blood` | Semicolonless code test |

### Modified Files (Grammar Implementation)
| File | Changes |
|------|---------|
| `src/selfhost/lexer.blood` | 5 new keyword tokens |
| `src/selfhost/token.blood` | New token variants |
| `src/selfhost/ast.blood` | SpecClause enum, UncheckedBlock node |
| `src/selfhost/parser_item.blood` | Spec clause parsing after effect row |
| `src/selfhost/parser_expr.blood` | Unchecked block parsing |
| `src/selfhost/hir_item.blood` | SpecClause storage in FnDef |
| `src/selfhost/hir_lower_item.blood` | Spec clause lowering |
| `src/selfhost/hir_lower_expr.blood` | Unchecked block lowering |
| `src/selfhost/codegen_expr.blood` | Unchecked flag checking |
| `src/bootstrap/bloodc/src/` | Mirror all changes in Rust bootstrap |
| `docs/spec/GRAMMAR.md` | Version bump to v0.4.0 |

### Modified Files (Modernization)
All 71 files in `src/selfhost/` — mechanical `while`→`for` and `x = x + 1`→`x += 1` updates.

---

## 10. Open Questions

1. **Specification expression scope:** Can `ensures` reference local variables from the function body, or only parameters and `result`?
   - **Proposed:** Only parameters and `result`. Local variable references would require whole-function analysis.

2. **Unchecked block return type:** Does `unchecked(bounds) { expr }` have the type of `expr`?
   - **Proposed:** Yes, it is an expression that returns its body's value, identical to a plain block.

3. **Semicolon in REPL:** Should the REPL (future) always be semicolonless?
   - **Proposed:** Yes, REPL defaults to expression-mode (no semicolons needed).

4. **Pipeline operator left-hand argument position:** Is the LHS always the first argument?
   - **Current grammar:** Yes, `a |> f(b)` desugars to `f(a, b)`.
   - **Alternative:** Use `_` placeholder: `a |> f(_, b)` desugars to `f(a, b)`. Defer to future RFC.

5. **Specification on trait methods:** Are spec clauses allowed on trait method declarations?
   - **Proposed:** Yes. Implementations must satisfy at least the trait's spec.

---

## 11. Summary

| Category | Changes | Scope | Breaking? |
|----------|---------|-------|-----------|
| **A (Already Works)** | for loops, +=, \|>, continue | Source modernization only | No |
| **B (New Grammar)** | Specifications, safety controls | Parser + AST + HIR + codegen | No (additive) |
| **C (Philosophy)** | Semicolons, named args, effect shorthand | Parser behavior change | No (both styles work) |

**Net result:** Blood's flagship codebase becomes a showcase for the language's features, specifications become first-class syntax, and the language gains the safety controls needed for performance-critical systems code.

---

*Cross-references: [PROPOSAL_ANALYSIS.md](PROPOSAL_ANALYSIS.md) for feature roadmap, [GRAMMAR.md](../spec/GRAMMAR.md) for current grammar.*
