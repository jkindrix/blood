# Blood Escape Analysis Specification

**Version**: 0.1.0
**Status**: Specified
**Implementation**: `bloodc/src/mir/escape.rs`
**Last Updated**: 2026-01-14

This document specifies the escape analysis algorithm used in Blood to determine whether values can be stack-allocated or require heap allocation with generational references.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Escape States](#2-escape-states)
3. [Algorithm](#3-algorithm)
4. [Statement Analysis](#4-statement-analysis)
5. [Terminator Analysis](#5-terminator-analysis)
6. [Closure Capture Propagation](#6-closure-capture-propagation)
7. [Stack Promotion Rules](#7-stack-promotion-rules)
8. [Effect Capture Analysis](#8-effect-capture-analysis)
9. [Implementation](#9-implementation)

---

## 1. Overview

### 1.1 Purpose

Escape analysis determines the optimal memory allocation tier for each local variable:

| Escape State | Memory Tier | Generation Checks | Use Case |
|--------------|-------------|-------------------|----------|
| NoEscape | Stack (Tier 0) | NO | Local temporaries |
| ArgEscape | Region (Tier 1) | YES | Returned values |
| GlobalEscape | Region/Persistent | YES | Global storage |

### 1.2 Performance Target

Blood targets **>95% stack allocation** for typical programs (PERF-V-002), enabling:
- Zero-overhead local variables
- No generation check overhead for most operations
- Minimal heap allocation pressure

### 1.3 Related Specifications

- [MEMORY_MODEL.md](./MEMORY_MODEL.md) - Tiered memory architecture
- [MIR_LOWERING.md](./MIR_LOWERING.md) - MIR representation
- [EFFECTS_CODEGEN.md](./EFFECTS_CODEGEN.md) - Effect capture requirements

---

## 2. Escape States

### 2.1 Lattice Structure

Escape states form a three-element lattice:

```
        GlobalEscape
            |
        ArgEscape
            |
        NoEscape
```

The `join` operation computes the least upper bound:

```
join(s1, s2) = max(s1, s2)

NoEscape ⊔ NoEscape     = NoEscape
NoEscape ⊔ ArgEscape    = ArgEscape
NoEscape ⊔ GlobalEscape = GlobalEscape
ArgEscape ⊔ ArgEscape   = ArgEscape
ArgEscape ⊔ GlobalEscape = GlobalEscape
```

### 2.2 State Definitions

```
EscapeState ::=
    | NoEscape       -- Value stays within function, can use stack
    | ArgEscape      -- Value escapes via argument/return, needs region
    | GlobalEscape   -- Value escapes to global/heap, needs region
```

### 2.3 Memory Tier Mapping

```
FUNCTION recommended_tier(state: EscapeState) -> MemoryTier:
    CASE state OF:
        NoEscape    → Stack     -- Tier 0, thin pointer
        ArgEscape   → Region    -- Tier 1, gen-checked
        GlobalEscape → Region   -- Tier 1/2, gen-checked
```

---

## 3. Algorithm

### 3.1 Overview

The analysis uses a worklist-based dataflow algorithm:

1. Initialize all locals to NoEscape
2. Mark return place as ArgEscape
3. Collect closure capture relationships
4. Iterate until fixed point:
   - Analyze statements and terminators
   - Propagate escape through closure captures
5. Determine stack-promotable allocations

### 3.2 Pseudocode

```
ALGORITHM ESCAPE_ANALYSIS(body: MirBody):
    Input:
        body: MirBody -- Function body to analyze
    Output:
        EscapeResults -- Escape state for each local

    // Phase 1: Initialization
    states = {}
    FOR local IN body.locals:
        states[local.id] = NoEscape

    // Return place always escapes to caller
    states[body.return_place] = ArgEscape

    // Phase 2: Collect closure captures
    closure_captures = {}
    captured_by_closure = {}
    FOR block IN body.blocks:
        FOR stmt IN block.statements:
            IF stmt IS Assign(place, Aggregate(Closure, operands)):
                closure_captures[place.local] = extract_locals(operands)
                FOR op IN operands:
                    captured_by_closure.add(op.local)

    // Phase 3: Fixed-point iteration
    LOOP:
        changed = false

        // Phase 3a: Statement/terminator analysis
        FOR block IN body.blocks:
            FOR stmt IN block.statements:
                changed |= analyze_statement(stmt, states)
            IF block.terminator IS SOME(term):
                changed |= analyze_terminator(term, states)

        // Phase 3b: Closure propagation (worklist)
        worklist = {c | c ∈ closure_captures.keys ∧ states[c] ≠ NoEscape}
        processed = {}

        WHILE worklist NOT EMPTY:
            closure = worklist.pop_front()
            IF closure ∈ processed:
                CONTINUE
            processed.add(closure)

            closure_state = states[closure]
            IF closure_state = NoEscape:
                CONTINUE

            FOR captured IN closure_captures[closure]:
                IF update_state(captured, closure_state):
                    changed = true
                    IF captured ∈ closure_captures.keys ∧ captured ∉ processed:
                        worklist.add(captured)

        IF NOT changed:
            BREAK

    // Phase 4: Determine stack-promotable allocations
    stack_promotable = {}
    FOR (local, state) IN states:
        type = body.locals[local].ty
        is_copy = type.is_copy()
        escape_allows = state = NoEscape
                        ∧ local ∉ effect_captured
                        ∧ NOT is_captured_by_escaping_closure(local)

        IF is_copy OR escape_allows:
            stack_promotable.add(local)

    RETURN EscapeResults {
        states,
        effect_captured,
        stack_promotable,
        closure_captures,
        captured_by_closure,
    }
```

### 3.3 Complexity

| Phase | Complexity | Notes |
|-------|------------|-------|
| Initialization | O(n) | n = number of locals |
| Closure collection | O(s) | s = number of statements |
| Fixed-point iteration | O(s × k) | k = lattice height (3) |
| Closure propagation | O(c) | c = number of closures |
| **Total** | O(s) | Linear in function size |

---

## 4. Statement Analysis

### 4.1 Assignment Analysis

```
ALGORITHM ANALYZE_ASSIGNMENT(place, rvalue, states):
    target_state = place_escape_state(place, states)
    changed = false

    CASE rvalue OF:
        Use(operand):
            changed |= propagate_to_operand(operand, target_state)

        Ref(ref_place) | AddressOf(ref_place):
            -- Creating reference: if reference escapes, referent escapes
            changed |= update_state(ref_place.local, target_state)

        BinaryOp(_, left, right) | CheckedBinaryOp(_, left, right):
            changed |= propagate_to_operand(left, target_state)
            changed |= propagate_to_operand(right, target_state)

        UnaryOp(_, operand) | Cast(operand, _):
            changed |= propagate_to_operand(operand, target_state)

        Aggregate(_, operands):
            FOR op IN operands:
                changed |= propagate_to_operand(op, target_state)

        Discriminant(_) | Len(_) | ReadGeneration(_):
            -- Reading properties doesn't cause escape
            PASS

        ZeroInit(_):
            -- No locals referenced
            PASS

    RETURN changed
```

### 4.2 Place Escape State

```
ALGORITHM PLACE_ESCAPE_STATE(place, states):
    base_state = states.get(place.local, NoEscape)

    FOR elem IN place.projection:
        CASE elem OF:
            Deref:
                -- Dereferencing: pointee might have global scope
                RETURN GlobalEscape
            Field(_) | Index(_) | Downcast(_):
                -- Projections don't change escape state
                CONTINUE

    RETURN base_state
```

### 4.3 State Propagation

```
ALGORITHM PROPAGATE_TO_OPERAND(operand, target_state, states):
    CASE operand OF:
        Copy(place) | Move(place):
            RETURN update_state(place.local, target_state)
        Constant(_):
            RETURN false

ALGORITHM UPDATE_STATE(local, new_state, states):
    old_state = states.get(local, NoEscape)
    joined_state = join(old_state, new_state)
    IF joined_state ≠ old_state:
        states[local] = joined_state
        RETURN true
    RETURN false
```

---

## 5. Terminator Analysis

### 5.1 Rules

```
ALGORITHM ANALYZE_TERMINATOR(term, states):
    changed = false

    CASE term OF:
        Call(_, args, _, _, _):
            -- Arguments may escape through call
            FOR arg IN args:
                changed |= propagate_to_operand(arg, ArgEscape)

        Return:
            -- Return value already marked (handled in initialization)
            PASS

        Perform(_, _, args, _, _):
            -- Effect operations capture values
            FOR arg IN args:
                IF arg HAS place:
                    effect_captured.add(place.local)
                    changed |= update_state(place.local, ArgEscape)

        DropAndReplace(_, value, _, _):
            changed |= propagate_to_operand(value, NoEscape)

        Goto(_) | Unreachable | Resume(_) | StaleReference(_):
            PASS

        Assert(cond, _, _, _) | SwitchInt(cond, _):
            -- Conditions don't cause escape
            PASS

    RETURN changed
```

---

## 6. Closure Capture Propagation

### 6.1 Problem

Closures capture references to outer variables. If a closure escapes, its captured variables must also be promoted to region allocation:

```blood
fn example() {
    let x = 42;                      // x: NoEscape initially
    let closure = || x;              // closure captures x
    return closure;                  // closure: ArgEscape
}                                    // Therefore: x must be ArgEscape
```

### 6.2 Algorithm

```
ALGORITHM PROPAGATE_CLOSURE_ESCAPES(closure_captures, states):
    worklist = {c | states[c] ≠ NoEscape}
    processed = {}
    changed = false

    WHILE worklist NOT EMPTY:
        closure = worklist.pop_front()

        IF closure ∈ processed:
            CONTINUE
        processed.add(closure)

        closure_state = states[closure]
        IF closure_state = NoEscape:
            CONTINUE

        captures = closure_captures.get(closure, [])
        FOR captured IN captures:
            IF update_state(captured, closure_state):
                changed = true
                -- If captured is also a closure, add to worklist
                IF captured ∈ closure_captures.keys ∧ captured ∉ processed:
                    worklist.add(captured)

    RETURN changed
```

### 6.3 Transitive Closure

The worklist algorithm handles transitive closure capture chains:

```blood
fn example() {
    let a = 1;
    let b = || a;           // b captures a
    let c = || b();         // c captures b
    return c;               // c: ArgEscape → b: ArgEscape → a: ArgEscape
}
```

---

## 7. Stack Promotion Rules

### 7.1 Criteria

A local can be stack-allocated if ANY of:

1. **Copy type**: Type implements Copy (values are duplicated, storage stays local)
2. **No escape**: NoEscape state AND not effect-captured AND not captured by escaping closure

### 7.2 Decision Tree

```
Can local L be stack-allocated?
│
├── Is type(L) Copy?
│   ├── YES → STACK ✓ (values are copied, storage doesn't escape)
│   └── NO
│       │
│       ├── Is state(L) = NoEscape?
│       │   ├── NO → HEAP ✗
│       │   └── YES
│       │       │
│       │       ├── Is L effect-captured?
│       │       │   ├── YES → HEAP ✗ (effects need gen checks)
│       │       │   └── NO
│       │       │       │
│       │       │       └── Is L captured by escaping closure?
│       │       │           ├── YES → HEAP ✗
│       │       │           └── NO → STACK ✓
```

### 7.3 Copy Type Detection

```
ALGORITHM IS_COPY(type, adt_fields):
    CASE type OF:
        -- Primitives are Copy
        Primitive(_) → true

        -- Unit is Copy
        Tuple([]) → true

        -- Never is Copy (vacuously)
        Never → true

        -- Tuples are Copy if all elements are Copy
        Tuple(elements) → all(is_copy(e) FOR e IN elements)

        -- Arrays are Copy if element is Copy
        Array(elem, _) → is_copy(elem)

        -- References are NOT Copy (they may refer to non-Copy data)
        Ref(_) → false

        -- Pointers are NOT Copy (same reason)
        Ptr(_) → false

        -- ADTs (structs/enums): Check if all fields are Copy
        Adt(def_id, _):
            fields = adt_fields(def_id)
            IF fields IS NONE:
                RETURN false
            RETURN all(is_copy(f) FOR f IN fields)

        -- Default: not Copy
        _ → false
```

---

## 8. Effect Capture Analysis

### 8.1 Problem

Effect operations may capture values in continuations. These values must survive across handler invocations, requiring region allocation:

```blood
fn example() / {State<i32>} {
    let x = 42;
    let y = get();      // x is live across effect operation
    x + y               // If handler suspends, x must survive
}
```

### 8.2 Detection

Effect capture is detected via:

1. **CaptureSnapshot statements**: Explicit snapshot capture
2. **Perform terminators**: Arguments to effect operations

```
ALGORITHM DETECT_EFFECT_CAPTURE(body):
    effect_captured = {}

    FOR block IN body.blocks:
        FOR stmt IN block.statements:
            IF stmt IS CaptureSnapshot(local):
                effect_captured.add(local)

        IF block.terminator IS Perform(_, _, args, _, _):
            FOR arg IN args:
                IF arg HAS place:
                    effect_captured.add(place.local)

    RETURN effect_captured
```

### 8.3 Impact on Stack Promotion

Effect-captured locals cannot be stack-allocated regardless of escape state:

```
can_stack_allocate(local) = state(local) = NoEscape
                            ∧ local ∉ effect_captured
                            ∧ ¬is_captured_by_escaping_closure(local)
```

---

## 9. Implementation

### 9.1 Data Structures

```rust
// From bloodc/src/mir/escape.rs

pub struct EscapeAnalyzer {
    states: HashMap<LocalId, EscapeState>,
    effect_captured: HashSet<LocalId>,
    globals: HashSet<DefId>,
    closure_captures: HashMap<LocalId, Vec<LocalId>>,
    captured_by_closure: HashSet<LocalId>,
}

pub struct EscapeResults {
    pub states: HashMap<LocalId, EscapeState>,
    pub effect_captured: HashSet<LocalId>,
    pub stack_promotable: HashSet<LocalId>,
    pub closure_captures: HashMap<LocalId, Vec<LocalId>>,
    pub captured_by_closure: HashSet<LocalId>,
}
```

### 9.2 Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `EscapeAnalyzer::analyze` | `escape.rs:473` | Main entry point |
| `analyze_statement` | `escape.rs:673` | Statement analysis |
| `analyze_terminator` | `escape.rs:751` | Terminator analysis |
| `analyze_assignment` | `escape.rs:703` | Assignment propagation |
| `place_escape_state` | `escape.rs:792` | Compute place escape |

### 9.3 Statistics Reporting

The implementation includes statistics for validating PERF-V-002:

```rust
pub struct EscapeStatistics {
    pub total_locals: usize,
    pub stack_promotable: usize,
    pub heap_required: usize,
    pub effect_captured: usize,
    pub closure_captured: usize,
    pub by_state: EscapeStateBreakdown,
}

impl EscapeStatistics {
    /// Key metric: >95% stack allocation target
    pub fn stack_percentage(&self) -> f64 {
        (self.stack_promotable as f64 / self.total_locals as f64) * 100.0
    }
}
```

---

## References

1. **Java Escape Analysis**
   - Choi, J., et al. (1999). "Escape Analysis for Java"
   - [ACM DL](https://dl.acm.org/doi/10.1145/320384.320386)

2. **Memory Model Specification**
   - [MEMORY_MODEL.md §5](./MEMORY_MODEL.md)

3. **Rust MIR**
   - [MIR Documentation](https://rustc-dev-guide.rust-lang.org/mir/index.html)

---

*Last updated: 2026-01-14*
