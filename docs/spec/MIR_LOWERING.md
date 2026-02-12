# Blood MIR Lowering Specification

**Version**: 0.1.0
**Status**: Specified
**Implementation**: `bloodc/src/mir/lowering/`
**Last Updated**: 2026-01-14

This document specifies the HIR to MIR lowering process in Blood, defining transformation rules for converting high-level typed IR to control-flow graph representation.

---

## Table of Contents

1. [Overview](#1-overview)
2. [MIR Structure](#2-mir-structure)
3. [Lowering Context](#3-lowering-context)
4. [Expression Lowering Rules](#4-expression-lowering-rules)
5. [Statement Lowering](#5-statement-lowering)
6. [Control Flow Lowering](#6-control-flow-lowering)
7. [Effect Lowering](#7-effect-lowering)
8. [Pattern Lowering](#8-pattern-lowering)
9. [Closure Lowering](#9-closure-lowering)
10. [Implementation](#10-implementation)

---

## 1. Overview

### 1.1 Purpose

MIR (Mid-level Intermediate Representation) serves as the bridge between HIR and LLVM IR:

| IR | Purpose | Characteristics |
|----|---------|-----------------|
| **HIR** | Typed high-level | Nested expressions, implicit control flow |
| **MIR** | Analysis target | Explicit CFG, no nested expressions |
| **LLVM** | Code generation | SSA form, machine-level |

### 1.2 Design Goals

1. **Explicit Control Flow** - All branches represented as CFG edges
2. **Flat Expressions** - No nested expressions; all temporaries explicit
3. **Memory Operations** - Storage liveness explicitly marked
4. **Effect Support** - Effect operations as terminators

### 1.3 Related Specifications

- [COMPILER_ARCHITECTURE.md](./COMPILER_ARCHITECTURE.md) - Pipeline overview
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) - Operational semantics
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) - Generation pointer semantics

---

## 2. MIR Structure

### 2.1 Core Types

```
MirBody ::= {
    def_id: DefId,
    blocks: Vec<BasicBlock>,
    locals: Vec<Local>,
    return_type: Type,
}

BasicBlock ::= {
    id: BasicBlockId,
    statements: Vec<Statement>,
    terminator: Option<Terminator>,
}

Local ::= {
    id: LocalId,
    name: Option<String>,
    ty: Type,
    kind: LocalKind,  -- Param | Temp | Return
}
```

### 2.2 Statements

```
Statement ::=
    | Assign(Place, Rvalue)           -- Assignment
    | StorageLive(LocalId)            -- Start of local's scope
    | StorageDead(LocalId)            -- End of local's scope
    | Drop(Place)                      -- Drop value
    | IncrementGeneration(Place)       -- Bump generation counter
    | CaptureSnapshot(LocalId)         -- Capture gen refs for effects
    | ValidateGeneration(Place, u32)   -- Check generation
    | PushHandler(DefId, Place)        -- Install effect handler
    | PopHandler                       -- Remove effect handler
    | Nop                              -- No operation
```

### 2.3 Terminators

```
Terminator ::=
    | Goto(BasicBlockId)                           -- Unconditional jump
    | SwitchInt(Operand, SwitchTargets)           -- Multi-way branch
    | Return                                       -- Return from function
    | Unreachable                                  -- Dead code
    | Call(func, args, dest, target, unwind)      -- Function call
    | Assert(cond, msg, target, unwind)           -- Assert condition
    | Perform(effect, op, args, dest, target)     -- Effect operation
    | Resume(value)                                -- Resume continuation
```

### 2.4 Places and Operands

```
Place ::=
    | Local(LocalId)                   -- Local variable
    | Place.field(FieldIdx)           -- Field projection
    | Place[Operand]                  -- Index projection
    | *Place                          -- Dereference

Operand ::=
    | Copy(Place)                      -- Copy from place
    | Move(Place)                      -- Move from place
    | Constant(Constant)               -- Constant value

Rvalue ::=
    | Use(Operand)                     -- Simple assignment
    | BinaryOp(op, Operand, Operand)  -- Binary operation
    | UnaryOp(op, Operand)            -- Unary operation
    | Ref(Place, Mutability)          -- Borrow
    | Aggregate(kind, Vec<Operand>)   -- Construct aggregate
    | Cast(Operand, Type)             -- Type cast
    | Discriminant(Place)             -- Get enum discriminant
    | Len(Place)                      -- Array/slice length
```

---

## 3. Lowering Context

### 3.1 State

```
LoweringContext ::= {
    builder: MirBodyBuilder,      -- Builds MIR body
    local_map: Map<HIR LocalId, MIR LocalId>,
    current_block: BasicBlockId,
    loop_stack: Vec<LoopContext>,
    temp_counter: u32,
    handler_depth: u32,
}

LoopContext ::= {
    break_block: BasicBlockId,    -- Target for break
    continue_block: BasicBlockId, -- Target for continue
    label: Option<Label>,
    result_place: Option<Place>,  -- For loop with value
}
```

### 3.2 Operations

```
// Create new temporary
fn new_temp(ty: Type) -> LocalId:
    local_id = builder.add_temp(ty)
    return local_id

// Map HIR local to MIR local
fn map_local(hir_local: LocalId) -> LocalId:
    return local_map[hir_local]

// Emit statement to current block
fn push_stmt(stmt: Statement):
    builder.push_statement(current_block, stmt)

// Set terminator for current block
fn terminate(term: Terminator):
    builder.set_terminator(current_block, term)

// Create new basic block
fn new_block() -> BasicBlockId:
    return builder.new_block()

// Switch to different block
fn switch_to(block: BasicBlockId):
    current_block = block
```

---

## 4. Expression Lowering Rules

### 4.1 Notation

```
⟦e⟧ : HIR Expr → (MIR Operand, Vec<Statement>)
```

Lowering an expression produces an operand and zero or more statements.

### 4.2 Literals

```
⟦c⟧ = (Constant(ty, c), [])

RULE E-LIT:
    ─────────────────────────────
    ⟦literal(c)⟧ = Constant(c)
```

### 4.3 Variables

```
RULE E-VAR:
    mir_local = map_local(x)
    ──────────────────────────────
    ⟦x⟧ = Copy(Local(mir_local))

    // Note: Move vs Copy determined by type linearity
```

### 4.4 Binary Operations

```
RULE E-BINOP:
    ⟦e₁⟧ = (op₁, stmts₁)
    ⟦e₂⟧ = (op₂, stmts₂)
    t = new_temp(result_type)
    ─────────────────────────────────────────────
    ⟦e₁ ⊕ e₂⟧ = (Copy(Local(t)),
                 stmts₁ ++ stmts₂ ++
                 [Assign(Local(t), BinaryOp(⊕, op₁, op₂))])
```

### 4.5 Function Calls

```
RULE E-CALL:
    ⟦callee⟧ = (func_op, stmts_f)
    ⟦arg₁⟧ = (op₁, stmts₁)
    ...
    ⟦argₙ⟧ = (opₙ, stmtsₙ)
    t = new_temp(result_type)
    next_bb = new_block()
    ─────────────────────────────────────────────
    ⟦callee(arg₁, ..., argₙ)⟧ =
        emit stmts_f ++ stmts₁ ++ ... ++ stmtsₙ
        terminate Call(func_op, [op₁,...,opₙ], Local(t), Some(next_bb))
        switch_to(next_bb)
        result = Copy(Local(t))
```

### 4.6 Block Expressions

```
RULE E-BLOCK:
    FOR stmt IN stmts:
        lower_stmt(stmt)
    IF tail IS Some(expr):
        ⟦expr⟧ = (result, tail_stmts)
        emit tail_stmts
        return result
    ELSE:
        return Constant(unit)
```

### 4.7 Tuple Construction

```
RULE E-TUPLE:
    ⟦e₁⟧ = (op₁, stmts₁)
    ...
    ⟦eₙ⟧ = (opₙ, stmtsₙ)
    t = new_temp(tuple_type)
    ─────────────────────────────────────────────
    ⟦(e₁, ..., eₙ)⟧ =
        emit stmts₁ ++ ... ++ stmtsₙ
        emit Assign(Local(t), Aggregate(Tuple, [op₁,...,opₙ]))
        result = Copy(Local(t))
```

### 4.8 Field Access

```
RULE E-FIELD:
    ⟦base⟧ = (base_op, stmts)
    base_place = operand_to_place(base_op)
    ─────────────────────────────────────────────
    ⟦base.field⟧ =
        emit stmts
        result = Copy(base_place.field(idx))
```

### 4.9 Index Access

```
RULE E-INDEX:
    ⟦base⟧ = (base_op, stmts_b)
    ⟦index⟧ = (idx_op, stmts_i)
    base_place = operand_to_place(base_op)
    ─────────────────────────────────────────────
    ⟦base[index]⟧ =
        emit stmts_b ++ stmts_i
        result = Copy(base_place[idx_op])
```

### 4.10 Reference Creation

```
RULE E-BORROW:
    ⟦inner⟧ = (inner_op, stmts)
    inner_place = operand_to_place(inner_op)
    t = new_temp(ref_type)
    ─────────────────────────────────────────────
    ⟦&mut? inner⟧ =
        emit stmts
        emit Assign(Local(t), Ref(inner_place, mut))
        result = Copy(Local(t))
```

### 4.11 Dereference

```
RULE E-DEREF:
    ⟦inner⟧ = (inner_op, stmts)
    inner_place = operand_to_place(inner_op)
    ─────────────────────────────────────────────
    ⟦*inner⟧ =
        emit stmts
        result = Copy(*inner_place)
```

---

## 5. Statement Lowering

### 5.1 Let Bindings

```
RULE S-LET:
    ⟦init⟧ = (init_op, stmts)
    mir_local = add_local(name, ty)
    local_map[hir_local] = mir_local
    ─────────────────────────────────────────────
    lower_stmt(let x: T = init) =
        emit stmts
        emit StorageLive(mir_local)
        emit Assign(Local(mir_local), Use(init_op))
```

### 5.2 Expression Statements

```
RULE S-EXPR:
    ⟦expr⟧ = (_, stmts)
    ─────────────────────────────────────────────
    lower_stmt(expr;) =
        emit stmts
        // Result discarded
```

---

## 6. Control Flow Lowering

### 6.1 If Expressions

```
RULE E-IF:
    ⟦cond⟧ = (cond_op, cond_stmts)
    then_bb = new_block()
    else_bb = new_block()
    join_bb = new_block()
    result = new_temp(result_type)
    ─────────────────────────────────────────────
    ⟦if cond { then } else { else }⟧ =
        emit cond_stmts
        terminate SwitchInt(cond_op, {0 → else_bb, _ → then_bb})

        switch_to(then_bb)
        ⟦then⟧ = (then_op, then_stmts)
        emit then_stmts
        emit Assign(Local(result), Use(then_op))
        terminate Goto(join_bb)

        switch_to(else_bb)
        ⟦else⟧ = (else_op, else_stmts)
        emit else_stmts
        emit Assign(Local(result), Use(else_op))
        terminate Goto(join_bb)

        switch_to(join_bb)
        return Copy(Local(result))
```

### 6.2 Match Expressions

```
RULE E-MATCH:
    ⟦scrutinee⟧ = (scrut_op, scrut_stmts)
    scrut_place = operand_to_place(scrut_op)
    result = new_temp(result_type)
    join_bb = new_block()

    FOR arm IN arms:
        arm_bb = new_block()
        blocks.append((arm.pattern, arm_bb))
    ─────────────────────────────────────────────
    ⟦match scrutinee { arms }⟧ =
        emit scrut_stmts

        // Generate decision tree
        decision_tree = build_decision_tree(scrut_place, arms)
        emit_decision_tree(decision_tree)

        FOR (pattern, body, arm_bb) IN arms:
            switch_to(arm_bb)
            bindings = extract_bindings(pattern, scrut_place)
            FOR (name, place) IN bindings:
                local = add_local(name, type_of(place))
                emit Assign(Local(local), Copy(place))
            ⟦body⟧ = (body_op, body_stmts)
            emit body_stmts
            emit Assign(Local(result), Use(body_op))
            terminate Goto(join_bb)

        switch_to(join_bb)
        return Copy(Local(result))
```

### 6.3 Loop Expressions

```
RULE E-LOOP:
    header_bb = new_block()
    body_bb = new_block()
    exit_bb = new_block()
    result = new_temp(result_type)

    push_loop_context(exit_bb, header_bb, Some(result))
    ─────────────────────────────────────────────
    ⟦loop { body }⟧ =
        terminate Goto(header_bb)

        switch_to(header_bb)
        terminate Goto(body_bb)

        switch_to(body_bb)
        ⟦body⟧ = (_, body_stmts)
        emit body_stmts
        terminate Goto(header_bb)  // Back edge

        pop_loop_context()
        switch_to(exit_bb)
        return Copy(Local(result))
```

### 6.4 While Loops

```
RULE E-WHILE:
    header_bb = new_block()
    body_bb = new_block()
    exit_bb = new_block()

    push_loop_context(exit_bb, header_bb, None)
    ─────────────────────────────────────────────
    ⟦while cond { body }⟧ =
        terminate Goto(header_bb)

        switch_to(header_bb)
        ⟦cond⟧ = (cond_op, cond_stmts)
        emit cond_stmts
        terminate SwitchInt(cond_op, {0 → exit_bb, _ → body_bb})

        switch_to(body_bb)
        ⟦body⟧ = (_, body_stmts)
        emit body_stmts
        terminate Goto(header_bb)

        pop_loop_context()
        switch_to(exit_bb)
        return Constant(unit)
```

### 6.5 Break and Continue

```
RULE E-BREAK:
    ctx = find_loop_context(label)
    ─────────────────────────────────────────────
    ⟦break label value⟧ =
        IF value IS Some(expr):
            ⟦expr⟧ = (val_op, stmts)
            emit stmts
            emit Assign(ctx.result_place, Use(val_op))
        terminate Goto(ctx.break_block)
        // Block is now unreachable
        return Unreachable

RULE E-CONTINUE:
    ctx = find_loop_context(label)
    ─────────────────────────────────────────────
    ⟦continue label⟧ =
        terminate Goto(ctx.continue_block)
        return Unreachable
```

### 6.6 Return

```
RULE E-RETURN:
    ─────────────────────────────────────────────
    ⟦return value⟧ =
        IF value IS Some(expr):
            ⟦expr⟧ = (val_op, stmts)
            emit stmts
            emit Assign(Local(0), Use(val_op))  // _0 is return place
        terminate Return
        return Unreachable
```

---

## 7. Effect Lowering

### 7.1 Handle Expression

```
RULE E-HANDLE:
    state_local = new_temp(handler_state_type)
    result = new_temp(result_type)
    body_bb = new_block()
    exit_bb = new_block()

    handler_depth += 1
    ─────────────────────────────────────────────
    ⟦with handler { state: init } handle body⟧ =
        // Initialize handler state
        ⟦init⟧ = (init_op, init_stmts)
        emit init_stmts
        emit Assign(Local(state_local), Use(init_op))

        // Push handler
        emit PushHandler(handler_id, Local(state_local))
        terminate Goto(body_bb)

        // Lower body
        switch_to(body_bb)
        ⟦body⟧ = (body_op, body_stmts)
        emit body_stmts

        // Call return clause
        emit CallReturnClause(handler_id, body_op, Local(state_local), Local(result))

        // Pop handler
        emit PopHandler
        handler_depth -= 1

        terminate Goto(exit_bb)
        switch_to(exit_bb)
        return Copy(Local(result))
```

### 7.2 Perform Expression

```
RULE E-PERFORM:
    ⟦arg₁⟧ = (op₁, stmts₁)
    ...
    ⟦argₙ⟧ = (opₙ, stmtsₙ)
    result = new_temp(result_type)
    resume_bb = new_block()
    is_tail_resumptive = check_tail_resumptive(effect, op)
    ─────────────────────────────────────────────
    ⟦perform Effect.op(arg₁, ..., argₙ)⟧ =
        emit stmts₁ ++ ... ++ stmtsₙ

        IF NOT is_tail_resumptive:
            // Capture generation snapshot for non-tail-resumptive ops
            snapshot_local = new_temp(snapshot_type)
            emit CaptureSnapshot(snapshot_local)

        terminate Perform {
            effect_id,
            op_index,
            args: [op₁, ..., opₙ],
            destination: Local(result),
            target: resume_bb,
            is_tail_resumptive,
        }

        switch_to(resume_bb)
        return Copy(Local(result))
```

### 7.3 Resume Expression

```
RULE E-RESUME:
    ⟦value⟧ = (val_op, stmts)
    ─────────────────────────────────────────────
    ⟦resume(value)⟧ =
        emit stmts
        terminate Resume { value: Some(val_op) }
        return Unreachable
```

---

## 8. Pattern Lowering

### 8.1 Pattern Binding Extraction

```
FUNCTION extract_bindings(pattern: Pattern, place: Place) -> Vec<(Name, Place)>:
    CASE pattern OF:
        Wildcard:
            return []

        Ident(name):
            return [(name, place)]

        Tuple(pats):
            result = []
            FOR (i, pat) IN enumerate(pats):
                result.extend(extract_bindings(pat, place.field(i)))
            return result

        Struct(fields):
            result = []
            FOR field IN fields:
                result.extend(extract_bindings(field.pattern, place.field(field.idx)))
            return result

        Variant(variant_idx, inner):
            // First check discriminant, then extract from variant payload
            return extract_bindings(inner, place.downcast(variant_idx))

        Literal(_), Range(_):
            return []  // No bindings
```

### 8.2 Decision Tree Construction

```
FUNCTION build_decision_tree(place: Place, arms: Vec<Arm>) -> DecisionTree:
    // Use standard pattern compilation algorithm
    // Reference: "Compiling Pattern Matching to Good Decision Trees" (Maranget 2008)

    IF all arms start with wildcard or variable:
        return Leaf(arms[0])

    // Find best column to split on
    column = select_best_column(arms)

    // Group arms by constructor at that column
    groups = group_by_constructor(arms, column)

    // Build switch node
    branches = []
    FOR (ctor, sub_arms) IN groups:
        specialized = specialize(sub_arms, ctor, column)
        sub_tree = build_decision_tree(place.at(column), specialized)
        branches.append((ctor, sub_tree))

    return Switch(place.at(column), branches)
```

---

## 9. Closure Lowering

### 9.1 Closure Expression

```
RULE E-CLOSURE:
    closure_def_id = next_closure_def_id()
    env_type = struct_type(captures.map(|c| c.ty))
    env_local = new_temp(env_type)
    ─────────────────────────────────────────────
    ⟦|params| body  with captures [c₁, ..., cₙ]⟧ =
        // Build closure environment
        FOR (i, capture) IN enumerate(captures):
            ⟦capture.expr⟧ = (cap_op, cap_stmts)
            emit cap_stmts

        emit Assign(Local(env_local), Aggregate(Closure(closure_def_id), [cap_ops...]))

        // Queue closure body for lowering
        pending_closures.push((body_id, closure_def_id, captures))

        return Copy(Local(env_local))
```

### 9.2 Closure Body Lowering

Closure bodies are lowered as separate functions with the environment as an implicit first parameter:

```
FUNCTION lower_closure_body(body_id, def_id, captures):
    // Add environment parameter
    env_param = add_param("env", env_type)

    // Map captured variables to environment fields
    FOR (i, capture) IN enumerate(captures):
        local_map[capture.local] = env_param.field(i)

    // Lower body normally
    lower_expr(body)
```

---

## 10. Implementation

### 10.1 File Locations

| File | Purpose |
|------|---------|
| `mir/types.rs` | MIR type definitions |
| `mir/body.rs` | MirBody and MirBodyBuilder |
| `mir/lowering/mod.rs` | Lowering entry point |
| `mir/lowering/function.rs` | FunctionLowering implementation |
| `mir/lowering/closure.rs` | Closure-specific lowering |
| `mir/lowering/util.rs` | Helper traits and functions |

### 10.2 Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `FunctionLowering::lower` | `function.rs:114` | Entry point for function lowering |
| `lower_expr` | `function.rs:140` | Expression lowering dispatcher |
| `lower_if` | `function.rs` | If expression to CFG |
| `lower_match` | `function.rs` | Match expression to decision tree |
| `lower_perform` | `function.rs` | Effect operation lowering |
| `lower_handle` | `function.rs` | Handler installation |

### 10.3 Invariants

The lowering process maintains these invariants:

1. **Single Assignment**: Each temporary is assigned exactly once
2. **Terminated Blocks**: Every reachable block has a terminator
3. **Type Preservation**: MIR types match HIR types
4. **Local Mapping**: Every HIR local maps to exactly one MIR local
5. **Effect Ordering**: PushHandler/PopHandler are properly nested

---

## References

1. **Rust MIR**
   - [MIR Documentation](https://rustc-dev-guide.rust-lang.org/mir/index.html)
   - [RFC 1211](https://rust-lang.github.io/rfcs/1211-mir.html)

2. **Pattern Compilation**
   - Maranget, L. (2008). "Compiling Pattern Matching to Good Decision Trees"
   - [Paper](http://moscova.inria.fr/~maranget/papers/ml05e-maranget.pdf)

3. **Effect Compilation**
   - [Generalized Evidence Passing](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)

---

*Last updated: 2026-01-14*
