# RFC: Granular Safety Controls for Blood

**Status:** Draft Proposal
**Author:** Human + Claude collaboration
**Date:** 2026-01-17

---

## Summary

Blood should provide **granular, explicit, auditable** controls for disabling safety checks in performance-critical code. Safety remains the default; unsafe regions are opt-in and visible in source code.

---

## Motivation

Blood achieves C-level performance on compute-bound workloads but pays 13-50% overhead on pointer-heavy code due to:

1. **Generation checks** (~1-2 cycles per dereference)
2. **128-bit fat pointers** (2x memory bandwidth for pointer arrays)
3. **Bounds checking** (array/slice access validation)
4. **Overflow checking** (integer arithmetic)

For safety-critical systems, this overhead is acceptable. For hot inner loops in trusted code, developers need an escape hatch.

### Design Principles

1. **Safe by default** — All checks enabled unless explicitly disabled
2. **Explicit opt-out** — Unsafe regions visible in source code
3. **Auditable** — `grep` can find all unsafe annotations
4. **Granular** — Disable specific checks, not "all safety"
5. **Scoped** — Annotations apply to minimal regions
6. **Composable** — Module-level defaults with function overrides

---

## Proposed Design

### 1. Safety Attributes

Individual safety checks can be disabled with attributes:

```blood
// Disable generation checks for this function
#[unchecked(generation)]
fn hot_loop(data: &[f64]) -> f64 {
    // Generation validation skipped on every dereference
    let mut sum: f64 = 0.0;
    let mut i: usize = 0;
    while i < data.len() {
        sum = sum + data[i];
        i = i + 1;
    }
    sum
}

// Disable bounds checking
#[unchecked(bounds)]
fn trusted_index(arr: &[i32], idx: usize) -> i32 {
    arr[idx]  // No bounds check
}

// Disable overflow checking
#[unchecked(overflow)]
fn wrapping_add(a: u32, b: u32) -> u32 {
    a + b  // Wraps on overflow instead of panicking
}

// Disable multiple checks
#[unchecked(generation, bounds)]
fn unsafe_memcpy(dst: &mut [u8], src: &[u8]) {
    // Both generation and bounds checks disabled
}
```

### 2. Block-Level Scoping

For finer control, use `unchecked` blocks:

```blood
fn process_buffer(data: &mut [f64]) {
    // Safe setup code
    let len = data.len();
    validate_buffer(data);

    // Performance-critical inner loop
    unchecked(generation, bounds) {
        let mut i: usize = 0;
        while i < len {
            data[i] = data[i] * 2.0;
            i = i + 1;
        }
    }

    // Safe cleanup code
    log_completion(len);
}
```

### 3. Available Safety Checks

| Check | Attribute | Default | Overhead | Risk if Disabled |
|-------|-----------|---------|----------|------------------|
| `generation` | `#[unchecked(generation)]` | Enabled | ~1-2 cycles/deref | Use-after-free, dangling pointers |
| `bounds` | `#[unchecked(bounds)]` | Enabled | ~2-5 cycles/access | Buffer overflow, out-of-bounds read |
| `overflow` | `#[unchecked(overflow)]` | Enabled | ~1 cycle/op | Integer overflow, wrap-around bugs |
| `null` | `#[unchecked(null)]` | Enabled | ~1 cycle/deref | Null pointer dereference |
| `alignment` | `#[unchecked(alignment)]` | Enabled | ~1 cycle/access | Misaligned memory access |

### 4. The `unsafe` Superset

For maximum performance (and maximum risk), disable all checks:

```blood
// Equivalent to #[unchecked(generation, bounds, overflow, null, alignment)]
#[unsafe]
fn c_interop_hot_path(ptr: *mut u8, len: usize) {
    // All safety checks disabled
    // You are now writing C with Blood syntax
}
```

### 5. Module-Level Defaults

Set defaults for an entire module:

```blood
// At top of file: disable generation checks for all functions
#![default_unchecked(generation)]

module hot_paths;

// This function inherits module default (no generation checks)
fn inner_loop_1(data: &[f64]) -> f64 { ... }

// This function inherits module default
fn inner_loop_2(data: &[f64]) -> f64 { ... }

// Explicitly re-enable for this function
#[checked(generation)]
fn safe_function(data: &[f64]) -> f64 { ... }
```

