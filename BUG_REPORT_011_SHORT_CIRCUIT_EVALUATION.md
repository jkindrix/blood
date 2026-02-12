# BUG-011: `&&` and `||` Operators Do Not Implement Short-Circuit Evaluation

**Status:** FIXED

## Summary

blood-rust compiles `&&` and `||` as **eager evaluation** — both operands are always evaluated unconditionally, and the results are combined with a bitwise `and`/`or` instruction. This violates the universally expected short-circuit semantics where `a && b` skips `b` when `a` is `false`, and `a || b` skips `b` when `a` is `true`.

**Root cause:** The codegen for `BinOp::And` / `BinOp::Or` emits both operands sequentially into the same basic block with no conditional branching, then combines with LLVM `and i1` / `or i1`.

**Fix:** Implemented short-circuit evaluation in two locations:
1. **MIR lowering** (`mir/lowering/util.rs`): `lower_short_circuit()` emits conditional control flow with separate basic blocks for RHS evaluation and short-circuit constant.
2. **HIR expression codegen** (`codegen/context/expr.rs`): `compile_short_circuit()` emits LLVM conditional branch + phi node.

Tests: `t06_short_circuit_and`, `t06_short_circuit_or`, `t06_short_circuit_guard`, `t06_short_circuit_chain`, `t06_short_circuit_while`, `t06_short_circuit_or_mutate`.

**Impact:** This is a **critical, self-hosting-blocking** bug. The self-hosted Blood compiler (`first_gen`) crashes with SIGSEGV when compiling itself because a `while j >= 0 && str_greater_than(...)` loop eagerly evaluates `str_greater_than(&v[j as usize], ...)` when `j = -1`, producing a wildly out-of-bounds memory access (`j as usize` = `0xFFFFFFFFFFFFFFFF`).

## Severity

**Critical** — Blocks the entire self-hosting pipeline. first_gen cannot compile main.blood to produce a second-generation binary.

## Environment

- **blood-rust version:** Current HEAD (post BUG-010 fix)
- **Platform:** Linux x86_64 (Debian, kernel 6.1.0-39-amd64)
- **LLVM version:** 14.0.6

## Reproduction

### Minimal Test Case 1: `&&` does not short-circuit

```blood
// File: short_circuit_and.blood
// Expected output: DONE
// Actual output:   CALLED!DONE

fn side_effect() -> bool {
    print("CALLED!");
    true
}

fn main() -> i32 {
    let x: bool = false;
    if x && side_effect() {
        print("INSIDE");
    }
    print("DONE\n");
    0
}
```

### Minimal Test Case 2: `||` does not short-circuit

```blood
// File: short_circuit_or.blood
// Expected output: INSIDE DONE
// Actual output:   CALLED!INSIDE DONE

fn side_effect() -> bool {
    print("CALLED!");
    true
}

fn main() -> i32 {
    let x: bool = true;
    if x || side_effect() {
        print("INSIDE ");
    }
    print("DONE\n");
    0
}
```

### Minimal Test Case 3: Crash from non-short-circuit (models the actual first_gen crash)

```blood
// File: short_circuit_crash.blood
// Expected: prints "SAFE" and exits 0
// Actual:   SIGSEGV (in the real-world case with 55-element Vec<String>)
//
// Note: this may not crash in isolation because the OOB read at
// index 0xFFFFFFFFFFFFFFFF may happen to hit mapped memory in small heaps.
// In the real compiler with 55 String entries, it reliably crashes.

fn check_element(v: &Vec<String>, idx: usize) -> bool {
    let s = &v[idx];
    s.len() > 5
}

fn main() -> i32 {
    let mut v: Vec<String> = Vec::new();
    v.push(str_to_string("hello"));
    v.push(str_to_string("world"));

    let j: i64 = -1;
    // Short-circuit should prevent check_element from being called when j < 0
    if j >= 0 && check_element(&v, j as usize) {
        print("FOUND");
    }
    print("SAFE\n");
    0
}
```

### Steps to Reproduce

