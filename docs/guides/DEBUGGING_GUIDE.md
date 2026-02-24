# Blood Debugging and Profiling Guide

**Version**: 0.1.0
**Status**: Reference
**Last Updated**: 2026-01-14

This guide covers debugging strategies, error interpretation, profiling techniques, and tooling usage for Blood programs.

---

## Table of Contents

1. [Understanding Blood Error Messages](#1-understanding-blood-error-messages)
2. [Using the Blood Compiler CLI](#2-using-the-blood-compiler-cli)
3. [Working with blood-lsp](#3-working-with-blood-lsp)
4. [Debugging Effect Handlers](#4-debugging-effect-handlers)
5. [Debugging Memory Issues](#5-debugging-memory-issues)
6. [Profiling Blood Programs](#6-profiling-blood-programs)
7. [Common Debugging Scenarios](#7-common-debugging-scenarios)
8. [Troubleshooting Guide](#8-troubleshooting-guide)

---

## 1. Understanding Blood Error Messages

### 1.1 Error Message Anatomy

Blood error messages follow a consistent format designed for maximum clarity:

```
level[CODE]: main message
  --> file.blood:line:column
   |
 NN | source code line
   |        ^^^^ primary span label
   |
   = note: additional context
   = help: how to fix it
```

**Components:**

| Component | Purpose | Example |
|-----------|---------|---------|
| Level | Severity (error, warning, info) | `error` |
| Code | Unique identifier for lookup | `E0201` |
| Main message | Brief description | `type mismatch` |
| Location | File, line, column | `src/main.blood:15:12` |
| Source context | Relevant code lines | Shows actual source |
| Primary span | Exact error location | `^^^^` underlines |
| Notes | Additional context | `= note: ...` |
| Help | Fix suggestions | `= help: ...` |

### 1.2 Error Code Categories

| Range | Category | Description |
|-------|----------|-------------|
| E0001-E0099 | Lexer | Invalid characters, unclosed strings |
| E0100-E0199 | Parser | Syntax errors, unexpected tokens |
| E0200-E0299 | Type | Type mismatches, inference failures |
| E0300-E0399 | Effect | Unhandled effects, handler errors |
| E0400-E0499 | Ownership | Linear type violations |
| E0500-E0599 | Borrow | Use-after-move errors |
| E0700-E0799 | Pattern | Non-exhaustive matches |
| E9000-E9999 | ICE | Internal compiler errors (bugs) |

### 1.3 Reading Type Errors

Type errors (E0200-E0299) are the most common. Here's how to interpret them:

**E0201: Type Mismatch**

```
error[E0201]: type mismatch: expected `i32`, found `String`
  --> src/main.blood:3:12
   |
 1 | fn compute() -> i32 {
   |                 --- expected `i32` because of return type
 2 |
 3 |     "hello"
   |     ^^^^^^^ expected `i32`, found `String`
```

**Reading this error:**
1. The function declares return type `i32` (line 1)
2. But the actual return value is `"hello"` which is `String` (line 3)
3. Fix: Either change the return type or return an `i32` value

**E0202: Cannot Infer Type**

```
error[E0202]: type annotations needed
  --> src/main.blood:2:9
   |
 2 |     let x = Vec::new();
   |         ^ cannot infer type
   |
   = help: consider giving `x` an explicit type: `let x: Vec<i32> = Vec::new()`
```

**Reading this error:**
1. The compiler can't determine what type `x` should be
2. `Vec::new()` creates an empty vector, but of what element type?
3. Fix: Add type annotation as suggested

### 1.4 Reading Effect Errors

Effect errors (E0300-E0399) are unique to Blood. Here's how to interpret them:

**E0301: Unhandled Effect**

```
error[E0301]: unhandled effect `IO`
  --> src/main.blood:6:5
   |
 5 | fn pure_function() -> String / pure {
   |                                ---- function declared as `pure`
 6 |     perform IO::read_line()
   |     ^^^^^^^^^^^^^^^^^^^^^^^ performs `IO` effect
   |
   = note: `read_line` has effect signature `/ {IO}`
   = help: either:
           - add `IO` to the effect row: `fn pure_function() -> String / {IO}`
           - handle the effect with a handler
```

**Reading this error:**
1. The function is declared `pure` (no effects allowed)
2. But it performs `IO::read_line()` which has the `IO` effect
3. Fix: Either add `IO` to the function signature or handle it

**E0302: Effect Signature Mismatch**

```
error[E0302]: effect mismatch: expected `State<String>`, found `State<i32>`
  --> src/main.blood:11:5
   |
11 |     with IntState handle {
   |          ^^^^^^^^
   |          handles: `State<i32>`
   |          required: `State<String>`
```

**Reading this error:**
1. The handler provides `State<i32>`
2. But the code inside needs `State<String>`
3. Fix: Use a handler with the correct type parameter

### 1.5 Fixing Common Errors

| Error | Common Cause | Quick Fix |
|-------|--------------|-----------|
| E0201 | Wrong type returned | Check function signature |
| E0202 | Ambiguous generic | Add type annotation |
| E0211 | Wrong argument count | Check function definition |
| E0301 | Effect not handled | Add effect to signature or handle |
| E0701 | Missing match arm | Add missing patterns |

---

## 2. Using the Blood Compiler CLI

### 2.1 Basic Commands

```bash
# Check without compiling
$ blood check src/main.blood

# Build executable
$ blood build src/main.blood

# Build with optimizations
$ blood build --release src/main.blood

# Run directly
$ blood run src/main.blood
```

### 2.2 Diagnostic Options

```bash
# Show all errors (no limit)
$ blood check --error-limit 0 src/main.blood

# Treat warnings as errors
$ blood check --deny warnings src/main.blood

# Allow specific warning
$ blood check --allow W0101 src/main.blood

# Show verbose type information
$ blood check --verbose-types src/main.blood
```

### 2.3 Output Formats

```bash
# Human-readable (default)
$ blood check src/main.blood

# JSON for tooling integration
$ blood check --message-format json src/main.blood

# SARIF for IDE integration
$ blood check --message-format sarif src/main.blood
```

### 2.4 Debug Information

```bash
# Build with debug symbols
$ blood build --debug src/main.blood

# Emit MIR for inspection
$ blood build --emit mir src/main.blood

# Emit LLVM IR for inspection
$ blood build --emit llvm-ir src/main.blood

# Show escape analysis results
$ blood build --emit escape-stats src/main.blood
```

### 2.5 Useful Environment Variables

```bash
# Enable verbose compiler output
$ BLOOD_VERBOSE=1 blood build src/main.blood

# Show type inference steps
$ BLOOD_DEBUG_TYPECK=1 blood check src/main.blood

# Show effect inference
$ BLOOD_DEBUG_EFFECTS=1 blood check src/main.blood

# Enable MIR optimization dumps
$ BLOOD_DUMP_MIR=1 blood build src/main.blood
```

---

## 3. Working with blood-lsp

### 3.1 LSP Features

The Blood Language Server provides IDE support:

| Feature | Status | Description |
|---------|--------|-------------|
| Diagnostics | Implemented | Real-time error checking |
| Hover | Implemented | Keyword and type documentation |
| Completion | Implemented | Basic keyword completions |
| Go to Definition | Implemented | Navigate to declarations |
| Document Symbols | Implemented | Outline view (functions, types) |
| Semantic Tokens | Implemented | Full syntax highlighting |
| Inlay Hints | Implemented | Type and effect annotations |
| Code Lens | Implemented | Run/Test buttons |
| Folding Ranges | Implemented | Code folding |

### 3.2 Setting Up blood-lsp

**VS Code:**

```json
// .vscode/settings.json
{
  "blood.lsp.path": "/path/to/blood-lsp",
  "blood.lsp.trace.server": "verbose"
}
```

**Neovim (with lspconfig):**

```lua
-- init.lua
require('lspconfig').blood_lsp.setup{
  cmd = { '/path/to/blood-lsp' },
  filetypes = { 'blood' },
  root_dir = function(fname)
    return vim.fn.getcwd()
  end,
}
```

**Helix:**

```toml
# languages.toml
[[language]]
name = "blood"
scope = "source.blood"
file-types = ["blood"]
roots = ["Blood.toml"]
language-server = { command = "blood-lsp" }
```

### 3.3 Inlay Hints

The LSP provides inlay hints for:

**Type Annotations:**
```blood
let x = 42;        // Shows: `: i32` after `x`
let y = Vec::new();  // Shows: `: Vec<T>` after `y`
```

**Effect Annotations:**
```blood
fn compute() -> i32 {  // Shows: `/ pure` before `{`
    42
}
```

**Parameter Names:**
```blood
process(42, "hello");  // Shows: `count: `, `name: ` before arguments
```

### 3.4 Code Lens

Code lens annotations appear above:

- **main functions**: "Run" button
- **test functions**: "Run Test" button
- **effect declarations**: "Find Handlers" button
- **handler declarations**: "Go to Effect" button

### 3.5 Troubleshooting LSP

**LSP not starting:**
```bash
# Check if blood-lsp is in PATH
$ which blood-lsp

# Run manually to see errors
$ blood-lsp --stdio 2>&1 | head
```

**Diagnostics not updating:**
```bash
# Check LSP logs (VS Code)
# View > Output > Blood Language Server

# Check for file watching issues
$ blood-lsp --check-watchers
```

---

## 4. Debugging Effect Handlers

### 4.1 Effect Flow Tracing

To understand effect flow, add logging to handlers:

```blood
effect Debug {
    op trace(msg: String) -> ();
}

deep handler DebugHandler for Debug {
    return(x) { x }
    op trace(msg) {
        println!("TRACE: {}", msg);
        resume(())
    }
}

// Wrap your computation
fn debug_computation() {
    with DebugHandler handle {
        perform Debug::trace("Starting computation");
        let result = my_effectful_function();
        perform Debug::trace(format!("Result: {:?}", result));
        result
    }
}
```

### 4.2 Handler State Inspection

For stateful handlers, add inspection operations:

```blood
effect State<T> {
    op get() -> T;
    op put(value: T) -> ();
    op inspect() -> String;  // Add for debugging
}

deep handler DebugState<T: Debug> for State<T> {
    let mut state: T

    return(x) { x }

    op get() {
        println!("GET: {:?}", state);
        resume(state.clone())
    }

    op put(value) {
        println!("PUT: {:?} -> {:?}", state, value);
        state = value;
        resume(())
    }

    op inspect() {
        resume(format!("{:?}", state))
    }
}
```

### 4.3 Handler Stack Visualization

When debugging nested handlers, visualize the stack:

```blood
effect HandlerDebug {
    op enter(name: String) -> ();
    op exit(name: String) -> ();
}

deep handler StackTracer for HandlerDebug {
    let mut depth: i32 = 0

    return(x) { x }

    op enter(name) {
        let indent = "  ".repeat(depth as usize);
        println!("{}ENTER: {}", indent, name);
        depth += 1;
        resume(())
    }

    op exit(name) {
        depth -= 1;
        let indent = "  ".repeat(depth as usize);
        println!("{}EXIT: {}", indent, name);
        resume(())
    }
}
```

### 4.4 Common Handler Issues

**Issue: Handler not found**
```
error[E0301]: unhandled effect `State<i32>`
```

**Diagnosis:**
1. Check handler scope - is `with handler handle { }` wrapping the call?
2. Check handler type parameters match
3. Check effect row in function signature

**Issue: Resume type mismatch**
```
error[E0303]: resume type mismatch: expected `String`, found `i32`
```

**Diagnosis:**
1. Check the operation's return type in the effect definition
2. Ensure `resume()` is called with the correct type

**Issue: Linear value in multi-shot handler**
```
error[E0304]: linear value captured in multi-shot handler
```

**Diagnosis:**
1. Linear values can't be resumed multiple times
2. Use `clone()` before capture, or change to deep handler

---

## 5. Debugging Memory Issues

### 5.1 Understanding Generational References

Blood uses generational references for memory safety. Runtime errors look like:

```
error: use after free detected
  --> src/main.blood:7:20
   |
 3 |     let reference = &container[0];
   |                     ^^^^^^^^^^^^^ reference created (generation 42)
 5 |     container.clear();
   |     ----------------- invalidated (generation now 43)
 7 |     println!("{}", *reference);
   |                    ^^^^^^^^^^ attempted access with stale generation
   |
   = note: reference holds generation 42, but current generation is 43
```

**Reading this error:**
1. A reference was created at generation 42
2. The container was modified, incrementing to generation 43
3. Accessing the stale reference fails the generation check

### 5.2 Tracking Generation Changes

```blood
// Enable generation tracking in debug builds
fn debug_generations() {
    let mut data = vec![1, 2, 3];

    // Each mutation may increment generation
    data.push(4);       // May reallocate, new generation
    data.clear();       // Invalidates all references
    data.reserve(100);  // May reallocate
}
```

### 5.3 Common Memory Patterns

**Safe pattern: Copy before mutation**
```blood
fn safe_access() {
    let mut data = vec![1, 2, 3];
    let first = data[0];  // Copy the value
    data.push(4);         // Safe - first is copied
    println!("{}", first);
}
```

**Unsafe pattern: Reference across mutation**
```blood
fn unsafe_access() {
    let mut data = vec![1, 2, 3];
    let first = &data[0];  // Reference
    data.push(4);          // May invalidate reference
    println!("{}", *first); // RUNTIME ERROR
}
```

### 5.4 Memory Debugging Tools

```bash
# Show allocation statistics
$ BLOOD_ALLOC_STATS=1 blood run src/main.blood

# Enable generation check logging
$ BLOOD_GEN_DEBUG=1 blood run src/main.blood

# Show escape analysis decisions
$ blood build --emit escape-stats src/main.blood
```

---

## 6. Profiling Blood Programs

### 6.1 Built-in Profiling

```bash
# Time compilation phases
$ blood build --timings src/main.blood

# Memory usage during compilation
$ blood build --memory-profile src/main.blood
```

### 6.2 Runtime Profiling

**CPU Profiling with perf (Linux):**
```bash
# Build with debug symbols
$ blood build --debug --release src/main.blood

# Record profile
$ perf record -g ./target/release/my_program

# View results
$ perf report
```

**Memory Profiling with Valgrind:**
```bash
# Check for leaks
$ valgrind --leak-check=full ./target/release/my_program

# Profile heap usage
$ valgrind --tool=massif ./target/release/my_program
```

### 6.3 Effect Handler Profiling

Effect handlers have measurable costs:

| Operation | Typical Cost |
|-----------|--------------|
| Evidence vector create | ~5.4ns |
| Handler push | ~635ps |
| Handler lookup (depth 3) | ~386ps |
| Tail-resumptive resume | ~423ps |
| Continuation create | ~48ns |
| Continuation resume | ~20.5ns |

**Profiling handler overhead:**
```blood
// Time effect-heavy code
fn profile_effects() {
    let start = std::time::Instant::now();

    with MyHandler handle {
        for i in 0..10000 {
            perform operation();
        }
    }

    let elapsed = start.elapsed();
    println!("Time: {:?}", elapsed);
}
```

### 6.4 Memory Profiling

**Check stack allocation rate:**
```bash
# Show escape analysis statistics
$ blood build --emit escape-stats src/main.blood

# Expected output:
# Escape Analysis Statistics:
#   Total locals: 150
#   Stack promotable: 142 (94.7%)
#   Heap required: 8 (5.3%)
#   Target: >95% stack allocation
```

**Identify heap allocations:**
```blood
// Use regions for temporary allocations
fn efficient_processing(items: &[Item]) -> Summary {
    region temp {
        let results = Vec::with_capacity(items.len());
        for item in items {
            results.push(process(item));
        }
        summarize(results)
    }  // Bulk deallocation - O(1)
}
```

### 6.5 Benchmarking

**Use Criterion for microbenchmarks:**
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_handler(c: &mut Criterion) {
    c.bench_function("state_handler", |b| {
        b.iter(|| {
            with StateHandler { state: 0 } handle {
                for _ in 0..1000 {
                    perform State::get();
                }
            }
        })
    });
}

criterion_group!(benches, benchmark_handler);
criterion_main!(benches);
```

---

## 7. Common Debugging Scenarios

### 7.1 "Why doesn't this type check?"

**Step 1: Get verbose type information**
```bash
$ blood check --verbose-types src/main.blood
```

**Step 2: Check type inference**
```bash
$ BLOOD_DEBUG_TYPECK=1 blood check src/main.blood
```

**Step 3: Add explicit annotations**
```blood
// Before: unclear what types are inferred
let x = compute();

// After: explicit types reveal mismatches
let x: ExpectedType = compute();
```

### 7.2 "Why isn't this effect being handled?"

**Step 1: Check effect signatures**
```blood
// Verify function effect rows match
fn caller() / {State<i32>} {  // Declares State<i32>
    callee()  // Must handle State<i32>
}
```

**Step 2: Verify handler scope**
```blood
// WRONG: handler doesn't wrap the call
with Handler handle { }
perform operation();  // Not handled!

// RIGHT: handler wraps the call
with Handler handle {
    perform operation();  // Handled
}
```

**Step 3: Check handler type parameters**
```blood
// Handler provides State<i32>
deep handler IntState for State<i32> { ... }

// But code needs State<String>
with IntState handle {
    let s: String = perform State::get();  // Type mismatch!
}
```

### 7.3 "Why is my program slow?"

**Step 1: Profile first**
```bash
$ perf record -g ./my_program
$ perf report
```

**Step 2: Check effect overhead**
- Are there effects in hot loops?
- Are handlers tail-resumptive?

**Step 3: Check memory patterns**
```bash
$ blood build --emit escape-stats src/main.blood
```

**Step 4: Optimize based on findings**
- Move effect operations outside loops
- Use stack allocation where possible
- Prefer arrays over linked structures

### 7.4 "Why is this value escaping?"

Check escape analysis results:
```bash
$ blood build --emit escape-stats src/main.blood
```

Common causes:
1. **Returned from function**: Must escape to caller
2. **Stored in struct field**: May escape if struct escapes
3. **Captured by closure**: If closure escapes, captures escape
4. **Passed to effect operation**: Effect captures for continuation

### 7.5 "Why did I get a stale reference error?"

**Step 1: Enable generation debugging**
```bash
$ BLOOD_GEN_DEBUG=1 blood run src/main.blood
```

**Step 2: Track reference creation and invalidation**
```blood
let mut container = vec![1, 2, 3];
let ref = &container[0];  // Created at generation N

// Any of these invalidate:
container.push(4);      // May reallocate
container.clear();      // Invalidates all
container.insert(0, 0); // May reallocate

*ref  // FAILS: generation is now N+1
```

**Step 3: Fix by copying or restructuring**
```blood
// Option 1: Copy before mutation
let value = container[0];  // Copy
container.push(4);
println!("{}", value);     // Works

// Option 2: Complete access before mutation
let first = container[0];
let second = container[1];
container.push(4);         // Now safe
```

---

## 8. Troubleshooting Guide

### 8.1 Compilation Errors

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| "unexpected token" | Syntax error | Check for missing semicolons, braces |
| "type mismatch" | Wrong type used | Add type annotations to clarify |
| "cannot infer type" | Ambiguous generics | Add explicit type parameters |
| "unhandled effect" | Missing handler | Add effect to signature or handle |
| "linear value used twice" | Linearity violation | Clone before second use |

### 8.2 Runtime Errors

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| "stale generation" | Use-after-free | Don't hold references across mutations |
| "handler not found" | Missing handler | Check handler scope covers the call |
| "stack overflow" | Infinite recursion | Add base case or use iteration |
| "out of memory" | Excessive allocation | Use regions, check for leaks |

### 8.3 LSP Issues

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| No completions | LSP not running | Check blood-lsp path in settings |
| Stale diagnostics | File not saved | Save file or enable auto-save |
| Wrong errors | Out-of-sync state | Restart LSP server |
| High CPU usage | Large project | Increase LSP timeout settings |

### 8.4 Performance Issues

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| Slow startup | Handler initialization | Use lazy initialization |
| Memory growth | Unbounded allocations | Use regions, check lifetimes |
| Effect overhead | Non-tail-resumptive | Make handlers tail-resumptive |
| Cache misses | Pointer-heavy structures | Use arrays instead of linked lists |

---

## Quick Reference

### Error Lookup

```bash
# Get detailed explanation for error code
$ blood explain E0201

# List all error codes
$ blood explain --list
```

### Debugging Commands

```bash
# Check syntax only
$ blood check --parse-only src/main.blood

# Check types only (no codegen)
$ blood check src/main.blood

# Build with debug info
$ blood build --debug src/main.blood

# Run with verbose output
$ BLOOD_VERBOSE=1 blood run src/main.blood
```

### Profiling Commands

```bash
# Show compilation timings
$ blood build --timings src/main.blood

# Show escape analysis stats
$ blood build --emit escape-stats src/main.blood

# Profile with perf
$ perf record -g ./program && perf report
```

---

## Related Documentation

- [DIAGNOSTICS.md](../spec/DIAGNOSTICS.md) - Complete error code reference
- [PERFORMANCE_GUIDE.md](./PERFORMANCE_GUIDE.md) - Optimization strategies
- [EFFECTS_COOKBOOK.md](./EFFECTS_COOKBOOK.md) - Effect patterns and debugging
- [ERROR_MESSAGES.md](../comparisons/ERROR_MESSAGES.md) - Error message philosophy

---

*Last updated: 2026-01-14*