### 6. Conditional Safety (Build Profiles)

Allow safety levels to vary by build profile:

```blood
// Checked in debug, unchecked in release
#[unchecked(generation, when = "release")]
fn optimized_path(data: &[f64]) -> f64 { ... }

// Always checked (even in release)
#[checked(generation, always)]
fn critical_path(data: &[f64]) -> f64 { ... }
```

### 7. Effect Interaction

Effects remain tracked even in unchecked regions:

```blood
#[unchecked(generation)]
fn fast_emit(value: i32) / {Emit<i32>} {
    // Generation checks disabled
    // But effect is still tracked in type signature
    perform Emit.emit(value);
}
```

To disable effect tracking (rare, for FFI):

```blood
#[untracked_effects]
extern "C" fn c_callback(data: *mut void) {
    // Effects not tracked - this is raw C interop
}
```

---

## Safety Contracts

### 8. Preconditions and Postconditions

Document requirements for unchecked code:

```blood
#[unchecked(bounds)]
#[requires(idx < arr.len())]  // Precondition
fn get_unchecked(arr: &[i32], idx: usize) -> i32 {
    arr[idx]
}

#[unchecked(generation)]
#[requires(ptr.is_valid())]  // Caller must ensure validity
#[ensures(result.is_valid())] // Guarantees valid result
fn deref_unchecked<T>(ptr: &T) -> T {
    *ptr
}
```

In debug builds, `#[requires]` and `#[ensures]` are checked at runtime.
In release builds, they're documentation only (or optionally verified).

### 9. Unsafe Boundaries

Mark functions that require caller discipline:

```blood
// Caller must ensure safety invariants
#[unsafe_boundary]
fn from_raw_parts<T>(ptr: *mut T, len: usize) -> &mut [T] {
    // Creates a slice from raw pointer
    // Caller responsible for:
    // - ptr is valid for len elements
    // - ptr is properly aligned
    // - no aliasing violations
    intrinsic::slice_from_raw_parts_mut(ptr, len)
}
```

---

## Auditing Tools

### 10. Compiler Warnings

```bash
$ blood build --warn-unchecked
warning: 3 functions use #[unchecked] attributes
  --> src/hot_paths.blood:15:1
   |
15 | #[unchecked(generation)]
   | ^^^^^^^^^^^^^^^^^^^^^^^^ generation checks disabled
   |
  --> src/hot_paths.blood:42:1
   |
42 | #[unchecked(bounds)]
   | ^^^^^^^^^^^^^^^^^^^^ bounds checks disabled
```

### 11. Audit Report

```bash
$ blood audit --safety
Safety Audit Report
===================

Unchecked Regions: 5
  - src/hot_paths.blood:15  #[unchecked(generation)]
  - src/hot_paths.blood:42  #[unchecked(bounds)]
  - src/ffi.blood:8         #[unsafe]
  - src/ffi.blood:23        #[unsafe]
  - src/simd.blood:102      #[unchecked(bounds, alignment)]

Unsafe Boundaries: 2
  - src/slice.blood:45      from_raw_parts
  - src/ptr.blood:12        read_unaligned

Effect Tracking Disabled: 1
  - src/ffi.blood:8         c_callback

Recommendation: Review all #[unsafe] regions for memory safety.
```

### 12. Certification Mode

For safety-critical deployments:

```bash
$ blood build --certification-mode
error: #[unsafe] attribute not allowed in certification mode
  --> src/ffi.blood:8:1
   |
 8 | #[unsafe]
   | ^^^^^^^^^ forbidden in certification mode

error: #[unchecked(generation)] requires #[certified_by("...")]
  --> src/hot_paths.blood:15:1
   |
15 | #[unchecked(generation)]
   | ^^^^^^^^^^^^^^^^^^^^^^^^ missing certification annotation
```

In certification mode, unchecked regions require explicit sign-off:

```blood
#[unchecked(generation)]
#[certified_by("Jane Doe, 2026-01-15, FMEA-2026-0042")]
fn verified_hot_loop(data: &[f64]) -> f64 {
    // This function has been formally reviewed
}
```

---

## Comparison to Other Languages

| Language | Approach | Granularity | Auditable |
|----------|----------|-------------|-----------|
| **Rust** | `unsafe` blocks | Block/function | Yes |
| **Zig** | `@setRuntimeSafety(false)` | Block | Yes |
| **C** | None (always unsafe) | N/A | N/A |
| **Ada/SPARK** | `pragma Suppress` | Check-specific | Yes |
| **Blood** | `#[unchecked(...)]` | Check-specific, block/function | Yes |

Blood's approach is most similar to Ada/SPARK's `pragma Suppress`, allowing specific checks to be disabled while maintaining others.

---

## Standard Library Patterns

### Safe Wrappers Around Unsafe Operations

```blood
// Public safe API
pub fn get(slice: &[T], index: usize) -> Option<T> {
    if index < slice.len() {
        Some(get_unchecked(slice, index))
    } else {
        None
    }
}

// Internal fast path
#[unchecked(bounds)]
#[inline]
fn get_unchecked<T>(slice: &[T], index: usize) -> T {
    slice[index]
}
```

### Iterator Optimization

```blood
impl<T> Iterator for SliceIter<T> {
    fn next(&mut self) -> Option<T> {
        if self.pos < self.len {
            let value = unchecked(bounds) {
                self.data[self.pos]
            };
            self.pos = self.pos + 1;
            Some(value)
        } else {
            None
        }
    }
}
```

---

## Migration Path

### Phase 1: Syntax Support (v0.6)
- Add `#[unchecked(...)]` attribute parsing
- Add `unchecked { }` block syntax
- Compiler ignores attributes (all checks still enabled)
- Warning: "unchecked attributes are not yet implemented"

### Phase 2: Generation Check Toggle (v0.7)
- Implement `#[unchecked(generation)]`
- Add `--warn-unchecked` flag
- Add basic `blood audit` command

### Phase 3: Full Implementation (v0.8)
- Implement all check toggles
- Add `#[requires]`/`#[ensures]` contracts
- Add certification mode
- Complete audit tooling

### Phase 4: Ecosystem Guidelines (v1.0)
- Document best practices
- Publish safety guidelines
- Standard library uses patterns consistently

---

## Open Questions

1. **Should `#[unsafe]` be a keyword or attribute?**
   - Keyword: More visible, like Rust's `unsafe`
   - Attribute: Consistent with other annotations

2. **Should unchecked regions be allowed in public APIs?**
   - Option A: Allow, but warn
   - Option B: Require `#[unsafe_boundary]` for public unchecked functions
   - Option C: Forbid in public APIs

3. **Should effects be disableable?**
   - Currently proposed: `#[untracked_effects]` for FFI only
   - Alternative: Effects always tracked, even in unsafe code

4. **Contract checking in release builds?**
   - Option A: Always disabled (documentation only)
   - Option B: Configurable via build flag
   - Option C: Enabled by default, explicit opt-out

---

## Conclusion

This proposal enables Blood to offer:

> **C-level performance when you need it, full safety when you don't.**

The key insight is that safety and performance aren't binary—developers need granular control over which checks to disable, with full visibility into what's been disabled.

By making unsafe regions explicit, scoped, and auditable, Blood can serve both:
- **Safety-critical systems** that need full checking
- **Performance-critical code** that needs to skip specific checks

The certification mode ensures that safety-critical deployments can enforce policies about which checks may be disabled and require formal review of any unchecked code.

---

## References

- [Zig's Runtime Safety](https://ziglang.org/documentation/master/#Runtime-Safety)
- [Rust's Unsafe Guidelines](https://rust-lang.github.io/unsafe-code-guidelines/)
- [Ada's Pragma Suppress](http://www.ada-auth.org/standards/rm12_w_tc1/html/RM-11-5.html)
- [MISRA C Guidelines](https://www.misra.org.uk/)
- [DO-178C Software Considerations](https://en.wikipedia.org/wiki/DO-178C)