```bash
# Test case 1 — observable side effect
blood run short_circuit_and.blood
# Expected: "DONE"
# Actual:   "CALLED!DONE"

# Test case 2 — observable side effect
blood run short_circuit_or.blood
# Expected: "INSIDE DONE"
# Actual:   "CALLED!INSIDE DONE"

# Real-world crash — self-hosted compiler building itself
cd blood-std/std/compiler
blood build main.blood   # builds first_gen
./first_gen build main.blood -o second_gen.ll
# SIGSEGV in blood$build_cache$str_greater_than$Rstr$Rstr
```

## Root Cause Analysis

### Generated LLVM IR (Unoptimized)

For `x && side_effect()`, blood-rust generates:

```llvm
define i32 @blood_main() {
bb0:
  %_1_stack = alloca i1, align 1
  %_2_stack = alloca i1, align 1
  store i1 false, i1* %_1_stack, align 1          ; x = false
  %call_result = call i1 @"blood$side_effect"()    ; UNCONDITIONALLY called!
  store i1 %call_result, i1* %_2_stack, align 1
  br label %bb1

bb1:
  %load = load i1, i1* %_1_stack, align 1          ; load x
  %load1 = load i1, i1* %_2_stack, align 1         ; load side_effect result
  %and = and i1 %load, %load1                      ; bitwise AND (too late!)
  store i1 %and, i1* %_3_stack, align 1
  %load2 = load i1, i1* %_3_stack, align 1
  switch i1 %load2, label %bb3 [
    i1 true, label %bb2
  ]
  ; ...
}
```

**Problem:** `side_effect()` is called at line 4 (bb0) **before** the value of `x` is even checked. The `and i1` at line 10 (bb1) combines the results, but by then the damage is done — the side effect has already occurred, or in the crash case, the out-of-bounds access has already happened.

### Expected LLVM IR (Short-Circuit)

```llvm
define i32 @blood_main() {
bb0:
  %_1_stack = alloca i1, align 1
  store i1 false, i1* %_1_stack, align 1          ; x = false
  %load = load i1, i1* %_1_stack, align 1
  br i1 %load, label %eval_rhs, label %short_circuit_false  ; branch on x!

eval_rhs:                                          ; only reached if x is true
  %call_result = call i1 @"blood$side_effect"()
  br label %merge

short_circuit_false:                               ; x was false, skip RHS
  br label %merge

merge:
  %result = phi i1 [ false, %short_circuit_false ], [ %call_result, %eval_rhs ]
  switch i1 %result, label %bb3 [
    i1 true, label %bb2
  ]
  ; ...
}
```

### How This Causes the first_gen Crash

In `build_cache.blood`, the insertion sort function has:

```blood
fn sort_strings(v: &mut Vec<String>) {
    // ...
    while j >= 0 && str_greater_than(&v[j as usize], &key) {
        // ...
        j = j - 1;
    }
}
```

**Step-by-step crash sequence:**

1. The sort processes 55 source file paths collected from the self-hosted compiler's module declarations
2. During an insertion step, `j` decrements to `-1`
3. The `while` condition evaluates `j >= 0` → `false`
4. **Due to BUG-011:** `str_greater_than(&v[j as usize], &key)` is evaluated anyway
5. `j as usize` = `0xFFFFFFFFFFFFFFFF` (18446744073709551615)
6. `v[j as usize]` computes: `v.data + 0xFFFFFFFFFFFFFFFF * 24` — 24 bytes **before** `v.data`
7. That memory contains string content bytes from an adjacent heap allocation (the tail of `"codegen_ctx.blood"` = ASCII `0x6c622e7874635f6e` = `"n_ctx.bl"`)
8. `str_greater_than` interprets those bytes as a `String` struct, treating the ASCII data as a pointer
9. Dereferencing `0x6c622e7874635f6e` as a pointer → **SIGSEGV**

### GDB Stack Trace

```
Program terminated with signal SIGSEGV, Segmentation fault.
#0  blood$build_cache$str_greater_than$Rstr$Rstr
#1  blood$build_cache$sort_strings$RmD_Vec_str
#2  blood$build_cache$collect_source_files$Rrawstr$Rrawstr
#3  blood$build_cache$try_cache$Rrawstr$Rrawstr
#4  blood$compile_file_streaming$Rrawstr$Rrawstr
#5  blood$run_build$RD_Args
#6  blood_main
```

