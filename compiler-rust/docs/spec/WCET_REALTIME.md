# Blood Worst-Case Execution Time (WCET) and Real-Time Guarantees

**Version**: 0.1.0
**Status**: Specified
**Last Updated**: 2026-01-13

This document specifies Blood's timing characteristics, WCET analysis methodology, and guarantees for real-time systems development.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Runtime Operation Timing](#2-runtime-operation-timing)
3. [Memory Operations](#3-memory-operations)
4. [Effect System Timing](#4-effect-system-timing)
5. [Fiber Scheduling](#5-fiber-scheduling)
6. [WCET Analysis Methodology](#6-wcet-analysis-methodology)
7. [Real-Time Profiles](#7-real-time-profiles)
8. [Compiler Optimizations](#8-compiler-optimizations)
9. [Certification Considerations](#9-certification-considerations)

---

## 1. Overview

### 1.1 Design Goals

Blood's timing model provides:

1. **Predictable execution** - Bounded worst-case times for all operations
2. **No GC pauses** - Deterministic memory management without stop-the-world
3. **Analyzable control flow** - Effects enable static timing analysis
4. **Configurable guarantees** - Real-time profiles for different requirements

### 1.2 Timing Model Philosophy

Blood achieves real-time predictability through:

| Aspect | Approach |
|--------|----------|
| **Memory** | Generational checks with O(1) overhead |
| **Effects** | Handler dispatch with bounded stack usage |
| **Concurrency** | Cooperative scheduling with known switch points |
| **Allocation** | Tiered allocation with tier-specific bounds |

### 1.3 Measured Performance Baselines

Current implementation benchmarks (x86-64):

| Operation | Measured Time | WCET Bound |
|-----------|---------------|------------|
| Generation check | ~3-4 cycles | 10 cycles |
| Snapshot capture (per ref) | ~6.6ns | 20ns |
| Snapshot validation (per ref) | ~6ns | 15ns |
| Continuation resume | 13-18ns | 50ns |
| Channel send/recv | ~17ns | 50ns |
| Fiber context switch | ~50-100ns | 200ns |

---

## 2. Runtime Operation Timing

### 2.1 Bounded Operations

All Blood runtime operations have bounded worst-case execution times:

#### Pointer Operations

| Operation | Complexity | WCET (cycles) | Notes |
|-----------|------------|---------------|-------|
| `blood_validate_generation` | O(1) | 10 | Single comparison |
| `blood_deref` | O(1) | 15 | Validation + load |
| `blood_deref_mut` | O(1) | 15 | Validation + store |
| Null check (if nullable) | O(1) | 3 | Branch on address |

#### Type Operations

| Operation | Complexity | WCET (cycles) | Notes |
|-----------|------------|---------------|-------|
| Type fingerprint compare | O(1) | 5 | 24-bit comparison |
| VFT lookup | O(1) | 10 | Indexed array access |
| Dispatch resolution | O(k) | 10 + 5k | k = dispatch arity |

### 2.2 Unbounded Operations

These operations have data-dependent timing and should be avoided in hard real-time contexts:

| Operation | Complexity | Bound Strategy |
|-----------|------------|----------------|
| Dynamic allocation (Tier 2) | O(allocator) | Use pre-allocated pools |
| Pattern matching (deep) | O(depth) | Limit match depth |
| Recursive functions | O(depth) | Use explicit loops |
| Multi-shot handler resume | O(copies) | Limit continuation size |

---

## 3. Memory Operations

### 3.1 Tier-Specific Timing

Each memory tier has distinct timing characteristics:

#### Tier 0: Stack Allocation

| Operation | WCET | Notes |
|-----------|------|-------|
| Push (per slot) | 3 cycles | Stack pointer adjustment |
| Pop (per slot) | 3 cycles | Stack pointer adjustment |
| Access | 3-5 cycles | Cache-resident |

**Real-time guarantee**: Constant-time allocation and deallocation.

#### Tier 1: Region Allocation

| Operation | WCET | Notes |
|-----------|------|-------|
| Region create | 50 cycles | Metadata setup |
| Region allocate | 20 cycles | Bump allocation |
| Region free (bulk) | 30 + 5n cycles | n = generation updates |

**Real-time guarantee**: O(1) allocation within region, O(n) bulk deallocation.

#### Tier 2: Persistent Allocation

| Operation | WCET | Notes |
|-----------|------|-------|
| Allocate | Unbounded | System allocator dependent |
| Reference count update | 10 cycles | Atomic increment/decrement |
| Deallocation cascade | O(refs) | May chain deallocations |

**Real-time guarantee**: Use pre-allocated pools for bounded timing.

### 3.2 Generation Operations

| Operation | WCET | Notes |
|-----------|------|-------|
| Generation increment | 5 cycles | Atomic increment |
| Generation snapshot capture | 20ns per ref | Linear in captured refs |
| Snapshot validation | 15ns per ref | Linear in validated refs |

#### Reserved Generation Values

Reserved values ensure generation overflow safety:

| Value | Name | Purpose |
|-------|------|---------|
| `0x0000_0000` | `NEVER_VALID` | Uninitialized/freed slots |
| `0x0000_0001` | `STACK_ONLY` | Stack-only allocations |
| `0xFFFF_FFFE` | `PERSISTENT_MARKER` | Persistent tier objects |
| `0xFFFF_FFFF` | `OVERFLOW_SENTINEL` | Overflow detection |

**Overflow handling**: On reaching `OVERFLOW_SENTINEL - 1`, the slot is promoted to persistent tier.

---

## 4. Effect System Timing

### 4.1 Effect Operation Costs

| Operation | WCET | Notes |
|-----------|------|-------|
| `perform` dispatch | 50-100 cycles | Handler lookup + call |
| `resume` (tail-resumptive) | ~0 cycles | Direct return |
| `resume` (single-shot) | 50-100 cycles | Segment switch |
| `resume` (multi-shot) | 100-500 cycles | Continuation copy |

### 4.2 Handler Stack Depth

Effect handlers are stack-based with bounded depth:

```
Maximum handler stack depth = MIN(
    available_stack / handler_frame_size,
    MAX_HANDLER_DEPTH (configurable, default 256)
)
```

#### Handler Frame Size

| Component | Size | Notes |
|-----------|------|-------|
| Handler pointer | 8 bytes | Points to handler vtable |
| Handler state | Variable | Handler-specific |
| Return address | 8 bytes | Continuation resume point |
| Generation snapshot | 8 bytes/ref | Captured references |

### 4.3 Effect Timing Patterns

Different effect patterns have distinct timing characteristics:

| Pattern | Timing | Use Case |
|---------|--------|----------|
| **Tail-resumptive** | O(1) | State read operations |
| **Single-shot** | O(stack depth) | Exception-like control flow |
| **Multi-shot** | O(continuation size) | Backtracking, coroutines |

#### Tail-Resumptive Optimization

When resume is in tail position, no continuation capture is needed:

```blood
// Tail-resumptive: O(1) timing
op get() { resume(state) }

// Non-tail: requires continuation capture
op compute() {
    let result = resume(initial);
    process(result)  // Post-resume work
}
```

---

## 5. Fiber Scheduling

### 5.1 Scheduler Timing

| Operation | WCET | Notes |
|-----------|------|-------|
| Fiber yield | 50-100ns | Context save + switch |
| Fiber spawn | 200ns | Stack allocation + setup |
| Fiber resume | 13-18ns | Context restore |
| Scheduler tick | O(ready fibers) | Work-stealing queue |

### 5.2 Yield Points

Yield points are explicit and predictable:

- Effect `perform` operations
- Explicit `yield` calls
- Channel operations (send/recv)
- I/O operations

**Guarantee**: No implicit preemption; timing depends on explicit yield points.

### 5.3 Channel Operations

| Operation | WCET | Notes |
|-----------|------|-------|
| Bounded send (non-blocking) | 17ns | Atomic queue operation |
| Bounded send (blocking) | + yield time | If queue full |
| Bounded recv (non-blocking) | 17ns | Atomic queue operation |
| Bounded recv (blocking) | + yield time | If queue empty |
| Unbounded send | Variable | May allocate |

**Real-time recommendation**: Use bounded channels with known capacity.

---

## 6. WCET Analysis Methodology

### 6.1 Static Analysis Approach

Blood supports WCET analysis through:

1. **Control flow analysis** - All branches are explicit
2. **Loop bound annotation** - Required for analyzable loops
3. **Effect tracking** - Effect signatures reveal operation costs
4. **Call graph extraction** - For inter-procedural analysis

### 6.2 Loop Bound Annotations

For WCET analysis, loops must have known bounds:

```blood
#[loop_bound(100)]
for i in 0..n {
    process(i);
}

#[loop_bound(max_iterations = 1000)]
while condition {
    // bounded iteration
}
```

### 6.3 WCET Attributes

Function-level WCET annotations:

```blood
#[wcet(max_cycles = 10000)]
fn time_critical_operation(data: &[u8]) -> u32 / pure {
    // Implementation verified against bound
}

#[wcet(max_ns = 1000)]
fn fast_path(x: i32) -> i32 / pure {
    x * 2
}
```

### 6.4 Compiler WCET Output

```bash
# Generate WCET report
bloodc --wcet-analysis src/realtime.blood

# Output format
bloodc --wcet-format=json src/realtime.blood > wcet_report.json
```

Report includes:
- Function WCET estimates
- Critical path analysis
- Loop bound verification
- Unbounded operation warnings

---

## 7. Real-Time Profiles

### 7.1 Profile Definitions

Blood supports configurable real-time profiles:

#### Hard Real-Time Profile (`rt-hard`)

```toml
[profile.rt-hard]
# No dynamic allocation
allow_tier2_allocation = false
# No unbounded recursion
max_recursion_depth = 16
# No multi-shot handlers
allow_multishot_handlers = false
# Bounded loop requirement
require_loop_bounds = true
# Panic behavior
panic_mode = "abort"
```

#### Soft Real-Time Profile (`rt-soft`)

```toml
[profile.rt-soft]
# Pre-allocated pools only
allow_tier2_allocation = "pools_only"
# Bounded recursion
max_recursion_depth = 64
# Single-shot handlers only
allow_multishot_handlers = false
# Loop bounds recommended
require_loop_bounds = false
# Panic behavior
panic_mode = "handler"
```

#### General Profile (`default`)

```toml
[profile.default]
# All features enabled
allow_tier2_allocation = true
allow_multishot_handlers = true
require_loop_bounds = false
panic_mode = "unwind"
```

### 7.2 Profile Enforcement

```bash
# Compile with real-time profile
bloodc --profile=rt-hard src/safety_critical.blood

# Verify WCET bounds with profile
bloodc --profile=rt-hard --verify-wcet src/safety_critical.blood
```

---

## 8. Compiler Optimizations

### 8.1 WCET-Safe Optimizations

Optimizations that maintain or improve WCET:

| Optimization | Effect on WCET | Safety |
|--------------|----------------|--------|
| Dead code elimination | Reduces | Safe |
| Constant folding | Reduces | Safe |
| Inlining (bounded) | May increase | Bounded |
| Loop unrolling (bounded) | Trade-off | Bounded |
| Generation check elision | Reduces | Safe (escape analysis) |
| Tail call optimization | Reduces stack | Safe |

### 8.2 WCET-Dangerous Optimizations

Optimizations that may increase WCET unpredictably:

| Optimization | Risk | Mitigation |
|--------------|------|------------|
| Speculative inlining | Code size growth | Limit inline depth |
| Aggressive unrolling | Cache effects | Bound unroll factor |
| Auto-vectorization | Variable speedup | Profile-guided |

### 8.3 Profile-Specific Optimization

```bash
# Optimize for WCET (minimize worst case)
bloodc --opt-wcet src/critical.blood

# Optimize for average case (standard optimization)
bloodc --opt-level=3 src/general.blood
```

---

## 9. Certification Considerations

### 9.1 Standards Mapping

| Standard | Blood Support | Notes |
|----------|---------------|-------|
| **DO-178C** (DAL A-D) | Partial | WCET analysis, MC/DC coverage |
| **ISO 26262** (ASIL A-D) | Partial | Bounded execution, assertions |
| **IEC 62304** | Partial | Risk-based analysis support |
| **AUTOSAR** | Planned | OS-level integration |

### 9.2 Certification Artifacts

Blood generates artifacts for timing certification:

| Artifact | Purpose | Format |
|----------|---------|--------|
| WCET report | Timing analysis | JSON/XML |
| Call graph | Stack usage | DOT/JSON |
| Loop bounds | Termination proof | JSON |
| Effect flow | Control flow | DOT/JSON |

### 9.3 Tool Qualification

For certified systems, Blood compiler qualification includes:

1. **Compiler test suite** - Regression tests for correctness
2. **WCET validation** - Measured vs. estimated bounds
3. **Transformation verification** - Optimization correctness

---

## 10. Implementation Notes

### 10.1 Current Limitations

1. **WCET analysis** - Annotations specified, analysis tool planned
2. **Hardware timing models** - x86-64 only, ARM planned
3. **Cache analysis** - Not yet integrated
4. **Multi-core timing** - Interference analysis planned

### 10.2 Future Enhancements

1. **aiT/AbsInt integration** - Industry WCET tool support
2. **Hardware timing database** - Per-platform cycle counts
3. **Cache-aware analysis** - Data and instruction cache modeling
4. **Probabilistic WCET** - pWCET for soft real-time

### 10.3 References

- [Vale Generational References](https://vale.dev/)
- [WCET Analysis Handbook](https://www.absint.com/aiT.htm)
- [DO-178C Guidelines](https://www.rtca.org/)
- Blood MEMORY_MODEL.md - Generational pointer specification
- Blood CONCURRENCY.md - Fiber and channel timing
- Blood SAFETY_CERTIFICATION.md - Certification overview
