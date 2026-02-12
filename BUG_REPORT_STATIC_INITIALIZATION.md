# Bug Report: Static Variable Initialization Generates Incorrect LLVM IR

## Summary

Static mutable variables are emitted as dummy functions instead of proper LLVM global variables, causing runtime segmentation faults when the initializer function attempts to write to the static.

## Severity

**Critical** - Blocks self-hosting of the Blood compiler

## Environment

- **blood-rust commit**: aed10a6 (docs: comprehensive audit fixing Rust-paradigm terminology)
- **Platform**: Linux x86_64 (Debian)
- **LLVM version**: 14.0.6

## Reproduction

### Source Code (`interner.blood`)

```blood
static mut GLOBAL_INTERNER: Option<StringInterner> = Option::None;

pub fn init_global_interner() {
    @unsafe {
        GLOBAL_INTERNER = Option::Some(StringInterner::new());
    }
}
```

### Steps to Reproduce

```bash
cd blood-std/std/compiler
blood-rust build main.blood
./main --help  # SIGSEGV at NULL
```

### Expected LLVM IR

The static variable should be emitted as a global variable:

```llvm
@def529_GLOBAL_INTERNER = global { i8, { ptr, { ptr, i64, i64 } } } zeroinitializer
```

And `init_global_interner` should store to that global:

```llvm
define void @def530_init_global_interner() {
entry:
    ; ... construct Option::Some(StringInterner::new()) ...
    store { i8, { ptr, { ptr, i64, i64 } } } %some_value, ptr @def529_GLOBAL_INTERNER
    ret void
}
```

### Actual LLVM IR

The static is emitted as a **dummy function**:

```llvm
define void @def529_GLOBAL_INTERNER() { ret void }
```

And `init_global_interner` generates broken IR:

```llvm
define void @def530_init_global_interner() {
entry:
    %_0 = alloca {}
    %_1 = alloca ptr
    %_2 = alloca ptr                    ; <-- ALLOCATED BUT NEVER INITIALIZED
    %_3 = alloca { ptr, { ptr, i64, i64 } }
    br label %bb0
bb0:
    store ptr @def529_GLOBAL_INTERNER, ptr %_1
    %tmp0 = call { ptr, { ptr, i64, i64 } } @def435_new()
    store { ptr, { ptr, i64, i64 } } %tmp0, ptr %_3
    br label %bb1
bb1:
    %tmp1 = load { ptr, { ptr, i64, i64 } }, ptr %_3
    %tmp2 = getelementptr { { ptr, { ptr, i64, i64 } } }, ptr %_2, i64 0, i32 0  ; <-- USES UNINITIALIZED %_2!
    store { ptr, { ptr, i64, i64 } } %tmp1, ptr %tmp2
    %tmp3 = load ptr, ptr %_2           ; <-- READS GARBAGE
    store ptr %tmp3, ptr %_1
    store {} undef, ptr %_0
    ret void
}
```

The GEP on `%_2` (line with comment) uses an uninitialized local pointer, causing undefined behavior and a segmentation fault.

## Root Cause Analysis

### Observation 1: Static body treated as function body

The code in `compile_static_items` (bloodc/src/codegen/context/mod.rs:1461) correctly creates global variables using `module.add_global()`. However, static bodies are **also** being processed through the function compilation pipeline.

### Observation 2: MIR lowering processes static bodies

When iterating over HIR bodies to generate MIR, static initializer bodies are included and processed as if they were function bodies. This causes:

1. A function definition to be emitted with the static's DefId
2. The "real" global variable from `compile_static_items` may be shadowed or the function may override it

### Observation 3: Assignment codegen incorrect for static targets

When lowering `GLOBAL_INTERNER = Option::Some(...)`:
- The assignment target (a path to a static) is resolved
- MIR lowering may not properly handle `PlaceBase::Static`
- Codegen emits code that uses an uninitialized temporary instead of the global

## Investigation Notes

### blood-rust has proper infrastructure

The Rust compiler has:

1. `compile_static_items()` that creates proper global variables:
   ```rust
   let global = self.module.add_global(llvm_type, Some(AddressSpace::default()), &item.name);
   global.set_initializer(&init_value);
   ```