### Register State at Crash

```
rcx  0x6c622e7874635f6e    ← ASCII "n_ctx.bl" interpreted as pointer
r14  0xffffffffffffffe8    ← -24 (negative offset into vec data)
r15  0xffffffffffffffff    ← -1 (j as usize)
```

### Optimized IR Evidence (first_gen_ir.ll)

In the optimized LLVM IR for `sort_strings`, the non-short-circuit pattern is visible even after LLVM optimizations. The `str_greater_than` function is inlined:

```llvm
; Line 1227: j >= 0 check — result stored but NOT branched on
  %ge71 = icmp sge i64 %sub, 0

; Lines 1228-1232: v[j as usize] access — UNCONDITIONAL, no guard
  %vec_data_ptr1872 = load i8*, i8** %vec_data_ptr_ptr17, align 8
  %byte_offset1973 = mul i64 %sub, 24           ; sub = j, could be -1!
  %elem_ptr_i82074 = getelementptr inbounds i8, i8* %vec_data_ptr1872, i64 %byte_offset1973
  %ref_vec_elem_ptr2175 = bitcast i8* %elem_ptr_i82074 to { i8*, i64, i64 }*
  %load2276 = load { i8*, i64, i64 }, { i8*, i64, i64 }* %ref_vec_elem_ptr2175, align 8

; Lines 1234-1307: str_greater_than body executes (inlined), dereferences loaded data

; Line 1308: ONLY NOW combines the results — too late, crash already happened
  %and78 = and i1 %ge71, %common.ret.op.i
  %cond7079 = icmp eq i1 %and78, true
  br i1 %cond7079, label %bb11, label %bb12
```

The `%ge71` (j >= 0) result is computed at line 1227, but the code proceeds to lines 1228-1232 **unconditionally** to compute `v[j]`, then runs the entire inlined `str_greater_than` body (lines 1234-1307), and only at line 1308 does it combine with `and i1`. The crash occurs during the unconditional load at line 1232 when `j = -1`.

## Scope of Impact

There are **568 uses of `&&` and 200+ uses of `||`** across the self-hosted compiler source files. Most are coincidentally safe because:

- `i < len && bytes[i] == X` — the OOB `bytes[i]` read typically hits allocated-but-unused Vec capacity and the result is discarded
- `x != 0 && y > threshold` — both sides are pure arithmetic with no memory safety implications

The dangerous patterns are those where the RHS:
1. Performs a function call with a value derived from the LHS guard (like `v[j as usize]` guarded by `j >= 0`)
2. Dereferences a pointer that is only valid when the LHS is true (like `ptr != null && ptr.field > 0`)
3. Has observable side effects that should be skipped (like logging, I/O, mutation)

## Suggested Fix

### Approach: Conditional Branch with Phi Node

Lower `a && b` to:

```llvm
entry:
  %a = <evaluate LHS>
  br i1 %a, label %eval_rhs, label %short_circuit

eval_rhs:
  %b = <evaluate RHS>
  br label %merge

short_circuit:
  br label %merge

merge:
  %result = phi i1 [ false, %short_circuit ], [ %b, %eval_rhs ]
```

Lower `a || b` to:

```llvm
entry:
  %a = <evaluate LHS>
  br i1 %a, label %short_circuit, label %eval_rhs

eval_rhs:
  %b = <evaluate RHS>
  br label %merge

short_circuit:
  br label %merge

merge:
  %result = phi i1 [ true, %short_circuit ], [ %b, %eval_rhs ]
```

### Where to Change in blood-rust

The fix should be in the expression codegen path that handles `BinOp::And` and `BinOp::Or`. Currently these likely go through the same path as arithmetic binary ops, emitting both operands then combining. They need special-case handling that:

1. Evaluates the LHS
2. Emits a conditional branch based on the LHS result
3. Creates a new basic block for the RHS
4. Evaluates the RHS only in that block
5. Merges with a phi node

### Chained Expressions

Blood code commonly chains: `a && b && c`. This should naturally compose — each `&&` creates its own conditional branch. After lowering, the control flow is:

```
eval_a → (false? → result=false) | (true? → eval_b → (false? → result=false) | (true? → eval_c → result=c))
```

### Nested in While Loops

The pattern `while guard && body_check { ... }` is common. The short-circuit must work correctly when the `&&` result feeds into a loop condition:

```llvm
loop_header:
  %guard = <evaluate guard>
  br i1 %guard, label %eval_check, label %loop_exit

eval_check:
  %check = <evaluate body_check>
  br i1 %check, label %loop_body, label %loop_exit

loop_body:
  ; ...
  br label %loop_header

loop_exit:
  ; continue after loop
```

## Suggested Test Cases

### Test 1: Basic `&&` short-circuit

```blood
// EXPECT: PASS
fn side_effect_and() -> bool { print("FAIL"); true }
fn main() -> i32 {
    if false && side_effect_and() { print("BAD"); }
    print("PASS\n");
    0
}
```

### Test 2: Basic `||` short-circuit

```blood
// EXPECT: PASS
fn side_effect_or() -> bool { print("FAIL"); false }
fn main() -> i32 {
    if true || side_effect_or() { }
    print("PASS\n");
    0
}
```

### Test 3: Guard-before-access pattern (`&&`)

```blood
// EXPECT: SAFE
fn main() -> i32 {
    let mut v: Vec<i64> = Vec::new();
    v.push(42);
    let j: i64 = -1;
    if j >= 0 && v[j as usize] > 0 {
        print("FOUND");
    }
    print("SAFE\n");
    0
}
```

### Test 4: Chained `&&`

```blood
// EXPECT: PASS
fn effect_a() -> bool { print("A"); true }
fn effect_b() -> bool { print("B"); true }
fn main() -> i32 {
    if false && effect_a() && effect_b() { }
    print("PASS\n");
    // Should print: PASS (neither A nor B called)
    0
}
```

### Test 5: `&&` in while loop condition

```blood
// EXPECT: 10 20 DONE
fn main() -> i32 {
    let mut v: Vec<i64> = Vec::new();
    v.push(10);
    v.push(20);
    v.push(30);

    let mut j: i64 = 1;
    while j >= 0 && v[j as usize] > 15 {
        print_i64(v[j as usize]);
        print(" ");
        j = j - 1;
    }
    // j=1: v[1]=20 > 15 → print 20, j=0
    // j=0: v[0]=10 > 15 → false → exit
    // Should NOT access v[-1]
    print("DONE\n");
    0
}
```

### Test 6: `||` short-circuit prevents mutation

```blood
// EXPECT: 0
fn mutate(x: &mut i32) -> bool {
    *x = 99;
    true
}

fn main() -> i32 {
    let mut val: i32 = 0;
    if true || mutate(&mut val) { }
    print_i64(val as i64);
    // val should still be 0 because mutate was short-circuited
    print("\n");
    0
}
```

## Real-World Impact

### Blocked: Self-Hosting Pipeline

```
blood-rust builds first_gen → first_gen crashes on `build main.blood`
                                         ↑
                                    SIGSEGV in sort_strings
                                    due to eager && evaluation
```

Without this fix, the self-hosted compiler cannot compile itself, blocking:
- Second-generation binary verification
- Pattern field resolution fix verification (BUG with SIGFPE)
- Any further self-hosting progress

### Latent Bugs in Compiled Programs

Every Blood program compiled by blood-rust that uses `&&` or `||` with non-trivial RHS expressions has potentially incorrect behavior. Programs that happen to work do so by accident (OOB reads hitting allocated memory, side effects not mattering). This is a correctness issue, not just a crash issue.

## Related Files

- `bloodc/src/codegen/` — Expression codegen (binary ops `And`/`Or`)
- `bloodc/src/hir/` — HIR representation of `&&`/`||` (may already be `LogicalAnd`/`LogicalOr` vs `BitwiseAnd`/`BitwiseOr`)
- `bloodc/src/mir/` — MIR representation (if `&&`/`||` are lowered to control flow at MIR level, the fix may be there instead)

## Related Bugs

- **BUG-008** (FIXED): If-expression branch elimination — also a codegen control flow issue
- **BUG-010** (FIXED): Handler op push_str ABI — struct-to-pointer coercion in call paths