2. `PlaceBase::Static(DefId)` variant in MIR to represent static places:
   ```rust
   pub enum PlaceBase {
       Local(LocalId),
       Static(DefId),
   }
   ```

3. `get_place_base_type()` that handles static places

### Likely cause

The issue appears to be in how MIR bodies are selected for compilation. Static bodies should be:
1. Evaluated at compile time via `evaluate_const_expr()` for the initializer
2. **NOT** compiled as function bodies

However, something is causing static bodies to be compiled as functions, resulting in the dummy `define void @def529_GLOBAL_INTERNER() { ret void }`.

## Suggested Fix Approach

1. **Filter static bodies from function compilation loop**: Before compiling MIR bodies, check if the body belongs to a static item and skip it.

2. **Verify static compilation order**: Ensure `compile_static_items()` runs before function body compilation and that its globals aren't overwritten.

3. **Add MIR validation**: Add a check that warns if a function is being emitted for a DefId that should be a static.

## Impact

This bug:
- Prevents the self-hosted Blood compiler from running
- Affects any Blood program using `static mut` with non-trivial initializers
- Causes silent memory corruption followed by segfault

## Related Files

- `bloodc/src/codegen/context/mod.rs` - `compile_static_items()` (line 1461)
- `bloodc/src/codegen/mod.rs` - Function compilation entry points
- `bloodc/src/mir/types.rs` - `PlaceBase` enum definition (line 510)

## Test Case

A minimal reproduction would be:

```blood
struct Foo { value: i32 }

static mut GLOBAL: Option<Foo> = Option::None;

fn init() {
    @unsafe {
        GLOBAL = Option::Some(Foo { value: 42 });
    }
}

fn main() -> i32 {
    init();
    0
}
```

Expected: Compiles and runs successfully, returning 0.
Actual: Segmentation fault on `init()` call.

---

## Investigation Update (2026-02-05)

### Investigation Results

After thorough investigation, **the described bug could not be reproduced** with the current codebase. Test cases matching the bug report's scenario compile and run correctly:

1. **Test case `t05_static_item.blood`**: Simple `static mut COUNTER: i32` - passes
2. **Test case `t06_static_option_struct.blood`**: `static mut` with `Option<Struct>` - passes
3. **Custom test cases with `Option::None` initialization and `Option::Some` assignment** - all pass

### Code Analysis Findings

The codebase correctly handles static items:

1. **MIR Lowering** (`bloodc/src/mir/lowering/mod.rs:175-185`): Explicitly skips `ItemKind::Static` via the catch-all pattern. Static items do NOT get MIR bodies generated.

2. **Static Compilation** (`bloodc/src/codegen/context/mod.rs:1461-1497`): `compile_static_items()` correctly creates LLVM global variables using `module.add_global()`.

3. **MIR Body Compilation** (`bloodc/src/codegen/mir_codegen/mod.rs:196-203`): Checks if the function was declared; undeclared DefIds are skipped.

4. **Place Codegen** (`bloodc/src/codegen/mir_codegen/place.rs:62-76`): Correctly handles `PlaceBase::Static(def_id)` by looking up `static_globals`.

### Defensive Safeguards Added

Even though the bug couldn't be reproduced, defensive safeguards were added to prevent future regressions:

1. **Explicit static check in `compile_mir_body()`**: Now checks `static_globals.contains_key(&def_id)` and skips compilation if the DefId is a static item.

2. **Improved documentation**: Added detailed comments explaining why static items are skipped during MIR lowering.

3. **New test case**: Added `tests/ground-truth/t06_static_option_struct.blood` to exercise static items with `Option<Struct>` types.

### Possible Explanations

1. **Historical issue**: The bug may have existed in an earlier version and was fixed.
2. **Environment-specific**: The bug may occur only in specific build configurations or LLVM versions.
3. **Anticipated issue**: The bug report may describe an anticipated rather than observed issue.
4. **Different code path**: The actual problematic code may involve features not covered by the test cases (e.g., complex generic types, specific struct layouts).

### Status

**Unable to reproduce** - Defensive safeguards added. If the issue reoccurs, additional investigation will be needed with a concrete reproduction case.
