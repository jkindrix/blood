//! # FFI Exports
//!
//! C-compatible exports for linking compiled Blood programs against the runtime.
//!
//! ## Design
//!
//! This module exposes the Blood runtime functionality via C ABI so that
//! compiled Blood programs (which use LLVM codegen) can link against
//! a static or shared library built from this crate.
//!
//! ## Usage
//!
//! Build as a cdylib or staticlib:
//! ```toml
//! [lib]
//! crate-type = ["cdylib", "staticlib", "rlib"]
//! ```
//!
//! Then link compiled Blood programs with `-lblood_runtime`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::io::{self, Write};
use std::sync::OnceLock;

use parking_lot::Mutex;

use crate::memory::{
    BloodPtr, PointerMetadata, generation, get_slot_generation, Region,
    size_class_for, slot_size_for_class, unregister_allocation, register_allocation,
    register_allocation_with_region, get_slot_info, record_system_alloc, record_system_free,
    system_alloc_stats, system_alloc_live_bytes, collect_cycles,
};
use crate::continuation::{
    ContinuationRef, Continuation, EffectContext,
    register_continuation, take_continuation, has_continuation,
};

/// Fiber handle for continuation capture.
pub type FiberHandle = u64;

/// Continuation handle for FFI.
pub type ContinuationHandle = u64;

// ============================================================================
// Realloc Diagnostics
// ============================================================================

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// Total realloc calls inside a region.
static REALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
/// Total bytes abandoned by realloc (old buffer sizes).
static REALLOC_WASTED_BYTES: AtomicU64 = AtomicU64::new(0);
/// Reallocs where old buffer was the most recent bump allocation (in-place eligible).
static REALLOC_INPLACE_ELIGIBLE: AtomicU64 = AtomicU64::new(0);
/// Total bytes that would be saved by in-place realloc (old_size for eligible reallocs).
static REALLOC_INPLACE_SAVED_BYTES: AtomicU64 = AtomicU64::new(0);
/// Actual offset delta saved: (would-have-been offset advance) - (actual offset advance).
static REALLOC_OFFSET_DELTA_SAVED: AtomicU64 = AtomicU64::new(0);

// ============================================================================
// Allocation Size Histogram
// ============================================================================

/// Allocation size histogram buckets (by log2 size class + large buckets).
/// Bucket indices:
///   0: 1-8 bytes       6: 513-1024        12: 32K-64K
///   1: 9-16            7: 1K-2K           13: 64K-128K
///   2: 17-32           8: 2K-4K           14: 128K-256K
///   3: 33-64           9: 4K-8K           15: 256K-512K
///   4: 65-128         10: 8K-16K          16: 512K-1M
///   5: 129-256        11: 16K-32K         17: >1M
const ALLOC_HIST_BUCKETS: usize = 18;
static ALLOC_HIST_COUNT: [AtomicU64; ALLOC_HIST_BUCKETS] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; ALLOC_HIST_BUCKETS]
};
static ALLOC_HIST_BYTES: [AtomicU64; ALLOC_HIST_BUCKETS] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; ALLOC_HIST_BUCKETS]
};

#[inline]
fn alloc_hist_bucket(size: usize) -> usize {
    if size == 0 { return 0; }
    let bits = (usize::BITS - size.leading_zeros()) as usize; // ceil(log2(size)) + 1
    // bits=1 for size=1, bits=4 for size=8..15, bits=14 for 8192..16383
    let bucket = if bits <= 3 { 0 } else { bits - 3 }; // 0 for 1-8, 1 for 9-16, etc.
    bucket.min(ALLOC_HIST_BUCKETS - 1)
}

#[inline]
fn record_alloc_hist(size: usize) {
    let b = alloc_hist_bucket(size);
    ALLOC_HIST_COUNT[b].fetch_add(1, AtomicOrdering::Relaxed);
    ALLOC_HIST_BYTES[b].fetch_add(size as u64, AtomicOrdering::Relaxed);
}

// ============================================================================
// Region-Aware Allocation
// ============================================================================

thread_local! {
    /// Stack of active region IDs for the current thread.
    /// Supports nested region activation: inner regions push onto the stack,
    /// deactivation pops and restores the previous region.
    static ACTIVE_REGION_STACK: RefCell<Vec<u64>> = RefCell::new(Vec::new());
}

/// Get the currently active region ID (top of stack), or 0 if none.
fn current_active_region() -> u64 {
    ACTIVE_REGION_STACK.with(|stack| {
        stack.borrow().last().copied().unwrap_or(0)
    })
}

/// Region-aware allocation. When a region is active, allocates from it.
/// Otherwise falls back to the global allocator.
unsafe fn runtime_alloc(layout: std::alloc::Layout) -> *mut u8 {
    let region_id = current_active_region();
    if region_id != 0 {
        blood_region_alloc(region_id, layout.size(), layout.align()) as *mut u8
    } else {
        std::alloc::alloc(layout)
    }
}

/// Region-aware deallocation. When a region is active, dealloc is a no-op
/// because Blood lacks a borrow checker — dropped Vecs/Strings may still
/// have live references to their backing buffers. Freeing them would cause
/// use-after-free when the free-list slot gets reused. The region's bulk
/// destroy reclaims all memory at once.
unsafe fn runtime_dealloc(ptr: *mut u8, layout: std::alloc::Layout) {
    let region_id = current_active_region();
    if region_id == 0 {
        std::alloc::dealloc(ptr, layout);
    }
    // Inside a region: no-op. Memory is reclaimed when the region is destroyed.
}

/// Region-aware reallocation with in-place extension.
///
/// When a region is active and the buffer being realloc'd is the most recent
/// bump allocation, extends it in place — no copy, no abandoned buffer. This
/// eliminates the primary source of region memory waste from Vec/String growth.
///
/// Fallback: allocates new space and copies. The old buffer is abandoned (not
/// freed) because Blood lacks a borrow checker — other code may still reference
/// old buffer contents through dangling pointers.
unsafe fn runtime_realloc(ptr: *mut u8, old_layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
    let region_id = current_active_region();
    if region_id != 0 {
        let old_size = old_layout.size();

        // --- Diagnostic counters ---
        REALLOC_COUNT.fetch_add(1, AtomicOrdering::Relaxed);

        // Compute actual allocation size including slab rounding, for accurate
        // waste tracking. The bump allocator rounds up to slot_size for slab allocs.
        let class = crate::memory::size_class_for(old_size);
        let actual_old_size = if class != crate::memory::SIZE_CLASS_LARGE {
            let slot = crate::memory::slot_size_for_class(class);
            slot.max(old_size)
        } else {
            old_size
        };

        // Compute slab-rounded new size for offset delta tracking
        let new_class = crate::memory::size_class_for(new_size);
        let actual_new_size = if new_class != crate::memory::SIZE_CLASS_LARGE {
            let slot = crate::memory::slot_size_for_class(new_class);
            slot.max(new_size)
        } else {
            new_size
        };

        // Try in-place extension: if the old buffer is the most recent bump
        // allocation, we can just advance the offset without copying.
        if !ptr.is_null() && old_size > 0 && new_size > old_size {
            let registry = get_region_registry();
            let reg = registry.lock();
            if let Some(region) = reg.get(&region_id) {
                let offset_before = region.used();
                if region.try_realloc_in_place(ptr, old_size, new_size) {
                    let offset_after = region.used();
                    // Without in-place: offset would advance by actual_new_size (new alloc)
                    // With in-place: offset advanced by (actual_new_size - actual_old_size)
                    // Delta saved = actual_old_size
                    // But let's measure the REAL delta for debugging:
                    let actual_advance = offset_after - offset_before;
                    let would_be_advance = actual_new_size; // full new alloc (ignoring alignment)
                    if would_be_advance > actual_advance {
                        REALLOC_OFFSET_DELTA_SAVED.fetch_add(
                            (would_be_advance - actual_advance) as u64,
                            AtomicOrdering::Relaxed,
                        );
                    }
                    REALLOC_INPLACE_ELIGIBLE.fetch_add(1, AtomicOrdering::Relaxed);
                    REALLOC_INPLACE_SAVED_BYTES.fetch_add(actual_old_size as u64, AtomicOrdering::Relaxed);
                    return ptr; // Same pointer, buffer extended in place
                }
            }
        }

        // Fallback: allocate new buffer and copy
        REALLOC_WASTED_BYTES.fetch_add(actual_old_size as u64, AtomicOrdering::Relaxed);
        let new_ptr = blood_region_alloc(region_id, new_size, old_layout.align()) as *mut u8;
        if !new_ptr.is_null() && !ptr.is_null() {
            std::ptr::copy_nonoverlapping(ptr, new_ptr, old_size.min(new_size));
        }
        new_ptr
    } else {
        std::alloc::realloc(ptr, old_layout, new_size)
    }
}

/// Activate a region for the current thread. Pushes onto the region stack,
/// supporting nested regions.
#[no_mangle]
pub extern "C" fn blood_region_activate(region_id: u64) {
    ACTIVE_REGION_STACK.with(|stack| stack.borrow_mut().push(region_id));
}

/// Deactivate the current region, popping the stack to restore the previous
/// region (or global allocation if the stack is empty).
#[no_mangle]
pub extern "C" fn blood_region_deactivate() {
    ACTIVE_REGION_STACK.with(|stack| { stack.borrow_mut().pop(); });
}

// ============================================================================
// I/O Functions
// ============================================================================

/// Print an integer (no newline).
#[no_mangle]
pub extern "C" fn print_int(n: i32) {
    print!("{n}");
    let _ = io::stdout().flush();
}

/// Print an integer with newline.
#[no_mangle]
pub extern "C" fn println_int(n: i32) {
    println!("{n}");
}

/// Print a 64-bit integer with newline.
#[no_mangle]
pub extern "C" fn println_i64(n: i64) {
    println!("{n}");
}

/// Print just a newline.
#[no_mangle]
pub extern "C" fn println() {
    println!();
}

/// Print just a newline (alias for println).
#[no_mangle]
pub extern "C" fn print_newline() {
    println!();
}

/// Blood str slice representation {ptr, len}.
#[repr(C)]
pub struct BloodStr {
    /// Pointer to the string data.
    pub ptr: *const u8,
    /// Length of the string in bytes.
    pub len: u64,
}

/// Blood owned String representation {ptr, len, capacity}.
#[repr(C)]
pub struct BloodString {
    /// Pointer to the string data.
    pub ptr: *mut u8,
    /// Length of the string in bytes.
    pub len: i64,
    /// Capacity in bytes.
    pub capacity: i64,
}

/// Create a new empty String.
///
/// # Arguments
/// * `out` - Output buffer to write the String struct to
///
/// # Safety
/// `out` must be a valid pointer to uninitialized BloodString memory.
#[no_mangle]
pub unsafe extern "C" fn string_new(out: *mut BloodString) {
    (*out).ptr = std::ptr::null_mut();
    (*out).len = 0;
    (*out).capacity = 0;
}

/// Get the str slice from a String.
///
/// # Safety
/// `s` must be a valid pointer to a BloodString.
#[no_mangle]
pub unsafe extern "C" fn string_as_str(s: *const BloodString) -> BloodStr {
    BloodStr {
        ptr: (*s).ptr,
        len: (*s).len as u64,
    }
}

/// Get the length of a String in bytes.
///
/// # Safety
/// `s` must be a valid pointer to a BloodString.
#[no_mangle]
pub unsafe extern "C" fn string_len(s: *const BloodString) -> i64 {
    (*s).len
}

/// Check if a String is empty.
///
/// # Safety
/// `s` must be a valid pointer to a BloodString.
#[no_mangle]
pub unsafe extern "C" fn string_is_empty(s: *const BloodString) -> i32 {
    if (*s).len == 0 { 1 } else { 0 }
}

/// Push a character to a String.
///
/// # Safety
/// `s` must be a valid pointer to a BloodString.
#[no_mangle]
pub unsafe extern "C" fn string_push(s: *mut BloodString, ch: i32) {
    let c = char::from_u32(ch as u32).unwrap_or('\u{FFFD}');
    let mut buf = [0u8; 4];
    let encoded = c.encode_utf8(&mut buf);
    let bytes = encoded.as_bytes();

    // Ensure capacity
    let new_len = (*s).len + bytes.len() as i64;
    if new_len > (*s).capacity {
        let new_cap = std::cmp::max(new_len, (*s).capacity * 2).max(8);
        let new_ptr = if (*s).ptr.is_null() {
            let layout = std::alloc::Layout::from_size_align(new_cap as usize, 1).unwrap();
            runtime_alloc(layout)
        } else {
            let old_layout = std::alloc::Layout::from_size_align((*s).capacity as usize, 1).unwrap();
            runtime_realloc((*s).ptr, old_layout, new_cap as usize)
        };
        (*s).ptr = new_ptr;
        (*s).capacity = new_cap;
    }

    // Copy bytes
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*s).ptr.add((*s).len as usize), bytes.len());
    (*s).len = new_len;
}

/// Push a str to a String.
///
/// # Safety
/// `s` must be a valid pointer to a BloodString.
/// `other` must be a valid pointer to a BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_push_str(s: *mut BloodString, other: *const BloodStr) {
    if (*other).ptr.is_null() || (*other).len == 0 {
        return;
    }

    let bytes_len = (*other).len as i64;
    let new_len = (*s).len + bytes_len;

    // Ensure capacity
    if new_len > (*s).capacity {
        let new_cap = std::cmp::max(new_len, (*s).capacity * 2).max(8);
        let new_ptr = if (*s).ptr.is_null() {
            let layout = std::alloc::Layout::from_size_align(new_cap as usize, 1).unwrap();
            runtime_alloc(layout)
        } else {
            let old_layout = std::alloc::Layout::from_size_align((*s).capacity as usize, 1).unwrap();
            runtime_realloc((*s).ptr, old_layout, new_cap as usize)
        };
        (*s).ptr = new_ptr;
        (*s).capacity = new_cap;
    }

    // Copy bytes
    std::ptr::copy_nonoverlapping((*other).ptr, (*s).ptr.add((*s).len as usize), bytes_len as usize);
    (*s).len = new_len;
}

/// Clear a String's contents.
///
/// # Safety
/// `s` must be a valid pointer to a BloodString.
#[no_mangle]
pub unsafe extern "C" fn string_clear(s: *mut BloodString) {
    (*s).len = 0;
}

/// Convert a str slice to an owned String.
///
/// This allocates new memory and copies the string data.
///
/// # Arguments
/// * `s` - Pointer to the input BloodStr slice
/// * `out` - Output buffer to write the String struct to
///
/// # Safety
/// `s` must be a valid pointer to a BloodStr.
/// `out` must be a valid pointer to uninitialized BloodString memory.
#[no_mangle]
pub unsafe extern "C" fn str_to_string(s: *const BloodStr, out: *mut BloodString) {
    if s.is_null() || out.is_null() {
        // Initialize to empty string on null input
        (*out).ptr = std::ptr::null_mut();
        (*out).len = 0;
        (*out).capacity = 0;
        return;
    }

    let input = &*s;
    let len = input.len as i64;

    if len == 0 || input.ptr.is_null() {
        // Empty string
        (*out).ptr = std::ptr::null_mut();
        (*out).len = 0;
        (*out).capacity = 0;
        return;
    }

    // Allocate memory for the new string
    let capacity = len;
    let layout = std::alloc::Layout::from_size_align(capacity as usize, 1).unwrap();
    let new_ptr = runtime_alloc(layout);

    if new_ptr.is_null() {
        // Allocation failed
        (*out).ptr = std::ptr::null_mut();
        (*out).len = 0;
        (*out).capacity = 0;
        return;
    }

    // Copy the data
    std::ptr::copy_nonoverlapping(input.ptr, new_ptr, len as usize);

    // Initialize the output String
    (*out).ptr = new_ptr;
    (*out).len = len;
    (*out).capacity = capacity;
}

/// Extract a substring from a String by byte indices.
///
/// Creates a new String containing the bytes from `start` to `end` (exclusive).
///
/// # Arguments
/// * `s` - Pointer to the input BloodString
/// * `start` - Start byte index (inclusive)
/// * `end` - End byte index (exclusive)
/// * `out` - Output buffer to write the new String to
///
/// # Safety
/// `s` must be a valid pointer to a BloodString.
/// `out` must be a valid pointer to uninitialized BloodString memory.
/// `start` and `end` should be valid byte indices (clamped if out of bounds).
#[no_mangle]
pub unsafe extern "C" fn string_substring(
    s: *const BloodString,
    start: i64,
    end: i64,
    out: *mut BloodString,
) {
    if s.is_null() || out.is_null() {
        (*out).ptr = std::ptr::null_mut();
        (*out).len = 0;
        (*out).capacity = 0;
        return;
    }

    let input = &*s;

    // Clamp indices to valid range
    let len = input.len;
    let start = start.max(0).min(len) as usize;
    let end = end.max(0).min(len) as usize;

    if start >= end || input.ptr.is_null() {
        // Empty substring
        (*out).ptr = std::ptr::null_mut();
        (*out).len = 0;
        (*out).capacity = 0;
        return;
    }

    let substr_len = (end - start) as i64;

    // Allocate memory for the new string
    let layout = std::alloc::Layout::from_size_align(substr_len as usize, 1).unwrap();
    let new_ptr = runtime_alloc(layout);

    if new_ptr.is_null() {
        (*out).ptr = std::ptr::null_mut();
        (*out).len = 0;
        (*out).capacity = 0;
        return;
    }

    // Copy the substring data
    std::ptr::copy_nonoverlapping(input.ptr.add(start), new_ptr, substr_len as usize);

    // Initialize the output String
    (*out).ptr = new_ptr;
    (*out).len = substr_len;
    (*out).capacity = substr_len;
}

/// Print a string (no newline).
///
/// Takes a Blood str slice {ptr, len}.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn print_str(s: BloodStr) {
    if !s.ptr.is_null() && s.len > 0 {
        let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
        if let Ok(str_val) = std::str::from_utf8(slice) {
            print!("{str_val}");
            let _ = io::stdout().flush();
        }
    }
}

/// Print a string with newline.
///
/// Takes a Blood str slice {ptr, len}.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn println_str(s: BloodStr) {
    if !s.ptr.is_null() && s.len > 0 {
        let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
        if let Ok(str_val) = std::str::from_utf8(slice) {
            println!("{str_val}");
        }
    } else {
        // Empty string - just print newline
        println!();
    }
}

/// Print a string to stderr (no newline).
///
/// Takes a Blood str slice {ptr, len}.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn eprint_str(s: BloodStr) {
    use std::io::Write;
    if !s.ptr.is_null() && s.len > 0 {
        let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
        if let Ok(str_val) = std::str::from_utf8(slice) {
            eprint!("{str_val}");
            let _ = std::io::stderr().flush();
        }
    }
}

/// Print a string to stderr with newline.
///
/// Takes a Blood str slice {ptr, len}.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn eprintln_str(s: BloodStr) {
    if !s.ptr.is_null() && s.len > 0 {
        let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
        if let Ok(str_val) = std::str::from_utf8(slice) {
            eprintln!("{str_val}");
        }
    } else {
        // Empty string - just print newline
        eprintln!();
    }
}

/// Get the length of a string in bytes.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub extern "C" fn str_len(s: BloodStr) -> i64 {
    s.len as i64
}

/// Get the length of a string in bytes, returning usize.
/// Takes a pointer to the string struct (for method call semantics).
///
/// # Safety
/// The pointer must point to a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn str_len_usize(s: *const BloodStr) -> usize {
    if s.is_null() {
        return 0;
    }
    (*s).len as usize
}

/// Compare two strings for equality.
///
/// # Safety
/// Both pointers must be valid for their respective lengths.
#[no_mangle]
pub unsafe extern "C" fn str_eq(a: BloodStr, b: BloodStr) -> bool {
    if a.len != b.len {
        return false;
    }
    if a.len == 0 {
        return true;  // Both empty
    }
    if a.ptr.is_null() || b.ptr.is_null() {
        return a.ptr.is_null() && b.ptr.is_null();
    }
    let slice_a = std::slice::from_raw_parts(a.ptr, a.len as usize);
    let slice_b = std::slice::from_raw_parts(b.ptr, b.len as usize);
    slice_a == slice_b
}

/// Concatenate two strings, returning a newly allocated string.
///
/// # Safety
/// Both pointers must be valid for their respective lengths.
#[no_mangle]
pub unsafe extern "C" fn blood_str_concat(a: BloodStr, b: BloodStr) -> BloodStr {
    let len_a = if a.ptr.is_null() { 0 } else { a.len as usize };
    let len_b = if b.ptr.is_null() { 0 } else { b.len as usize };
    let total_len = len_a + len_b;

    if total_len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    // Allocate buffer for concatenated string
    let layout = std::alloc::Layout::from_size_align(total_len, 1).unwrap();
    let ptr = runtime_alloc(layout);
    if ptr.is_null() {
        eprintln!("blood: out of memory in str_concat");
        std::process::exit(1);
    }

    // Copy both strings into the buffer
    if len_a > 0 && !a.ptr.is_null() {
        std::ptr::copy_nonoverlapping(a.ptr, ptr, len_a);
    }
    if len_b > 0 && !b.ptr.is_null() {
        std::ptr::copy_nonoverlapping(b.ptr, ptr.add(len_a), len_b);
    }

    BloodStr {
        ptr: ptr as *const u8,
        len: total_len as u64,
    }
}

/// Representation of Option<char> for FFI.
///
/// Uses a tagged union representation where `tag` discriminates between None and Some.
#[repr(C)]
pub struct BloodOptionChar {
    /// Discriminant tag: 0 = None, 1 = Some.
    pub tag: i32,
    /// The character value as i32 (valid UTF-32 codepoint when tag == 1).
    pub value: i32,
}

/// Get character at byte index from a string slice.
///
/// Returns Option<char> as { tag: i32, value: i32 }.
/// tag=0 means None (index out of bounds or invalid UTF-8),
/// tag=1 means Some(char) with the character as value.
///
/// Note: This operates on byte indices, not character indices.
/// For UTF-8 strings, use char_at_char_index for character-based indexing.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn str_char_at(s: *const BloodStr, index: u64) -> BloodOptionChar {
    if s.is_null() {
        return BloodOptionChar { tag: 0, value: 0 };
    }
    let s = &*s;
    if s.ptr.is_null() || index >= s.len {
        return BloodOptionChar { tag: 0, value: 0 };
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    if let Ok(str_val) = std::str::from_utf8(slice) {
        // Get the character at the byte index
        if let Some(ch) = str_val.get(index as usize..).and_then(|s| s.chars().next()) {
            return BloodOptionChar { tag: 1, value: ch as i32 };
        }
    }
    BloodOptionChar { tag: 0, value: 0 }
}

/// Get character at character (not byte) index from a string slice.
///
/// Returns Option<char> as { tag: i32, value: i32 }.
/// This is slower than str_char_at because it iterates through characters.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn str_char_at_index(s: *const BloodStr, char_index: u64) -> BloodOptionChar {
    if s.is_null() {
        return BloodOptionChar { tag: 0, value: 0 };
    }
    let s = &*s;
    if s.ptr.is_null() {
        return BloodOptionChar { tag: 0, value: 0 };
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    if let Ok(str_val) = std::str::from_utf8(slice) {
        if let Some(ch) = str_val.chars().nth(char_index as usize) {
            return BloodOptionChar { tag: 1, value: ch as i32 };
        }
    }
    BloodOptionChar { tag: 0, value: 0 }
}

/// Get character at byte index from a String (heap-allocated).
///
/// String layout: { ptr: *u8, len: u64, capacity: u64 }
/// We only use ptr and len for this operation.
///
/// # Safety
/// The pointer must be valid.
#[no_mangle]
pub unsafe extern "C" fn string_char_at(s: *const BloodStr, index: u64) -> BloodOptionChar {
    // String has same ptr/len layout at the beginning as BloodStr
    str_char_at(s, index)
}

/// Convert a str to a byte slice.
///
/// Since str is already stored as {ptr, len} which is the same as &[u8],
/// this is essentially an identity operation. We return the same fat pointer.
///
/// Returns: {ptr: *u8, len: i64} - a byte slice fat pointer
///
/// # Safety
/// The pointer must point to a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn str_as_bytes(s: *const BloodStr) -> BloodStr {
    if s.is_null() {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }
    // Return the same {ptr, len} - str bytes ARE the byte slice
    BloodStr { ptr: (*s).ptr, len: (*s).len }
}

/// Convert a String to a byte slice.
///
/// String layout: { ptr: *u8, len: u64, capacity: u64 }
/// We only use ptr and len to create the slice.
///
/// Returns: {ptr: *u8, len: i64} - a byte slice fat pointer
///
/// # Safety
/// The pointer must point to a valid BloodString.
#[no_mangle]
pub unsafe extern "C" fn string_as_bytes(s: *const BloodStr) -> BloodStr {
    // String has same ptr/len layout at the beginning as BloodStr
    str_as_bytes(s)
}

/// Count the number of UTF-8 characters in a string (not bytes).
///
/// Returns: i64 - the character count
///
/// # Safety
/// The pointer must point to a valid BloodStr containing valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn str_len_chars(s: *const BloodStr) -> i64 {
    if s.is_null() {
        return 0;
    }
    let s = &*s;
    if s.ptr.is_null() || s.len == 0 {
        return 0;
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    if let Ok(str_val) = std::str::from_utf8(slice) {
        str_val.chars().count() as i64
    } else {
        0
    }
}

/// Count the number of UTF-8 characters in a String (not bytes).
///
/// String layout: { ptr: *u8, len: u64, capacity: u64 }
/// We only use ptr and len.
///
/// Returns: i64 - the character count
///
/// # Safety
/// The pointer must point to a valid BloodString containing valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn string_len_chars(s: *const BloodStr) -> i64 {
    // String has same ptr/len layout at the beginning as BloodStr
    str_len_chars(s)
}

/// Check if a String contains a substring.
///
/// # Arguments
/// * `s` - The string to search in
/// * `pattern` - The substring to search for
///
/// # Returns
/// 1 if the pattern is found, 0 otherwise
///
/// # Safety
/// Both pointers must point to valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_contains(s: *const BloodStr, pattern: *const BloodStr) -> i32 {
    if s.is_null() || pattern.is_null() {
        return 0;
    }
    let s = &*s;
    let pattern = &*pattern;

    if s.ptr.is_null() || s.len == 0 {
        // Empty string only contains empty pattern
        return if pattern.ptr.is_null() || pattern.len == 0 { 1 } else { 0 };
    }
    if pattern.ptr.is_null() || pattern.len == 0 {
        return 1; // Empty pattern is in every string
    }

    let s_slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    let p_slice = std::slice::from_raw_parts(pattern.ptr, pattern.len as usize);

    // Simple substring search
    if p_slice.len() > s_slice.len() {
        return 0;
    }
    for i in 0..=(s_slice.len() - p_slice.len()) {
        if &s_slice[i..i+p_slice.len()] == p_slice {
            return 1;
        }
    }
    0
}

/// Check if a String starts with a prefix.
///
/// # Arguments
/// * `s` - The string to check
/// * `prefix` - The prefix to look for
///
/// # Returns
/// 1 if the string starts with the prefix, 0 otherwise
///
/// # Safety
/// Both pointers must point to valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_starts_with(s: *const BloodStr, prefix: *const BloodStr) -> i32 {
    if s.is_null() || prefix.is_null() {
        return 0;
    }
    let s = &*s;
    let prefix = &*prefix;

    if prefix.ptr.is_null() || prefix.len == 0 {
        return 1; // Empty prefix matches everything
    }
    if s.ptr.is_null() || s.len == 0 {
        return 0; // Non-empty prefix can't match empty string
    }
    if prefix.len > s.len {
        return 0;
    }

    let s_slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    let p_slice = std::slice::from_raw_parts(prefix.ptr, prefix.len as usize);

    if s_slice.starts_with(p_slice) { 1 } else { 0 }
}

/// Check if a String ends with a suffix.
///
/// # Arguments
/// * `s` - The string to check
/// * `suffix` - The suffix to look for
///
/// # Returns
/// 1 if the string ends with the suffix, 0 otherwise
///
/// # Safety
/// Both pointers must point to valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_ends_with(s: *const BloodStr, suffix: *const BloodStr) -> i32 {
    if s.is_null() || suffix.is_null() {
        return 0;
    }
    let s = &*s;
    let suffix = &*suffix;

    if suffix.ptr.is_null() || suffix.len == 0 {
        return 1; // Empty suffix matches everything
    }
    if s.ptr.is_null() || s.len == 0 {
        return 0; // Non-empty suffix can't match empty string
    }
    if suffix.len > s.len {
        return 0;
    }

    let s_slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    let p_slice = std::slice::from_raw_parts(suffix.ptr, suffix.len as usize);

    if s_slice.ends_with(p_slice) { 1 } else { 0 }
}

/// Find the first occurrence of a substring in a String.
///
/// # Arguments
/// * `s` - The string to search in
/// * `pattern` - The substring to search for
/// * `out` - Output buffer for Option<usize> result { tag: i32, payload: i64 }
///
/// # Safety
/// All pointers must be valid. `out` must have space for the Option struct.
#[no_mangle]
pub unsafe extern "C" fn string_find(s: *const BloodStr, pattern: *const BloodStr, out: *mut u8) {
    if out.is_null() {
        return;
    }

    // Option<usize> layout: { tag: i32, payload: i64 }
    // tag=0 is None, tag=1 is Some
    let out_tag = out as *mut i32;
    let out_payload = out.add(8) as *mut i64;

    if s.is_null() || pattern.is_null() {
        *out_tag = 0; // None
        return;
    }
    let s = &*s;
    let pattern = &*pattern;

    if s.ptr.is_null() || s.len == 0 {
        if pattern.ptr.is_null() || pattern.len == 0 {
            *out_tag = 1; // Some(0)
            *out_payload = 0;
        } else {
            *out_tag = 0; // None
        }
        return;
    }

    if pattern.ptr.is_null() || pattern.len == 0 {
        *out_tag = 1; // Some(0) - empty pattern found at start
        *out_payload = 0;
        return;
    }

    let s_slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    let p_slice = std::slice::from_raw_parts(pattern.ptr, pattern.len as usize);

    if p_slice.len() > s_slice.len() {
        *out_tag = 0; // None
        return;
    }

    for i in 0..=(s_slice.len() - p_slice.len()) {
        if &s_slice[i..i+p_slice.len()] == p_slice {
            *out_tag = 1; // Some(i)
            *out_payload = i as i64;
            return;
        }
    }
    *out_tag = 0; // None
}

/// Find the last occurrence of a substring in a String.
///
/// # Arguments
/// * `s` - The string to search in
/// * `pattern` - The substring to search for
/// * `out` - Output buffer for Option<usize> result { tag: i32, payload: i64 }
///
/// # Safety
/// All pointers must be valid. `out` must have space for the Option struct.
#[no_mangle]
pub unsafe extern "C" fn string_rfind(s: *const BloodStr, pattern: *const BloodStr, out: *mut u8) {
    if out.is_null() {
        return;
    }

    // Option<usize> layout: { tag: i32, payload: i64 }
    let out_tag = out as *mut i32;
    let out_payload = out.add(8) as *mut i64;

    if s.is_null() || pattern.is_null() {
        *out_tag = 0; // None
        return;
    }
    let s = &*s;
    let pattern = &*pattern;

    if s.ptr.is_null() || s.len == 0 {
        if pattern.ptr.is_null() || pattern.len == 0 {
            *out_tag = 1; // Some(0)
            *out_payload = 0;
        } else {
            *out_tag = 0; // None
        }
        return;
    }

    if pattern.ptr.is_null() || pattern.len == 0 {
        *out_tag = 1; // Some(len) - empty pattern found at end
        *out_payload = s.len as i64;
        return;
    }

    let s_slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    let p_slice = std::slice::from_raw_parts(pattern.ptr, pattern.len as usize);

    if p_slice.len() > s_slice.len() {
        *out_tag = 0; // None
        return;
    }

    // Search from end
    for i in (0..=(s_slice.len() - p_slice.len())).rev() {
        if &s_slice[i..i+p_slice.len()] == p_slice {
            *out_tag = 1; // Some(i)
            *out_payload = i as i64;
            return;
        }
    }
    *out_tag = 0; // None
}

/// Trim whitespace from both ends of a String.
///
/// Returns a new BloodStr pointing to the trimmed portion (no allocation).
///
/// # Arguments
/// * `s` - The string to trim
///
/// # Returns
/// BloodStr pointing to trimmed portion
///
/// # Safety
/// The pointer must point to a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_trim(s: *const BloodStr) -> BloodStr {
    if s.is_null() {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }
    let s = &*s;
    if s.ptr.is_null() || s.len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);

    // Find start (skip leading whitespace)
    let mut start = 0;
    while start < slice.len() && is_ascii_whitespace(slice[start]) {
        start += 1;
    }

    // Find end (skip trailing whitespace)
    let mut end = slice.len();
    while end > start && is_ascii_whitespace(slice[end - 1]) {
        end -= 1;
    }

    BloodStr {
        ptr: s.ptr.add(start),
        len: (end - start) as u64,
    }
}

/// Trim whitespace from the start of a String.
///
/// # Arguments
/// * `s` - The string to trim
///
/// # Returns
/// BloodStr pointing to trimmed portion
///
/// # Safety
/// The pointer must point to a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_trim_start(s: *const BloodStr) -> BloodStr {
    if s.is_null() {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }
    let s = &*s;
    if s.ptr.is_null() || s.len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);

    // Find start (skip leading whitespace)
    let mut start = 0;
    while start < slice.len() && is_ascii_whitespace(slice[start]) {
        start += 1;
    }

    BloodStr {
        ptr: s.ptr.add(start),
        len: (s.len as usize - start) as u64,
    }
}

/// Trim whitespace from the end of a String.
///
/// # Arguments
/// * `s` - The string to trim
///
/// # Returns
/// BloodStr pointing to trimmed portion
///
/// # Safety
/// The pointer must point to a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_trim_end(s: *const BloodStr) -> BloodStr {
    if s.is_null() {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }
    let s = &*s;
    if s.ptr.is_null() || s.len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);

    // Find end (skip trailing whitespace)
    let mut end = slice.len();
    while end > 0 && is_ascii_whitespace(slice[end - 1]) {
        end -= 1;
    }

    BloodStr {
        ptr: s.ptr,
        len: end as u64,
    }
}

/// Convert string to uppercase.
///
/// # Arguments
/// * `s` - Pointer to the source BloodStr
///
/// # Returns
/// A new BloodVec containing the uppercase string.
///
/// # Safety
/// The pointer must point to a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_to_uppercase(s: *const BloodStr) -> BloodVec {
    if s.is_null() {
        return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 };
    }
    let s = &*s;
    if s.ptr.is_null() || s.len == 0 {
        return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 };
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    let str_slice = match std::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 },
    };
    let upper = str_slice.to_uppercase();
    let bytes = upper.into_bytes();

    let len = bytes.len() as i64;
    let capacity = len;
    let ptr = if len > 0 {
        let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
        let ptr = runtime_alloc(layout);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len as usize);
        ptr
    } else {
        std::ptr::null_mut()
    };

    BloodVec { ptr, len, capacity }
}

/// Convert string to lowercase.
///
/// # Arguments
/// * `s` - Pointer to the source BloodStr
///
/// # Returns
/// A new BloodVec containing the lowercase string.
///
/// # Safety
/// The pointer must point to a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn string_to_lowercase(s: *const BloodStr) -> BloodVec {
    if s.is_null() {
        return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 };
    }
    let s = &*s;
    if s.ptr.is_null() || s.len == 0 {
        return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 };
    }

    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    let str_slice = match std::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 },
    };
    let lower = str_slice.to_lowercase();
    let bytes = lower.into_bytes();

    let len = bytes.len() as i64;
    let capacity = len;
    let ptr = if len > 0 {
        let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
        let ptr = runtime_alloc(layout);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len as usize);
        ptr
    } else {
        std::ptr::null_mut()
    };

    BloodVec { ptr, len, capacity }
}

/// Replace all occurrences of a pattern in a string.
///
/// # Arguments
/// * `s` - Pointer to the source BloodStr
/// * `from` - Pointer to the pattern to replace
/// * `to` - Pointer to the replacement string
///
/// # Returns
/// A new BloodVec containing the string with replacements.
///
/// # Safety
/// All pointers must point to valid BloodStr structs.
#[no_mangle]
pub unsafe extern "C" fn string_replace(
    s: *const BloodStr,
    from: *const BloodStr,
    to: *const BloodStr,
) -> BloodVec {
    if s.is_null() {
        return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 };
    }
    let s_ref = &*s;
    if s_ref.ptr.is_null() || s_ref.len == 0 {
        return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 };
    }

    let slice = std::slice::from_raw_parts(s_ref.ptr, s_ref.len as usize);
    let str_slice = match std::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 },
    };

    // Get the from pattern
    let from_str = if from.is_null() {
        ""
    } else {
        let from_ref = &*from;
        if from_ref.ptr.is_null() || from_ref.len == 0 {
            ""
        } else {
            match std::str::from_utf8(
                std::slice::from_raw_parts(from_ref.ptr, from_ref.len as usize)
            ) {
                Ok(s) => s,
                Err(_) => return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 },
            }
        }
    };

    // Get the to replacement
    let to_str = if to.is_null() {
        ""
    } else {
        let to_ref = &*to;
        if to_ref.ptr.is_null() || to_ref.len == 0 {
            ""
        } else {
            match std::str::from_utf8(
                std::slice::from_raw_parts(to_ref.ptr, to_ref.len as usize)
            ) {
                Ok(s) => s,
                Err(_) => return BloodVec { ptr: std::ptr::null_mut(), len: 0, capacity: 0 },
            }
        }
    };

    // If from is empty, return the original string
    if from_str.is_empty() {
        let len = s_ref.len as i64;
        let capacity = len;
        let ptr = if len > 0 {
            let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
            let ptr = runtime_alloc(layout);
            std::ptr::copy_nonoverlapping(s_ref.ptr, ptr, len as usize);
            ptr
        } else {
            std::ptr::null_mut()
        };
        return BloodVec { ptr, len, capacity };
    }

    let result = str_slice.replace(from_str, to_str);
    let bytes = result.into_bytes();

    let len = bytes.len() as i64;
    let capacity = len;
    let ptr = if len > 0 {
        let layout = std::alloc::Layout::from_size_align(len as usize, 1).unwrap();
        let ptr = runtime_alloc(layout);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len as usize);
        ptr
    } else {
        std::ptr::null_mut()
    };

    BloodVec { ptr, len, capacity }
}

/// Convert str to uppercase (alias for string_to_uppercase).
///
/// # Safety
///
/// `s` must be a valid pointer to a `BloodStr` with a valid UTF-8 `ptr`/`len` pair.
#[no_mangle]
pub unsafe extern "C" fn str_to_uppercase(s: *const BloodStr) -> BloodVec {
    string_to_uppercase(s)
}

/// Convert str to lowercase (alias for string_to_lowercase).
///
/// # Safety
///
/// `s` must be a valid pointer to a `BloodStr` with a valid UTF-8 `ptr`/`len` pair.
#[no_mangle]
pub unsafe extern "C" fn str_to_lowercase(s: *const BloodStr) -> BloodVec {
    string_to_lowercase(s)
}

/// Replace all occurrences in str (alias for string_replace).
///
/// # Safety
///
/// All three pointers (`s`, `from`, `to`) must be valid pointers to `BloodStr`
/// values with valid UTF-8 `ptr`/`len` pairs.
#[no_mangle]
pub unsafe extern "C" fn str_replace(
    s: *const BloodStr,
    from: *const BloodStr,
    to: *const BloodStr,
) -> BloodVec {
    string_replace(s, from, to)
}

/// Helper function to check if a byte is ASCII whitespace.
#[inline]
fn is_ascii_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
}

// ============================================================================
// Input Functions
// ============================================================================

/// Read a line from stdin into a statically allocated buffer.
///
/// Returns a BloodStr pointing to the line (without trailing newline).
/// The buffer is reused on each call, so the string is only valid until
/// the next call to read_line.
///
/// On EOF or error, returns an empty string (ptr=null, len=0).
#[no_mangle]
pub extern "C" fn read_line() -> BloodStr {
    use std::io::BufRead;
    use std::cell::RefCell;

    // Thread-local buffer for holding the read line.
    // Using thread_local avoids the unsafety of static mut.
    thread_local! {
        static LINE_BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    let stdin = std::io::stdin();
    let mut handle = stdin.lock();

    LINE_BUFFER.with(|buffer| {
        let mut buf = buffer.borrow_mut();
        buf.clear();

        match handle.read_until(b'\n', &mut buf) {
            Ok(0) => BloodStr { ptr: std::ptr::null(), len: 0 }, // EOF
            Ok(n) => {
                // Strip trailing newline if present
                let len = if n > 0 && buf[n - 1] == b'\n' {
                    n - 1
                } else {
                    n
                };
                // Also strip carriage return if present (Windows)
                let len = if len > 0 && buf[len - 1] == b'\r' {
                    len - 1
                } else {
                    len
                };
                BloodStr {
                    ptr: buf.as_ptr(),
                    len: len as u64,
                }
            }
            Err(_) => BloodStr { ptr: std::ptr::null(), len: 0 },
        }
    })
}

/// Read an integer from stdin.
///
/// Reads a line from stdin, trims whitespace, and parses as i32.
/// Returns 0 on parse error or EOF.
#[no_mangle]
pub extern "C" fn read_int() -> i32 {
    use std::io::BufRead;

    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let mut line = String::new();

    match handle.read_line(&mut line) {
        Ok(_) => line.trim().parse::<i32>().unwrap_or(0),
        Err(_) => 0,
    }
}

// ============================================================================
// Path Security Functions
// ============================================================================

/// Validate that a path does not contain traversal attempts.
///
/// Returns `Some(canonical_path)` if the path is safe, `None` if it contains
/// traversal attempts or cannot be canonicalized.
///
/// This function:
/// - Rejects paths containing `..` components
/// - Rejects paths containing null bytes
/// - Canonicalizes the path to resolve symlinks
fn validate_and_canonicalize_path(path_str: &str) -> Option<std::path::PathBuf> {
    use std::path::Path;

    // Reject paths with null bytes (shouldn't happen with valid UTF-8, but defense in depth)
    if path_str.contains('\0') {
        return None;
    }

    let path = Path::new(path_str);

    // Check for obvious traversal attempts in the raw path
    // This catches attempts even before canonicalization
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            // Path contains `..` - this is a traversal attempt
            return None;
        }
    }

    // Canonicalize to resolve symlinks and get the absolute path
    // This also catches symlink-based traversal attacks
    match std::fs::canonicalize(path) {
        Ok(canonical) => Some(canonical),
        Err(_) => {
            // If the file doesn't exist yet (for write operations),
            // canonicalize the parent directory and append the filename
            if let Some(parent) = path.parent() {
                if parent.as_os_str().is_empty() {
                    // No parent directory specified, use current directory
                    if let Ok(cwd) = std::env::current_dir() {
                        if let Some(filename) = path.file_name() {
                            return Some(cwd.join(filename));
                        }
                    }
                } else if let Ok(canonical_parent) = std::fs::canonicalize(parent) {
                    if let Some(filename) = path.file_name() {
                        return Some(canonical_parent.join(filename));
                    }
                }
            }
            None
        }
    }
}

/// Canonicalize a path, resolving symlinks and normalizing the path.
///
/// This function is essential for security - it should be used before any
/// file operation to prevent path traversal attacks.
///
/// # Arguments
/// * `path` - Path to canonicalize as a BloodStr
///
/// # Returns
/// * BloodStr containing the canonical path on success
/// * Empty BloodStr (ptr=null, len=0) on error or if path is unsafe
///
/// # Safety
/// The path pointer must be valid for `path.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn path_canonicalize(path: BloodStr) -> BloodStr {
    if path.ptr.is_null() || path.len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return BloodStr { ptr: std::ptr::null(), len: 0 },
    };

    match validate_and_canonicalize_path(path_str) {
        Some(canonical) => {
            let canonical_str = canonical.to_string_lossy().into_owned();
            string_to_blood_str(canonical_str)
        }
        None => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

/// Check if a path is safe (does not contain traversal attempts).
///
/// This is a quick check that can be used before file operations to detect
/// obvious path traversal attacks without the overhead of full canonicalization.
///
/// # Arguments
/// * `path` - Path to check as a BloodStr
///
/// # Returns
/// * true if the path appears safe
/// * false if the path contains traversal attempts or is invalid
///
/// # Safety
/// The path pointer must be valid for `path.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn path_is_safe(path: BloodStr) -> bool {
    use std::path::Path;

    if path.ptr.is_null() || path.len == 0 {
        return false;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Reject null bytes
    if path_str.contains('\0') {
        return false;
    }

    // Check for traversal components
    let p = Path::new(path_str);
    for component in p.components() {
        if let std::path::Component::ParentDir = component {
            return false;
        }
    }

    true
}

/// Check if a path is within a given base directory.
///
/// This is useful for sandboxing file operations to a specific directory.
/// The path is canonicalized and checked to ensure it starts with the
/// canonicalized base directory.
///
/// # Arguments
/// * `path` - Path to check as a BloodStr
/// * `base` - Base directory that the path must be within
///
/// # Returns
/// * true if the canonicalized path starts with the canonicalized base
/// * false otherwise (including if either path is invalid)
///
/// # Safety
/// Both pointers must be valid for their respective lengths.
#[no_mangle]
pub unsafe extern "C" fn path_is_within(path: BloodStr, base: BloodStr) -> bool {
    if path.ptr.is_null() || path.len == 0 || base.ptr.is_null() || base.len == 0 {
        return false;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let base_slice = std::slice::from_raw_parts(base.ptr, base.len as usize);
    let base_str = match std::str::from_utf8(base_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Canonicalize both paths
    let canonical_path = match validate_and_canonicalize_path(path_str) {
        Some(p) => p,
        None => return false,
    };

    let canonical_base = match std::fs::canonicalize(base_str) {
        Ok(b) => b,
        Err(_) => return false,
    };

    // Check if path starts with base
    canonical_path.starts_with(&canonical_base)
}

// ============================================================================
// File I/O Functions
// ============================================================================

/// Open a file and return a file descriptor.
///
/// # Arguments
/// * `path` - Path to the file as a BloodStr
/// * `mode` - Mode string: "r" (read), "w" (write), "a" (append), "rw" (read/write)
///
/// # Returns
/// * File descriptor (>= 0) on success
/// * -1 on error
///
/// # Safety
/// The path pointer must be valid for `path.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn file_open(path: BloodStr, mode: BloodStr) -> i64 {
    use std::fs::{File, OpenOptions};
    use std::os::unix::io::IntoRawFd;

    if path.ptr.is_null() || path.len == 0 {
        return -1;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    // Validate path for traversal attacks
    let canonical_path = match validate_and_canonicalize_path(path_str) {
        Some(p) => p,
        None => return -1, // Path contains traversal attempt or is invalid
    };

    let mode_str = if mode.ptr.is_null() || mode.len == 0 {
        "r"
    } else {
        let mode_slice = std::slice::from_raw_parts(mode.ptr, mode.len as usize);
        match std::str::from_utf8(mode_slice) {
            Ok(s) => s,
            Err(_) => return -1,
        }
    };

    let file_result = match mode_str {
        "r" => File::open(&canonical_path),
        "w" => File::create(&canonical_path),
        "a" => OpenOptions::new().append(true).create(true).open(&canonical_path),
        "rw" => OpenOptions::new().read(true).write(true).open(&canonical_path),
        "rw+" => OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&canonical_path),
        _ => return -1,
    };

    match file_result {
        Ok(file) => file.into_raw_fd() as i64,
        Err(_) => -1,
    }
}

/// Read from a file descriptor into a buffer.
///
/// # Arguments
/// * `fd` - File descriptor
/// * `buf` - Buffer to read into
/// * `count` - Maximum number of bytes to read
///
/// # Returns
/// * Number of bytes read (>= 0) on success
/// * -1 on error
///
/// # Safety
/// The buffer must be valid for `count` bytes.
#[no_mangle]
pub unsafe extern "C" fn file_read(fd: i64, buf: *mut u8, count: u64) -> i64 {
    use std::os::unix::io::FromRawFd;
    use std::io::Read;

    if fd < 0 || buf.is_null() {
        return -1;
    }

    // Create a File from the raw fd, but use ManuallyDrop to avoid closing it
    let mut file = std::mem::ManuallyDrop::new(std::fs::File::from_raw_fd(fd as i32));
    let buffer = std::slice::from_raw_parts_mut(buf, count as usize);

    match file.read(buffer) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Write to a file descriptor from a buffer.
///
/// # Arguments
/// * `fd` - File descriptor
/// * `buf` - Buffer to write from
/// * `count` - Number of bytes to write
///
/// # Returns
/// * Number of bytes written (>= 0) on success
/// * -1 on error
///
/// # Safety
/// The buffer must be valid for `count` bytes.
#[no_mangle]
pub unsafe extern "C" fn file_write(fd: i64, buf: *const u8, count: u64) -> i64 {
    use std::os::unix::io::FromRawFd;
    use std::io::Write;

    if fd < 0 || buf.is_null() {
        return -1;
    }

    // Create a File from the raw fd, but use ManuallyDrop to avoid closing it
    let mut file = std::mem::ManuallyDrop::new(std::fs::File::from_raw_fd(fd as i32));
    let buffer = std::slice::from_raw_parts(buf, count as usize);

    match file.write(buffer) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Close a file descriptor.
///
/// # Arguments
/// * `fd` - File descriptor to close
///
/// # Returns
/// * 0 on success
/// * -1 on error
#[no_mangle]
pub extern "C" fn file_close(fd: i64) -> i32 {
    use std::os::unix::io::FromRawFd;

    if fd < 0 {
        return -1;
    }

    // Create a File from the raw fd and let it drop (which closes the fd)
    let _file = unsafe { std::fs::File::from_raw_fd(fd as i32) };
    0
}

/// Read an entire file into a string.
///
/// This is a convenience function that opens a file, reads its entire contents,
/// and returns it as a BloodStr. The returned string is allocated on the heap
/// and must be freed by the caller using `blood_str_free`.
///
/// # Arguments
/// * `path` - Path to the file as a BloodStr
///
/// # Returns
/// * BloodStr containing file contents on success
/// * Empty BloodStr (ptr=null, len=0) on error
///
/// # Safety
/// The path pointer must be valid for `path.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn file_read_to_string(path: BloodStr) -> BloodStr {
    use std::fs;

    if path.ptr.is_null() || path.len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return BloodStr { ptr: std::ptr::null(), len: 0 },
    };

    // Validate path for traversal attacks
    let canonical_path = match validate_and_canonicalize_path(path_str) {
        Some(p) => p,
        None => return BloodStr { ptr: std::ptr::null(), len: 0 },
    };

    match fs::read_to_string(&canonical_path) {
        Ok(contents) => string_to_blood_str(contents),
        Err(_) => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

/// Write a string to a file, creating or truncating it.
///
/// # Arguments
/// * `path` - Path to the file as a BloodStr
/// * `content` - Content to write as a BloodStr
///
/// # Returns
/// * true on success
/// * false on error
///
/// # Safety
/// Both pointers must be valid for their respective lengths.
#[no_mangle]
pub unsafe extern "C" fn file_write_string(path: BloodStr, content: BloodStr) -> bool {
    use std::fs;

    if path.ptr.is_null() || path.len == 0 {
        return false;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Validate path for traversal attacks
    let canonical_path = match validate_and_canonicalize_path(path_str) {
        Some(p) => p,
        None => return false,
    };

    let content_str = if content.ptr.is_null() || content.len == 0 {
        ""
    } else {
        let content_slice = std::slice::from_raw_parts(content.ptr, content.len as usize);
        match std::str::from_utf8(content_slice) {
            Ok(s) => s,
            Err(_) => return false,
        }
    };

    fs::write(&canonical_path, content_str).is_ok()
}

/// Append a string to a file, creating it if it doesn't exist.
///
/// # Arguments
/// * `path` - Path to the file as a BloodStr
/// * `content` - Content to append as a BloodStr
///
/// # Returns
/// * true on success
/// * false on error
///
/// # Safety
/// Both pointers must be valid for their respective lengths.
#[no_mangle]
pub unsafe extern "C" fn file_append_string(path: BloodStr, content: BloodStr) -> bool {
    use std::fs::OpenOptions;
    use std::io::Write;

    if path.ptr.is_null() || path.len == 0 {
        return false;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Validate path for traversal attacks
    let canonical_path = match validate_and_canonicalize_path(path_str) {
        Some(p) => p,
        None => return false,
    };

    let content_str = if content.ptr.is_null() || content.len == 0 {
        ""
    } else {
        let content_slice = std::slice::from_raw_parts(content.ptr, content.len as usize);
        match std::str::from_utf8(content_slice) {
            Ok(s) => s,
            Err(_) => return false,
        }
    };

    match OpenOptions::new().append(true).create(true).open(&canonical_path) {
        Ok(mut file) => file.write_all(content_str.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Check if a file exists.
///
/// # Arguments
/// * `path` - Path to check as a BloodStr
///
/// # Returns
/// * true if the file exists
/// * false otherwise
///
/// # Safety
/// The path pointer must be valid for `path.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn file_exists(path: BloodStr) -> bool {
    use std::path::Path;

    if path.ptr.is_null() || path.len == 0 {
        return false;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Check for traversal attempts (reject `..` components)
    // Note: We don't require full canonicalization here since the file might not exist
    let p = Path::new(path_str);
    for component in p.components() {
        if let std::path::Component::ParentDir = component {
            return false; // Traversal attempt
        }
    }

    p.exists()
}

/// Delete a file.
///
/// # Arguments
/// * `path` - Path to the file to delete as a BloodStr
///
/// # Returns
/// * true on success
/// * false on error
///
/// # Safety
/// The path pointer must be valid for `path.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn file_delete(path: BloodStr) -> bool {
    use std::fs;

    if path.ptr.is_null() || path.len == 0 {
        return false;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Validate path for traversal attacks
    let canonical_path = match validate_and_canonicalize_path(path_str) {
        Some(p) => p,
        None => return false,
    };

    fs::remove_file(&canonical_path).is_ok()
}

/// Get the size of a file in bytes.
///
/// # Arguments
/// * `path` - Path to the file as a BloodStr
///
/// # Returns
/// * File size in bytes (>= 0) on success
/// * -1 on error
///
/// # Safety
/// The path pointer must be valid for `path.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn file_size(path: BloodStr) -> i64 {
    use std::fs;

    if path.ptr.is_null() || path.len == 0 {
        return -1;
    }

    let path_slice = std::slice::from_raw_parts(path.ptr, path.len as usize);
    let path_str = match std::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    // Validate path for traversal attacks
    let canonical_path = match validate_and_canonicalize_path(path_str) {
        Some(p) => p,
        None => return -1,
    };

    match fs::metadata(&canonical_path) {
        Ok(metadata) => metadata.len() as i64,
        Err(_) => -1,
    }
}

// ============================================================================
// Secure Temporary Files
// ============================================================================

/// Registry of open temporary files.
/// Maps handle IDs to (File, PathBuf, delete_on_close) tuples.
static TEMP_FILE_REGISTRY: OnceLock<Mutex<HashMap<u64, (std::fs::File, std::path::PathBuf, bool)>>> = OnceLock::new();

/// Counter for generating unique temp file handles.
static TEMP_FILE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn get_temp_file_registry() -> &'static Mutex<HashMap<u64, (std::fs::File, std::path::PathBuf, bool)>> {
    TEMP_FILE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Create a secure temporary file.
///
/// Creates a temporary file with a unique name in the system's temp directory.
/// The file is created with restricted permissions (owner read/write only on Unix).
///
/// # Arguments
/// * `prefix` - Optional prefix for the filename (can be empty)
/// * `suffix` - Optional suffix/extension for the filename (can be empty)
/// * `delete_on_close` - If true, the file will be deleted when closed
///
/// # Returns
/// * Handle ID (> 0) on success
/// * 0 on error
///
/// # Safety
/// The prefix and suffix pointers must be valid for their respective lengths.
#[no_mangle]
pub unsafe extern "C" fn temp_file_create(
    prefix: BloodStr,
    suffix: BloodStr,
    delete_on_close: bool,
) -> u64 {
    use std::fs::OpenOptions;

    // Parse prefix (default to "blood_")
    let prefix_str = if prefix.ptr.is_null() || prefix.len == 0 {
        "blood_"
    } else {
        let prefix_slice = std::slice::from_raw_parts(prefix.ptr, prefix.len as usize);
        match std::str::from_utf8(prefix_slice) {
            Ok(s) => s,
            Err(_) => "blood_",
        }
    };

    // Parse suffix (default to empty)
    let suffix_str = if suffix.ptr.is_null() || suffix.len == 0 {
        ""
    } else {
        let suffix_slice = std::slice::from_raw_parts(suffix.ptr, suffix.len as usize);
        match std::str::from_utf8(suffix_slice) {
            Ok(s) => s,
            Err(_) => "",
        }
    };

    // Validate prefix and suffix don't contain path separators (security)
    if prefix_str.contains('/') || prefix_str.contains('\\') ||
       suffix_str.contains('/') || suffix_str.contains('\\') {
        return 0;
    }

    // Get system temp directory
    let temp_dir = std::env::temp_dir();

    // Generate unique filename using secure random bytes
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 100;

    while attempts < MAX_ATTEMPTS {
        // Generate random component
        let random_component: u64 = {
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let counter = TEMP_FILE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Mix time with counter and process ID for uniqueness
            nanos.wrapping_mul(31).wrapping_add(counter).wrapping_mul(17).wrapping_add(std::process::id() as u64)
        };

        let filename = format!("{}{:016x}{}", prefix_str, random_component, suffix_str);
        let path = temp_dir.join(&filename);

        // Create file with O_EXCL to prevent race conditions
        #[cfg(unix)]
        let file_result = {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)  // O_EXCL - fail if exists
                .mode(0o600)       // Owner read/write only
                .open(&path)
        };

        #[cfg(not(unix))]
        let file_result = {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)  // Fail if exists
                .open(&path)
        };

        match file_result {
            Ok(file) => {
                let handle = TEMP_FILE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let registry = get_temp_file_registry();
                let mut guard = registry.lock();
                guard.insert(handle, (file, path, delete_on_close));
                return handle;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Try again with different random name
                attempts += 1;
                continue;
            }
            Err(_) => {
                return 0;
            }
        }
    }

    0 // Failed after max attempts
}

/// Write data to a temporary file.
///
/// # Arguments
/// * `handle` - Temp file handle from temp_file_create
/// * `data` - Data to write
///
/// # Returns
/// * Number of bytes written (>= 0) on success
/// * -1 on error
///
/// # Safety
/// The data pointer must be valid for `data.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn temp_file_write(handle: u64, data: BloodStr) -> i64 {
    use std::io::Write;

    if handle == 0 {
        return -1;
    }

    let registry = get_temp_file_registry();
    let mut guard = registry.lock();

    let entry = match guard.get_mut(&handle) {
        Some(e) => e,
        None => return -1,
    };

    if data.ptr.is_null() || data.len == 0 {
        return 0; // Nothing to write
    }

    let data_slice = std::slice::from_raw_parts(data.ptr, data.len as usize);

    match entry.0.write(data_slice) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Read data from a temporary file.
///
/// Reads up to `max_len` bytes from the current position.
///
/// # Arguments
/// * `handle` - Temp file handle from temp_file_create
/// * `buf` - Buffer to read into
/// * `max_len` - Maximum number of bytes to read
///
/// # Returns
/// * Number of bytes read (>= 0) on success
/// * -1 on error
///
/// # Safety
/// The buffer must be valid for `max_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn temp_file_read(handle: u64, buf: *mut u8, max_len: u64) -> i64 {
    use std::io::Read;

    if handle == 0 || buf.is_null() {
        return -1;
    }

    let registry = get_temp_file_registry();
    let mut guard = registry.lock();

    let entry = match guard.get_mut(&handle) {
        Some(e) => e,
        None => return -1,
    };

    let buffer = std::slice::from_raw_parts_mut(buf, max_len as usize);

    match entry.0.read(buffer) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Seek to a position in a temporary file.
///
/// # Arguments
/// * `handle` - Temp file handle
/// * `position` - Position to seek to from the beginning of the file
///
/// # Returns
/// * New position on success
/// * -1 on error
#[no_mangle]
pub extern "C" fn temp_file_seek(handle: u64, position: u64) -> i64 {
    use std::io::{Seek, SeekFrom};

    if handle == 0 {
        return -1;
    }

    let registry = get_temp_file_registry();
    let mut guard = registry.lock();

    let entry = match guard.get_mut(&handle) {
        Some(e) => e,
        None => return -1,
    };

    match entry.0.seek(SeekFrom::Start(position)) {
        Ok(pos) => pos as i64,
        Err(_) => -1,
    }
}

/// Get the path of a temporary file.
///
/// # Arguments
/// * `handle` - Temp file handle
///
/// # Returns
/// * BloodStr containing the file path on success
/// * Empty BloodStr on error
#[no_mangle]
pub extern "C" fn temp_file_path(handle: u64) -> BloodStr {
    if handle == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    let registry = get_temp_file_registry();
    let guard = registry.lock();

    let entry = match guard.get(&handle) {
        Some(e) => e,
        None => return BloodStr { ptr: std::ptr::null(), len: 0 },
    };

    let path_str = entry.1.to_string_lossy().into_owned();
    string_to_blood_str(path_str)
}

/// Close a temporary file.
///
/// If the file was created with delete_on_close=true, the file will be deleted.
///
/// # Arguments
/// * `handle` - Temp file handle to close
///
/// # Returns
/// * true on success
/// * false on error
#[no_mangle]
pub extern "C" fn temp_file_close(handle: u64) -> bool {
    if handle == 0 {
        return false;
    }

    let registry = get_temp_file_registry();
    let mut guard = registry.lock();

    let entry = match guard.remove(&handle) {
        Some(e) => e,
        None => return false,
    };

    let (file, path, delete_on_close) = entry;

    // Drop the file handle first to close it
    drop(file);

    // Delete if requested
    if delete_on_close {
        let _ = std::fs::remove_file(&path);
    }

    true
}

/// Persist a temporary file by moving it to a permanent location.
///
/// This closes the temp file and moves it to the specified destination.
/// The file is removed from the temp file registry.
///
/// # Arguments
/// * `handle` - Temp file handle
/// * `dest` - Destination path
///
/// # Returns
/// * true on success
/// * false on error
///
/// # Safety
/// The dest pointer must be valid for `dest.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn temp_file_persist(handle: u64, dest: BloodStr) -> bool {
    if handle == 0 || dest.ptr.is_null() || dest.len == 0 {
        return false;
    }

    let dest_slice = std::slice::from_raw_parts(dest.ptr, dest.len as usize);
    let dest_str = match std::str::from_utf8(dest_slice) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Validate destination path
    let dest_path = match validate_and_canonicalize_path(dest_str) {
        Some(p) => p,
        None => return false,
    };

    let registry = get_temp_file_registry();
    let mut guard = registry.lock();

    let entry = match guard.remove(&handle) {
        Some(e) => e,
        None => return false,
    };

    let (file, src_path, _delete_on_close) = entry;

    // Close the file handle first
    drop(file);

    // Move the file to the destination
    std::fs::rename(&src_path, &dest_path).is_ok()
}

// ============================================================================
// Shell Command Execution
// ============================================================================

/// Execute a shell command and return its exit code.
///
/// Runs the command via `sh -c "<cmd>"` and returns:
/// - The process exit code on success
/// - -1 on invalid input or execution failure
///
/// # Safety
/// The command string must be valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn system(cmd: BloodStr) -> i32 {
    use std::process::Command;

    if cmd.ptr.is_null() || cmd.len == 0 {
        return -1;
    }

    let cmd_slice = std::slice::from_raw_parts(cmd.ptr, cmd.len as usize);
    let cmd_str = match std::str::from_utf8(cmd_slice) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    match Command::new("sh").arg("-c").arg(cmd_str).status() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(_) => -1,
    }
}

// ============================================================================
// Environment Variable Functions
// ============================================================================

/// Get the value of an environment variable.
///
/// # Arguments
/// * `name` - The environment variable name as a BloodStr
///
/// # Returns
/// * BloodStr containing the variable's value, or empty BloodStr if unset
///
/// Named `env_get` (not `getenv`) to avoid colliding with libc's `getenv` symbol.
/// Follows the `args_get` naming pattern.
#[no_mangle]
pub unsafe extern "C" fn env_get(name: BloodStr) -> BloodStr {
    if name.ptr.is_null() || name.len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    let name_slice = std::slice::from_raw_parts(name.ptr, name.len as usize);
    let name_str = match std::str::from_utf8(name_slice) {
        Ok(s) => s,
        Err(_) => return BloodStr { ptr: std::ptr::null(), len: 0 },
    };

    match std::env::var(name_str) {
        Ok(value) => string_to_blood_str(value),
        Err(_) => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

// ============================================================================
// Command-Line Argument Functions
// ============================================================================

/// Global storage for command-line arguments.
/// Initialized once at program start via `blood_init_args`.
static ARGS: OnceLock<Vec<String>> = OnceLock::new();

/// Initialize command-line arguments.
///
/// This should be called once at program startup (typically from the entry point)
/// with argc and argv from main().
///
/// # Safety
/// argv must be a valid array of argc null-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn blood_init_args(argc: c_int, argv: *const *const c_char) {
    let mut args = Vec::with_capacity(argc as usize);
    for i in 0..argc as isize {
        let arg_ptr = *argv.offset(i);
        if !arg_ptr.is_null() {
            if let Ok(arg_str) = CStr::from_ptr(arg_ptr).to_str() {
                args.push(arg_str.to_string());
            }
        }
    }
    let _ = ARGS.set(args);
}

/// Get the number of command-line arguments.
///
/// # Returns
/// * Number of arguments (including program name)
/// * 0 if arguments haven't been initialized
#[no_mangle]
pub extern "C" fn args_count() -> i32 {
    ARGS.get().map(|args| args.len() as i32).unwrap_or(0)
}

/// Get a command-line argument by index.
///
/// # Arguments
/// * `index` - Zero-based index of the argument
///
/// # Returns
/// * BloodStr containing the argument on success
/// * Empty BloodStr (ptr=null, len=0) if index is out of bounds or args not initialized
#[no_mangle]
pub extern "C" fn args_get(index: i32) -> BloodStr {
    if index < 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    match ARGS.get() {
        Some(args) => {
            if let Some(arg) = args.get(index as usize) {
                BloodStr {
                    ptr: arg.as_ptr(),
                    len: arg.len() as u64,
                }
            } else {
                BloodStr { ptr: std::ptr::null(), len: 0 }
            }
        }
        None => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

/// Get all command-line arguments as a single string separated by spaces.
///
/// # Returns
/// * BloodStr containing all arguments separated by spaces
/// * Empty BloodStr if args not initialized
#[no_mangle]
pub extern "C" fn args_join() -> BloodStr {
    match ARGS.get() {
        Some(args) => {
            let joined = args.join(" ");
            string_to_blood_str(joined)
        }
        None => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

// ============================================================================
// Size Functions
// ============================================================================

/// Get the size of an i32 in bytes.
#[no_mangle]
pub extern "C" fn size_of_i32() -> i64 {
    std::mem::size_of::<i32>() as i64
}

/// Get the size of an i64 in bytes.
#[no_mangle]
pub extern "C" fn size_of_i64() -> i64 {
    std::mem::size_of::<i64>() as i64
}

/// Get the size of a bool in bytes.
#[no_mangle]
pub extern "C" fn size_of_bool() -> i64 {
    std::mem::size_of::<bool>() as i64
}

// ============================================================================
// Type Conversion Functions
// ============================================================================

/// Convert an i32 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn int_to_string(n: i32) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert an i64 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn i64_to_string(n: i64) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert a u64 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn u64_to_string(n: u64) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert a bool to a string ("true" or "false").
///
/// Returns a newly allocated BloodStr.
#[no_mangle]
pub extern "C" fn bool_to_string(b: bool) -> BloodStr {
    let s = if b { "true" } else { "false" };
    string_to_blood_str(s.to_string())
}

/// Convert a char to a string.
///
/// Returns a newly allocated BloodStr.
#[no_mangle]
pub extern "C" fn char_to_string(c: u32) -> BloodStr {
    if let Some(ch) = char::from_u32(c) {
        string_to_blood_str(ch.to_string())
    } else {
        // Invalid unicode codepoint - return replacement character
        string_to_blood_str('\u{FFFD}'.to_string())
    }
}

/// Convert an f32 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn f32_to_string(n: f32) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert an f64 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn f64_to_string(n: f64) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert an i32 to an i64.
#[no_mangle]
pub extern "C" fn i32_to_i64(val: i32) -> i64 {
    val as i64
}

/// Convert an i64 to an i32 (truncating).
#[no_mangle]
pub extern "C" fn i64_to_i32(val: i64) -> i32 {
    val as i32
}

/// Convert a u32 code point to a char (returned as u32).
///
/// Returns the Unicode replacement character U+FFFD for invalid code points.
#[no_mangle]
pub extern "C" fn char_from_u32(code: u32) -> u32 {
    char::from_u32(code).unwrap_or('\u{FFFD}') as u32
}

/// Parse a string as f64.
///
/// Returns 0.0 on parse failure.
///
/// # Safety
/// The string pointer must be valid for `s.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn parse_f64(s: BloodStr) -> f64 {
    if s.ptr.is_null() || s.len == 0 {
        return 0.0;
    }
    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    std::str::from_utf8(slice)
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Parse a string as i64 with the given radix.
///
/// Returns 0 on parse failure.
///
/// # Safety
/// The string pointer must be valid for `s.len` bytes.
#[no_mangle]
pub unsafe extern "C" fn parse_i64_radix(s: BloodStr, radix: u32) -> i64 {
    if s.ptr.is_null() || s.len == 0 {
        return 0;
    }
    let slice = std::slice::from_raw_parts(s.ptr, s.len as usize);
    std::str::from_utf8(slice)
        .ok()
        .and_then(|s| i64::from_str_radix(s, radix).ok())
        .unwrap_or(0)
}

/// Convert an i8 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn i8_to_string(n: i8) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert an i16 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn i16_to_string(n: i16) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert an i128 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn i128_to_string(n: i128) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert a u8 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn u8_to_string(n: u8) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert a u16 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn u16_to_string(n: u16) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert a u32 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn u32_to_string(n: u32) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Convert a u128 to a string.
///
/// Returns a newly allocated BloodStr containing the string representation.
#[no_mangle]
pub extern "C" fn u128_to_string(n: u128) -> BloodStr {
    let s = n.to_string();
    string_to_blood_str(s)
}

/// Helper function to convert a Rust String to a BloodStr.
///
/// Allocates memory for the string and returns a BloodStr pointing to it.
fn string_to_blood_str(s: String) -> BloodStr {
    let bytes = s.into_bytes();
    let len = bytes.len();

    if len == 0 {
        return BloodStr { ptr: std::ptr::null(), len: 0 };
    }

    // Allocate buffer for the string
    let layout = std::alloc::Layout::from_size_align(len, 1).unwrap();
    let ptr = unsafe { runtime_alloc(layout) };
    if ptr.is_null() {
        eprintln!("blood: out of memory in string_to_blood_str");
        std::process::exit(1);
    }

    // Copy the string bytes into the allocated buffer
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
    }

    BloodStr {
        ptr: ptr as *const u8,
        len: len as u64,
    }
}

// ============================================================================
// Memory Management (Generational References)
// ============================================================================

/// Allocate memory with generational reference.
///
/// Returns a 128-bit Blood pointer (as two 64-bit values via out parameters).
/// Returns 0 on success, non-zero on failure.
///
/// The allocation is automatically registered in the global slot registry,
/// enabling generation validation for stale reference detection.
///
/// # Layout of out_gen_meta
/// - Bits 0-31: Metadata (tier, flags, type fingerprint)
/// - Bits 32-63: Generation
///
/// # Safety
///
/// `out_addr` and `out_gen_meta` must be valid pointers to writable u64 locations.
#[no_mangle]
pub unsafe extern "C" fn blood_alloc(
    size: usize,
    out_addr: *mut u64,
    out_gen_meta: *mut u64,
) -> c_int {
    if out_addr.is_null() || out_gen_meta.is_null() {
        return -1;
    }

    // Use 16-byte alignment for BloodPtr compatibility
    let align = 16.max(std::mem::align_of::<usize>());
    let layout = match std::alloc::Layout::from_size_align(size, align) {
        Ok(l) => l,
        Err(_) => return -2,
    };

    let ptr = std::alloc::alloc(layout);
    if ptr.is_null() {
        return -3;
    }

    let address = ptr as u64;

    // Register the allocation in the global slot registry
    // This enables generation validation on dereference
    let gen = register_allocation(address, size);

    // Create a BloodPtr with the assigned generation (region allocation)
    let blood_ptr = BloodPtr::new(
        ptr as usize,
        gen,
        PointerMetadata::REGION,
    );

    *out_addr = blood_ptr.address() as u64;
    *out_gen_meta = ((blood_ptr.generation() as u64) << 32) | (blood_ptr.metadata().bits() as u64);

    0
}

/// Allocate memory with generational reference, aborting on failure.
///
/// This is a simpler version of `blood_alloc` that aborts on failure instead of
/// returning an error code. This allows the compiler to call it without needing
/// conditional branches for error handling.
///
/// Returns the address directly. The generation is written to `out_generation`.
///
/// # Panics
///
/// Panics if allocation fails (out of memory, invalid size, etc.).
///
/// # Safety
///
/// `out_generation` must be a valid pointer to a writable u32 location.
#[no_mangle]
pub unsafe extern "C" fn blood_alloc_or_abort(
    size: usize,
    out_generation: *mut u32,
) -> u64 {
    if out_generation.is_null() {
        blood_panic(c"blood_alloc_or_abort: null out_generation pointer".as_ptr());
    }

    // Check if a region is active — if so, allocate from it.
    let region_id = current_active_region();
    if region_id != 0 {
        let align = 16.max(std::mem::align_of::<usize>());
        let addr = blood_region_alloc(region_id, size, align);
        if addr == 0 {
            blood_panic(c"blood_alloc_or_abort: region allocation failed (out of region memory)".as_ptr());
        }
        // Region allocations use generation 0 — their lifetime is scoped to the
        // region, so generational tracking is unnecessary.
        *out_generation = 0;
        return addr;
    }

    // No active region — use global allocator with generation tracking.
    let align = 16.max(std::mem::align_of::<usize>());
    let layout = match std::alloc::Layout::from_size_align(size, align) {
        Ok(l) => l,
        Err(_) => {
            blood_panic(c"blood_alloc_or_abort: invalid layout".as_ptr());
        }
    };

    let ptr = std::alloc::alloc(layout);
    if ptr.is_null() {
        blood_panic(c"blood_alloc_or_abort: out of memory".as_ptr());
    }

    let address = ptr as u64;

    // Register the allocation in the global slot registry
    // This enables generation validation on dereference
    let gen = register_allocation(address, size);

    *out_generation = gen;
    address
}

/// Free memory allocated with blood_alloc.
///
/// This function unregisters the allocation from the slot registry (which
/// increments the generation) before deallocating the memory. Any subsequent
/// dereference of a pointer with the old generation will fail validation.
///
/// # Safety
/// The address must have been allocated with blood_alloc.
#[no_mangle]
pub unsafe extern "C" fn blood_free(addr: u64, size: usize) {
    if addr == 0 {
        return;
    }

    // Unregister from slot registry BEFORE deallocation
    // This increments the generation, invalidating any existing references
    let _ = unregister_allocation(addr);

    // Use matching alignment from blood_alloc
    let align = 16.max(std::mem::align_of::<usize>());
    let layout = match std::alloc::Layout::from_size_align(size, align) {
        Ok(l) => l,
        Err(_) => return,
    };

    std::alloc::dealloc(addr as *mut u8, layout);
}

/// Check if a generational reference is valid.
///
/// Returns 1 if valid, 0 if stale.
#[no_mangle]
pub extern "C" fn blood_check_generation(
    expected_gen: u32,
    slot_gen: u32,
) -> c_int {
    // Persistent objects always valid
    if expected_gen == generation::PERSISTENT {
        return 1;
    }

    if expected_gen == slot_gen {
        1
    } else {
        0
    }
}

/// Get current generation from a memory slot.
///
/// Returns the current generation for the given address, or `FIRST` (1) if
/// the address is not tracked in the slot registry.
///
/// # Safety
/// The address should point to memory allocated via `blood_alloc`.
#[no_mangle]
pub unsafe extern "C" fn blood_get_generation(addr: u64) -> u32 {
    if addr == 0 {
        return 0;
    }

    // Look up the actual generation from the slot registry
    get_slot_generation(addr).unwrap_or(generation::FIRST)
}

// ============================================================================
// Effect Runtime Support
// ============================================================================

/// Evidence vector for effect handlers.
///
/// Opaque handle to an evidence vector.
pub type EvidenceHandle = *mut c_void;

/// An entry in the evidence vector.
///
/// Each entry holds the registry index and the instance-specific state pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct EvidenceEntry {
    /// Index into the global handler registry.
    registry_index: u64,
    /// Instance-specific state pointer (set at handler instantiation).
    state: *mut c_void,
}

/// Create a new evidence vector.
///
/// If there is a current evidence vector, this clones it so that nested handlers
/// inherit outer handlers. Otherwise creates an empty vector.
#[no_mangle]
pub extern "C" fn blood_evidence_create() -> EvidenceHandle {
    // Check if there's a current evidence vector to inherit from
    let current = blood_evidence_current();
    let ev = if current.is_null() {
        Box::new(Vec::<EvidenceEntry>::new())
    } else {
        // Clone the current evidence vector so nested handlers inherit outer handlers
        let current_vec = unsafe { &*(current as *const Vec<EvidenceEntry>) };
        Box::new(current_vec.clone())
    };
    Box::into_raw(ev) as EvidenceHandle
}

/// Destroy an evidence vector.
///
/// # Safety
/// The handle must have been created with blood_evidence_create.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_destroy(ev: EvidenceHandle) {
    if !ev.is_null() {
        let _ = Box::from_raw(ev as *mut Vec<EvidenceEntry>);
    }
}

/// Push a handler onto the evidence vector (with null state).
///
/// # Safety
/// The handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_push(ev: EvidenceHandle, registry_index: u64) {
    if !ev.is_null() {
        let vec = &mut *(ev as *mut Vec<EvidenceEntry>);
        vec.push(EvidenceEntry {
            registry_index,
            state: std::ptr::null_mut(),
        });
    }
}

/// Push a handler by effect ID onto the evidence vector (with null state).
///
/// This looks up the handler for the given effect_id in the global registry
/// and pushes its index onto the evidence vector. For handlers that need state,
/// use blood_evidence_push_with_state instead.
///
/// # Safety
/// The handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_push_by_effect(ev: EvidenceHandle, effect_id: i64) {
    blood_evidence_push_with_state(ev, effect_id, std::ptr::null_mut());
}

/// Push a handler by effect ID onto the evidence vector with instance state.
///
/// This looks up the handler for the given effect_id in the global registry
/// and pushes its index along with the instance-specific state pointer.
///
/// # Safety
/// The handle and state pointer must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_push_with_state(
    ev: EvidenceHandle,
    effect_id: i64,
    state: *mut c_void,
) {
    if ev.is_null() {
        return;
    }

    // Find the handler for this effect in the registry
    let registry = get_effect_registry();
    let reg = registry.lock();

    for (index, entry) in reg.iter().enumerate() {
        if entry.effect_id == effect_id {
            // Found the handler - push entry with state
            let vec = &mut *(ev as *mut Vec<EvidenceEntry>);
            vec.push(EvidenceEntry {
                registry_index: index as u64,
                state,
            });
            return;
        }
    }

    // Handler not found - this shouldn't happen if effect is properly registered
    eprintln!(
        "BLOOD RUNTIME WARNING: No handler found for effect_id={} during push",
        effect_id
    );
}

/// Pop a handler from the evidence vector.
///
/// # Safety
/// The handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_pop(ev: EvidenceHandle) -> u64 {
    if ev.is_null() {
        return 0;
    }
    let vec = &mut *(ev as *mut Vec<EvidenceEntry>);
    vec.pop().map(|e| e.registry_index).unwrap_or(0)
}

/// Get handler registry index at evidence vector index.
///
/// # Safety
/// The handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_get(ev: EvidenceHandle, index: usize) -> u64 {
    if ev.is_null() {
        return 0;
    }
    let vec = &*(ev as *const Vec<EvidenceEntry>);
    vec.get(index).map(|e| e.registry_index).unwrap_or(0)
}

// ============================================================================
// Effect Handler Registration and Dispatch
// ============================================================================

/// Effect handler entry in the registry.
/// Contains effect_id, operation function pointers, and state pointer.
#[repr(C)]
struct EffectHandlerEntry {
    effect_id: i64,
    operations: Vec<*const c_void>,  // Function pointers for each operation
    state: *mut c_void,               // Handler state (for State<T> effect)
}

// Safety: The raw pointers are only accessed through the mutex,
// and we ensure proper synchronization in all FFI functions.
unsafe impl Send for EffectHandlerEntry {}
unsafe impl Sync for EffectHandlerEntry {}

/// Global effect handler registry.
/// Maps effect IDs to their handler entries.
static EFFECT_REGISTRY: OnceLock<Mutex<Vec<EffectHandlerEntry>>> = OnceLock::new();

// Thread-local current evidence vector for effect dispatch.
thread_local! {
    static CURRENT_EVIDENCE: std::cell::RefCell<Option<EvidenceHandle>> = const { std::cell::RefCell::new(None) };
}

/// Inline evidence entry for single-handler scopes (EFF-OPT-003).
///
/// When exactly one handler is in scope, we store its effect_id in a thread-local
/// fast-path slot. The `blood_perform` function checks this slot first: if the
/// effect_id matches, it knows the most-recently-pushed evidence entry is the
/// correct handler, enabling O(1) dispatch instead of reverse-linear search.
#[derive(Clone, Copy, Debug)]
struct InlineEvidenceSlot {
    /// Effect ID for matching.
    effect_id: u64,
}

// Thread-local inline evidence slot for single-handler optimization.
thread_local! {
    static INLINE_EVIDENCE: std::cell::RefCell<Option<InlineEvidenceSlot>> = const { std::cell::RefCell::new(None) };
}

/// Get or initialize the effect registry.
fn get_effect_registry() -> &'static Mutex<Vec<EffectHandlerEntry>> {
    EFFECT_REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register an effect handler with its operations.
///
/// # Arguments
/// * `ev` - Evidence vector handle
/// * `effect_id` - Unique identifier for the effect type
/// * `ops` - Pointer to array of operation function pointers
/// * `op_count` - Number of operations
///
/// # Safety
/// The ops pointer must point to a valid array of function pointers.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_register(
    ev: EvidenceHandle,
    effect_id: i64,
    ops: *const *const c_void,
    op_count: i64,
) {
    // ops must be valid
    if ops.is_null() {
        return;
    }

    // Collect operation function pointers
    let mut operations = Vec::with_capacity(op_count as usize);
    for i in 0..op_count {
        let op_ptr = *ops.add(i as usize);
        operations.push(op_ptr);
    }

    // Create handler entry
    let entry = EffectHandlerEntry {
        effect_id,
        operations,
        state: std::ptr::null_mut(),
    };

    // Add to registry
    let registry = get_effect_registry();
    let mut reg = registry.lock();
    reg.push(entry);

    // Push handler index onto evidence vector (if evidence is provided)
    // If ev is null, this is a global registration at program startup
    if !ev.is_null() {
        let handler_index = (reg.len() - 1) as u64;
        blood_evidence_push(ev, handler_index);
    }
}

/// Set state for the topmost handler in the evidence vector.
///
/// # Safety
/// The evidence handle and state pointer must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_set_state(ev: EvidenceHandle, state: *mut c_void) {
    if ev.is_null() {
        return;
    }

    // Set state on the topmost evidence entry directly
    let vec = &mut *(ev as *mut Vec<EvidenceEntry>);
    if let Some(entry) = vec.last_mut() {
        entry.state = state;
    }
}

/// Get state for a handler at given index in the evidence vector.
///
/// # Safety
/// The evidence handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_evidence_get_state(ev: EvidenceHandle, index: i64) -> *mut c_void {
    if ev.is_null() {
        return std::ptr::null_mut();
    }

    let vec = &*(ev as *const Vec<EvidenceEntry>);
    if let Some(entry) = vec.get(index as usize) {
        return entry.state;
    }
    std::ptr::null_mut()
}

/// Get the current thread's evidence vector.
///
/// Returns the evidence vector set by the current effect handler scope,
/// or null if no handler is active.
#[no_mangle]
pub extern "C" fn blood_evidence_current() -> EvidenceHandle {
    CURRENT_EVIDENCE.with(|ev| {
        ev.borrow().unwrap_or(std::ptr::null_mut())
    })
}

/// Set the current thread's evidence vector.
///
/// Called internally when entering a handler scope.
#[no_mangle]
pub extern "C" fn blood_evidence_set_current(ev: EvidenceHandle) {
    CURRENT_EVIDENCE.with(|current| {
        *current.borrow_mut() = Some(ev);
    });
}

/// Set an inline evidence hint for single-handler scopes (EFF-OPT-003).
///
/// When a scope has exactly one handler, this stores the effect_id in a
/// thread-local fast-path slot. The `blood_perform` function checks this
/// slot first: if the effect_id matches, it uses the last entry in the
/// evidence vector directly (O(1)) instead of searching (O(n)).
///
/// # Arguments
/// * `effect_id` - The effect type ID to set as the inline hint
#[no_mangle]
pub extern "C" fn blood_evidence_set_inline(effect_id: u64) {
    INLINE_EVIDENCE.with(|slot| {
        *slot.borrow_mut() = Some(InlineEvidenceSlot { effect_id });
    });
}

/// Clear the inline evidence slot.
///
/// Called when leaving a single-handler scope.
#[no_mangle]
pub extern "C" fn blood_evidence_clear_inline() {
    INLINE_EVIDENCE.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// Perform an effect operation.
///
/// Dispatches to the appropriate handler based on the effect_id and op_index.
/// This is the core runtime dispatch mechanism for algebraic effects.
///
/// # Arguments
/// * `effect_id` - The effect type being performed
/// * `op_index` - The operation index within the effect
/// * `args` - Pointer to argument array (as i64 values)
/// * `arg_count` - Number of arguments
/// * `continuation` - Continuation handle for non-tail-resumptive handlers (0 for tail-resumptive)
///
/// # Returns
/// The result of the operation as an i64, or 0 if dispatch fails.
///
/// # Safety
/// The args pointer must point to a valid array of i64 values.
#[no_mangle]
pub unsafe extern "C" fn blood_perform(
    effect_id: i64,
    op_index: i32,
    args: *const i64,
    arg_count: i64,
    continuation: i64,
) -> i64 {
    // EFF-OPT-003: Check inline evidence slot first (fast path for single-handler scopes).
    // When the inline slot matches the effect_id, we know the last entry in the evidence
    // vector is the correct handler. This avoids reverse-linear search through the vector.
    let has_inline_hint = INLINE_EVIDENCE.with(|slot| {
        slot.borrow()
            .map(|entry| entry.effect_id == effect_id as u64)
            .unwrap_or(false)
    });

    if has_inline_hint {
        // Fast path: use the last entry in the evidence vector directly
        let ev = blood_evidence_current();
        if !ev.is_null() {
            let vec = &mut *(ev as *mut Vec<EvidenceEntry>);
            if let Some(ev_entry) = vec.last() {
                let registry = get_effect_registry();
                let reg = registry.lock();
                if let Some(registry_entry) = reg.get(ev_entry.registry_index as usize) {
                    if registry_entry.effect_id == effect_id {
                        if let Some(&op_fn) = registry_entry.operations.get(op_index as usize) {
                            if !op_fn.is_null() {
                                let instance_state = ev_entry.state;
                                drop(reg);

                                // Temporarily remove handler entry to implement delimited
                                // continuation semantics — prevents the handler from catching
                                // effects it performs itself (e.g., forwarding to outer handler)
                                let idx = vec.len() - 1;
                                let removed_entry = vec.remove(idx);

                                // Use extern "C" ABI matching compiled handler signatures
                                type OpHandler = unsafe extern "C" fn(*mut c_void, *const i64, i64, i64) -> i64;
                                let handler: OpHandler = std::mem::transmute(op_fn);
                                let result = handler(instance_state, args, arg_count, continuation);

                                // Restore the handler entry
                                vec.insert(idx, removed_entry);

                                return result;
                            }
                        }
                    }
                }
            }
        }
        // Inline hint didn't resolve - fall through to normal path
    }

    // Get current evidence vector
    let ev = blood_evidence_current();
    if ev.is_null() {
        // No handler installed - this is an unhandled effect error
        eprintln!(
            "BLOOD RUNTIME ERROR: Unhandled effect! effect_id={}, op_index={}",
            effect_id, op_index
        );
        return 0;
    }

    // Find handler for this effect in evidence vector
    let vec = &mut *(ev as *mut Vec<EvidenceEntry>);

    let registry = get_effect_registry();
    let reg = registry.lock();

    // Search from most recent to oldest handler (reverse order)
    // We need to find the index and then temporarily remove it to prevent
    // the handler from catching effects it performs itself (delimited continuation semantics)
    let mut found_idx = None;
    let mut handler_info = None;

    for (i, ev_entry) in vec.iter().enumerate().rev() {
        let handler_index = ev_entry.registry_index;
        if let Some(registry_entry) = reg.get(handler_index as usize) {
            if registry_entry.effect_id == effect_id {
                // Found the handler for this effect
                if let Some(&op_fn) = registry_entry.operations.get(op_index as usize) {
                    if !op_fn.is_null() {
                        found_idx = Some(i);
                        handler_info = Some((ev_entry.state, op_fn));
                        break;
                    }
                }
            }
        }
    }

    // Drop the registry lock before calling handler (handler may need it)
    drop(reg);

    if let (Some(idx), Some((instance_state, op_fn))) = (found_idx, handler_info) {
        // Remove the handler entry temporarily to implement delimited continuation semantics
        // This prevents the handler from catching effects it performs itself
        let removed_entry = vec.remove(idx);

        // Call the operation handler
        // The handler signature is: fn(state: *void, args: *i64, arg_count: i64, continuation: i64) -> i64
        type OpHandler = unsafe extern "C" fn(*mut c_void, *const i64, i64, i64) -> i64;
        let handler: OpHandler = std::mem::transmute(op_fn);
        let result = handler(instance_state, args, arg_count, continuation);

        // Restore the handler entry after the handler returns
        // Insert at the same position to maintain the correct order
        vec.insert(idx, removed_entry);

        return result;
    }

    // No handler found
    eprintln!(
        "BLOOD RUNTIME ERROR: No handler found for effect_id={}, op_index={}",
        effect_id, op_index
    );
    0
}

/// Get the handler depth for a given effect.
///
/// Returns the number of handlers for this effect currently in scope.
/// Useful for effects that need to know their nesting level.
#[no_mangle]
pub extern "C" fn blood_handler_depth(effect_id: i64) -> i64 {
    let ev = blood_evidence_current();
    if ev.is_null() {
        return 0;
    }

    unsafe {
        let vec = &*(ev as *const Vec<EvidenceEntry>);
        let registry = get_effect_registry();
        let reg = registry.lock();

        let mut depth = 0i64;
        for ev_entry in vec.iter() {
            if let Some(registry_entry) = reg.get(ev_entry.registry_index as usize) {
                if registry_entry.effect_id == effect_id {
                    depth += 1;
                }
            }
        }
        depth
    }
}

// ============================================================================
// Multiple Dispatch Runtime
// ============================================================================

/// Dispatch table entry for multiple dispatch.
#[derive(Clone)]
struct DispatchEntry {
    method_slot: i64,
    type_tag: i64,
    impl_ptr: *const c_void,
}

// Safety: The impl_ptr is only used for function calls and we ensure
// thread-safe access through the mutex.
unsafe impl Send for DispatchEntry {}
unsafe impl Sync for DispatchEntry {}

/// Global dispatch table for multiple dispatch.
static DISPATCH_TABLE: OnceLock<Mutex<Vec<DispatchEntry>>> = OnceLock::new();

/// Get or initialize the dispatch table.
fn get_dispatch_table() -> &'static Mutex<Vec<DispatchEntry>> {
    DISPATCH_TABLE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Look up an implementation in the dispatch table.
///
/// # Arguments
/// * `method_slot` - The method identifier
/// * `type_tag` - The runtime type tag of the receiver
///
/// # Returns
/// Function pointer to the implementation, or null if not found.
#[no_mangle]
pub extern "C" fn blood_dispatch_lookup(method_slot: i64, type_tag: i64) -> *const c_void {
    let table = get_dispatch_table();
    let entries = table.lock();

    // Linear search for now - can optimize with hash table later
    for entry in entries.iter() {
        if entry.method_slot == method_slot && entry.type_tag == type_tag {
            return entry.impl_ptr;
        }
    }

    std::ptr::null()
}

/// Register an implementation in the dispatch table.
///
/// # Arguments
/// * `method_slot` - The method identifier
/// * `type_tag` - The runtime type tag this implementation handles
/// * `impl_ptr` - Function pointer to the implementation
#[no_mangle]
pub extern "C" fn blood_dispatch_register(
    method_slot: i64,
    type_tag: i64,
    impl_ptr: *const c_void,
) {
    let table = get_dispatch_table();
    let mut entries = table.lock();

    // Check if entry already exists
    for entry in entries.iter_mut() {
        if entry.method_slot == method_slot && entry.type_tag == type_tag {
            // Update existing entry
            entry.impl_ptr = impl_ptr;
            return;
        }
    }

    // Add new entry
    entries.push(DispatchEntry {
        method_slot,
        type_tag,
        impl_ptr,
    });
}

/// Get the runtime type tag from an object.
///
/// Blood objects have a header containing their type tag.
/// This function extracts it for dispatch purposes.
///
/// # Safety
/// The object pointer must point to a valid Blood object with a header.
#[no_mangle]
pub unsafe extern "C" fn blood_get_type_tag(obj: *const c_void) -> i64 {
    if obj.is_null() {
        return 0;
    }

    // Blood object layout: [type_tag: i64][...fields...]
    // The type tag is stored as the first 8 bytes of the object.
    let tag_ptr = obj as *const i64;
    *tag_ptr
}

// ============================================================================
// Generation Snapshot Support (for effect handler continuations)
// ============================================================================

/// A snapshot entry for generation validation.
#[repr(C)]
#[derive(Clone, Copy)]
struct SnapshotEntry {
    address: u64,
    generation: u32,
}

/// A generation snapshot for validating captured references.
///
/// ## Snapshot Sharing for Nested Handlers
///
/// When effect handlers are nested, each handler can create a child snapshot
/// that references its parent. This reduces memory from O(n²) to O(n) for
/// deeply nested handlers:
///
/// ```text
/// Snapshot_C → Snapshot_B → Snapshot_A → NULL
///    │              │             │
///    └── delta_C    └── delta_B   └── full_snapshot
/// ```
///
/// Each child snapshot only stores entries added in its scope; validation
/// walks the entire chain to verify all references.
struct GenerationSnapshot {
    /// Entries for this snapshot scope only.
    entries: Vec<SnapshotEntry>,
    /// Parent snapshot for nested handlers (not owned - managed separately).
    parent: SnapshotHandle,
}

/// Opaque handle to a generation snapshot.
pub type SnapshotHandle = *mut c_void;

/// Create a new generation snapshot (root, no parent).
///
/// Returns a handle to an empty snapshot, or null on failure.
#[no_mangle]
pub extern "C" fn blood_snapshot_create() -> SnapshotHandle {
    let snapshot = Box::new(GenerationSnapshot {
        entries: Vec::new(),
        parent: std::ptr::null_mut(),
    });
    Box::into_raw(snapshot) as SnapshotHandle
}

/// Create a child snapshot that references a parent snapshot.
///
/// This is used for nested effect handlers to share snapshot data.
/// The child only needs to store entries for references captured in its
/// scope; validation will walk the parent chain automatically.
///
/// # Arguments
/// * `parent` - Handle to the parent snapshot (can be null for root)
///
/// # Returns
/// Handle to a new child snapshot, or null on failure.
///
/// # Note
/// The child does NOT own the parent. The parent's lifetime must be
/// managed separately (typically by the enclosing handler scope).
#[no_mangle]
pub extern "C" fn blood_snapshot_create_child(parent: SnapshotHandle) -> SnapshotHandle {
    let snapshot = Box::new(GenerationSnapshot {
        entries: Vec::new(),
        parent,
    });
    Box::into_raw(snapshot) as SnapshotHandle
}

/// Get the parent snapshot of a snapshot.
///
/// # Safety
/// The snapshot handle must be valid.
///
/// # Returns
/// The parent snapshot handle, or null if this is a root snapshot.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_get_parent(snapshot: SnapshotHandle) -> SnapshotHandle {
    if snapshot.is_null() {
        return std::ptr::null_mut();
    }
    let snap = &*(snapshot as *const GenerationSnapshot);
    snap.parent
}

/// Set the parent snapshot of a snapshot.
///
/// This allows linking snapshots after creation if needed.
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_set_parent(snapshot: SnapshotHandle, parent: SnapshotHandle) {
    if snapshot.is_null() {
        return;
    }
    let snap = &mut *(snapshot as *mut GenerationSnapshot);
    snap.parent = parent;
}

/// Check if a snapshot has a parent (is a child snapshot).
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_has_parent(snapshot: SnapshotHandle) -> bool {
    if snapshot.is_null() {
        return false;
    }
    let snap = &*(snapshot as *const GenerationSnapshot);
    !snap.parent.is_null()
}

/// Get the depth of the snapshot chain (1 for root, 2 for child of root, etc.).
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_chain_depth(snapshot: SnapshotHandle) -> usize {
    let mut depth = 0;
    let mut current = snapshot;

    while !current.is_null() {
        depth += 1;
        let snap = &*(current as *const GenerationSnapshot);
        current = snap.parent;
    }

    depth
}

/// Add an entry to a generation snapshot.
///
/// # Arguments
/// * `snapshot` - Handle to the snapshot
/// * `address` - Memory address of the generational reference
/// * `generation` - Expected generation value
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_add_entry(
    snapshot: SnapshotHandle,
    address: u64,
    generation: u32,
) {
    if snapshot.is_null() {
        return;
    }

    // Skip persistent pointers (generation = 0xFFFFFFFF)
    if generation == 0xFFFFFFFF {
        return;
    }

    let snap = &mut *(snapshot as *mut GenerationSnapshot);
    snap.entries.push(SnapshotEntry { address, generation });
}

/// Validate a generation snapshot against current memory state.
///
/// Returns 0 if all generations match (valid), or a positive value
/// indicating a stale reference was found. For snapshots without parents,
/// the return value is the 1-based index of the stale entry.
///
/// For snapshots with parents (nested handlers), validation walks the
/// entire chain from child to root. A stale entry in a parent will
/// also cause validation to fail.
///
/// This function uses the global slot registry to look up the current
/// generation for each address and compare it against the expected
/// generation captured in the snapshot. If any generation mismatches
/// (indicating the memory was freed and potentially reallocated), the
/// reference is considered stale.
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_validate(snapshot: SnapshotHandle) -> i64 {
    if snapshot.is_null() {
        return 0; // Empty/null snapshot is valid
    }

    // Walk the entire chain from this snapshot to root
    let mut current = snapshot;
    let mut chain_offset: i64 = 0;

    while !current.is_null() {
        let snap = &*(current as *const GenerationSnapshot);

        for (i, entry) in snap.entries.iter().enumerate() {
            // Skip persistent references (generation = 0xFFFFFFFF)
            if entry.generation == generation::PERSISTENT {
                continue;
            }

            // Look up the current generation from the slot registry
            match get_slot_generation(entry.address) {
                Some(actual_gen) => {
                    if actual_gen != entry.generation {
                        // Generation mismatch - reference is stale
                        // Return 1-based index accounting for chain position
                        return chain_offset + (i + 1) as i64;
                    }
                }
                None => {
                    // Address not found in registry - could be:
                    // 1. Memory was freed and slot entry was removed
                    // 2. Address was never registered (stack/static memory)
                    //
                    // For now, treat unregistered addresses as potentially valid
                    // (they might be stack or static memory which isn't tracked).
                    // A stricter implementation could require all genrefs be registered.
                    //
                    // However, if the captured generation is not FIRST (1), the address
                    // was likely heap memory that has been freed and its entry cleaned up.
                    if entry.generation > generation::FIRST {
                        return chain_offset + (i + 1) as i64;
                    }
                }
            }
        }

        // Move to parent and track offset for error reporting
        chain_offset += snap.entries.len() as i64;
        current = snap.parent;
    }

    0 // All valid
}

/// Validate only this snapshot's entries, not walking the parent chain.
///
/// This is useful when you know the parent has already been validated
/// and want to avoid redundant work.
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_validate_local(snapshot: SnapshotHandle) -> i64 {
    if snapshot.is_null() {
        return 0; // Empty/null snapshot is valid
    }

    let snap = &*(snapshot as *const GenerationSnapshot);

    // Empty snapshot is valid
    if snap.entries.is_empty() {
        return 0;
    }

    for (i, entry) in snap.entries.iter().enumerate() {
        // Skip persistent references (generation = 0xFFFFFFFF)
        if entry.generation == generation::PERSISTENT {
            continue;
        }

        // Look up the current generation from the slot registry
        match get_slot_generation(entry.address) {
            Some(actual_gen) => {
                if actual_gen != entry.generation {
                    // Generation mismatch - reference is stale
                    // Return 1-based index of the stale entry
                    return (i + 1) as i64;
                }
            }
            None => {
                // If the captured generation is not FIRST (1), the address
                // was likely heap memory that has been freed
                if entry.generation > generation::FIRST {
                    return (i + 1) as i64;
                }
            }
        }
    }

    0 // All valid
}

/// Get the number of entries in this snapshot only (not including parents).
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_len(snapshot: SnapshotHandle) -> usize {
    if snapshot.is_null() {
        return 0;
    }
    let snap = &*(snapshot as *const GenerationSnapshot);
    snap.entries.len()
}

/// Get the total number of entries in the snapshot chain (including all parents).
///
/// This walks the parent chain and sums the entry counts.
///
/// # Safety
/// The snapshot handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_total_len(snapshot: SnapshotHandle) -> usize {
    let mut total = 0;
    let mut current = snapshot;

    while !current.is_null() {
        let snap = &*(current as *const GenerationSnapshot);
        total += snap.entries.len();
        current = snap.parent;
    }

    total
}

/// Destroy a generation snapshot.
///
/// # Important
/// This only destroys this snapshot, NOT its parent. Parent snapshots must
/// be destroyed separately (typically when their handler scope exits).
///
/// # Safety
/// The snapshot handle must have been created with blood_snapshot_create
/// or blood_snapshot_create_child.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_destroy(snapshot: SnapshotHandle) {
    if !snapshot.is_null() {
        let _ = Box::from_raw(snapshot as *mut GenerationSnapshot);
    }
}

/// Get the stale entry details from a snapshot after validation failure.
///
/// This function retrieves information about which entry failed validation,
/// useful for generating detailed error messages.
///
/// # Arguments
/// * `snapshot` - Handle to the snapshot
/// * `index` - 1-based index returned by blood_snapshot_validate (must be > 0)
/// * `out_address` - Output pointer for the stale address
/// * `out_expected_gen` - Output pointer for the expected generation
///
/// # Returns
/// 0 on success, -1 on invalid arguments.
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_get_stale_entry(
    snapshot: SnapshotHandle,
    index: i64,
    out_address: *mut u64,
    out_expected_gen: *mut u32,
) -> c_int {
    if snapshot.is_null() || index <= 0 || out_address.is_null() || out_expected_gen.is_null() {
        return -1;
    }

    let snap = &*(snapshot as *const GenerationSnapshot);
    let idx = (index - 1) as usize;

    if idx >= snap.entries.len() {
        return -1;
    }

    let entry = &snap.entries[idx];
    *out_address = entry.address;
    *out_expected_gen = entry.generation;

    0
}

// ============================================================================
// Slot Registry FFI (for allocation tracking)
// ============================================================================

use crate::memory::{persistent_alloc, persistent_increment, persistent_decrement, persistent_is_alive};

/// Register a new allocation in the slot registry.
///
/// This should be called by the runtime allocator when memory is allocated.
/// Returns the generation assigned to this allocation.
///
/// # Arguments
/// * `address` - The allocated memory address
/// * `size` - Size of the allocation in bytes
///
/// # Returns
/// The generation number assigned to this allocation.
#[no_mangle]
pub extern "C" fn blood_register_allocation(address: u64, size: u64) -> u32 {
    register_allocation(address, size as usize)
}

/// Get the number of entries in the slot registry (lock-free).
///
/// This is useful for debugging and memory analysis to understand
/// how many allocations are being tracked. Uses an atomic counter
/// so it doesn't contend with allocation/deallocation operations.
///
/// # Returns
/// The number of unique addresses ever registered in the slot registry.
#[no_mangle]
pub extern "C" fn blood_slot_registry_len() -> u64 {
    crate::memory::slot_registry().len() as u64
}

/// Get the current live bytes allocated via system allocator (non-region).
///
/// This is useful for debugging and memory analysis. Returns the difference
/// between total allocated and total freed bytes.
#[no_mangle]
pub extern "C" fn blood_system_alloc_live_bytes() -> u64 {
    system_alloc_live_bytes()
}

/// Get total bytes ever allocated via system allocator (non-region).
#[no_mangle]
pub extern "C" fn blood_system_alloc_total_bytes() -> u64 {
    system_alloc_stats().0
}

/// Get total bytes ever freed via system allocator (non-region).
#[no_mangle]
pub extern "C" fn blood_system_free_total_bytes() -> u64 {
    system_alloc_stats().1
}

/// Get total number of system allocations (non-region).
#[no_mangle]
pub extern "C" fn blood_system_alloc_count() -> u64 {
    system_alloc_stats().2
}

/// Get total number of system frees (non-region).
#[no_mangle]
pub extern "C" fn blood_system_free_count() -> u64 {
    system_alloc_stats().3
}

/// Unregister an allocation from the slot registry and add to region free list.
///
/// This should be called by the runtime allocator when memory is freed.
/// The slot is marked as deallocated but retained for stale reference detection.
/// If the allocation belongs to a region, it is added to the region's free list
/// for potential reuse.
///
/// # Lock Ordering
///
/// This function acquires locks in a consistent order to prevent deadlocks:
/// 1. Slot registry (via `unregister_allocation`) - released before step 2
/// 2. Region registry (if region_id != 0)
/// 3. Region's internal free list mutex (via `region.deallocate`)
///
/// # Arguments
/// * `address` - The address being freed
#[no_mangle]
pub extern "C" fn blood_unregister_allocation(address: u64) {
    // Step 1: Unregister from slot registry and get region info.
    // The slot registry lock is released when unregister_allocation returns.
    let info = unregister_allocation(address);

    // Step 2: If it belongs to a region, add to the region's free list.
    if let Some((region_id, size_class)) = info {
        if region_id != 0 {
            let registry = get_region_registry();
            let reg = registry.lock();
            if let Some(region) = reg.get(&region_id) {
                // Step 3: Region's internal mutex is acquired here.
                let _ = region.deallocate(address, size_class);
            }
        }
    }
}

/// Deallocate memory from a specific region, adding it to the free list.
///
/// This is the explicit region deallocation function that enables memory reuse.
/// The slot's generation is incremented (invalidating old references) and the
/// address is added to the region's free list for the appropriate size class.
///
/// # Lock Ordering
///
/// Locks are acquired in consistent order to prevent deadlocks:
/// 1. Slot registry (via `unregister_allocation`) - released before step 2
/// 2. Region registry
/// 3. Region's internal free list mutex
///
/// # Arguments
/// * `region_id` - The region ID
/// * `address` - The address to deallocate
///
/// # Returns
/// 1 if successfully added to free list, 0 otherwise.
#[no_mangle]
pub extern "C" fn blood_region_dealloc(region_id: u64, address: u64) -> c_int {
    // Step 1: Unregister from slot registry (increments generation).
    // Lock is released when this returns.
    let size_class = if let Some((_, sc)) = unregister_allocation(address) {
        sc
    } else {
        return 0;
    };

    // Step 2 & 3: Acquire region registry, then region's free list.
    let registry = get_region_registry();
    let reg = registry.lock();
    if let Some(region) = reg.get(&region_id) {
        if region.deallocate(address, size_class) {
            1
        } else {
            0
        }
    } else {
        0
    }
}

/// Get region allocation statistics.
///
/// # Arguments
/// * `region_id` - The region ID
/// * `out_allocations` - Output: total allocations
/// * `out_reused` - Output: allocations from free list
/// * `out_bumped` - Output: allocations from bump allocator
/// * `out_deallocations` - Output: total deallocations
///
/// # Safety
/// All output pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_region_get_stats(
    region_id: u64,
    out_allocations: *mut u64,
    out_reused: *mut u64,
    out_bumped: *mut u64,
    out_deallocations: *mut u64,
) {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        let stats = region.stats();
        *out_allocations = stats.allocations();
        *out_reused = stats.reused();
        *out_bumped = stats.bumped();
        *out_deallocations = stats.deallocations();
    } else {
        *out_allocations = 0;
        *out_reused = 0;
        *out_bumped = 0;
        *out_deallocations = 0;
    }
}

/// Get the size class for a given allocation size.
///
/// Returns the size class index (0-11) or 255 for large allocations.
#[no_mangle]
pub extern "C" fn blood_size_class_for(size: usize) -> u8 {
    size_class_for(size)
}

/// Get the slot size for a size class.
///
/// Returns 0 for large allocations (class 255).
#[no_mangle]
pub extern "C" fn blood_slot_size_for_class(class: u8) -> usize {
    slot_size_for_class(class)
}

/// Validate a single address against an expected generation.
///
/// Returns 0 if valid, 1 if stale (generation mismatch).
///
/// # Arguments
/// * `address` - The address to validate
/// * `expected_generation` - The expected generation
#[no_mangle]
pub extern "C" fn blood_validate_generation(address: u64, expected_generation: u32) -> c_int {
    match get_slot_generation(address) {
        Some(actual_gen) => {
            if actual_gen == expected_generation || expected_generation == generation::PERSISTENT {
                0 // Valid
            } else {
                1 // Stale
            }
        }
        None => {
            // Not in registry - assume valid for untracked memory (stack/static)
            if expected_generation > generation::FIRST {
                1 // Likely was heap memory that got freed
            } else {
                0 // Probably stack/static memory
            }
        }
    }
}

// ============================================================================
// Persistent Tier (Tier 3) RC Operations
// ============================================================================

/// Allocate a persistent (Tier 3) value with reference counting.
///
/// Returns a pointer to the allocated memory. The slot ID is written to `out_id`.
/// Returns null on failure.
///
/// # Arguments
/// * `size` - Size of the allocation in bytes
/// * `align` - Required alignment
/// * `type_fp` - Type fingerprint for cycle collection
/// * `out_id` - Output pointer for the assigned slot ID
///
/// # Safety
/// `out_id` must be a valid pointer to a `u64`.
#[no_mangle]
pub unsafe extern "C" fn blood_persistent_alloc(
    size: usize,
    align: usize,
    type_fp: u32,
    out_id: *mut u64,
) -> *mut u8 {
    match persistent_alloc(size, align, type_fp) {
        Some((id, ptr)) => {
            *out_id = id;
            ptr
        }
        None => {
            *out_id = 0;
            std::ptr::null_mut()
        }
    }
}

/// Increment the reference count for a persistent slot.
///
/// Returns 1 on success (slot exists), 0 on failure (slot not found).
///
/// # Arguments
/// * `id` - The slot ID returned by `blood_persistent_alloc`
#[no_mangle]
pub extern "C" fn blood_persistent_increment(id: u64) -> c_int {
    if persistent_increment(id) { 1 } else { 0 }
}

/// Decrement the reference count for a persistent slot.
///
/// When the count reaches zero, the slot is queued for deferred cleanup.
///
/// # Arguments
/// * `id` - The slot ID returned by `blood_persistent_alloc`
#[no_mangle]
pub extern "C" fn blood_persistent_decrement(id: u64) {
    persistent_decrement(id)
}

/// Check if a persistent slot is alive (has refcount > 0).
///
/// Returns 1 if alive, 0 if dead or not found.
///
/// # Arguments
/// * `id` - The slot ID to check
#[no_mangle]
pub extern "C" fn blood_persistent_is_alive(id: u64) -> c_int {
    if persistent_is_alive(id) { 1 } else { 0 }
}

/// Trigger a cycle collection pass to detect and collect circular references
/// in Tier 3 (persistent) memory.
///
/// Returns the number of slots collected.
///
/// This function performs a full mark-and-sweep cycle collection using the
/// global `CycleCollector`. It is safe to call at any time — concurrent
/// collection requests are serialized (subsequent calls return 0 while
/// a collection is already in progress).
#[no_mangle]
pub extern "C" fn blood_cycle_collect() -> u64 {
    collect_cycles(&[]) as u64
}

// ============================================================================
// Region Management (for scoped allocation with effect suspension)
// ============================================================================

/// Global region registry for tracking regions by ID.
static REGION_REGISTRY: OnceLock<Mutex<HashMap<u64, Region>>> = OnceLock::new();

/// Get or initialize the region registry.
fn get_region_registry() -> &'static Mutex<HashMap<u64, Region>> {
    REGION_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Create a new region with the given initial and maximum sizes.
///
/// Returns the region ID (0 on failure).
#[no_mangle]
pub extern "C" fn blood_region_create(initial_size: usize, max_size: usize) -> u64 {
    let region = Region::new(initial_size, max_size);
    let id = region.id().as_u64();

    let registry = get_region_registry();
    let mut reg = registry.lock();
    reg.insert(id, region);

    id
}

/// Destroy a region.
///
/// This frees all memory associated with the region.
/// Any pointers into this region become stale.
#[no_mangle]
pub extern "C" fn blood_region_destroy(region_id: u64) {
    let registry = get_region_registry();
    let mut reg = registry.lock();
    reg.remove(&region_id);
}

/// Get a region by ID.
///
/// Returns a raw pointer to the region, or null if not found.
/// The pointer is only valid while holding the registry lock.
///
/// # Safety
/// The returned pointer must not be used after the registry lock is released.
fn get_region_by_id(reg: &mut HashMap<u64, Region>, region_id: u64) -> Option<&mut Region> {
    reg.get_mut(&region_id)
}

/// Suspend a region (increment suspend count).
///
/// Called when an effect captures a continuation that references this region.
/// Returns the new suspend count.
#[no_mangle]
pub extern "C" fn blood_region_suspend(region_id: u64) -> u32 {
    let registry = get_region_registry();
    let mut reg = registry.lock();

    if let Some(region) = get_region_by_id(&mut reg, region_id) {
        region.suspend()
    } else {
        0
    }
}

/// Resume a region (decrement suspend count).
///
/// Called when a continuation referencing this region resumes or is dropped.
/// Returns (new_suspend_count, should_deallocate) packed as: count | (should_dealloc << 16).
#[no_mangle]
pub extern "C" fn blood_region_resume(region_id: u64) -> u32 {
    let registry = get_region_registry();
    let mut reg = registry.lock();

    if let Some(region) = get_region_by_id(&mut reg, region_id) {
        let (count, should_dealloc) = region.resume();
        count | ((should_dealloc as u32) << 16)
    } else {
        0
    }
}

/// Exit a region scope.
///
/// Called at the end of a region's lexical scope.
/// Returns 1 if the region should be deallocated immediately, 0 if deferred.
#[no_mangle]
pub extern "C" fn blood_region_exit_scope(region_id: u64) -> c_int {
    let registry = get_region_registry();
    let mut reg = registry.lock();

    if let Some(region) = get_region_by_id(&mut reg, region_id) {
        if region.exit_scope() {
            1 // Deallocate immediately
        } else {
            0 // Deferred
        }
    } else {
        1 // Region not found, nothing to do
    }
}

/// Get the suspend count of a region.
#[no_mangle]
pub extern "C" fn blood_region_get_suspend_count(region_id: u64) -> u32 {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        region.suspend_count()
    } else {
        0
    }
}

/// Get the status of a region.
///
/// Returns: 0 = Active, 1 = Suspended, 2 = PendingDeallocation.
#[no_mangle]
pub extern "C" fn blood_region_get_status(region_id: u64) -> u32 {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        region.status() as u32
    } else {
        0 // Active (default)
    }
}

/// Allocate memory from a region.
///
/// Returns the address of the allocated memory, or 0 on failure.
#[no_mangle]
pub extern "C" fn blood_region_alloc(region_id: u64, size: usize, align: usize) -> u64 {
    record_alloc_hist(size);
    let registry = get_region_registry();
    let mut reg = registry.lock();

    if let Some(region) = get_region_by_id(&mut reg, region_id) {
        region.allocate(size, align).map(|p| p as u64).unwrap_or(0)
    } else {
        0
    }
}

/// Check if a region is suspended.
#[no_mangle]
pub extern "C" fn blood_region_is_suspended(region_id: u64) -> c_int {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        if region.is_suspended() { 1 } else { 0 }
    } else {
        0
    }
}

/// Check if a region is pending deallocation.
#[no_mangle]
pub extern "C" fn blood_region_is_pending_deallocation(region_id: u64) -> c_int {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        if region.is_pending_deallocation() { 1 } else { 0 }
    } else {
        0
    }
}

/// Get the current used bytes in a region.
#[no_mangle]
pub extern "C" fn blood_region_used(region_id: u64) -> usize {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        region.used()
    } else {
        0
    }
}

/// Returns the committed (physically backed) bytes for a region.
#[no_mangle]
pub extern "C" fn blood_region_committed(region_id: u64) -> usize {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        region.capacity()
    } else {
        0
    }
}

/// Returns the allocation count for a region.
#[no_mangle]
pub extern "C" fn blood_region_alloc_count(region_id: u64) -> u64 {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        region.stats().allocations.load(std::sync::atomic::Ordering::Relaxed)
    } else {
        0
    }
}

// ============================================================================
// Realloc Diagnostic FFI
// ============================================================================

/// Get realloc diagnostic counters. Fills 4 u64 values:
/// [0] realloc_count, [1] wasted_bytes, [2] inplace_eligible, [3] inplace_saved_bytes
#[no_mangle]
pub extern "C" fn blood_realloc_stats(out: *mut u64) {
    if out.is_null() { return; }
    unsafe {
        *out = REALLOC_COUNT.load(AtomicOrdering::Relaxed);
        *out.add(1) = REALLOC_WASTED_BYTES.load(AtomicOrdering::Relaxed);
        *out.add(2) = REALLOC_INPLACE_ELIGIBLE.load(AtomicOrdering::Relaxed);
        *out.add(3) = REALLOC_INPLACE_SAVED_BYTES.load(AtomicOrdering::Relaxed);
    }
}

/// Get total realloc calls inside regions.
#[no_mangle]
pub extern "C" fn blood_realloc_diag_count() -> u64 {
    REALLOC_COUNT.load(AtomicOrdering::Relaxed)
}

/// Get total bytes wasted by realloc (abandoned old buffers).
#[no_mangle]
pub extern "C" fn blood_realloc_diag_wasted() -> u64 {
    REALLOC_WASTED_BYTES.load(AtomicOrdering::Relaxed)
}

/// Get count of reallocs eligible for in-place growth.
#[no_mangle]
pub extern "C" fn blood_realloc_diag_inplace() -> u64 {
    REALLOC_INPLACE_ELIGIBLE.load(AtomicOrdering::Relaxed)
}

/// Get bytes that would be saved by in-place realloc.
#[no_mangle]
pub extern "C" fn blood_realloc_diag_inplace_bytes() -> u64 {
    REALLOC_INPLACE_SAVED_BYTES.load(AtomicOrdering::Relaxed)
}

/// Reset realloc diagnostic counters to zero.
#[no_mangle]
pub extern "C" fn blood_realloc_stats_reset() {
    REALLOC_COUNT.store(0, AtomicOrdering::Relaxed);
    REALLOC_WASTED_BYTES.store(0, AtomicOrdering::Relaxed);
    REALLOC_INPLACE_ELIGIBLE.store(0, AtomicOrdering::Relaxed);
    REALLOC_INPLACE_SAVED_BYTES.store(0, AtomicOrdering::Relaxed);
    REALLOC_OFFSET_DELTA_SAVED.store(0, AtomicOrdering::Relaxed);
}

/// Get the actual offset delta saved by in-place reallocs (debugging counter).
#[no_mangle]
pub extern "C" fn blood_realloc_diag_offset_delta() -> u64 {
    REALLOC_OFFSET_DELTA_SAVED.load(AtomicOrdering::Relaxed)
}

/// Get allocation histogram: writes count and bytes for each of 18 size buckets.
/// Output buffer must hold at least 36 u64 values (18 counts + 18 bytes).
#[no_mangle]
pub extern "C" fn blood_alloc_hist_get(out: *mut u64) {
    if out.is_null() { return; }
    unsafe {
        for i in 0..ALLOC_HIST_BUCKETS {
            *out.add(i) = ALLOC_HIST_COUNT[i].load(AtomicOrdering::Relaxed);
            *out.add(ALLOC_HIST_BUCKETS + i) = ALLOC_HIST_BYTES[i].load(AtomicOrdering::Relaxed);
        }
    }
}

/// Reset allocation histogram counters.
#[no_mangle]
pub extern "C" fn blood_alloc_hist_reset() {
    for i in 0..ALLOC_HIST_BUCKETS {
        ALLOC_HIST_COUNT[i].store(0, AtomicOrdering::Relaxed);
        ALLOC_HIST_BYTES[i].store(0, AtomicOrdering::Relaxed);
    }
}

/// Print allocation histogram to stderr and reset.
/// Each line shows: size range, allocation count, total bytes, avg size.
#[no_mangle]
pub extern "C" fn blood_print_alloc_hist() {
    let labels = [
        "1-8B", "9-16B", "17-32B", "33-64B", "65-128B", "129-256B",
        "257-512B", "513B-1K", "1K-2K", "2K-4K", "4K-8K", "8K-16K",
        "16K-32K", "32K-64K", "64K-128K", "128K-256K", "256K-512K", ">512K",
    ];
    let mut total_count: u64 = 0;
    let mut total_bytes: u64 = 0;
    for i in 0..ALLOC_HIST_BUCKETS {
        let count = ALLOC_HIST_COUNT[i].load(AtomicOrdering::Relaxed);
        let bytes = ALLOC_HIST_BYTES[i].load(AtomicOrdering::Relaxed);
        total_count += count;
        total_bytes += bytes;
        if count > 0 {
            let avg = if count > 0 { bytes / count } else { 0 };
            eprintln!("    {:>12}  count={:>12}  bytes={:>14}  avg={:>6}",
                labels[i], format_count(count), format_bytes(bytes), avg);
        }
    }
    eprintln!("    {:>12}  count={:>12}  bytes={:>14}",
        "TOTAL", format_count(total_count), format_bytes(total_bytes));
    blood_alloc_hist_reset();
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000_000 { format!("{:.2}B", n as f64 / 1e9) }
    else if n >= 1_000_000 { format!("{:.2}M", n as f64 / 1e6) }
    else if n >= 1_000 { format!("{:.1}K", n as f64 / 1e3) }
    else { format!("{}", n) }
}

fn format_bytes(b: u64) -> String {
    if b >= 1_073_741_824 { format!("{:.2} GB", b as f64 / 1_073_741_824.0) }
    else if b >= 1_048_576 { format!("{:.2} MB", b as f64 / 1_048_576.0) }
    else if b >= 1024 { format!("{:.1} KB", b as f64 / 1024.0) }
    else { format!("{} B", b) }
}

/// Trim a region's committed pages above the current allocation offset.
///
/// Releases physical pages via madvise(MADV_DONTNEED) while keeping the
/// virtual address space mapped. Call at phase boundaries to reclaim memory
/// that was needed temporarily.
#[no_mangle]
pub extern "C" fn blood_region_trim(region_id: u64) {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        #[cfg(unix)]
        region.trim();
    }
}

// ============================================================================
// Continuation Region Tracking
// ============================================================================

/// Global side table for tracking suspended regions per continuation.
///
/// Maps continuation ID -> list of region IDs that were suspended when
/// the continuation was captured. Used to restore regions on resume.
static CONTINUATION_REGIONS: OnceLock<Mutex<HashMap<u64, Vec<u64>>>> = OnceLock::new();

/// Get or initialize the continuation regions table.
fn get_continuation_regions() -> &'static Mutex<HashMap<u64, Vec<u64>>> {
    CONTINUATION_REGIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Add a suspended region to a continuation.
///
/// This should be called when capturing a continuation that references
/// allocations in a region. It increments the region's suspend count
/// and tracks the region ID for later restoration on resume.
#[no_mangle]
pub extern "C" fn blood_continuation_add_suspended_region(
    continuation_id: u64,
    region_id: u64,
) {
    // First suspend the region
    blood_region_suspend(region_id);

    // Then track it in the continuation's region list
    let regions = get_continuation_regions();
    let mut reg = regions.lock();
    reg.entry(continuation_id).or_default().push(region_id);
}

/// Get and clear the suspended regions for a continuation.
///
/// Called when resuming a continuation to restore region state.
/// Returns the count of regions, and fills the provided buffer.
/// Also decrements suspend counts and handles deferred deallocation.
///
/// # Safety
/// The buffer must be large enough to hold all region IDs.
#[no_mangle]
pub unsafe extern "C" fn blood_continuation_take_suspended_regions(
    continuation_id: u64,
    out_regions: *mut u64,
    max_count: usize,
) -> usize {
    let regions = get_continuation_regions();
    let mut reg = regions.lock();

    if let Some(region_ids) = reg.remove(&continuation_id) {
        let count = region_ids.len().min(max_count);

        if !out_regions.is_null() {
            for (i, &rid) in region_ids.iter().take(count).enumerate() {
                *out_regions.add(i) = rid;
            }
        }

        // Resume all regions (decrement suspend count)
        for rid in &region_ids {
            let result = blood_region_resume(*rid);
            let should_dealloc = (result >> 16) != 0;

            if should_dealloc {
                // Region's lexical scope ended and this was the last continuation
                // Deallocate now
                blood_region_destroy(*rid);
            }
        }

        count
    } else {
        0
    }
}

/// Resume a continuation with region validation.
///
/// This wrapper around blood_continuation_resume also handles:
/// 1. Validating the generation snapshot
/// 2. Restoring suspended regions
///
/// # Safety
/// The continuation must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_continuation_resume_with_regions(
    continuation: ContinuationHandle,
    value: i64,
) -> i64 {
    // First, restore suspended regions
    let mut region_buffer = [0u64; 64]; // Max 64 suspended regions
    let region_count = blood_continuation_take_suspended_regions(
        continuation,
        region_buffer.as_mut_ptr(),
        region_buffer.len(),
    );

    // Log for debugging if regions were restored
    if region_count > 0 {
        // Regions have been resumed via blood_continuation_take_suspended_regions
        // No additional action needed here
    }

    // Now resume the continuation
    blood_continuation_resume(continuation, value)
}

// ============================================================================
// Fiber/Continuation Support (for effect handlers)
// ============================================================================

// Thread-local effect context for the current operation.
std::thread_local! {
    static EFFECT_CONTEXT: std::cell::RefCell<Option<EffectContext>> =
        const { std::cell::RefCell::new(None) };
}

/// Create a new fiber for continuation capture.
///
/// Returns a fiber handle (0 on failure).
#[no_mangle]
pub extern "C" fn blood_fiber_create() -> FiberHandle {
    // Use the global scheduler if available
    if let Some(scheduler) = GLOBAL_SCHEDULER.get() {
        let sched = scheduler.lock();
        sched.spawn(|| {}).as_u64()
    } else {
        // Fallback: return a unique ID
        static NEXT_FIBER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT_FIBER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}

/// Suspend current fiber and capture continuation.
///
/// This function captures the current execution context as a continuation
/// that can later be resumed with a value.
///
/// # Returns
/// The captured continuation ID, or 0 if capture failed.
///
/// # Implementation Notes
///
/// For Phase 2.1 (tail-resumptive handlers), this is rarely called because
/// tail-resumptive effects don't need continuation capture.
///
/// For Phase 2.3 (full continuations), this will use one of:
/// - Closure-based CPS (current implementation)
/// - Stack segment copying (future optimization)
/// - Platform-specific assembly (future optimization)
#[no_mangle]
pub extern "C" fn blood_fiber_suspend() -> ContinuationHandle {
    // Check if we're in an effect context
    EFFECT_CONTEXT.with(|ctx| {
        let ctx_ref = ctx.borrow();
        if let Some(effect_ctx) = ctx_ref.as_ref() {
            if let Some(k_ref) = effect_ctx.continuation {
                return k_ref.id;
            }
        }
        0
    })
}

/// Resume a suspended continuation with a value.
///
/// # Arguments
/// * `continuation` - The continuation handle from blood_fiber_suspend
/// * `value` - The value to resume with (passed to the continuation)
///
/// # Returns
/// The result of the continuation as an i64.
///
/// # Safety
/// The continuation handle must be valid and not already consumed.
#[no_mangle]
pub extern "C" fn blood_continuation_resume(continuation: ContinuationHandle, value: i64) -> i64 {
    let k_ref = ContinuationRef { id: continuation };

    if let Some(k) = take_continuation(k_ref) {
        // Resume the continuation with the provided value
        // For now, we use i64 as the universal value type
        match k.try_resume::<i64, i64>(value) {
            Some(result) => result,
            None => {
                eprintln!(
                    "BLOOD RUNTIME PANIC: Failed to resume continuation {} \
                     (continuation callback returned None)",
                    continuation
                );
                std::process::abort();
            }
        }
    } else {
        // Continuation not found: either the handle is invalid (never created)
        // or the continuation was already consumed (single-shot used twice).
        // Both are programmer errors that must not be silently ignored.
        eprintln!(
            "BLOOD RUNTIME PANIC: Continuation {} not found or already consumed. \
             A continuation can only be resumed once (single-shot). \
             For multi-shot semantics, use blood_continuation_clone before resuming.",
            continuation
        );
        std::process::abort();
    }
}

/// Check if a continuation handle is valid.
#[no_mangle]
pub extern "C" fn blood_continuation_valid(continuation: ContinuationHandle) -> bool {
    has_continuation(ContinuationRef { id: continuation })
}

/// Resume a suspended fiber with a value (legacy API).
///
/// # Safety
/// The fiber handle must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_fiber_resume(fiber: FiberHandle, value: u64) {
    // For compatibility, treat fiber handle as a continuation handle
    blood_continuation_resume(fiber, value as i64);
}

/// A Send-safe wrapper for a continuation callback with its context.
///
/// # Safety
/// The user must ensure that the wrapped callback and context are safe to
/// access from any thread. This is typically ensured by the caller of
/// blood_continuation_create.
#[derive(Clone, Copy)]
struct ContinuationCallback {
    callback: extern "C" fn(i64, *mut c_void) -> i64,
    context: *mut c_void,
}

// Safety: This is intentionally marked as Send.
// The caller of blood_continuation_create is responsible for ensuring
// the callback and context remain valid and are safe to access from any thread.
unsafe impl Send for ContinuationCallback {}

impl ContinuationCallback {
    /// Invoke the callback with a value.
    fn call(&self, value: i64) -> i64 {
        (self.callback)(value, self.context)
    }
}

// ============================================================================
// Multi-Shot Continuation Support
// ============================================================================

/// Registry for multi-shot continuation callbacks.
///
/// This stores the original callback/context so continuations can be cloned.
/// For single-shot continuations, the callback is consumed on resume.
/// For multi-shot continuations, we keep the callback here for cloning.
static MULTISHOT_CALLBACKS: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashMap<u64, ContinuationCallback>>> = std::sync::OnceLock::new();

fn get_multishot_registry() -> &'static parking_lot::Mutex<std::collections::HashMap<u64, ContinuationCallback>> {
    MULTISHOT_CALLBACKS.get_or_init(|| parking_lot::Mutex::new(std::collections::HashMap::new()))
}

/// Create a multi-shot continuation from a callback function.
///
/// Unlike `blood_continuation_create`, this continuation can be cloned
/// multiple times using `blood_continuation_clone`.
///
/// # Arguments
/// * `callback` - Function pointer: fn(value: i64, context: *mut c_void) -> i64
/// * `context` - User context pointer passed to callback
///
/// # Returns
/// Continuation handle, or 0 on failure.
///
/// # Safety
/// The callback and context must remain valid until all clones are resumed.
#[no_mangle]
pub unsafe extern "C" fn blood_continuation_create_multishot(
    callback: extern "C" fn(i64, *mut c_void) -> i64,
    context: *mut c_void,
) -> ContinuationHandle {
    let cb = ContinuationCallback { callback, context };

    // Create the continuation
    let cb_clone = cb;
    let k = Continuation::new(move |value: i64| -> i64 {
        cb_clone.call(value)
    });

    let id = k.id().as_u64();

    // Store callback in multi-shot registry for cloning
    get_multishot_registry().lock().insert(id, cb);

    // Register the continuation
    let k_ref = register_continuation(k);
    k_ref.id
}

/// Clone a multi-shot continuation.
///
/// Creates a new continuation with a new ID but the same callback/context.
/// The original continuation remains valid and can also be resumed.
///
/// # Arguments
/// * `handle` - Handle of the continuation to clone
///
/// # Returns
/// New continuation handle, or 0 if the original was not found or not multi-shot.
///
/// # Safety
/// The original continuation must have been created with `blood_continuation_create_multishot`.
#[no_mangle]
pub unsafe extern "C" fn blood_continuation_clone(handle: ContinuationHandle) -> ContinuationHandle {
    // Look up the callback in the multi-shot registry
    let cb = {
        let registry = get_multishot_registry().lock();
        registry.get(&handle).copied()
    };

    let Some(cb) = cb else {
        eprintln!("BLOOD RUNTIME ERROR: Cannot clone continuation {} - not found in multi-shot registry", handle);
        return 0;
    };

    // Create a new continuation with the same callback
    let k = Continuation::new(move |value: i64| -> i64 {
        cb.call(value)
    });

    let new_id = k.id().as_u64();

    // Also register the clone in the multi-shot registry
    get_multishot_registry().lock().insert(new_id, cb);

    // Register and return
    let k_ref = register_continuation(k);
    k_ref.id
}

/// Clean up a multi-shot continuation's registry entry.
///
/// Should be called when a multi-shot continuation is no longer needed.
/// This removes the callback from the multi-shot registry.
#[no_mangle]
pub extern "C" fn blood_continuation_drop_multishot(handle: ContinuationHandle) {
    get_multishot_registry().lock().remove(&handle);
    // Also remove from continuation registry if present
    let _ = take_continuation(ContinuationRef { id: handle });
}

/// Create a continuation from a callback function.
///
/// This is used by the compiler to wrap the "rest of the computation"
/// after an effect is performed.
///
/// # Arguments
/// * `callback` - Function pointer: fn(value: i64, context: *mut c_void) -> i64
/// * `context` - User context pointer passed to callback
///
/// # Returns
/// Continuation handle, or 0 on failure.
///
/// # Safety
/// The callback and context must remain valid until the continuation is resumed.
/// The context pointer must be safe to access from any thread.
#[no_mangle]
pub unsafe extern "C" fn blood_continuation_create(
    callback: extern "C" fn(i64, *mut c_void) -> i64,
    context: *mut c_void,
) -> ContinuationHandle {
    // Wrap the callback and context in a Send-safe wrapper
    let cb = ContinuationCallback { callback, context };

    // Create a continuation that wraps the callback
    let k = Continuation::new(move |value: i64| -> i64 {
        cb.call(value)
    });

    let k_ref = register_continuation(k);
    k_ref.id
}

/// Set up effect context for the current operation.
///
/// Called by the runtime before invoking a handler operation.
#[no_mangle]
pub extern "C" fn blood_effect_context_begin(is_tail_resumptive: bool) {
    EFFECT_CONTEXT.with(|ctx| {
        let mut ctx_ref = ctx.borrow_mut();
        *ctx_ref = Some(if is_tail_resumptive {
            EffectContext::tail_resumptive()
        } else {
            EffectContext::with_continuation(ContinuationRef::null())
        });
    });
}

/// Clean up effect context after an operation.
#[no_mangle]
pub extern "C" fn blood_effect_context_end() {
    EFFECT_CONTEXT.with(|ctx| {
        let mut ctx_ref = ctx.borrow_mut();
        *ctx_ref = None;
    });
}

/// Check if current handler is tail-resumptive.
#[no_mangle]
pub extern "C" fn blood_effect_is_tail_resumptive() -> bool {
    EFFECT_CONTEXT.with(|ctx| {
        let ctx_ref = ctx.borrow();
        ctx_ref.as_ref().map(|c| c.is_tail_resumptive).unwrap_or(true)
    })
}

/// Set the generation snapshot for the current effect context.
///
/// This should be called during `perform` to capture the generations of
/// references that will be accessed after the handler resumes.
#[no_mangle]
pub extern "C" fn blood_effect_context_set_snapshot(snapshot: SnapshotHandle) {
    EFFECT_CONTEXT.with(|ctx| {
        let mut ctx_ref = ctx.borrow_mut();
        if let Some(ref mut effect_ctx) = *ctx_ref {
            // Convert the raw SnapshotHandle to the continuation module's SnapshotHandle type
            effect_ctx.set_snapshot(snapshot as crate::continuation::SnapshotHandle);
        }
    });
}

/// Get the generation snapshot from the current effect context.
///
/// This should be called during `resume` to validate captured references
/// before returning to user code. Returns null if no snapshot was captured.
#[no_mangle]
pub extern "C" fn blood_effect_context_get_snapshot() -> SnapshotHandle {
    EFFECT_CONTEXT.with(|ctx| {
        let ctx_ref = ctx.borrow();
        ctx_ref
            .as_ref()
            .and_then(|c| c.snapshot)
            .map(|s| s as SnapshotHandle)
            .unwrap_or(std::ptr::null_mut())
    })
}

/// Take the generation snapshot from the current effect context.
///
/// This transfers ownership - the caller is responsible for destroying it.
/// Returns null if no snapshot was captured.
#[no_mangle]
pub extern "C" fn blood_effect_context_take_snapshot() -> SnapshotHandle {
    EFFECT_CONTEXT.with(|ctx| {
        let mut ctx_ref = ctx.borrow_mut();
        ctx_ref
            .as_mut()
            .and_then(|c| c.take_snapshot())
            .map(|s| s as SnapshotHandle)
            .unwrap_or(std::ptr::null_mut())
    })
}

// ============================================================================
// Work-Stealing Scheduler FFI
// ============================================================================

use crate::scheduler::Scheduler;
use crate::SchedulerConfig;

/// Global scheduler instance for FFI access.
static GLOBAL_SCHEDULER: OnceLock<Mutex<Scheduler>> = OnceLock::new();

/// Scheduler thread handle for background execution.
static SCHEDULER_THREAD: OnceLock<Mutex<Option<std::thread::JoinHandle<()>>>> = OnceLock::new();

/// Initialize the global scheduler with the given number of workers.
///
/// Returns 0 on success, -1 if already initialized.
#[no_mangle]
pub extern "C" fn blood_scheduler_init(num_workers: usize) -> c_int {
    let workers = if num_workers == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        num_workers
    };

    let config = SchedulerConfig {
        num_workers: workers,
        ..Default::default()
    };

    match GLOBAL_SCHEDULER.set(Mutex::new(Scheduler::new(config))) {
        Ok(()) => {
            // Initialize thread handle storage
            let _ = SCHEDULER_THREAD.set(Mutex::new(None));
            0
        }
        Err(_) => -1, // Already initialized
    }
}

/// Get the global scheduler, initializing with defaults if needed.
fn get_or_init_scheduler() -> &'static Mutex<Scheduler> {
    GLOBAL_SCHEDULER.get_or_init(|| {
        let _ = SCHEDULER_THREAD.set(Mutex::new(None));
        Mutex::new(Scheduler::new(SchedulerConfig::default()))
    })
}

/// Spawn a task on the scheduler.
///
/// The `task_fn` is a function pointer that takes a single `arg` parameter.
/// Returns the fiber ID on success, 0 on failure.
///
/// # Safety
/// The function pointer must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_scheduler_spawn(
    task_fn: extern "C" fn(*mut c_void),
    arg: *mut c_void,
) -> u64 {
    let scheduler = get_or_init_scheduler();
    let sched = scheduler.lock();

    // Capture the function pointer and argument
    let fn_ptr = task_fn;
    let arg_ptr = arg as usize; // Convert to usize for Send

    let fiber_id = sched.spawn(move || {
        fn_ptr(arg_ptr as *mut c_void);
    });

    fiber_id.as_u64()
}

/// Spawn a simple task (no arguments).
///
/// # Safety
/// The function pointer must be valid.
#[no_mangle]
pub unsafe extern "C" fn blood_scheduler_spawn_simple(
    task_fn: extern "C" fn(),
) -> u64 {
    let scheduler = get_or_init_scheduler();
    let sched = scheduler.lock();

    // Wrap extern "C" fn in a closure to satisfy FnOnce trait
    let fiber_id = sched.spawn(move || task_fn());
    fiber_id.as_u64()
}

/// Yield the current fiber to the scheduler.
///
/// This allows other fibers to run.
#[no_mangle]
pub extern "C" fn blood_scheduler_yield() {
    std::thread::yield_now();
}

/// Run the scheduler in the current thread.
///
/// This function blocks until the scheduler is shut down.
/// Call blood_scheduler_shutdown() from another thread to stop it.
#[no_mangle]
pub extern "C" fn blood_scheduler_run() {
    let scheduler = get_or_init_scheduler();
    let mut sched = scheduler.lock();
    sched.run();
}

/// Run the scheduler in a background thread.
///
/// Returns 0 on success, -1 if already running.
#[no_mangle]
pub extern "C" fn blood_scheduler_run_background() -> c_int {
    // Ensure scheduler is initialized
    let _ = get_or_init_scheduler();

    if let Some(thread_holder) = SCHEDULER_THREAD.get() {
        let mut thread_guard = thread_holder.lock();

        // Check if already running
        if thread_guard.is_some() {
            return -1;
        }

        // Start scheduler in background thread
        let handle = std::thread::Builder::new()
            .name("blood-scheduler".to_string())
            .spawn(move || {
                let scheduler = get_or_init_scheduler();
                let mut sched = scheduler.lock();
                sched.run();
            })
            .ok();

        *thread_guard = handle;
        0
    } else {
        -1
    }
}

/// Shutdown the scheduler.
///
/// This signals all workers to stop and returns immediately.
/// Use blood_scheduler_wait() to wait for completion.
#[no_mangle]
pub extern "C" fn blood_scheduler_shutdown() {
    if let Some(scheduler) = GLOBAL_SCHEDULER.get() {
        let sched = scheduler.lock();
        sched.shutdown();
    }
}

/// Wait for the scheduler to finish.
///
/// This blocks until all workers have stopped.
#[no_mangle]
pub extern "C" fn blood_scheduler_wait() {
    if let Some(thread_holder) = SCHEDULER_THREAD.get() {
        let mut thread_guard = thread_holder.lock();
        if let Some(handle) = thread_guard.take() {
            let _ = handle.join();
        }
    }
}

/// Get the number of active fibers.
#[no_mangle]
pub extern "C" fn blood_scheduler_active_fibers() -> usize {
    if let Some(scheduler) = GLOBAL_SCHEDULER.get() {
        let sched = scheduler.lock();
        sched.active_fiber_count()
    } else {
        0
    }
}

/// Get the number of runnable fibers.
#[no_mangle]
pub extern "C" fn blood_scheduler_runnable_fibers() -> usize {
    if let Some(scheduler) = GLOBAL_SCHEDULER.get() {
        let sched = scheduler.lock();
        sched.runnable_fiber_count()
    } else {
        0
    }
}

/// Check if the scheduler is running.
#[no_mangle]
pub extern "C" fn blood_scheduler_is_running() -> c_int {
    if let Some(scheduler) = GLOBAL_SCHEDULER.get() {
        let sched = scheduler.lock();
        if sched.is_shutting_down() { 0 } else { 1 }
    } else {
        0
    }
}

// ============================================================================
// Panic/Error Handling
// ============================================================================

/// Well-known effect ID for StaleReference (must match bloodc/src/effects/std_effects.rs)
const STALE_REFERENCE_EFFECT_ID: i64 = 0x1004;

/// Operation indices for StaleReference effect
#[allow(dead_code)] // Kept for documentation - validate_ptr is handled differently
const STALE_REFERENCE_OP_VALIDATE_PTR: i32 = 0;
const STALE_REFERENCE_OP_STALE_ERROR: i32 = 1;

/// Called when a stale reference is dereferenced.
///
/// This attempts to perform the StaleReference.stale_error effect.
/// If a handler is installed, it will be invoked.
/// If no handler is installed, the program aborts.
#[no_mangle]
pub extern "C" fn blood_stale_reference_panic(expected: u32, actual: u32) -> ! {
    // Check if there's a StaleReference handler installed
    let depth = blood_handler_depth(STALE_REFERENCE_EFFECT_ID);

    if depth > 0 {
        // Handler is installed - perform the stale_error effect
        // Pack the error information into args
        let args: [i64; 2] = [expected as i64, actual as i64];

        unsafe {
            // Perform the stale_error operation (op_index = 1)
            // This may not return if the handler decides to abort
            let _result = blood_perform(
                STALE_REFERENCE_EFFECT_ID,
                STALE_REFERENCE_OP_STALE_ERROR,
                args.as_ptr(),
                2,
                0, // No continuation for panic handler
            );
            // If we get here, the handler resumed (which is wrong for stale_error -> !)
            // but we handle it gracefully by aborting anyway
        }
    }

    // No handler or handler returned - abort with error message
    eprintln!(
        "BLOOD RUNTIME ERROR: Stale reference detected!\n\
         Expected generation: {expected}, Actual: {actual}\n\
         This indicates use-after-free. Aborting."
    );
    std::process::abort();
}

/// Called when snapshot validation fails during effect resume.
///
/// This is called when `blood_snapshot_validate` returns a non-zero value,
/// indicating that one or more generational references have become stale
/// while the continuation was suspended.
///
/// # Arguments
/// * `snapshot` - The snapshot that was validated
/// * `stale_index` - The 1-based index of the first stale entry (from blood_snapshot_validate)
///
/// # Safety
/// - `snapshot` must be a valid pointer to a `GenerationSnapshot` created by
///   `blood_snapshot_capture`, or null.
/// - If not null, `snapshot` must not be concurrently modified or freed.
/// - `stale_index` should be a positive value returned from `blood_snapshot_validate`.
#[no_mangle]
pub unsafe extern "C" fn blood_snapshot_stale_panic(snapshot: SnapshotHandle, stale_index: i64) -> ! {
    if !snapshot.is_null() && stale_index > 0 {
        let snap = &*(snapshot as *const GenerationSnapshot);
        let idx = (stale_index - 1) as usize;

        if let Some(entry) = snap.entries.get(idx) {
            let actual_gen = get_slot_generation(entry.address).unwrap_or(0);
            eprintln!(
                "BLOOD RUNTIME ERROR: Stale reference detected on resume!\n\
                 Address: 0x{:x}\n\
                 Expected generation: {}, Actual: {}\n\
                 This indicates use-after-free while continuation was suspended. Aborting.",
                entry.address, entry.generation, actual_gen
            );
            std::process::abort();
        }
    }

    eprintln!(
        "BLOOD RUNTIME ERROR: Stale reference detected on resume!\n\
         Snapshot validation failed at entry {}. Aborting.",
        stale_index
    );
    std::process::abort();
}

/// Called on unrecoverable runtime errors.
///
/// # Safety
/// The message must be a valid C string.
#[no_mangle]
pub unsafe extern "C" fn blood_panic(msg: *const c_char) -> ! {
    let message = if msg.is_null() {
        "unknown error"
    } else {
        CStr::from_ptr(msg).to_str().unwrap_or("invalid UTF-8")
    };
    eprintln!("BLOOD RUNTIME PANIC: {message}");
    std::process::abort();
}

/// Panic with a Blood str slice message.
///
/// # Safety
/// The pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn panic(msg: BloodStr) -> ! {
    let message = if msg.ptr.is_null() || msg.len == 0 {
        "explicit panic"
    } else {
        let slice = std::slice::from_raw_parts(msg.ptr, msg.len as usize);
        std::str::from_utf8(slice).unwrap_or("invalid UTF-8")
    };
    eprintln!("PANIC: {message}");
    std::process::abort();
}

// ============================================================================
// Runtime Initialization
// ============================================================================

/// Initialize the Blood runtime.
///
/// Must be called before any other runtime functions.
/// Returns 0 on success, non-zero on failure.
#[no_mangle]
pub extern "C" fn blood_runtime_init() -> c_int {
    // Initialize any global state here
    0
}

/// Shutdown the Blood runtime.
///
/// Should be called when the program exits.
#[no_mangle]
pub extern "C" fn blood_runtime_shutdown() {
    // Cleanup any global state here
}

// ============================================================================
// Runtime Configuration
// ============================================================================

/// Runtime configuration handle for FFI.
#[repr(C)]
pub struct RuntimeConfigHandle {
    /// Number of worker threads.
    pub num_workers: u64,
    /// Initial fiber stack size in bytes.
    pub initial_stack_size: u64,
    /// Maximum fiber stack size in bytes.
    pub max_stack_size: u64,
    /// Enable work stealing (1 = true, 0 = false).
    pub work_stealing: u8,
    /// Maximum heap size in bytes (0 = unlimited).
    pub max_heap_size: u64,
    /// Maximum region size in bytes.
    pub max_region_size: u64,
    /// GC cycle collection threshold.
    pub gc_threshold: u64,
    /// Default timeout in milliseconds (0 = none).
    pub default_timeout_ms: u64,
    /// Graceful shutdown timeout in milliseconds.
    pub graceful_shutdown_ms: u64,
    /// Log level (0=off, 1=error, 2=warn, 3=info, 4=debug, 5=trace).
    pub log_level: u8,
}

impl Default for RuntimeConfigHandle {
    fn default() -> Self {
        Self {
            num_workers: std::thread::available_parallelism()
                .map(|n| n.get() as u64)
                .unwrap_or(1),
            initial_stack_size: 8 * 1024,
            max_stack_size: 1024 * 1024,
            work_stealing: 1,
            max_heap_size: 0,
            max_region_size: 16 * 1024 * 1024,
            gc_threshold: 1000,
            default_timeout_ms: 0,
            graceful_shutdown_ms: 5000,
            log_level: 3, // Info
        }
    }
}

/// Get the default runtime configuration.
///
/// # Safety
/// `out` must be a valid pointer to uninitialized RuntimeConfigHandle memory.
#[no_mangle]
pub unsafe extern "C" fn runtime_config_default(out: *mut RuntimeConfigHandle) {
    if out.is_null() {
        return;
    }
    *out = RuntimeConfigHandle::default();
}

/// Load runtime configuration from environment variables.
///
/// Reads `BLOOD_*` environment variables and populates the configuration.
/// Values that are not set will use defaults.
///
/// # Safety
/// `out` must be a valid pointer to uninitialized RuntimeConfigHandle memory.
#[no_mangle]
pub unsafe extern "C" fn runtime_config_from_env(out: *mut RuntimeConfigHandle) {
    use crate::config::RuntimeConfig;

    if out.is_null() {
        return;
    }

    let config = RuntimeConfig::from_env();

    *out = RuntimeConfigHandle {
        num_workers: config.scheduler.num_workers as u64,
        initial_stack_size: config.scheduler.initial_stack_size as u64,
        max_stack_size: config.scheduler.max_stack_size as u64,
        work_stealing: if config.scheduler.work_stealing { 1 } else { 0 },
        max_heap_size: config.memory.max_heap_size as u64,
        max_region_size: config.memory.max_region_size as u64,
        gc_threshold: config.memory.gc_threshold as u64,
        default_timeout_ms: config.timeout.default_timeout
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        graceful_shutdown_ms: config.timeout.graceful_shutdown.as_millis() as u64,
        log_level: config.log.level as u8,
    };
}

/// Initialize the runtime with the given configuration.
///
/// Returns 0 on success, non-zero on failure.
///
/// # Safety
/// `config` must be a valid pointer to a RuntimeConfigHandle.
#[no_mangle]
pub unsafe extern "C" fn runtime_init_with_config(config: *const RuntimeConfigHandle) -> c_int {
    use crate::config::{RuntimeConfig, LogLevel};
    use std::time::Duration;

    if config.is_null() {
        return -1;
    }

    let handle = &*config;

    // Validate basic constraints
    if handle.num_workers == 0 || handle.initial_stack_size < 4096 {
        return -2;
    }

    if handle.max_stack_size < handle.initial_stack_size {
        return -3;
    }

    // Build the RuntimeConfig
    let log_level = match handle.log_level {
        0 => LogLevel::Off,
        1 => LogLevel::Error,
        2 => LogLevel::Warn,
        3 => LogLevel::Info,
        4 => LogLevel::Debug,
        _ => LogLevel::Trace,
    };

    let default_timeout = if handle.default_timeout_ms > 0 {
        Some(Duration::from_millis(handle.default_timeout_ms))
    } else {
        None
    };

    let config = RuntimeConfig::builder()
        .num_workers(handle.num_workers as usize)
        .initial_stack_size(handle.initial_stack_size as usize)
        .max_stack_size(handle.max_stack_size as usize)
        .work_stealing(handle.work_stealing != 0)
        .max_heap_size(handle.max_heap_size as usize)
        .max_region_size(handle.max_region_size as usize)
        .gc_threshold(handle.gc_threshold as usize)
        .default_timeout(default_timeout)
        .graceful_shutdown(Duration::from_millis(handle.graceful_shutdown_ms))
        .log_level(log_level)
        .build_unchecked();

    // Initialize the scheduler with the config
    let scheduler_config = config.to_scheduler_config();
    let _ = GLOBAL_SCHEDULER.set(Mutex::new(crate::scheduler::Scheduler::new(scheduler_config)));

    0
}

/// Get a configuration value as an integer.
///
/// # Arguments
/// * `key` - Configuration key name
///
/// # Returns
/// * Value on success
/// * -1 if key is unknown or not set
///
/// # Safety
/// `key` must be a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn runtime_config_get_int(key: BloodStr) -> i64 {
    if key.ptr.is_null() || key.len == 0 {
        return -1;
    }

    let key_slice = std::slice::from_raw_parts(key.ptr, key.len as usize);
    let key_str = match std::str::from_utf8(key_slice) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    // Try to get from current runtime config
    if let Some(config) = crate::runtime_config() {
        match key_str {
            "num_workers" => return config.scheduler.num_workers as i64,
            "initial_stack_size" => return config.scheduler.initial_stack_size as i64,
            "max_stack_size" => return config.scheduler.max_stack_size as i64,
            "work_stealing" => return if config.scheduler.work_stealing { 1 } else { 0 },
            "max_heap_size" => return config.memory.max_heap_size as i64,
            "max_region_size" => return config.memory.max_region_size as i64,
            "gc_threshold" => return config.memory.gc_threshold as i64,
            "default_timeout_ms" => {
                return config.timeout.default_timeout
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
            }
            "graceful_shutdown_ms" => return config.timeout.graceful_shutdown.as_millis() as i64,
            "log_level" => return config.log.level as i64,
            _ => {}
        }
    }

    // Fall back to defaults
    let defaults = RuntimeConfigHandle::default();
    match key_str {
        "num_workers" => defaults.num_workers as i64,
        "initial_stack_size" => defaults.initial_stack_size as i64,
        "max_stack_size" => defaults.max_stack_size as i64,
        "work_stealing" => defaults.work_stealing as i64,
        "max_heap_size" => defaults.max_heap_size as i64,
        "max_region_size" => defaults.max_region_size as i64,
        "gc_threshold" => defaults.gc_threshold as i64,
        "default_timeout_ms" => defaults.default_timeout_ms as i64,
        "graceful_shutdown_ms" => defaults.graceful_shutdown_ms as i64,
        "log_level" => defaults.log_level as i64,
        _ => -1,
    }
}

// ============================================================================
// Signal Handling
// ============================================================================

/// Signal types for FFI.
///
/// Values:
/// - 0: None (no signal)
/// - 1: SIGTERM (termination request)
/// - 2: SIGINT (interrupt, Ctrl+C)
/// - 3: SIGHUP (hangup)
pub type SignalType = u8;

/// Install signal handlers for graceful shutdown.
///
/// This installs handlers for SIGTERM, SIGINT, and SIGHUP.
/// Calling this multiple times is safe (subsequent calls are no-ops).
///
/// Returns 1 if handlers were installed, 0 if already installed.
#[no_mangle]
pub extern "C" fn signal_install_handlers() -> c_int {
    if crate::signal::install_handlers() { 1 } else { 0 }
}

/// Check if shutdown has been requested.
///
/// Returns 1 if shutdown was requested (SIGTERM or SIGINT received), 0 otherwise.
#[no_mangle]
pub extern "C" fn signal_shutdown_requested() -> c_int {
    if crate::signal::shutdown_requested() { 1 } else { 0 }
}

/// Request graceful shutdown.
///
/// This triggers the shutdown sequence as if SIGTERM was received.
#[no_mangle]
pub extern "C" fn signal_request_shutdown() {
    crate::signal::request_shutdown();
}

/// Get the last received signal.
///
/// Returns the SignalType of the last received signal:
/// - 0: None
/// - 1: SIGTERM
/// - 2: SIGINT
/// - 3: SIGHUP
#[no_mangle]
pub extern "C" fn signal_last() -> SignalType {
    crate::signal::global_handler().last_signal() as u8
}

/// Get the number of signals received.
#[no_mangle]
pub extern "C" fn signal_count() -> u8 {
    crate::signal::global_handler().signal_count()
}

/// Wait for a shutdown signal.
///
/// Blocks until a shutdown signal is received or the timeout expires.
///
/// # Arguments
/// * `timeout_ms` - Timeout in milliseconds (0 = wait forever)
///
/// # Returns
/// * 1 if shutdown was requested
/// * 0 if timeout expired
#[no_mangle]
pub extern "C" fn signal_wait_shutdown(timeout_ms: u64) -> c_int {
    use std::time::Duration;

    let timeout = if timeout_ms == 0 {
        Duration::from_secs(u64::MAX / 1000) // Effectively infinite
    } else {
        Duration::from_millis(timeout_ms)
    };

    if crate::signal::global_handler().wait_for_shutdown(timeout) {
        1
    } else {
        0
    }
}

/// Reset signal state.
///
/// Clears the shutdown flag and signal history.
/// This is mainly useful for testing.
#[no_mangle]
pub extern "C" fn signal_reset() {
    crate::signal::global_handler().reset();
}

// ============================================================================
// Cancellation
// ============================================================================

/// Cancellation token handle for FFI.
pub type CancellationHandle = u64;

/// Registry for cancellation sources (owned by Blood code).
static CANCELLATION_REGISTRY: OnceLock<Mutex<HashMap<u64, crate::cancellation::CancellationSource>>> =
    OnceLock::new();

/// Counter for cancellation handles.
static CANCELLATION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn get_cancellation_registry() -> &'static Mutex<HashMap<u64, crate::cancellation::CancellationSource>> {
    CANCELLATION_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Create a new cancellation source.
///
/// Returns a handle that can be used to cancel operations and create tokens.
/// The handle must be freed with `cancellation_free` when no longer needed.
///
/// Returns 0 on error.
#[no_mangle]
pub extern "C" fn cancellation_create() -> CancellationHandle {
    let source = crate::cancellation::CancellationSource::new();
    let handle = CANCELLATION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let registry = get_cancellation_registry();
    let mut guard = registry.lock();
    guard.insert(handle, source);
    handle
}

/// Create a new cancellation source with a parent.
///
/// The child cancellation source is cancelled when either:
/// - The child is directly cancelled
/// - The parent is cancelled
///
/// Returns a handle, or 0 on error.
#[no_mangle]
pub extern "C" fn cancellation_create_child(parent: CancellationHandle) -> CancellationHandle {
    let registry = get_cancellation_registry();
    let parent_token = {
        let guard = registry.lock();
        match guard.get(&parent) {
            Some(source) => source.token(),
            None => return 0,
        }
    };

    let child_source = crate::cancellation::CancellationSource::with_parent(parent_token);
    let handle = CANCELLATION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let mut guard = registry.lock();
    guard.insert(handle, child_source);
    handle
}

/// Cancel a cancellation source.
///
/// All tokens derived from this source will report as cancelled.
#[no_mangle]
pub extern "C" fn cancellation_cancel(handle: CancellationHandle) {
    let registry = get_cancellation_registry();
    let guard = registry.lock();
    if let Some(source) = guard.get(&handle) {
        source.cancel();
    }
}

/// Cancel with a reason message.
///
/// # Safety
/// `reason` must be a valid BloodStr.
#[no_mangle]
pub unsafe extern "C" fn cancellation_cancel_with_reason(handle: CancellationHandle, reason: BloodStr) {
    let reason_str = if reason.ptr.is_null() || reason.len == 0 {
        None
    } else {
        let slice = std::slice::from_raw_parts(reason.ptr, reason.len as usize);
        std::str::from_utf8(slice).ok().map(|s| s.to_string())
    };

    let registry = get_cancellation_registry();
    let guard = registry.lock();
    if let Some(source) = guard.get(&handle) {
        source.cancel_with_reason(reason_str);
    }
}

/// Check if cancellation has been requested.
///
/// Returns 1 if cancelled, 0 if not cancelled.
#[no_mangle]
pub extern "C" fn cancellation_is_cancelled(handle: CancellationHandle) -> c_int {
    let registry = get_cancellation_registry();
    let guard = registry.lock();
    if let Some(source) = guard.get(&handle) {
        return if source.is_cancelled() { 1 } else { 0 };
    }
    0
}

/// Wait for cancellation.
///
/// Blocks until cancellation is requested or the timeout expires.
///
/// # Arguments
/// * `handle` - Cancellation handle
/// * `timeout_ms` - Timeout in milliseconds (0 = wait forever)
///
/// # Returns
/// * 1 if cancelled
/// * 0 if timeout expired
#[no_mangle]
pub extern "C" fn cancellation_wait(handle: CancellationHandle, timeout_ms: u64) -> c_int {
    use std::time::Duration;

    let token = {
        let registry = get_cancellation_registry();
        let guard = registry.lock();
        match guard.get(&handle) {
            Some(source) => source.token(),
            None => return 0,
        }
    };

    let timeout = if timeout_ms == 0 {
        Duration::from_secs(u64::MAX / 1000)
    } else {
        Duration::from_millis(timeout_ms)
    };

    if token.wait(timeout) { 1 } else { 0 }
}

/// Free a cancellation source.
///
/// The source is removed from the registry. Any tokens created from
/// this source remain valid but can no longer be cancelled through
/// this handle.
#[no_mangle]
pub extern "C" fn cancellation_free(handle: CancellationHandle) {
    let registry = get_cancellation_registry();
    let mut guard = registry.lock();
    guard.remove(&handle);
}

/// Cancel all registered cancellation sources.
///
/// This is typically called during shutdown to cancel all in-flight operations.
#[no_mangle]
pub extern "C" fn cancellation_cancel_all() {
    crate::cancellation::cancel_all();

    // Also cancel all sources in our registry
    let registry = get_cancellation_registry();
    let guard = registry.lock();
    for source in guard.values() {
        source.cancel();
    }
}

/// Get the number of active cancellation sources.
#[no_mangle]
pub extern "C" fn cancellation_active_count() -> u64 {
    let registry = get_cancellation_registry();
    let guard = registry.lock();
    guard.len() as u64
}

// ============================================================================
// Vec<T> Runtime Functions
// ============================================================================

// ============================================================================
// Memory Limits
// ============================================================================

/// Set the maximum heap size in bytes.
///
/// Allocations that would exceed this limit will fail.
/// Set to 0 for unlimited (default).
#[no_mangle]
pub extern "C" fn memory_set_limit(max_bytes: u64) {
    crate::memory::set_max_heap_size(max_bytes);
}

/// Get the current maximum heap size in bytes.
///
/// Returns 0 if unlimited.
#[no_mangle]
pub extern "C" fn memory_get_limit() -> u64 {
    crate::memory::max_heap_size()
}

/// Get current live memory usage in bytes.
#[no_mangle]
pub extern "C" fn memory_live_bytes() -> u64 {
    crate::memory::system_alloc_live_bytes()
}

/// Check if an allocation of the given size would be allowed.
///
/// Returns 1 if allowed, 0 if it would exceed the limit.
#[no_mangle]
pub extern "C" fn memory_check_allowed(size: u64) -> c_int {
    if crate::memory::check_allocation_allowed(size as usize) { 1 } else { 0 }
}

/// Memory usage summary for FFI.
#[repr(C)]
pub struct MemoryUsage {
    /// Total bytes allocated.
    pub allocated_bytes: u64,
    /// Total bytes freed.
    pub freed_bytes: u64,
    /// Currently live bytes (allocated - freed).
    pub live_bytes: u64,
    /// Total number of allocations.
    pub allocation_count: u64,
    /// Total number of frees.
    pub free_count: u64,
    /// Memory limit in bytes (0 = unlimited).
    pub limit_bytes: u64,
    /// Usage as a percentage of limit (0.0 - 100.0, 0 if unlimited).
    pub usage_percent: f64,
}

/// Get memory usage summary.
///
/// # Safety
/// `out` must be a valid pointer to uninitialized MemoryUsage memory.
#[no_mangle]
pub unsafe extern "C" fn memory_usage(out: *mut MemoryUsage) {
    if out.is_null() {
        return;
    }

    let summary = crate::memory::memory_usage_summary();
    (*out) = MemoryUsage {
        allocated_bytes: summary.allocated_bytes,
        freed_bytes: summary.freed_bytes,
        live_bytes: summary.live_bytes,
        allocation_count: summary.allocation_count,
        free_count: summary.free_count,
        limit_bytes: summary.limit_bytes.unwrap_or(0),
        usage_percent: summary.usage_percent,
    };
}

// ============================================================================
// Panic Handling
// ============================================================================

/// Panic info for FFI.
#[repr(C)]
pub struct PanicInfoHandle {
    /// Panic message (BloodStr).
    pub message_ptr: *const u8,
    /// Message length.
    pub message_len: u64,
    /// File name (BloodStr), null if unavailable.
    pub file_ptr: *const u8,
    /// File name length.
    pub file_len: u64,
    /// Line number (0 if unavailable).
    pub line: u32,
    /// Column number (0 if unavailable).
    pub column: u32,
    /// Panic count.
    pub count: u64,
    /// Thread ID.
    pub thread_id: u64,
}

/// Install the panic handler.
///
/// This installs a Rust panic hook that captures panic information.
/// Should be called during runtime initialization.
#[no_mangle]
pub extern "C" fn panic_install_handler() {
    crate::panic::install_panic_handler();
}

/// Get the number of panics that have occurred.
#[no_mangle]
pub extern "C" fn panic_count() -> u64 {
    crate::panic::panic_count()
}

/// Check if a panic has occurred.
///
/// Returns 1 if at least one panic has occurred, 0 otherwise.
#[no_mangle]
pub extern "C" fn panic_has_occurred() -> c_int {
    if crate::panic::panic_count() > 0 { 1 } else { 0 }
}

/// Get the last panic message.
///
/// Returns a BloodStr containing the message, or empty if no panic has occurred.
#[no_mangle]
pub extern "C" fn panic_last_message() -> BloodStr {
    match crate::panic::last_panic() {
        Some(info) => string_to_blood_str(info.message().to_string()),
        None => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

/// Get the last panic backtrace.
///
/// Returns a BloodStr containing the backtrace, or empty if unavailable.
#[no_mangle]
pub extern "C" fn panic_last_backtrace() -> BloodStr {
    match crate::panic::last_panic() {
        Some(info) => match info.backtrace() {
            Some(bt) => string_to_blood_str(bt.to_string()),
            None => BloodStr { ptr: std::ptr::null(), len: 0 },
        },
        None => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

/// Capture a backtrace at the current location.
///
/// Returns a BloodStr containing the backtrace, or empty if unavailable.
/// The returned string must be freed with `blood_str_free`.
#[no_mangle]
pub extern "C" fn backtrace_capture() -> BloodStr {
    match crate::panic::capture_backtrace() {
        Some(bt) => string_to_blood_str(bt),
        None => BloodStr { ptr: std::ptr::null(), len: 0 },
    }
}

/// Check if backtraces are available.
///
/// Returns 1 if backtraces can be captured, 0 otherwise.
/// Backtraces require RUST_BACKTRACE=1 environment variable.
#[no_mangle]
pub extern "C" fn backtrace_available() -> c_int {
    if crate::panic::backtraces_available() { 1 } else { 0 }
}

/// Callback type for panic hooks.
pub type PanicCallback = extern "C" fn(*const PanicInfoHandle);

/// Panic hook registry for FFI callbacks.
static PANIC_CALLBACK: std::sync::OnceLock<Mutex<Option<PanicCallback>>> = std::sync::OnceLock::new();

fn get_panic_callback() -> &'static Mutex<Option<PanicCallback>> {
    PANIC_CALLBACK.get_or_init(|| Mutex::new(None))
}

/// Register a panic callback.
///
/// The callback will be invoked when a panic occurs.
/// Only one callback can be registered at a time; calling this
/// replaces any previously registered callback.
///
/// # Safety
/// The callback must be a valid function pointer that remains valid
/// for the lifetime of the program.
#[no_mangle]
pub unsafe extern "C" fn panic_set_callback(callback: PanicCallback) {
    // Store the callback
    {
        let mut guard = get_panic_callback().lock();
        *guard = Some(callback);
    }

    // Register a Rust hook that invokes the FFI callback
    crate::panic::set_panic_hook(move |info| {
        let callback = {
            let guard = get_panic_callback().lock();
            *guard
        };

        if let Some(cb) = callback {
            // Convert to FFI struct
            let message = info.message();
            let (file_ptr, file_len) = match info.location() {
                Some(loc) => (loc.file.as_ptr(), loc.file.len() as u64),
                None => (std::ptr::null(), 0),
            };
            let (line, column) = match info.location() {
                Some(loc) => (loc.line, loc.column),
                None => (0, 0),
            };

            let handle = PanicInfoHandle {
                message_ptr: message.as_ptr(),
                message_len: message.len() as u64,
                file_ptr,
                file_len,
                line,
                column,
                count: info.count(),
                thread_id: info.thread_id(),
            };

            cb(&handle);
        }
    });
}

/// Clear the panic callback.
#[no_mangle]
pub extern "C" fn panic_clear_callback() {
    let mut guard = get_panic_callback().lock();
    *guard = None;
    crate::panic::clear_panic_hooks();
}

// ============================================================================
// Panic Recovery (catch_panic)
// ============================================================================

/// Result of a panic catch operation.
#[repr(C)]
pub struct CatchPanicResult {
    /// 0 = success (no panic), 1 = panic occurred
    pub panicked: c_int,
    /// Panic message (only valid if panicked == 1)
    pub message_ptr: *const u8,
    /// Panic message length
    pub message_len: u64,
    /// Backtrace (may be null)
    pub backtrace_ptr: *const u8,
    /// Backtrace length
    pub backtrace_len: u64,
}

/// Function pointer type for the closure to execute.
///
/// Takes a user-provided context pointer and returns:
/// - 0 for success
/// - non-zero for failure (panic will be caught and reported)
pub type CatchPanicFn = extern "C" fn(*mut std::ffi::c_void) -> c_int;

/// Execute a function with panic catching.
///
/// This allows Blood programs to catch panics and recover from them.
///
/// # Arguments
/// * `func` - Function to execute
/// * `context` - User context passed to the function
/// * `result` - Output buffer for the result
///
/// # Returns
/// * 0 if the function completed (check result.panicked for panic info)
/// * -1 if recovery is not available
///
/// # Safety
/// * `func` must be a valid function pointer
/// * `context` must be valid for the duration of the call
/// * `result` must be a valid pointer to uninitialized CatchPanicResult
#[no_mangle]
pub unsafe extern "C" fn panic_catch(
    func: CatchPanicFn,
    context: *mut std::ffi::c_void,
    result: *mut CatchPanicResult,
) -> c_int {
    if !crate::panic::recovery_available() {
        return -1;
    }

    // Wrap the extern "C" function call
    let catch_result = crate::panic::catch_panic_unchecked(|| {
        func(context)
    });

    match catch_result {
        crate::panic::CatchResult::Ok(_) => {
            (*result).panicked = 0;
            (*result).message_ptr = std::ptr::null();
            (*result).message_len = 0;
            (*result).backtrace_ptr = std::ptr::null();
            (*result).backtrace_len = 0;
        }
        crate::panic::CatchResult::Panicked(info) => {
            (*result).panicked = 1;

            // Allocate and copy the message
            let message = info.message().to_string();
            let message_bytes = message.into_bytes().into_boxed_slice();
            (*result).message_len = message_bytes.len() as u64;
            (*result).message_ptr = Box::into_raw(message_bytes) as *const u8;

            // Allocate and copy the backtrace if available
            if let Some(bt) = info.backtrace() {
                let backtrace_bytes = bt.to_string().into_bytes().into_boxed_slice();
                (*result).backtrace_len = backtrace_bytes.len() as u64;
                (*result).backtrace_ptr = Box::into_raw(backtrace_bytes) as *const u8;
            } else {
                (*result).backtrace_ptr = std::ptr::null();
                (*result).backtrace_len = 0;
            }
        }
    }

    0
}

/// Free memory allocated by panic_catch.
///
/// # Safety
/// * `result` must be a result returned by `panic_catch`
/// * Must only be called once per result
#[no_mangle]
pub unsafe extern "C" fn panic_catch_result_free(result: *mut CatchPanicResult) {
    if result.is_null() {
        return;
    }

    // Free message if allocated
    if !(*result).message_ptr.is_null() && (*result).message_len > 0 {
        let slice = std::slice::from_raw_parts_mut(
            (*result).message_ptr as *mut u8,
            (*result).message_len as usize,
        );
        let _ = Box::from_raw(slice);
    }

    // Free backtrace if allocated
    if !(*result).backtrace_ptr.is_null() && (*result).backtrace_len > 0 {
        let slice = std::slice::from_raw_parts_mut(
            (*result).backtrace_ptr as *mut u8,
            (*result).backtrace_len as usize,
        );
        let _ = Box::from_raw(slice);
    }

    // Reset the struct
    (*result).panicked = 0;
    (*result).message_ptr = std::ptr::null();
    (*result).message_len = 0;
    (*result).backtrace_ptr = std::ptr::null();
    (*result).backtrace_len = 0;
}

/// Check if panic recovery is available.
///
/// Returns 1 if catch_panic will work, 0 if panics cannot be caught
/// (e.g., compiled with panic=abort).
#[no_mangle]
pub extern "C" fn panic_recovery_available() -> c_int {
    if crate::panic::recovery_available() { 1 } else { 0 }
}

// ============================================================================
// Logging Infrastructure
// ============================================================================

/// Log level constants.
pub mod log_level {
    /// Trace level (most verbose).
    pub const TRACE: u8 = 0;
    /// Debug level.
    pub const DEBUG: u8 = 1;
    /// Info level.
    pub const INFO: u8 = 2;
    /// Warning level.
    pub const WARN: u8 = 3;
    /// Error level.
    pub const ERROR: u8 = 4;
    /// Off (no logging).
    pub const OFF: u8 = 5;
}

/// Log format constants.
pub mod log_format {
    /// Plain text format.
    pub const PLAIN: u8 = 0;
    /// JSON format.
    pub const JSON: u8 = 1;
}

/// Initialize the logger.
#[no_mangle]
pub extern "C" fn log_init() {
    crate::log::init();
}

/// Initialize the logger with a specific level.
///
/// # Arguments
/// * `level` - Log level (0=TRACE, 1=DEBUG, 2=INFO, 3=WARN, 4=ERROR, 5=OFF)
#[no_mangle]
pub extern "C" fn log_init_with_level(level: u8) {
    if let Some(log_level) = crate::log::LogLevel::from_u8(level) {
        crate::log::init_with_level(log_level);
    }
}

/// Set the minimum log level.
///
/// # Arguments
/// * `level` - Log level (0=TRACE, 1=DEBUG, 2=INFO, 3=WARN, 4=ERROR, 5=OFF)
#[no_mangle]
pub extern "C" fn log_set_level(level: u8) {
    if let Some(log_level) = crate::log::LogLevel::from_u8(level) {
        crate::log::set_level(log_level);
    }
}

/// Get the current minimum log level.
///
/// Returns the level as u8 (0=TRACE, 1=DEBUG, 2=INFO, 3=WARN, 4=ERROR, 5=OFF).
#[no_mangle]
pub extern "C" fn log_get_level() -> u8 {
    crate::log::level() as u8
}

/// Set the output format.
///
/// # Arguments
/// * `format` - 0=PLAIN, 1=JSON
#[no_mangle]
pub extern "C" fn log_set_format(format: u8) {
    if let Some(log_format) = crate::log::LogFormat::from_u8(format) {
        crate::log::set_format(log_format);
    }
}

/// Enable or disable logging.
///
/// # Arguments
/// * `enabled` - 1 to enable, 0 to disable
#[no_mangle]
pub extern "C" fn log_set_enabled(enabled: c_int) {
    crate::log::set_enabled(enabled != 0);
}

/// Check if logging is enabled.
///
/// Returns 1 if enabled, 0 if disabled.
#[no_mangle]
pub extern "C" fn log_is_enabled() -> c_int {
    if crate::log::is_enabled() { 1 } else { 0 }
}

/// Check if a log level would be logged.
///
/// Returns 1 if the level would be logged, 0 otherwise.
#[no_mangle]
pub extern "C" fn log_would_log(level: u8) -> c_int {
    if let Some(log_level) = crate::log::LogLevel::from_u8(level) {
        if crate::log::would_log(log_level) { 1 } else { 0 }
    } else {
        0
    }
}

/// Log a message at the specified level.
///
/// # Arguments
/// * `level` - Log level
/// * `message_ptr` - Pointer to message bytes
/// * `message_len` - Message length
///
/// # Safety
/// `message_ptr` must be a valid pointer to `message_len` bytes of UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn log_message(level: u8, message_ptr: *const u8, message_len: u64) {
    let log_level = match crate::log::LogLevel::from_u8(level) {
        Some(l) => l,
        None => return,
    };

    if message_ptr.is_null() || message_len == 0 {
        return;
    }

    let message_bytes = std::slice::from_raw_parts(message_ptr, message_len as usize);
    let message = match std::str::from_utf8(message_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    crate::log::log(log_level, message);
}

/// Log a trace message.
///
/// # Safety
/// `message_ptr` must be a valid pointer to `message_len` bytes of UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn log_trace(message_ptr: *const u8, message_len: u64) {
    log_message(log_level::TRACE, message_ptr, message_len);
}

/// Log a debug message.
///
/// # Safety
/// `message_ptr` must be a valid pointer to `message_len` bytes of UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn log_debug(message_ptr: *const u8, message_len: u64) {
    log_message(log_level::DEBUG, message_ptr, message_len);
}

/// Log an info message.
///
/// # Safety
/// `message_ptr` must be a valid pointer to `message_len` bytes of UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn log_info(message_ptr: *const u8, message_len: u64) {
    log_message(log_level::INFO, message_ptr, message_len);
}

/// Log a warning message.
///
/// # Safety
/// `message_ptr` must be a valid pointer to `message_len` bytes of UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn log_warn(message_ptr: *const u8, message_len: u64) {
    log_message(log_level::WARN, message_ptr, message_len);
}

/// Log an error message.
///
/// # Safety
/// `message_ptr` must be a valid pointer to `message_len` bytes of UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn log_error(message_ptr: *const u8, message_len: u64) {
    log_message(log_level::ERROR, message_ptr, message_len);
}

/// Handle for a log entry being built.
#[repr(C)]
pub struct LogEntryHandle {
    /// Opaque pointer to the LogBuilder.
    ptr: *mut std::ffi::c_void,
}

/// Create a new log entry builder.
///
/// Returns a handle to a log entry being built.
/// The handle must be freed with `log_entry_emit` or `log_entry_drop`.
#[no_mangle]
pub extern "C" fn log_entry_new(level: u8) -> LogEntryHandle {
    let log_level = crate::log::LogLevel::from_u8(level).unwrap_or(crate::log::LogLevel::Info);
    let builder = Box::new(crate::log::LogBuilder::new(log_level));
    LogEntryHandle {
        ptr: Box::into_raw(builder) as *mut std::ffi::c_void,
    }
}

/// Set the message on a log entry.
///
/// # Safety
/// * `handle` must be a valid handle from `log_entry_new`
/// * `message_ptr` must be valid UTF-8 data
#[no_mangle]
pub unsafe extern "C" fn log_entry_set_message(
    handle: *mut LogEntryHandle,
    message_ptr: *const u8,
    message_len: u64,
) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let message = if message_ptr.is_null() || message_len == 0 {
        String::new()
    } else {
        let bytes = std::slice::from_raw_parts(message_ptr, message_len as usize);
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    };

    // Take ownership, modify, put back
    let builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    let builder = builder.message(message);
    (*handle).ptr = Box::into_raw(Box::new(builder)) as *mut std::ffi::c_void;
}

/// Set the target on a log entry.
///
/// # Safety
/// * `handle` must be a valid handle from `log_entry_new`
/// * `target_ptr` must be valid UTF-8 data
#[no_mangle]
pub unsafe extern "C" fn log_entry_set_target(
    handle: *mut LogEntryHandle,
    target_ptr: *const u8,
    target_len: u64,
) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let target = if target_ptr.is_null() || target_len == 0 {
        return;
    } else {
        let bytes = std::slice::from_raw_parts(target_ptr, target_len as usize);
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    };

    let builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    let builder = builder.target(target);
    (*handle).ptr = Box::into_raw(Box::new(builder)) as *mut std::ffi::c_void;
}

/// Add a string field to a log entry.
///
/// # Safety
/// All pointers must be valid UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn log_entry_add_field_str(
    handle: *mut LogEntryHandle,
    key_ptr: *const u8,
    key_len: u64,
    value_ptr: *const u8,
    value_len: u64,
) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let key = if key_ptr.is_null() || key_len == 0 {
        return;
    } else {
        let bytes = std::slice::from_raw_parts(key_ptr, key_len as usize);
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    };

    let value = if value_ptr.is_null() || value_len == 0 {
        String::new()
    } else {
        let bytes = std::slice::from_raw_parts(value_ptr, value_len as usize);
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    };

    let builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    let builder = builder.field_str(key, value);
    (*handle).ptr = Box::into_raw(Box::new(builder)) as *mut std::ffi::c_void;
}

/// Add an integer field to a log entry.
///
/// # Safety
/// `handle` must be a valid handle, `key_ptr` must be valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn log_entry_add_field_int(
    handle: *mut LogEntryHandle,
    key_ptr: *const u8,
    key_len: u64,
    value: i64,
) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let key = if key_ptr.is_null() || key_len == 0 {
        return;
    } else {
        let bytes = std::slice::from_raw_parts(key_ptr, key_len as usize);
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    };

    let builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    let builder = builder.field_int(key, value);
    (*handle).ptr = Box::into_raw(Box::new(builder)) as *mut std::ffi::c_void;
}

/// Add a float field to a log entry.
///
/// # Safety
/// `handle` must be a valid handle, `key_ptr` must be valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn log_entry_add_field_float(
    handle: *mut LogEntryHandle,
    key_ptr: *const u8,
    key_len: u64,
    value: f64,
) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let key = if key_ptr.is_null() || key_len == 0 {
        return;
    } else {
        let bytes = std::slice::from_raw_parts(key_ptr, key_len as usize);
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    };

    let builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    let builder = builder.field_float(key, value);
    (*handle).ptr = Box::into_raw(Box::new(builder)) as *mut std::ffi::c_void;
}

/// Add a boolean field to a log entry.
///
/// # Safety
/// `handle` must be a valid handle, `key_ptr` must be valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn log_entry_add_field_bool(
    handle: *mut LogEntryHandle,
    key_ptr: *const u8,
    key_len: u64,
    value: c_int,
) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let key = if key_ptr.is_null() || key_len == 0 {
        return;
    } else {
        let bytes = std::slice::from_raw_parts(key_ptr, key_len as usize);
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        }
    };

    let builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    let builder = builder.field_bool(key, value != 0);
    (*handle).ptr = Box::into_raw(Box::new(builder)) as *mut std::ffi::c_void;
}

/// Emit a log entry.
///
/// This emits the log entry and frees the handle.
///
/// # Safety
/// `handle` must be a valid handle from `log_entry_new`.
#[no_mangle]
pub unsafe extern "C" fn log_entry_emit(handle: *mut LogEntryHandle) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    builder.emit();
    (*handle).ptr = std::ptr::null_mut();
}

/// Drop a log entry without emitting.
///
/// # Safety
/// `handle` must be a valid handle from `log_entry_new`.
#[no_mangle]
pub unsafe extern "C" fn log_entry_drop(handle: *mut LogEntryHandle) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let _builder = Box::from_raw((*handle).ptr as *mut crate::log::LogBuilder);
    // Drop without emitting
    (*handle).ptr = std::ptr::null_mut();
}

// ============================================================================
// Subprocess Management
// ============================================================================

/// Handle to a child process.
#[repr(C)]
pub struct ProcessHandle {
    /// Opaque pointer to the Child struct.
    ptr: *mut std::ffi::c_void,
    /// Process ID assigned by the runtime.
    id: u64,
    /// OS process ID.
    pid: u32,
}

/// Process output structure.
#[repr(C)]
pub struct ProcessOutput {
    /// Exit code (-1 if terminated by signal).
    exit_code: i32,
    /// Signal that terminated the process (0 if exited normally).
    signal: i32,
    /// Pointer to stdout data.
    stdout_ptr: *mut u8,
    /// Length of stdout data.
    stdout_len: u64,
    /// Pointer to stderr data.
    stderr_ptr: *mut u8,
    /// Length of stderr data.
    stderr_len: u64,
}

/// Stdio configuration constants.
pub mod stdio_config {
    /// Inherit from parent.
    pub const INHERIT: u8 = 0;
    /// Redirect to null.
    pub const NULL: u8 = 1;
    /// Create a pipe.
    pub const PIPED: u8 = 2;
}

/// Create a new command builder.
///
/// # Arguments
/// * `program_ptr` - Pointer to program name bytes
/// * `program_len` - Length of program name
///
/// # Returns
/// Opaque pointer to Command, or null on error.
///
/// # Safety
/// `program_ptr` must be valid UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn process_command_new(
    program_ptr: *const u8,
    program_len: u64,
) -> *mut std::ffi::c_void {
    if program_ptr.is_null() || program_len == 0 {
        return std::ptr::null_mut();
    }

    let program_bytes = std::slice::from_raw_parts(program_ptr, program_len as usize);
    let program = match std::str::from_utf8(program_bytes) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let cmd = Box::new(crate::process::Command::new(program));
    Box::into_raw(cmd) as *mut std::ffi::c_void
}

/// Add an argument to a command.
///
/// # Safety
/// * `cmd` must be a valid command pointer
/// * `arg_ptr` must be valid UTF-8 data
#[no_mangle]
pub unsafe extern "C" fn process_command_arg(
    cmd: *mut std::ffi::c_void,
    arg_ptr: *const u8,
    arg_len: u64,
) {
    if cmd.is_null() || arg_ptr.is_null() {
        return;
    }

    let arg_bytes = std::slice::from_raw_parts(arg_ptr, arg_len as usize);
    let arg = match std::str::from_utf8(arg_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let command = &mut *(cmd as *mut crate::process::Command);
    command.arg(arg);
}

/// Set an environment variable for a command.
///
/// # Safety
/// * `cmd` must be a valid command pointer
/// * `key_ptr` and `val_ptr` must be valid UTF-8 data
#[no_mangle]
pub unsafe extern "C" fn process_command_env(
    cmd: *mut std::ffi::c_void,
    key_ptr: *const u8,
    key_len: u64,
    val_ptr: *const u8,
    val_len: u64,
) {
    if cmd.is_null() || key_ptr.is_null() {
        return;
    }

    let key_bytes = std::slice::from_raw_parts(key_ptr, key_len as usize);
    let key = match std::str::from_utf8(key_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let val = if val_ptr.is_null() || val_len == 0 {
        ""
    } else {
        let val_bytes = std::slice::from_raw_parts(val_ptr, val_len as usize);
        match std::str::from_utf8(val_bytes) {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    let command = &mut *(cmd as *mut crate::process::Command);
    command.env(key, val);
}

/// Set the working directory for a command.
///
/// # Safety
/// * `cmd` must be a valid command pointer
/// * `dir_ptr` must be valid UTF-8 data
#[no_mangle]
pub unsafe extern "C" fn process_command_current_dir(
    cmd: *mut std::ffi::c_void,
    dir_ptr: *const u8,
    dir_len: u64,
) {
    if cmd.is_null() || dir_ptr.is_null() {
        return;
    }

    let dir_bytes = std::slice::from_raw_parts(dir_ptr, dir_len as usize);
    let dir = match std::str::from_utf8(dir_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let command = &mut *(cmd as *mut crate::process::Command);
    command.current_dir(dir);
}

/// Set stdin configuration for a command.
///
/// # Arguments
/// * `cfg` - 0=INHERIT, 1=NULL, 2=PIPED
///
/// # Safety
/// `cmd` must be a valid command pointer.
#[no_mangle]
pub unsafe extern "C" fn process_command_stdin(cmd: *mut std::ffi::c_void, cfg: u8) {
    if cmd.is_null() {
        return;
    }

    let stdio = match cfg {
        0 => crate::process::Stdio::Inherit,
        1 => crate::process::Stdio::Null,
        2 => crate::process::Stdio::Piped,
        _ => return,
    };

    let command = &mut *(cmd as *mut crate::process::Command);
    command.stdin(stdio);
}

/// Set stdout configuration for a command.
///
/// # Safety
/// `cmd` must be a valid command pointer.
#[no_mangle]
pub unsafe extern "C" fn process_command_stdout(cmd: *mut std::ffi::c_void, cfg: u8) {
    if cmd.is_null() {
        return;
    }

    let stdio = match cfg {
        0 => crate::process::Stdio::Inherit,
        1 => crate::process::Stdio::Null,
        2 => crate::process::Stdio::Piped,
        _ => return,
    };

    let command = &mut *(cmd as *mut crate::process::Command);
    command.stdout(stdio);
}

/// Set stderr configuration for a command.
///
/// # Safety
/// `cmd` must be a valid command pointer.
#[no_mangle]
pub unsafe extern "C" fn process_command_stderr(cmd: *mut std::ffi::c_void, cfg: u8) {
    if cmd.is_null() {
        return;
    }

    let stdio = match cfg {
        0 => crate::process::Stdio::Inherit,
        1 => crate::process::Stdio::Null,
        2 => crate::process::Stdio::Piped,
        _ => return,
    };

    let command = &mut *(cmd as *mut crate::process::Command);
    command.stderr(stdio);
}

/// Spawn a command as a child process.
///
/// # Returns
/// ProcessHandle with valid ptr on success, null ptr on failure.
///
/// # Safety
/// `cmd` must be a valid command pointer. The command is consumed.
#[no_mangle]
pub unsafe extern "C" fn process_spawn(cmd: *mut std::ffi::c_void) -> ProcessHandle {
    if cmd.is_null() {
        return ProcessHandle {
            ptr: std::ptr::null_mut(),
            id: 0,
            pid: 0,
        };
    }

    let mut command = Box::from_raw(cmd as *mut crate::process::Command);

    match command.spawn() {
        Ok(child) => {
            let id = child.id();
            let pid = child.pid();
            ProcessHandle {
                ptr: Box::into_raw(Box::new(child)) as *mut std::ffi::c_void,
                id,
                pid,
            }
        }
        Err(_) => ProcessHandle {
            ptr: std::ptr::null_mut(),
            id: 0,
            pid: 0,
        },
    }
}

/// Execute a command and wait for it to complete.
///
/// # Returns
/// Exit code on success, -1 on error.
///
/// # Safety
/// `cmd` must be a valid command pointer. The command is consumed.
#[no_mangle]
pub unsafe extern "C" fn process_run(cmd: *mut std::ffi::c_void) -> i32 {
    if cmd.is_null() {
        return -1;
    }

    let mut command = Box::from_raw(cmd as *mut crate::process::Command);

    match command.status() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Execute a command and capture its output.
///
/// # Safety
/// * `cmd` must be a valid command pointer. The command is consumed.
/// * `output` must be a valid pointer to uninitialized ProcessOutput.
#[no_mangle]
pub unsafe extern "C" fn process_output(
    cmd: *mut std::ffi::c_void,
    output: *mut ProcessOutput,
) -> c_int {
    if cmd.is_null() || output.is_null() {
        return -1;
    }

    let mut command = Box::from_raw(cmd as *mut crate::process::Command);

    match command.output() {
        Ok(result) => {
            (*output).exit_code = result.status.code().unwrap_or(-1);

            #[cfg(unix)]
            {
                (*output).signal = result.status.signal().unwrap_or(0);
            }
            #[cfg(not(unix))]
            {
                (*output).signal = 0;
            }

            // Allocate and copy stdout
            if result.stdout.is_empty() {
                (*output).stdout_ptr = std::ptr::null_mut();
                (*output).stdout_len = 0;
            } else {
                let stdout = result.stdout.into_boxed_slice();
                (*output).stdout_len = stdout.len() as u64;
                (*output).stdout_ptr = Box::into_raw(stdout) as *mut u8;
            }

            // Allocate and copy stderr
            if result.stderr.is_empty() {
                (*output).stderr_ptr = std::ptr::null_mut();
                (*output).stderr_len = 0;
            } else {
                let stderr = result.stderr.into_boxed_slice();
                (*output).stderr_len = stderr.len() as u64;
                (*output).stderr_ptr = Box::into_raw(stderr) as *mut u8;
            }

            0
        }
        Err(_) => -1,
    }
}

/// Free memory allocated by process_output.
///
/// # Safety
/// `output` must be a result from `process_output`.
#[no_mangle]
pub unsafe extern "C" fn process_output_free(output: *mut ProcessOutput) {
    if output.is_null() {
        return;
    }

    if !(*output).stdout_ptr.is_null() && (*output).stdout_len > 0 {
        let _ = Box::from_raw(std::slice::from_raw_parts_mut(
            (*output).stdout_ptr,
            (*output).stdout_len as usize,
        ));
    }

    if !(*output).stderr_ptr.is_null() && (*output).stderr_len > 0 {
        let _ = Box::from_raw(std::slice::from_raw_parts_mut(
            (*output).stderr_ptr,
            (*output).stderr_len as usize,
        ));
    }

    (*output).stdout_ptr = std::ptr::null_mut();
    (*output).stdout_len = 0;
    (*output).stderr_ptr = std::ptr::null_mut();
    (*output).stderr_len = 0;
}

/// Wait for a child process to complete.
///
/// # Returns
/// Exit code on success, -1 on error.
///
/// # Safety
/// `handle` must be a valid process handle.
#[no_mangle]
pub unsafe extern "C" fn process_wait(handle: *mut ProcessHandle) -> i32 {
    if handle.is_null() || (*handle).ptr.is_null() {
        return -1;
    }

    let child = &mut *((*handle).ptr as *mut crate::process::Child);

    match child.wait() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Check if a child process has exited (non-blocking).
///
/// # Returns
/// * >= 0: Exit code (process has exited)
/// * -1: Process still running
/// * -2: Error
///
/// # Safety
/// `handle` must be a valid process handle.
#[no_mangle]
pub unsafe extern "C" fn process_try_wait(handle: *mut ProcessHandle) -> i32 {
    if handle.is_null() || (*handle).ptr.is_null() {
        return -2;
    }

    let child = &mut *((*handle).ptr as *mut crate::process::Child);

    match child.try_wait() {
        Ok(Some(status)) => status.code().unwrap_or(-2),
        Ok(None) => -1, // Still running
        Err(_) => -2,
    }
}

/// Kill a child process.
///
/// # Returns
/// 0 on success, -1 on error.
///
/// # Safety
/// `handle` must be a valid process handle.
#[no_mangle]
pub unsafe extern "C" fn process_kill(handle: *mut ProcessHandle) -> c_int {
    if handle.is_null() || (*handle).ptr.is_null() {
        return -1;
    }

    let child = &mut *((*handle).ptr as *mut crate::process::Child);

    match child.kill() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Free a process handle.
///
/// # Safety
/// `handle` must be a valid process handle.
#[no_mangle]
pub unsafe extern "C" fn process_free(handle: *mut ProcessHandle) {
    if handle.is_null() || (*handle).ptr.is_null() {
        return;
    }

    let _ = Box::from_raw((*handle).ptr as *mut crate::process::Child);
    (*handle).ptr = std::ptr::null_mut();
}

/// Free a command without executing it.
///
/// # Safety
/// `cmd` must be a valid command pointer.
#[no_mangle]
pub unsafe extern "C" fn process_command_free(cmd: *mut std::ffi::c_void) {
    if cmd.is_null() {
        return;
    }

    let _ = Box::from_raw(cmd as *mut crate::process::Command);
}

/// Check if a program exists in PATH.
///
/// # Returns
/// 1 if found, 0 if not found.
///
/// # Safety
/// `program_ptr` must be valid UTF-8 data.
#[no_mangle]
pub unsafe extern "C" fn process_which(program_ptr: *const u8, program_len: u64) -> c_int {
    if program_ptr.is_null() || program_len == 0 {
        return 0;
    }

    let program_bytes = std::slice::from_raw_parts(program_ptr, program_len as usize);
    let program = match std::str::from_utf8(program_bytes) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    if crate::process::which(program).is_some() {
        1
    } else {
        0
    }
}

/// Get the current process ID.
#[no_mangle]
pub extern "C" fn process_current_pid() -> u32 {
    crate::process::current_pid()
}

/// Execute a shell command and capture output.
///
/// # Safety
/// * `command_ptr` must be valid UTF-8 data
/// * `output` must be a valid pointer
#[no_mangle]
pub unsafe extern "C" fn process_shell(
    command_ptr: *const u8,
    command_len: u64,
    output: *mut ProcessOutput,
) -> c_int {
    if command_ptr.is_null() || command_len == 0 || output.is_null() {
        return -1;
    }

    let command_bytes = std::slice::from_raw_parts(command_ptr, command_len as usize);
    let command = match std::str::from_utf8(command_bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    match crate::process::shell(command) {
        Ok(result) => {
            (*output).exit_code = result.status.code().unwrap_or(-1);

            #[cfg(unix)]
            {
                (*output).signal = result.status.signal().unwrap_or(0);
            }
            #[cfg(not(unix))]
            {
                (*output).signal = 0;
            }

            if result.stdout.is_empty() {
                (*output).stdout_ptr = std::ptr::null_mut();
                (*output).stdout_len = 0;
            } else {
                let stdout = result.stdout.into_boxed_slice();
                (*output).stdout_len = stdout.len() as u64;
                (*output).stdout_ptr = Box::into_raw(stdout) as *mut u8;
            }

            if result.stderr.is_empty() {
                (*output).stderr_ptr = std::ptr::null_mut();
                (*output).stderr_len = 0;
            } else {
                let stderr = result.stderr.into_boxed_slice();
                (*output).stderr_len = stderr.len() as u64;
                (*output).stderr_ptr = Box::into_raw(stderr) as *mut u8;
            }

            0
        }
        Err(_) => -1,
    }
}

// ============================================================================
// Vec<T> Runtime Functions
// ============================================================================

/// Blood Vec representation.
/// Layout: { ptr: *void, len: i64, capacity: i64 }
#[repr(C)]
pub struct BloodVec {
    /// Pointer to element data.
    pub ptr: *mut u8,
    /// Number of elements.
    pub len: i64,
    /// Capacity (number of elements that can be stored).
    pub capacity: i64,
}

/// Create a new empty Vec.
///
/// # Arguments
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer to write the Vec struct to
///
/// # Safety
/// `out` must be a valid pointer to uninitialized BloodVec memory.
#[no_mangle]
pub unsafe extern "C" fn vec_new(elem_size: i64, out: *mut BloodVec) {
    // elem_size is stored implicitly - operations must pass it each time
    let _ = elem_size; // silence unused warning
    (*out).ptr = std::ptr::null_mut();
    (*out).len = 0;
    (*out).capacity = 0;
}

/// Create a new Vec with the given capacity.
///
/// # Arguments
/// * `elem_size` - Size of each element in bytes
/// * `capacity` - Initial capacity
/// * `out` - Output buffer to write the Vec struct to
///
/// # Safety
/// `out` must be a valid pointer to uninitialized BloodVec memory.
#[no_mangle]
pub unsafe extern "C" fn vec_with_capacity(elem_size: i64, capacity: i64, out: *mut BloodVec) {
    let ptr = if capacity > 0 {
        // Use 16-byte alignment to support enums with i128 payloads
        // which require 16-byte alignment for correct memory access
        let layout = std::alloc::Layout::from_size_align(
            (capacity * elem_size) as usize,
            16,
        ).unwrap();
        runtime_alloc(layout)
    } else {
        std::ptr::null_mut()
    };

    (*out).ptr = ptr;
    (*out).len = 0;
    (*out).capacity = capacity;
}

/// Get the length of a Vec.
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
#[no_mangle]
pub unsafe extern "C" fn vec_len(vec: *const BloodVec) -> i64 {
    if vec.is_null() {
        return 0;
    }
    (*vec).len
}

/// DEBUG: Dump Vec element discriminants for debugging enum corruption.
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
#[no_mangle]
pub unsafe extern "C" fn vec_debug_dump(vec: *const BloodVec, elem_size: i64, label: *const u8) {
    if vec.is_null() {
        eprintln!("[DEBUG vec_dump] null vec");
        return;
    }
    let v = &*vec;
    let label_str = if label.is_null() {
        "unnamed"
    } else {
        std::ffi::CStr::from_ptr(label as *const i8).to_str().unwrap_or("invalid")
    };
    eprintln!("[DEBUG vec_dump] {} ptr={:p} len={} cap={} elem_size={}",
        label_str, v.ptr, v.len, v.capacity, elem_size);
    if !v.ptr.is_null() && v.len > 0 && elem_size >= 4 {
        for i in 0..v.len.min(10) {
            let elem_ptr = v.ptr.add((i * elem_size) as usize);
            let tag = std::ptr::read(elem_ptr as *const i32);
            eprintln!("  [{}] tag={} addr={:p}", i, tag, elem_ptr);
        }
    }
}

/// Check if a Vec is empty.
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
#[no_mangle]
pub unsafe extern "C" fn vec_is_empty(vec: *const BloodVec) -> i32 {
    if vec.is_null() {
        return 1; // null vec is empty
    }
    if (*vec).len == 0 { 1 } else { 0 }
}

/// Get the capacity of a Vec.
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
#[no_mangle]
pub unsafe extern "C" fn vec_capacity(vec: *const BloodVec) -> i64 {
    if vec.is_null() {
        return 0;
    }
    (*vec).capacity
}

/// Push an element onto the Vec.
///
/// # Arguments
/// * `vec` - Pointer to the BloodVec
/// * `elem` - Pointer to the element to push (will be copied)
/// * `elem_size` - Size of the element in bytes
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
/// `elem` must be valid for `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn vec_push(vec: *mut BloodVec, elem: *const u8, elem_size: i64) {
    if vec.is_null() || elem.is_null() {
        return;
    }

    let v = &mut *vec;

    // Check if we need to grow
    if v.len >= v.capacity {
        let new_capacity = if v.capacity == 0 { 4 } else { v.capacity * 2 };
        let new_size = (new_capacity * elem_size) as usize;

        // Use 16-byte alignment to support enums with i128 payloads
        let new_ptr = if v.ptr.is_null() {
            let layout = std::alloc::Layout::from_size_align(new_size, 16).unwrap();
            runtime_alloc(layout)
        } else {
            let old_layout = std::alloc::Layout::from_size_align(
                (v.capacity * elem_size) as usize,
                16,
            ).unwrap();
            runtime_realloc(v.ptr, old_layout, new_size)
        };

        v.ptr = new_ptr;
        v.capacity = new_capacity;
    }

    // Copy element to the end
    let dest = v.ptr.add((v.len * elem_size) as usize);
    std::ptr::copy_nonoverlapping(elem, dest, elem_size as usize);
    v.len += 1;

    // Minimal debug: print only when self-hosted compiler is running to identify crash point
    if std::env::var("BLOOD_DEBUG_VEC").is_ok() {
        let tag = std::ptr::read(elem as *const i32);
        // Also read payload if we have enough size (assuming { i32, payload } layout)
        let payload = if elem_size >= 16 {
            std::ptr::read(elem.add(8) as *const i64)
        } else {
            -1
        };
        eprintln!("[vec_push] elem_size={} len={} tag={} payload={} dest={:p} src={:p} base={:p}",
            elem_size, v.len, tag, payload, dest, elem, v.ptr);
    }
}

/// Pop an element from the Vec.
///
/// # Arguments
/// * `vec` - Pointer to the BloodVec
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the popped element (must be at least elem_size bytes)
///
/// # Returns
/// 1 if an element was popped (Some), 0 if the vec was empty (None).
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
/// `out` must be valid for at least `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn vec_pop(vec: *mut BloodVec, elem_size: i64, out: *mut u8) -> i32 {
    if vec.is_null() {
        return 0;
    }

    let v = &mut *vec;

    if v.len == 0 {
        return 0; // None
    }

    v.len -= 1;

    // Copy the last element to the output buffer
    if !out.is_null() {
        let src = v.ptr.add((v.len * elem_size) as usize);
        std::ptr::copy_nonoverlapping(src, out, elem_size as usize);
    }

    1 // Some
}

/// Get an element from the Vec by index.
///
/// Returns Option<&T> as a struct: { tag: i32, ptr: *T }
/// - tag = 0 for None (out of bounds)
/// - tag = 1 for Some, with ptr pointing to the element in the Vec
///
/// # Arguments
/// * `vec` - Pointer to the BloodVec
/// * `index` - Index of the element to get
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the Option struct (tag + pointer)
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn vec_get(
    vec: *const BloodVec,
    index: i64,
    elem_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if vec.is_null() {
        // None
        *(out as *mut i32) = 0;
        return;
    }

    let v = &*vec;

    if index < 0 || index >= v.len {
        // None - out of bounds
        *(out as *mut i32) = 0;
        return;
    }

    // Some - write tag and pointer to element
    *(out as *mut i32) = 1;
    let ptr_offset = 8usize; // Pointer alignment (tag is 4 bytes, pad to 8)
    let elem_ptr = v.ptr.add((index * elem_size) as usize);
    *(out.add(ptr_offset) as *mut *const u8) = elem_ptr;
}

/// Get a pointer to an element in the Vec by index (for indexing operator).
///
/// # Arguments
/// * `vec` - Pointer to the BloodVec
/// * `index` - Index of the element to get
/// * `elem_size` - Size of each element in bytes
///
/// # Returns
/// Pointer to the element, or null if out of bounds.
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
#[no_mangle]
pub unsafe extern "C" fn vec_get_ptr(
    vec: *const BloodVec,
    index: i64,
    elem_size: i64,
) -> *mut u8 {
    if vec.is_null() {
        return std::ptr::null_mut();
    }

    let v = &*vec;

    if index < 0 || index >= v.len {
        eprintln!("Vec index out of bounds: index {} but len is {}", index, v.len);
        std::process::abort();
    }

    v.ptr.add((index * elem_size) as usize)
}

/// Check if the Vec contains an element.
///
/// # Arguments
/// * `vec` - Pointer to the BloodVec
/// * `elem` - Pointer to the element to search for
/// * `elem_size` - Size of each element in bytes
///
/// # Returns
/// 1 if found, 0 if not found.
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn vec_contains(
    vec: *const BloodVec,
    elem: *const u8,
    elem_size: i64,
) -> i32 {
    if vec.is_null() || elem.is_null() {
        return 0;
    }

    let v = &*vec;

    for i in 0..v.len {
        let item_ptr = v.ptr.add((i * elem_size) as usize);
        // Byte-by-byte comparison
        let mut matches = true;
        for j in 0..elem_size as usize {
            if *item_ptr.add(j) != *elem.add(j) {
                matches = false;
                break;
            }
        }
        if matches {
            return 1;
        }
    }

    0
}

/// Reverse the Vec in place.
///
/// # Arguments
/// * `vec` - Pointer to the BloodVec
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
#[no_mangle]
pub unsafe extern "C" fn vec_reverse(vec: *mut BloodVec, elem_size: i64) {
    if vec.is_null() {
        return;
    }

    let v = &mut *vec;

    if v.len <= 1 {
        return;
    }

    // Allocate temporary buffer for swapping
    let mut temp = vec![0u8; elem_size as usize];

    let mut left = 0i64;
    let mut right = v.len - 1;

    while left < right {
        let left_ptr = v.ptr.add((left * elem_size) as usize);
        let right_ptr = v.ptr.add((right * elem_size) as usize);

        // Swap using temp buffer
        std::ptr::copy_nonoverlapping(left_ptr, temp.as_mut_ptr(), elem_size as usize);
        std::ptr::copy_nonoverlapping(right_ptr, left_ptr, elem_size as usize);
        std::ptr::copy_nonoverlapping(temp.as_ptr(), right_ptr, elem_size as usize);

        left += 1;
        right -= 1;
    }
}

/// Clear the Vec (remove all elements but keep capacity).
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec.
#[no_mangle]
pub unsafe extern "C" fn vec_clear(vec: *mut BloodVec) {
    if vec.is_null() {
        return;
    }
    (*vec).len = 0;
}

/// Free a Vec's backing memory (but not the Vec struct itself).
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// The Vec struct itself is typically stack-allocated, so we only free the backing buffer.
#[no_mangle]
pub unsafe extern "C" fn vec_free(vec: *mut BloodVec, elem_size: i64) {
    if vec.is_null() {
        return;
    }

    // Read the Vec struct WITHOUT taking ownership - the struct is stack-allocated
    let v = std::ptr::read(vec);

    // Only free the backing buffer, not the Vec struct itself
    // Use 16-byte alignment to match vec_push allocation
    if !v.ptr.is_null() && v.capacity > 0 {
        let layout = std::alloc::Layout::from_size_align(
            (v.capacity * elem_size) as usize,
            16,
        ).unwrap();
        runtime_dealloc(v.ptr, layout);
    }

    // Zero out the Vec to prevent double-free if called again
    (*vec).ptr = std::ptr::null_mut();
    (*vec).len = 0;
    (*vec).capacity = 0;
}

/// Get a pointer to the first element of a Vec.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer to write the Option struct (tag + payload pointer)
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `out` must be valid for the Option<&T> struct size.
#[no_mangle]
pub unsafe extern "C" fn vec_first(vec: *const BloodVec, _elem_size: i64, out: *mut u8) {
    if out.is_null() {
        return;
    }

    if vec.is_null() || (*vec).len == 0 {
        // None
        *(out as *mut i32) = 0;
        return;
    }

    // Some - write tag and pointer to first element
    *(out as *mut i32) = 1;
    let ptr_offset = 8usize; // Pointer alignment
    *(out.add(ptr_offset) as *mut *const u8) = (*vec).ptr;
}

/// Get a pointer to the last element of a Vec.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer to write the Option struct (tag + payload pointer)
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `out` must be valid for the Option<&T> struct size.
#[no_mangle]
pub unsafe extern "C" fn vec_last(vec: *const BloodVec, elem_size: i64, out: *mut u8) {
    if out.is_null() {
        return;
    }

    if vec.is_null() || (*vec).len == 0 {
        // None
        *(out as *mut i32) = 0;
        return;
    }

    // Some - write tag and pointer to last element
    *(out as *mut i32) = 1;
    let ptr_offset = 8usize; // Pointer alignment
    let last_idx = (*vec).len - 1;
    let last_ptr = (*vec).ptr.add((last_idx * elem_size) as usize);
    *(out.add(ptr_offset) as *mut *const u8) = last_ptr;
}

/// Insert an element at the given index, shifting elements after it.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `index` - Index to insert at
/// * `elem` - Pointer to the element to insert
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `elem` must be valid for `elem_size` bytes.
/// `index` must be <= len.
#[no_mangle]
pub unsafe extern "C" fn vec_insert(vec: *mut BloodVec, index: i64, elem: *const u8, elem_size: i64) {
    if vec.is_null() || elem.is_null() || elem_size <= 0 || index < 0 {
        return;
    }

    let v = &mut *vec;
    if index > v.len {
        return; // Index out of bounds
    }

    // Ensure capacity
    if v.len >= v.capacity {
        vec_reserve(vec, 1, elem_size);
    }

    let idx = index as usize;
    let len = v.len as usize;
    let es = elem_size as usize;

    // Shift elements after index
    if idx < len {
        let src = v.ptr.add(idx * es);
        let dst = v.ptr.add((idx + 1) * es);
        std::ptr::copy(src, dst, (len - idx) * es);
    }

    // Insert the new element
    let dest = v.ptr.add(idx * es);
    std::ptr::copy_nonoverlapping(elem, dest, es);
    v.len += 1;
}

/// Remove and return the element at the given index, shifting elements after it.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `index` - Index to remove from
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the removed element
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `out` must be valid for `elem_size` bytes.
/// `index` must be < len.
#[no_mangle]
pub unsafe extern "C" fn vec_remove(vec: *mut BloodVec, index: i64, elem_size: i64, out: *mut u8) {
    if vec.is_null() || elem_size <= 0 || index < 0 {
        return;
    }

    let v = &mut *vec;
    if index >= v.len {
        return; // Index out of bounds
    }

    let idx = index as usize;
    let len = v.len as usize;
    let es = elem_size as usize;

    // Copy element to output
    let src = v.ptr.add(idx * es);
    if !out.is_null() {
        std::ptr::copy_nonoverlapping(src, out, es);
    }

    // Shift elements after index
    if idx + 1 < len {
        let next = v.ptr.add((idx + 1) * es);
        std::ptr::copy(next, src, (len - idx - 1) * es);
    }

    v.len -= 1;
}

/// Remove and return the element at the given index by swapping with the last element.
/// This is O(1) but doesn't preserve order.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `index` - Index to remove from
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the removed element
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `out` must be valid for `elem_size` bytes.
/// `index` must be < len.
#[no_mangle]
pub unsafe extern "C" fn vec_swap_remove(vec: *mut BloodVec, index: i64, elem_size: i64, out: *mut u8) {
    if vec.is_null() || elem_size <= 0 || index < 0 {
        return;
    }

    let v = &mut *vec;
    if index >= v.len {
        return; // Index out of bounds
    }

    let idx = index as usize;
    let es = elem_size as usize;

    // Copy element to output
    let elem_ptr = v.ptr.add(idx * es);
    if !out.is_null() {
        std::ptr::copy_nonoverlapping(elem_ptr, out, es);
    }

    // If not the last element, swap with last
    if index < v.len - 1 {
        let last_ptr = v.ptr.add((v.len as usize - 1) * es);
        std::ptr::copy_nonoverlapping(last_ptr, elem_ptr, es);
    }

    v.len -= 1;
}

/// Truncate the Vec to the given length.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `new_len` - New length (must be <= current length)
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
#[no_mangle]
pub unsafe extern "C" fn vec_truncate(vec: *mut BloodVec, new_len: i64) {
    if vec.is_null() || new_len < 0 {
        return;
    }

    let v = &mut *vec;
    if new_len < v.len {
        v.len = new_len;
    }
}

/// Get a mutable pointer to the first element of a Vec.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer to write the Option struct
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
#[no_mangle]
pub unsafe extern "C" fn vec_first_mut(vec: *mut BloodVec, _elem_size: i64, out: *mut u8) {
    if out.is_null() {
        return;
    }

    if vec.is_null() || (*vec).len == 0 {
        *(out as *mut i32) = 0; // None
        return;
    }

    *(out as *mut i32) = 1; // Some
    let ptr_offset = 8usize;
    *(out.add(ptr_offset) as *mut *mut u8) = (*vec).ptr;
}

/// Get a mutable pointer to the last element of a Vec.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer to write the Option struct
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
#[no_mangle]
pub unsafe extern "C" fn vec_last_mut(vec: *mut BloodVec, elem_size: i64, out: *mut u8) {
    if out.is_null() {
        return;
    }

    if vec.is_null() || (*vec).len == 0 {
        *(out as *mut i32) = 0; // None
        return;
    }

    *(out as *mut i32) = 1; // Some
    let ptr_offset = 8usize;
    let last_idx = (*vec).len - 1;
    let last_ptr = (*vec).ptr.add((last_idx * elem_size) as usize);
    *(out.add(ptr_offset) as *mut *mut u8) = last_ptr;
}

/// Get a mutable pointer to an element at a specific index.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `index` - Index to get
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer to write the Option struct
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
#[no_mangle]
pub unsafe extern "C" fn vec_get_mut(vec: *mut BloodVec, index: i64, elem_size: i64, out: *mut u8) {
    if out.is_null() {
        return;
    }

    if vec.is_null() || index < 0 || index >= (*vec).len {
        *(out as *mut i32) = 0; // None
        return;
    }

    *(out as *mut i32) = 1; // Some
    let ptr_offset = 8usize;
    let elem_ptr = (*vec).ptr.add((index * elem_size) as usize);
    *(out.add(ptr_offset) as *mut *mut u8) = elem_ptr;
}

/// Reserve additional capacity in a Vec.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `additional` - Additional capacity to reserve
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
#[no_mangle]
pub unsafe extern "C" fn vec_reserve(vec: *mut BloodVec, additional: i64, elem_size: i64) {
    if vec.is_null() || additional <= 0 || elem_size <= 0 {
        return;
    }

    let v = &mut *vec;
    let required = v.len + additional;

    if required <= v.capacity {
        return; // Already have enough capacity
    }

    // Calculate new capacity (at least double, or required)
    let new_capacity = std::cmp::max(v.capacity * 2, required);

    // Use 16-byte alignment to support enums with i128 payloads
    let new_layout = std::alloc::Layout::from_size_align(
        (new_capacity * elem_size) as usize,
        16,
    ).unwrap();

    let new_ptr = if v.ptr.is_null() || v.capacity == 0 {
        runtime_alloc(new_layout)
    } else {
        let old_layout = std::alloc::Layout::from_size_align(
            (v.capacity * elem_size) as usize,
            16,
        ).unwrap();
        runtime_realloc(v.ptr, old_layout, (new_capacity * elem_size) as usize)
    };

    v.ptr = new_ptr;
    v.capacity = new_capacity;
}

/// Shrink the Vec's capacity to fit its length.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
#[no_mangle]
pub unsafe extern "C" fn vec_shrink_to_fit(vec: *mut BloodVec, elem_size: i64) {
    if vec.is_null() || elem_size <= 0 {
        return;
    }

    let v = &mut *vec;
    if v.len == v.capacity || v.len == 0 {
        return; // Already optimal
    }

    let new_capacity = v.len;
    // Use 16-byte alignment to match vec_push allocation
    let old_layout = std::alloc::Layout::from_size_align(
        (v.capacity * elem_size) as usize,
        16,
    ).unwrap();

    let new_ptr = runtime_realloc(v.ptr, old_layout, (new_capacity * elem_size) as usize);
    v.ptr = new_ptr;
    v.capacity = new_capacity;
}

/// Get a slice view of the Vec's contents.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `out` - Output buffer for the slice fat pointer (ptr, len)
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `out` must be valid for a slice fat pointer (16 bytes).
#[no_mangle]
pub unsafe extern "C" fn vec_as_slice(vec: *const BloodVec, out: *mut u8) {
    if vec.is_null() || out.is_null() {
        if !out.is_null() {
            *(out as *mut *const u8) = std::ptr::null();
            *(out.add(8) as *mut i64) = 0;
        }
        return;
    }

    // Write slice fat pointer: { ptr, len }
    *(out as *mut *const u8) = (*vec).ptr;
    *(out.add(8) as *mut i64) = (*vec).len;
}

/// Get a mutable slice view of the Vec's contents.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `out` - Output buffer for the slice fat pointer (ptr, len)
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `out` must be valid for a slice fat pointer (16 bytes).
#[no_mangle]
pub unsafe extern "C" fn vec_as_mut_slice(vec: *mut BloodVec, out: *mut u8) {
    if vec.is_null() || out.is_null() {
        if !out.is_null() {
            *(out as *mut *mut u8) = std::ptr::null_mut();
            *(out.add(8) as *mut i64) = 0;
        }
        return;
    }

    // Write slice fat pointer: { ptr, len }
    *(out as *mut *mut u8) = (*vec).ptr;
    *(out.add(8) as *mut i64) = (*vec).len;
}

/// Append another Vec's contents to this Vec, leaving the other Vec empty.
///
/// # Arguments
/// * `vec` - Pointer to the destination Vec struct
/// * `other` - Pointer to the source Vec struct
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// Both `vec` and `other` must be valid pointers to BloodVec structs.
#[no_mangle]
pub unsafe extern "C" fn vec_append(vec: *mut BloodVec, other: *mut BloodVec, elem_size: i64) {
    if vec.is_null() || other.is_null() || elem_size <= 0 {
        return;
    }

    let src = &mut *other;
    if src.len == 0 {
        return; // Nothing to append
    }

    // Reserve capacity
    vec_reserve(vec, src.len, elem_size);

    let dst = &mut *vec;
    let es = elem_size as usize;

    // Copy elements
    let dst_end = dst.ptr.add((dst.len as usize) * es);
    std::ptr::copy_nonoverlapping(src.ptr, dst_end, (src.len as usize) * es);

    dst.len += src.len;
    src.len = 0;
}

/// Extend a Vec from a slice.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `slice` - Pointer to the slice fat pointer
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
/// `slice` must be a valid pointer to a slice fat pointer.
#[no_mangle]
pub unsafe extern "C" fn vec_extend_from_slice(vec: *mut BloodVec, slice: *const u8, elem_size: i64) {
    if vec.is_null() || slice.is_null() || elem_size <= 0 {
        return;
    }

    // Read slice fat pointer
    let slice_ptr = *(slice as *const *const u8);
    let slice_len = *(slice.add(8) as *const i64);

    if slice_len <= 0 || slice_ptr.is_null() {
        return;
    }

    // Reserve capacity
    vec_reserve(vec, slice_len, elem_size);

    let v = &mut *vec;
    let es = elem_size as usize;

    // Copy elements
    let dst = v.ptr.add((v.len as usize) * es);
    std::ptr::copy_nonoverlapping(slice_ptr, dst, (slice_len as usize) * es);

    v.len += slice_len;
}

/// Remove consecutive duplicate elements from a Vec.
///
/// # Arguments
/// * `vec` - Pointer to the Vec struct
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// `vec` must be a valid pointer to a BloodVec struct.
#[no_mangle]
pub unsafe extern "C" fn vec_dedup(vec: *mut BloodVec, elem_size: i64) {
    if vec.is_null() || elem_size <= 0 {
        return;
    }

    let v = &mut *vec;
    if v.len <= 1 {
        return; // Nothing to deduplicate
    }

    let es = elem_size as usize;
    let mut write_idx: usize = 1;

    for read_idx in 1..(v.len as usize) {
        let prev = v.ptr.add((write_idx - 1) * es);
        let curr = v.ptr.add(read_idx * es);

        // Compare bytes
        let mut equal = true;
        for j in 0..es {
            if *prev.add(j) != *curr.add(j) {
                equal = false;
                break;
            }
        }

        if !equal {
            // Keep this element
            if write_idx != read_idx {
                let dst = v.ptr.add(write_idx * es);
                std::ptr::copy_nonoverlapping(curr, dst, es);
            }
            write_idx += 1;
        }
    }

    v.len = write_idx as i64;
}

// ============================================================================
// Box<T> Runtime Functions
// ============================================================================

/// Box a value by allocating heap memory and copying the value.
///
/// # Arguments
/// * `value` - Pointer to the value to box
/// * `size` - Size of the value in bytes
///
/// # Returns
/// Pointer to the boxed value (heap-allocated copy).
///
/// # Safety
/// `value` must be valid for `size` bytes.
#[no_mangle]
pub unsafe extern "C" fn box_new(value: *const u8, size: i64) -> *mut u8 {
    if size <= 0 {
        return std::ptr::null_mut();
    }

    let layout = std::alloc::Layout::from_size_align(size as usize, 8).unwrap();
    let ptr = runtime_alloc(layout);

    if !ptr.is_null() && !value.is_null() {
        std::ptr::copy_nonoverlapping(value, ptr, size as usize);
    }

    ptr
}

/// Get a reference to the boxed value.
///
/// # Arguments
/// * `boxed` - Pointer to the boxed value
///
/// # Returns
/// The same pointer (identity function for reference semantics).
///
/// # Safety
/// `boxed` must be a valid pointer.
#[no_mangle]
pub extern "C" fn box_as_ref(boxed: *const u8) -> *const u8 {
    boxed
}

/// Get a mutable reference to the boxed value.
///
/// # Arguments
/// * `boxed` - Pointer to the boxed value
///
/// # Returns
/// The same pointer (identity function for reference semantics).
///
/// # Safety
/// `boxed` must be a valid pointer.
#[no_mangle]
pub extern "C" fn box_as_mut(boxed: *mut u8) -> *mut u8 {
    boxed
}

/// Free a boxed value.
///
/// # Arguments
/// * `boxed` - Pointer to the boxed value
/// * `size` - Size of the value in bytes
///
/// # Safety
/// `boxed` must be a pointer returned by `box_new`.
#[no_mangle]
pub unsafe extern "C" fn box_free(boxed: *mut u8, size: i64) {
    if boxed.is_null() || size <= 0 {
        return;
    }

    let layout = std::alloc::Layout::from_size_align(size as usize, 8).unwrap();
    runtime_dealloc(boxed, layout);
}

/// Extract the inner value from a Box and deallocate it.
///
/// This is equivalent to Rust's `*box` or `Box::into_inner()`.
/// The value is copied to the output buffer and the box is freed.
///
/// # Arguments
/// * `boxed` - Pointer to the boxed value
/// * `size` - Size of the value in bytes
/// * `out` - Output buffer for the extracted value
///
/// # Safety
/// `boxed` must be a pointer returned by `box_new`.
/// `out` must be valid for at least `size` bytes.
#[no_mangle]
pub unsafe extern "C" fn box_into_inner(boxed: *mut u8, size: i64, out: *mut u8) {
    if boxed.is_null() || size <= 0 {
        return;
    }

    // Copy the value to output buffer
    if !out.is_null() {
        std::ptr::copy_nonoverlapping(boxed, out, size as usize);
    }

    // Deallocate the box
    let layout = std::alloc::Layout::from_size_align(size as usize, 8).unwrap();
    runtime_dealloc(boxed, layout);
}

/// Convert a Box into a raw pointer without deallocating.
///
/// This is equivalent to Rust's `Box::into_raw()`.
/// The caller is responsible for eventually deallocating the memory.
///
/// # Arguments
/// * `boxed` - Pointer to the boxed value
///
/// # Returns
/// The same pointer (ownership transferred to caller)
///
/// # Safety
/// `boxed` must be a pointer returned by `box_new`.
#[no_mangle]
pub unsafe extern "C" fn box_into_raw(boxed: *mut u8) -> *mut u8 {
    // Just return the pointer - ownership is transferred to caller
    boxed
}

/// Create a Box from a raw pointer.
///
/// This is equivalent to Rust's `Box::from_raw()`.
/// Takes ownership of the pointer - it must have been allocated with box_new.
///
/// # Arguments
/// * `ptr` - Raw pointer previously obtained from box_into_raw
///
/// # Returns
/// The pointer as a Box
///
/// # Safety
/// `ptr` must be a pointer that was previously returned by `box_into_raw`.
#[no_mangle]
pub unsafe extern "C" fn box_from_raw(ptr: *mut u8) -> *mut u8 {
    // Just return the pointer - we're taking ownership back
    ptr
}

/// Leak a Box and return a reference to the value.
///
/// This is equivalent to Rust's `Box::leak()`.
/// The memory is intentionally leaked and will never be freed.
///
/// # Arguments
/// * `boxed` - Pointer to the boxed value
///
/// # Returns
/// The same pointer as a reference (memory leaked)
///
/// # Safety
/// `boxed` must be a pointer returned by `box_new`.
#[no_mangle]
pub unsafe extern "C" fn box_leak(boxed: *mut u8) -> *mut u8 {
    // Just return the pointer - memory is intentionally leaked
    boxed
}

// ============================================================================
// Slice [T] Runtime Functions
// ============================================================================
//
// Slices are represented as fat pointers: { ptr: *const T, len: usize }
// The ptr field is at offset 0 (8 bytes on 64-bit).
// The len field is at offset 8 (8 bytes).

/// Get the length of a slice.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
///
/// # Returns
/// The length of the slice.
///
/// # Safety
/// `slice` must be a valid pointer to a slice fat pointer.
#[no_mangle]
pub unsafe extern "C" fn slice_len(slice: *const u8) -> i64 {
    if slice.is_null() {
        return 0;
    }
    // len is at offset 8 in the fat pointer
    let len_ptr = slice.add(8) as *const i64;
    *len_ptr
}

/// Check if a slice is empty.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
///
/// # Returns
/// 1 if empty, 0 if not empty.
///
/// # Safety
/// `slice` must be a valid pointer to a slice fat pointer.
#[no_mangle]
pub unsafe extern "C" fn slice_is_empty(slice: *const u8) -> i32 {
    if slice.is_null() {
        return 1;
    }
    let len = slice_len(slice);
    if len == 0 { 1 } else { 0 }
}

/// Get the first element of a slice as Option<&T>.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the Option<&T> result
///
/// # Returns
/// Writes Option<&T> to `out` where tag=0 is None, tag=1 is Some.
///
/// # Safety
/// `slice` must be a valid pointer to a slice fat pointer.
/// `out` must be valid for writing an Option<&T>.
#[no_mangle]
pub unsafe extern "C" fn slice_first(slice: *const u8, _elem_size: i64, out: *mut u8) {
    if slice.is_null() || out.is_null() {
        if !out.is_null() {
            *(out as *mut i32) = 0; // None
        }
        return;
    }

    let len = slice_len(slice);
    if len == 0 {
        *(out as *mut i32) = 0; // None
        return;
    }

    // Get the data pointer from the fat pointer (offset 0)
    let data_ptr = *(slice as *const *const u8);

    // Write Some with pointer to first element
    *(out as *mut i32) = 1; // Some tag
    let ptr_offset = 8usize; // Pointer alignment
    *(out.add(ptr_offset) as *mut *const u8) = data_ptr;
}

/// Get the last element of a slice as Option<&T>.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the Option<&T> result
///
/// # Returns
/// Writes Option<&T> to `out` where tag=0 is None, tag=1 is Some.
///
/// # Safety
/// `slice` must be a valid pointer to a slice fat pointer.
/// `out` must be valid for writing an Option<&T>.
#[no_mangle]
pub unsafe extern "C" fn slice_last(slice: *const u8, elem_size: i64, out: *mut u8) {
    if slice.is_null() || out.is_null() {
        if !out.is_null() {
            *(out as *mut i32) = 0; // None
        }
        return;
    }

    let len = slice_len(slice);
    if len == 0 {
        *(out as *mut i32) = 0; // None
        return;
    }

    // Get the data pointer from the fat pointer (offset 0)
    let data_ptr = *(slice as *const *const u8);
    let last_idx = len - 1;
    let last_ptr = data_ptr.add((last_idx * elem_size) as usize);

    // Write Some with pointer to last element
    *(out as *mut i32) = 1; // Some tag
    let ptr_offset = 8usize; // Pointer alignment
    *(out.add(ptr_offset) as *mut *const u8) = last_ptr;
}

/// Get an element at an index from a slice as Option<&T>.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
/// * `index` - Index of the element to get
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the Option<&T> result
///
/// # Returns
/// Writes Option<&T> to `out` where tag=0 is None, tag=1 is Some.
///
/// # Safety
/// `slice` must be a valid pointer to a slice fat pointer.
/// `out` must be valid for writing an Option<&T>.
#[no_mangle]
pub unsafe extern "C" fn slice_get(slice: *const u8, index: i64, elem_size: i64, out: *mut u8) {
    if slice.is_null() || out.is_null() || index < 0 {
        if !out.is_null() {
            *(out as *mut i32) = 0; // None
        }
        return;
    }

    let len = slice_len(slice);
    if index >= len {
        *(out as *mut i32) = 0; // None
        return;
    }

    // Get the data pointer from the fat pointer (offset 0)
    let data_ptr = *(slice as *const *const u8);
    let elem_ptr = data_ptr.add((index * elem_size) as usize);

    // Write Some with pointer to element
    *(out as *mut i32) = 1; // Some tag
    let ptr_offset = 8usize; // Pointer alignment
    *(out.add(ptr_offset) as *mut *const u8) = elem_ptr;
}

/// Check if a slice contains a specific element.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
/// * `elem` - Pointer to the element to search for
/// * `elem_size` - Size of each element in bytes
///
/// # Returns
/// 1 if found, 0 if not found.
///
/// # Safety
/// `slice` must be a valid pointer to a slice fat pointer.
/// `elem` must be a valid pointer to an element of size `elem_size`.
#[no_mangle]
pub unsafe extern "C" fn slice_contains(slice: *const u8, elem: *const u8, elem_size: i64) -> i32 {
    if slice.is_null() || elem.is_null() || elem_size <= 0 {
        return 0;
    }

    let len = slice_len(slice);
    if len == 0 {
        return 0;
    }

    // Get the data pointer from the fat pointer (offset 0)
    let data_ptr = *(slice as *const *const u8);

    // Linear search through slice
    for i in 0..len {
        let current = data_ptr.add((i * elem_size) as usize);
        // Compare bytes
        let mut equal = true;
        for j in 0..elem_size as usize {
            if *current.add(j) != *elem.add(j) {
                equal = false;
                break;
            }
        }
        if equal {
            return 1;
        }
    }

    0
}

/// Get a mutable reference to an element at an index from a slice as Option<&mut T>.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
/// * `index` - Index of the element to get
/// * `elem_size` - Size of each element in bytes
/// * `out` - Output buffer for the Option<&mut T> result
///
/// # Returns
/// Writes Option<&mut T> to `out` where tag=0 is None, tag=1 is Some.
///
/// # Safety
/// `slice` must be a valid pointer to a mutable slice fat pointer.
/// `out` must be valid for writing an Option<&mut T>.
#[no_mangle]
pub unsafe extern "C" fn slice_get_mut(slice: *mut u8, index: i64, elem_size: i64, out: *mut u8) {
    if slice.is_null() || out.is_null() || index < 0 {
        if !out.is_null() {
            *(out as *mut i32) = 0; // None
        }
        return;
    }

    let len = slice_len(slice);
    if index >= len {
        *(out as *mut i32) = 0; // None
        return;
    }

    // Get the data pointer from the fat pointer (offset 0)
    let data_ptr = *(slice as *const *mut u8);
    let elem_ptr = data_ptr.add((index * elem_size) as usize);

    // Write Some with pointer to element
    *(out as *mut i32) = 1; // Some tag
    let ptr_offset = 8usize; // Pointer alignment
    *(out.add(ptr_offset) as *mut *mut u8) = elem_ptr;
}

/// Check if a slice starts with a given needle slice.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
/// * `needle` - Pointer to the needle slice fat pointer
/// * `elem_size` - Size of each element in bytes
///
/// # Returns
/// 1 if slice starts with needle, 0 otherwise.
///
/// # Safety
/// Both `slice` and `needle` must be valid pointers to slice fat pointers.
#[no_mangle]
pub unsafe extern "C" fn slice_starts_with(slice: *const u8, needle: *const u8, elem_size: i64) -> i32 {
    if slice.is_null() || needle.is_null() || elem_size <= 0 {
        return if needle.is_null() || slice_len(needle) == 0 { 1 } else { 0 };
    }

    let slice_len_val = slice_len(slice);
    let needle_len = slice_len(needle);

    if needle_len == 0 {
        return 1; // Empty needle always matches
    }

    if needle_len > slice_len_val {
        return 0; // Needle longer than slice
    }

    // Get data pointers
    let slice_ptr = *(slice as *const *const u8);
    let needle_ptr = *(needle as *const *const u8);

    // Compare each element
    for i in 0..needle_len {
        let slice_elem = slice_ptr.add((i * elem_size) as usize);
        let needle_elem = needle_ptr.add((i * elem_size) as usize);

        // Compare bytes
        for j in 0..elem_size as usize {
            if *slice_elem.add(j) != *needle_elem.add(j) {
                return 0;
            }
        }
    }

    1
}

/// Check if a slice ends with a given needle slice.
///
/// # Arguments
/// * `slice` - Pointer to the slice fat pointer
/// * `needle` - Pointer to the needle slice fat pointer
/// * `elem_size` - Size of each element in bytes
///
/// # Returns
/// 1 if slice ends with needle, 0 otherwise.
///
/// # Safety
/// Both `slice` and `needle` must be valid pointers to slice fat pointers.
#[no_mangle]
pub unsafe extern "C" fn slice_ends_with(slice: *const u8, needle: *const u8, elem_size: i64) -> i32 {
    if slice.is_null() || needle.is_null() || elem_size <= 0 {
        return if needle.is_null() || slice_len(needle) == 0 { 1 } else { 0 };
    }

    let slice_len_val = slice_len(slice);
    let needle_len = slice_len(needle);

    if needle_len == 0 {
        return 1; // Empty needle always matches
    }

    if needle_len > slice_len_val {
        return 0; // Needle longer than slice
    }

    // Get data pointers
    let slice_ptr = *(slice as *const *const u8);
    let needle_ptr = *(needle as *const *const u8);

    // Start offset in slice
    let start = slice_len_val - needle_len;

    // Compare each element
    for i in 0..needle_len {
        let slice_elem = slice_ptr.add(((start + i) * elem_size) as usize);
        let needle_elem = needle_ptr.add((i * elem_size) as usize);

        // Compare bytes
        for j in 0..elem_size as usize {
            if *slice_elem.add(j) != *needle_elem.add(j) {
                return 0;
            }
        }
    }

    1
}

/// Reverse a slice in place.
///
/// # Arguments
/// * `slice` - Pointer to the mutable slice fat pointer
/// * `elem_size` - Size of each element in bytes
///
/// # Safety
/// `slice` must be a valid pointer to a mutable slice fat pointer.
#[no_mangle]
pub unsafe extern "C" fn slice_reverse(slice: *mut u8, elem_size: i64) {
    if slice.is_null() || elem_size <= 0 {
        return;
    }

    let len = slice_len(slice);
    if len <= 1 {
        return; // Nothing to reverse
    }

    // Get the data pointer from the fat pointer (offset 0)
    let data_ptr = *(slice as *const *mut u8);

    // Allocate temp buffer for swapping
    let elem_size_usize = elem_size as usize;
    let mut temp = vec![0u8; elem_size_usize];

    // Swap elements from both ends
    let half = len / 2;
    for i in 0..half {
        let left = data_ptr.add((i * elem_size) as usize);
        let right = data_ptr.add(((len - 1 - i) * elem_size) as usize);

        // Copy left to temp
        std::ptr::copy_nonoverlapping(left, temp.as_mut_ptr(), elem_size_usize);
        // Copy right to left
        std::ptr::copy_nonoverlapping(right, left, elem_size_usize);
        // Copy temp to right
        std::ptr::copy_nonoverlapping(temp.as_ptr(), right, elem_size_usize);
    }
}

// ============================================================================
// Option<T> Runtime Functions
// ============================================================================
//
// Option<T> is represented as { tag: i32, payload: T }
// where tag=0 is None and tag=1 is Some(value).
//
// The tag is always at offset 0 (4 bytes).
// The payload offset depends on alignment of T.
// For most cases, we assume payload is at offset 4 or 8.

/// Check if Option is Some.
///
/// # Arguments
/// * `opt` - Pointer to the Option struct
///
/// # Returns
/// 1 if Some, 0 if None
///
/// # Safety
/// `opt` must be a valid pointer to an Option<T> struct.
#[no_mangle]
pub unsafe extern "C" fn option_is_some(opt: *const u8) -> i32 {
    if opt.is_null() {
        return 0;
    }
    // Tag is at offset 0, read as i32
    let tag = *(opt as *const i32);
    if tag == 1 { 1 } else { 0 }
}

/// Check if Option is None.
///
/// # Arguments
/// * `opt` - Pointer to the Option struct
///
/// # Returns
/// 1 if None, 0 if Some
///
/// # Safety
/// `opt` must be a valid pointer to an Option<T> struct.
#[no_mangle]
pub unsafe extern "C" fn option_is_none(opt: *const u8) -> i32 {
    if opt.is_null() {
        return 1; // null treated as None
    }
    // Tag is at offset 0, read as i32
    let tag = *(opt as *const i32);
    if tag == 0 { 1 } else { 0 }
}

/// Decode a packed size+alignment parameter into (size, payload_offset).
///
/// The codegen packs payload alignment into the upper 32 bits of the i64 size parameter:
///   packed = (alignment << 32) | size
///
/// The runtime decodes both and computes the correct payload offset within the
/// Option/Result struct (which has a 4-byte i32 tag at offset 0).
///
/// This is backward-compatible: old codegen passes raw size (upper bits = 0),
/// triggering the legacy size-based heuristic.
fn decode_size_and_offset(packed: i64) -> (usize, usize) {
    let raw = packed as u64;
    let size = (raw & 0xFFFF_FFFF) as usize;
    let align = ((raw >> 32) & 0xFFFF_FFFF) as usize;
    let offset = if align > 0 {
        std::cmp::max(4, align)
    } else {
        // Legacy fallback (old codegen without alignment info)
        if size > 4 { 8 } else { 4 }
    };
    (size, offset)
}

/// Unwrap an Option, panicking if None.
///
/// # Arguments
/// * `opt` - Pointer to the Option struct
/// * `payload_size_raw` - Packed size+alignment of the payload (see `decode_size_and_offset`)
/// * `out` - Output buffer for the unwrapped value
///
/// # Safety
/// `opt` must be a valid pointer to an Option<T> struct.
/// `out` must be valid for at least `payload_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn option_unwrap(opt: *const u8, payload_size_raw: i64, out: *mut u8) {
    if opt.is_null() {
        panic!("called `Option::unwrap()` on a `None` value (null pointer)");
    }

    // Tag is at offset 0
    let tag = *(opt as *const i32);

    if tag == 0 {
        panic!("called `Option::unwrap()` on a `None` value");
    }

    let (payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);
    let payload_ptr = opt.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, payload_size);
    }
}

/// Try to unwrap an Option, returning the tag.
///
/// # Arguments
/// * `opt` - Pointer to the Option struct
/// * `payload_size` - Size of the payload in bytes
/// * `out` - Output buffer for the unwrapped value (only written if Some)
///
/// # Returns
/// 0 if None, 1 if Some
///
/// # Safety
/// `opt` must be a valid pointer to an Option<T> struct.
/// `out` must be valid for at least `payload_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn option_try(opt: *const u8, payload_size_raw: i64, out: *mut u8) -> i32 {
    if opt.is_null() {
        return 0; // None
    }

    // Tag is at offset 0
    let tag = *(opt as *const i32);

    if tag == 0 {
        return 0; // None
    }

    let (payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);
    let payload_ptr = opt.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, payload_size);
    }

    1 // Some
}

/// Unwrap an Option with a custom panic message.
///
/// # Arguments
/// * `opt` - Pointer to the Option struct
/// * `payload_size` - Size of the payload in bytes
/// * `msg` - Panic message (pointer to string data)
/// * `msg_len` - Length of the message
/// * `out` - Output buffer for the unwrapped value
///
/// # Safety
/// `opt` must be a valid pointer to an Option<T> struct.
/// `msg` must be valid for `msg_len` bytes.
/// `out` must be valid for at least `payload_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn option_expect(
    opt: *const u8,
    payload_size_raw: i64,
    msg: *const u8,
    msg_len: i64,
    out: *mut u8,
) {
    if opt.is_null() {
        let message = if !msg.is_null() && msg_len > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(msg, msg_len as usize))
        } else {
            std::borrow::Cow::Borrowed("called `Option::expect()` on a `None` value")
        };
        panic!("{}", message);
    }

    let tag = *(opt as *const i32);

    if tag == 0 {
        let message = if !msg.is_null() && msg_len > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(msg, msg_len as usize))
        } else {
            std::borrow::Cow::Borrowed("called `Option::expect()` on a `None` value")
        };
        panic!("{}", message);
    }

    let (payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);
    let payload_ptr = opt.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, payload_size);
    }
}

/// Unwrap an Option or return a default value.
///
/// # Arguments
/// * `opt` - Pointer to the Option struct
/// * `payload_size` - Size of the payload in bytes
/// * `default_val` - Pointer to the default value
/// * `out` - Output buffer for the result
///
/// # Safety
/// `opt` must be a valid pointer to an Option<T> struct.
/// `default_val` must be valid for `payload_size` bytes.
/// `out` must be valid for at least `payload_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn option_unwrap_or(
    opt: *const u8,
    payload_size_raw: i64,
    default_val: *const u8,
    out: *mut u8,
) {
    let (payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);

    if out.is_null() {
        return;
    }

    if opt.is_null() {
        // Use default
        if !default_val.is_null() {
            std::ptr::copy_nonoverlapping(default_val, out, payload_size);
        }
        return;
    }

    let tag = *(opt as *const i32);

    if tag == 1 {
        // Some - copy the value
        let payload_ptr = opt.add(payload_offset);
        std::ptr::copy_nonoverlapping(payload_ptr, out, payload_size);
    } else {
        // None - use default
        if !default_val.is_null() {
            std::ptr::copy_nonoverlapping(default_val, out, payload_size);
        }
    }
}

/// Convert Option<T> to Result<T, E>, with an error value.
///
/// # Arguments
/// * `opt` - Pointer to the Option struct
/// * `payload_size` - Size of T in bytes
/// * `err_val` - Pointer to the error value E
/// * `err_size` - Size of E in bytes
/// * `out` - Output buffer for the Result struct
///
/// # Safety
/// `opt` must be a valid pointer to an Option<T> struct.
/// `err_val` must be valid for `err_size` bytes.
/// `out` must be valid for the Result<T, E> struct size.
#[no_mangle]
pub unsafe extern "C" fn option_ok_or(
    opt: *const u8,
    payload_size_raw: i64,
    err_val: *const u8,
    err_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    let (payload_size, src_offset) = decode_size_and_offset(payload_size_raw);
    // Output Result uses max of ok/err sizes for the payload offset heuristic
    let max_payload = std::cmp::max(payload_size, err_size as usize);
    let dst_offset = if max_payload > 4 { 8 } else { 4 };

    if opt.is_null() {
        // None -> Err(err_val)
        *(out as *mut i32) = 1; // Err tag
        if !err_val.is_null() {
            let dst_payload = out.add(dst_offset);
            std::ptr::copy_nonoverlapping(err_val, dst_payload, err_size as usize);
        }
        return;
    }

    let tag = *(opt as *const i32);

    if tag == 1 {
        // Some(val) -> Ok(val)
        *(out as *mut i32) = 0; // Ok tag
        let src_payload = opt.add(src_offset);
        let dst_payload = out.add(dst_offset);
        std::ptr::copy_nonoverlapping(src_payload, dst_payload, payload_size);
    } else {
        // None -> Err(err_val)
        *(out as *mut i32) = 1; // Err tag
        if !err_val.is_null() {
            let dst_payload = out.add(dst_offset);
            std::ptr::copy_nonoverlapping(err_val, dst_payload, err_size as usize);
        }
    }
}

/// Option<T>.and(self, other: Option<U>) -> Option<U>
/// Returns None if self is None, otherwise returns other.
///
/// # Arguments
/// * `opt` - Pointer to the Option<T> struct
/// * `other` - Pointer to the Option<U> struct
/// * `other_size` - Size of Option<U> in bytes
/// * `out` - Output buffer for the result
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn option_and(
    opt: *const u8,
    other: *const u8,
    other_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if opt.is_null() {
        // None -> None
        *(out as *mut i32) = 0;
        return;
    }

    let tag = *(opt as *const i32);

    if tag == 0 {
        // None -> None
        *(out as *mut i32) = 0;
    } else {
        // Some(_) -> return other
        if !other.is_null() {
            std::ptr::copy_nonoverlapping(other, out, other_size as usize);
        } else {
            *(out as *mut i32) = 0; // treat null other as None
        }
    }
}

/// Option<T>.or(self, other: Option<T>) -> Option<T>
/// Returns self if it contains a value, otherwise returns other.
///
/// # Arguments
/// * `opt` - Pointer to the Option<T> struct
/// * `other` - Pointer to the Option<T> struct
/// * `option_size` - Size of Option<T> in bytes
/// * `out` - Output buffer for the result
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn option_or(
    opt: *const u8,
    other: *const u8,
    option_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if opt.is_null() {
        // None -> return other
        if !other.is_null() {
            std::ptr::copy_nonoverlapping(other, out, option_size as usize);
        } else {
            *(out as *mut i32) = 0;
        }
        return;
    }

    let tag = *(opt as *const i32);

    if tag == 1 {
        // Some(_) -> return self
        std::ptr::copy_nonoverlapping(opt, out, option_size as usize);
    } else {
        // None -> return other
        if !other.is_null() {
            std::ptr::copy_nonoverlapping(other, out, option_size as usize);
        } else {
            *(out as *mut i32) = 0;
        }
    }
}

/// Option<T>.xor(self, other: Option<T>) -> Option<T>
/// Returns Some if exactly one of self and other is Some.
///
/// # Arguments
/// * `opt` - Pointer to the Option<T> struct
/// * `other` - Pointer to the Option<T> struct
/// * `option_size` - Size of Option<T> in bytes
/// * `out` - Output buffer for the result
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn option_xor(
    opt: *const u8,
    other: *const u8,
    option_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    let self_is_some = if opt.is_null() {
        false
    } else {
        *(opt as *const i32) == 1
    };

    let other_is_some = if other.is_null() {
        false
    } else {
        *(other as *const i32) == 1
    };

    match (self_is_some, other_is_some) {
        (true, false) => {
            // Return self
            std::ptr::copy_nonoverlapping(opt, out, option_size as usize);
        }
        (false, true) => {
            // Return other
            std::ptr::copy_nonoverlapping(other, out, option_size as usize);
        }
        _ => {
            // Both Some or both None -> None
            *(out as *mut i32) = 0;
        }
    }
}

/// Option<T>.as_ref(&self) -> Option<&T>
/// Converts &Option<T> to Option<&T>.
///
/// # Arguments
/// * `opt` - Pointer to the Option<T> struct
/// * `payload_size` - Size of T in bytes
/// * `out` - Output buffer for Option<&T> (tag + pointer)
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn option_as_ref(
    opt: *const u8,
    payload_size_raw: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if opt.is_null() {
        // None
        *(out as *mut i32) = 0;
        return;
    }

    let tag = *(opt as *const i32);

    if tag == 0 {
        // None
        *(out as *mut i32) = 0;
    } else {
        // Some(val) -> Some(&val)
        *(out as *mut i32) = 1;
        let (_payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);
        let payload_ptr = opt.add(payload_offset);
        // Store pointer to payload at offset 8 (pointer is 8 bytes aligned)
        *(out.add(8) as *mut *const u8) = payload_ptr;
    }
}

/// Option<T>.as_mut(&mut self) -> Option<&mut T>
/// Converts &mut Option<T> to Option<&mut T>.
///
/// # Arguments
/// * `opt` - Pointer to the Option<T> struct
/// * `payload_size` - Size of T in bytes
/// * `out` - Output buffer for Option<&mut T> (tag + pointer)
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn option_as_mut(
    opt: *mut u8,
    payload_size_raw: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if opt.is_null() {
        // None
        *(out as *mut i32) = 0;
        return;
    }

    let tag = *(opt as *const i32);

    if tag == 0 {
        // None
        *(out as *mut i32) = 0;
    } else {
        // Some(val) -> Some(&mut val)
        *(out as *mut i32) = 1;
        let (_payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);
        let payload_ptr = opt.add(payload_offset);
        // Store pointer to payload at offset 8
        *(out.add(8) as *mut *mut u8) = payload_ptr;
    }
}

/// Option<T>.take(&mut self) -> Option<T>
/// Takes the value out of the option, leaving None in its place.
///
/// # Arguments
/// * `opt` - Pointer to the Option<T> struct
/// * `payload_size` - Size of T in bytes
/// * `out` - Output buffer for Option<T>
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn option_take(
    opt: *mut u8,
    payload_size_raw: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if opt.is_null() {
        // None
        *(out as *mut i32) = 0;
        return;
    }

    let tag = *(opt as *const i32);

    if tag == 0 {
        // None
        *(out as *mut i32) = 0;
    } else {
        // Some(val) -> copy to out, set self to None
        let (payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);
        let total_size = payload_offset + payload_size;

        // Copy entire Option to out
        std::ptr::copy_nonoverlapping(opt as *const u8, out, total_size);

        // Set self to None
        *(opt as *mut i32) = 0;
    }
}

/// Option<T>.replace(&mut self, value: T) -> Option<T>
/// Replaces the value with the given one, returning the old value.
///
/// # Arguments
/// * `opt` - Pointer to the Option<T> struct
/// * `value` - Pointer to the new value
/// * `payload_size` - Size of T in bytes
/// * `out` - Output buffer for Option<T> (old value)
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn option_replace(
    opt: *mut u8,
    value: *const u8,
    payload_size_raw: i64,
    out: *mut u8,
) {
    if out.is_null() || opt.is_null() {
        return;
    }

    let (payload_size, payload_offset) = decode_size_and_offset(payload_size_raw);
    let total_size = payload_offset + payload_size;

    // Copy current Option to out
    std::ptr::copy_nonoverlapping(opt as *const u8, out, total_size);

    // Set opt to Some(value)
    *(opt as *mut i32) = 1;
    if !value.is_null() {
        let dst_payload = opt.add(payload_offset);
        std::ptr::copy_nonoverlapping(value, dst_payload, payload_size);
    }
}

// ============================================================================
// Result<T, E> Runtime Functions
// ============================================================================
//
// Result<T, E> is represented as { tag: i32, payload: max(T, E) }
// where tag=0 is Ok(T) and tag=1 is Err(E).
//
// The tag is always at offset 0 (4 bytes).
// The payload offset depends on alignment.

/// Check if Result is Ok.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
///
/// # Returns
/// 1 if Ok, 0 if Err
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
#[no_mangle]
pub unsafe extern "C" fn result_is_ok(res: *const u8) -> i32 {
    if res.is_null() {
        return 0;
    }
    // Tag is at offset 0, read as i32
    let tag = *(res as *const i32);
    if tag == 0 { 1 } else { 0 }
}

/// Check if Result is Err.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
///
/// # Returns
/// 1 if Err, 0 if Ok
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
#[no_mangle]
pub unsafe extern "C" fn result_is_err(res: *const u8) -> i32 {
    if res.is_null() {
        return 1; // null treated as Err
    }
    // Tag is at offset 0, read as i32
    let tag = *(res as *const i32);
    if tag == 1 { 1 } else { 0 }
}

/// Unwrap a Result, panicking if Err.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `ok_size` - Size of the Ok payload in bytes
/// * `out` - Output buffer for the unwrapped value
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `out` must be valid for at least `ok_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn result_unwrap(res: *const u8, ok_size_raw: i64, out: *mut u8) {
    if res.is_null() {
        panic!("called `Result::unwrap()` on an `Err` value (null pointer)");
    }

    // Tag is at offset 0
    let tag = *(res as *const i32);

    if tag != 0 {
        panic!("called `Result::unwrap()` on an `Err` value");
    }

    let (ok_size, payload_offset) = decode_size_and_offset(ok_size_raw);
    let payload_ptr = res.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, ok_size);
    }
}

/// Unwrap a Result error, panicking if Ok.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `err_size` - Size of the Err payload in bytes
/// * `out` - Output buffer for the unwrapped error
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `out` must be valid for at least `err_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn result_unwrap_err(res: *const u8, err_size_raw: i64, out: *mut u8) {
    if res.is_null() {
        panic!("called `Result::unwrap_err()` on null pointer");
    }

    // Tag is at offset 0
    let tag = *(res as *const i32);

    if tag != 1 {
        panic!("called `Result::unwrap_err()` on an `Ok` value");
    }

    let (err_size, payload_offset) = decode_size_and_offset(err_size_raw);
    let payload_ptr = res.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, err_size);
    }
}

/// Try to unwrap a Result, returning the tag.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `ok_size` - Size of the Ok payload in bytes
/// * `out` - Output buffer for the unwrapped value (only written if Ok)
///
/// # Returns
/// 0 if Ok, 1 if Err
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `out` must be valid for at least `ok_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn result_try(res: *const u8, ok_size_raw: i64, out: *mut u8) -> i32 {
    if res.is_null() {
        return 1; // Err
    }

    // Tag is at offset 0
    let tag = *(res as *const i32);

    if tag != 0 {
        return 1; // Err
    }

    let (ok_size, payload_offset) = decode_size_and_offset(ok_size_raw);
    let payload_ptr = res.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, ok_size);
    }

    0 // Ok
}

/// Convert Result<T, E> to Option<T>, discarding the error.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `ok_size` - Size of the Ok payload in bytes
/// * `out` - Output buffer for the Option struct
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `out` must be valid for the Option<T> struct size.
#[no_mangle]
pub unsafe extern "C" fn result_ok(res: *const u8, ok_size_raw: i64, out: *mut u8) {
    if res.is_null() || out.is_null() {
        // Write None to output
        *(out as *mut i32) = 0;
        return;
    }

    let tag = *(res as *const i32);

    if tag == 0 {
        // Ok(value) -> Some(value)
        *(out as *mut i32) = 1; // Some tag
        let (ok_size, payload_offset) = decode_size_and_offset(ok_size_raw);
        let src_payload = res.add(payload_offset);
        let dst_payload = out.add(payload_offset);
        std::ptr::copy_nonoverlapping(src_payload, dst_payload, ok_size);
    } else {
        // Err(_) -> None
        *(out as *mut i32) = 0;
    }
}

/// Convert Result<T, E> to Option<E>, discarding the ok value.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `err_size` - Size of the Err payload in bytes
/// * `out` - Output buffer for the Option struct
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `out` must be valid for the Option<E> struct size.
#[no_mangle]
pub unsafe extern "C" fn result_err(res: *const u8, err_size_raw: i64, out: *mut u8) {
    if res.is_null() || out.is_null() {
        // Write None to output
        *(out as *mut i32) = 0;
        return;
    }

    let tag = *(res as *const i32);

    if tag == 1 {
        // Err(value) -> Some(value)
        *(out as *mut i32) = 1; // Some tag
        let (err_size, payload_offset) = decode_size_and_offset(err_size_raw);
        let src_payload = res.add(payload_offset);
        let dst_payload = out.add(payload_offset);
        std::ptr::copy_nonoverlapping(src_payload, dst_payload, err_size);
    } else {
        // Ok(_) -> None
        *(out as *mut i32) = 0;
    }
}

/// Unwrap a Result with a custom panic message.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `ok_size` - Size of the Ok payload in bytes
/// * `msg` - Panic message (pointer to string data)
/// * `msg_len` - Length of the message
/// * `out` - Output buffer for the unwrapped value
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `msg` must be valid for `msg_len` bytes.
/// `out` must be valid for at least `ok_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn result_expect(
    res: *const u8,
    ok_size_raw: i64,
    msg: *const u8,
    msg_len: i64,
    out: *mut u8,
) {
    if res.is_null() {
        let message = if !msg.is_null() && msg_len > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(msg, msg_len as usize))
        } else {
            std::borrow::Cow::Borrowed("called `Result::expect()` on an `Err` value")
        };
        panic!("{}", message);
    }

    let tag = *(res as *const i32);

    if tag != 0 {
        let message = if !msg.is_null() && msg_len > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(msg, msg_len as usize))
        } else {
            std::borrow::Cow::Borrowed("called `Result::expect()` on an `Err` value")
        };
        panic!("{}", message);
    }

    let (ok_size, payload_offset) = decode_size_and_offset(ok_size_raw);
    let payload_ptr = res.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, ok_size);
    }
}

/// Unwrap a Result error with a custom panic message.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `err_size` - Size of the Err payload in bytes
/// * `msg` - Panic message (pointer to string data)
/// * `msg_len` - Length of the message
/// * `out` - Output buffer for the unwrapped error
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `msg` must be valid for `msg_len` bytes.
/// `out` must be valid for at least `err_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn result_expect_err(
    res: *const u8,
    err_size_raw: i64,
    msg: *const u8,
    msg_len: i64,
    out: *mut u8,
) {
    if res.is_null() {
        let message = if !msg.is_null() && msg_len > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(msg, msg_len as usize))
        } else {
            std::borrow::Cow::Borrowed("called `Result::expect_err()` on an `Ok` value")
        };
        panic!("{}", message);
    }

    let tag = *(res as *const i32);

    if tag != 1 {
        let message = if !msg.is_null() && msg_len > 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(msg, msg_len as usize))
        } else {
            std::borrow::Cow::Borrowed("called `Result::expect_err()` on an `Ok` value")
        };
        panic!("{}", message);
    }

    let (err_size, payload_offset) = decode_size_and_offset(err_size_raw);
    let payload_ptr = res.add(payload_offset);

    if !out.is_null() {
        std::ptr::copy_nonoverlapping(payload_ptr, out, err_size);
    }
}

/// Unwrap a Result or return a default value.
///
/// # Arguments
/// * `res` - Pointer to the Result struct
/// * `ok_size` - Size of the Ok payload in bytes
/// * `default_val` - Pointer to the default value
/// * `out` - Output buffer for the result
///
/// # Safety
/// `res` must be a valid pointer to a Result<T, E> struct.
/// `default_val` must be valid for `ok_size` bytes.
/// `out` must be valid for at least `ok_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn result_unwrap_or(
    res: *const u8,
    ok_size_raw: i64,
    default_val: *const u8,
    out: *mut u8,
) {
    let (ok_size, payload_offset) = decode_size_and_offset(ok_size_raw);

    if out.is_null() {
        return;
    }

    if res.is_null() {
        // Use default
        if !default_val.is_null() {
            std::ptr::copy_nonoverlapping(default_val, out, ok_size);
        }
        return;
    }

    let tag = *(res as *const i32);

    if tag == 0 {
        // Ok - copy the value
        let payload_ptr = res.add(payload_offset);
        std::ptr::copy_nonoverlapping(payload_ptr, out, ok_size);
    } else {
        // Err - use default
        if !default_val.is_null() {
            std::ptr::copy_nonoverlapping(default_val, out, ok_size);
        }
    }
}

/// Result<T, E>.and(self, other: Result<U, E>) -> Result<U, E>
/// Returns other if self is Ok, otherwise returns the Err value of self.
///
/// # Arguments
/// * `res` - Pointer to the Result<T, E> struct
/// * `other` - Pointer to the Result<U, E> struct
/// * `other_size` - Size of Result<U, E> in bytes
/// * `err_size` - Size of E in bytes
/// * `out` - Output buffer for the result
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn result_and(
    res: *const u8,
    other: *const u8,
    other_size: i64,
    err_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if res.is_null() {
        // Treat null as Err
        *(out as *mut i32) = 1;
        return;
    }

    let tag = *(res as *const i32);

    if tag == 0 {
        // Ok -> return other
        if !other.is_null() {
            std::ptr::copy_nonoverlapping(other, out, other_size as usize);
        } else {
            *(out as *mut i32) = 1; // treat null other as Err
        }
    } else {
        // Err -> return Err from self
        *(out as *mut i32) = 1;
        let (err_sz, payload_offset) = decode_size_and_offset(err_size);
        let src_payload = res.add(payload_offset);
        let dst_payload = out.add(payload_offset);
        std::ptr::copy_nonoverlapping(src_payload, dst_payload, err_sz);
    }
}

/// Result<T, E>.or(self, other: Result<T, F>) -> Result<T, F>
/// Returns self if it is Ok, otherwise returns other.
///
/// # Arguments
/// * `res` - Pointer to the Result<T, E> struct
/// * `other` - Pointer to the Result<T, F> struct
/// * `ok_size` - Size of T in bytes
/// * `other_size` - Size of Result<T, F> in bytes
/// * `out` - Output buffer for the result
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn result_or(
    res: *const u8,
    other: *const u8,
    ok_size: i64,
    other_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if res.is_null() {
        // Treat null as Err -> return other
        if !other.is_null() {
            std::ptr::copy_nonoverlapping(other, out, other_size as usize);
        } else {
            *(out as *mut i32) = 1;
        }
        return;
    }

    let tag = *(res as *const i32);

    if tag == 0 {
        // Ok -> return self (as Result<T, F>)
        *(out as *mut i32) = 0;
        let (ok_sz, payload_offset) = decode_size_and_offset(ok_size);
        let src_payload = res.add(payload_offset);
        let dst_payload = out.add(payload_offset);
        std::ptr::copy_nonoverlapping(src_payload, dst_payload, ok_sz);
    } else {
        // Err -> return other
        if !other.is_null() {
            std::ptr::copy_nonoverlapping(other, out, other_size as usize);
        } else {
            *(out as *mut i32) = 1;
        }
    }
}

/// Result<T, E>.as_ref(&self) -> Result<&T, &E>
/// Converts &Result<T, E> to Result<&T, &E>.
///
/// # Arguments
/// * `res` - Pointer to the Result<T, E> struct
/// * `ok_size` - Size of T in bytes
/// * `err_size` - Size of E in bytes
/// * `out` - Output buffer for Result<&T, &E> (tag + pointer)
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn result_as_ref(
    res: *const u8,
    ok_size: i64,
    err_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if res.is_null() {
        // Treat as Err with null pointer
        *(out as *mut i32) = 1;
        *(out.add(8) as *mut *const u8) = std::ptr::null();
        return;
    }

    let tag = *(res as *const i32);

    if tag == 0 {
        // Ok(val) -> Ok(&val)
        *(out as *mut i32) = 0;
        let (_ok_sz, payload_offset) = decode_size_and_offset(ok_size);
        let payload_ptr = res.add(payload_offset);
        *(out.add(8) as *mut *const u8) = payload_ptr;
    } else {
        // Err(e) -> Err(&e)
        *(out as *mut i32) = 1;
        let (_err_sz, payload_offset) = decode_size_and_offset(err_size);
        let payload_ptr = res.add(payload_offset);
        *(out.add(8) as *mut *const u8) = payload_ptr;
    }
}

/// Result<T, E>.as_mut(&mut self) -> Result<&mut T, &mut E>
/// Converts &mut Result<T, E> to Result<&mut T, &mut E>.
///
/// # Arguments
/// * `res` - Pointer to the Result<T, E> struct
/// * `ok_size` - Size of T in bytes
/// * `err_size` - Size of E in bytes
/// * `out` - Output buffer for Result<&mut T, &mut E> (tag + pointer)
///
/// # Safety
/// All pointers must be valid.
#[no_mangle]
pub unsafe extern "C" fn result_as_mut(
    res: *mut u8,
    ok_size: i64,
    err_size: i64,
    out: *mut u8,
) {
    if out.is_null() {
        return;
    }

    if res.is_null() {
        // Treat as Err with null pointer
        *(out as *mut i32) = 1;
        *(out.add(8) as *mut *mut u8) = std::ptr::null_mut();
        return;
    }

    let tag = *(res as *const i32);

    if tag == 0 {
        // Ok(val) -> Ok(&mut val)
        *(out as *mut i32) = 0;
        let (_ok_sz, payload_offset) = decode_size_and_offset(ok_size);
        let payload_ptr = res.add(payload_offset);
        *(out.add(8) as *mut *mut u8) = payload_ptr;
    } else {
        // Err(e) -> Err(&mut e)
        *(out as *mut i32) = 1;
        let (_err_sz, payload_offset) = decode_size_and_offset(err_size);
        let payload_ptr = res.add(payload_offset);
        *(out.add(8) as *mut *mut u8) = payload_ptr;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_functions() {
        // Just verify they don't panic
        print_int(42);
        println_int(42);
        println_i64(42);
    }

    #[test]
    fn test_evidence_vector() {
        unsafe {
            let ev = blood_evidence_create();
            assert!(!ev.is_null());

            blood_evidence_push(ev, 100);
            blood_evidence_push(ev, 200);

            assert_eq!(blood_evidence_get(ev, 0), 100);
            assert_eq!(blood_evidence_get(ev, 1), 200);

            assert_eq!(blood_evidence_pop(ev), 200);
            assert_eq!(blood_evidence_pop(ev), 100);

            blood_evidence_destroy(ev);
        }
    }

    #[test]
    fn test_generation_check() {
        assert_eq!(blood_check_generation(1, 1), 1);
        assert_eq!(blood_check_generation(1, 2), 0);
        assert_eq!(blood_check_generation(generation::PERSISTENT, 999), 1);
    }

    #[test]
    fn test_runtime_init() {
        assert_eq!(blood_runtime_init(), 0);
        blood_runtime_shutdown();
    }

    #[test]
    fn test_evidence_current() {
        // Initially null
        let ev = blood_evidence_current();
        assert!(ev.is_null());

        // Set current evidence
        let new_ev = blood_evidence_create();
        blood_evidence_set_current(new_ev);

        let current = blood_evidence_current();
        assert_eq!(current, new_ev);

        // Cleanup
        unsafe { blood_evidence_destroy(new_ev); }
    }

    #[test]
    fn test_dispatch_table() {
        // Mock function pointer
        extern "C" fn mock_impl() -> i64 { 42 }
        let impl_ptr = mock_impl as *const c_void;

        // Register implementation
        blood_dispatch_register(1, 100, impl_ptr);

        // Lookup should find it
        let found = blood_dispatch_lookup(1, 100);
        assert_eq!(found, impl_ptr);

        // Different slot/tag should not find it
        let not_found = blood_dispatch_lookup(2, 100);
        assert!(not_found.is_null());

        let not_found2 = blood_dispatch_lookup(1, 200);
        assert!(not_found2.is_null());
    }

    #[test]
    fn test_type_tag() {
        // Create a mock object with type tag
        let mock_obj: [i64; 2] = [42, 0]; // type_tag = 42
        unsafe {
            let tag = blood_get_type_tag(mock_obj.as_ptr() as *const c_void);
            assert_eq!(tag, 42);
        }

        // Null object returns 0
        unsafe {
            let tag = blood_get_type_tag(std::ptr::null());
            assert_eq!(tag, 0);
        }
    }

    #[test]
    fn test_handler_depth() {
        // Create evidence vector
        let ev = blood_evidence_create();
        blood_evidence_set_current(ev);

        // Initially no handlers
        let depth = blood_handler_depth(1);
        assert_eq!(depth, 0);

        // Register a handler (effect_id = 1)
        unsafe {
            extern "C" fn mock_op(_state: *mut c_void, _args: *const i64, _arg_count: i64, _continuation: i64) -> i64 { 0 }
            let ops: [*const c_void; 1] = [mock_op as *const c_void];
            blood_evidence_register(ev, 1, ops.as_ptr(), 1);
        }

        // Now depth should be 1
        let depth = blood_handler_depth(1);
        assert_eq!(depth, 1);

        // Different effect should still be 0
        let depth2 = blood_handler_depth(2);
        assert_eq!(depth2, 0);

        // Cleanup
        unsafe { blood_evidence_destroy(ev); }
    }

    #[test]
    fn test_blood_perform() {
        // Create evidence vector and set as current
        let ev = blood_evidence_create();
        blood_evidence_set_current(ev);

        // Register a handler that returns args[0] * 2
        // Handler signature: fn(state, args, arg_count, continuation) -> i64
        unsafe {
            extern "C" fn double_op(_state: *mut c_void, args: *const i64, _arg_count: i64, _continuation: i64) -> i64 {
                unsafe {
                    if args.is_null() { return 0; }
                    (*args) * 2
                }
            }
            let ops: [*const c_void; 1] = [double_op as *const c_void];
            blood_evidence_register(ev, 100, ops.as_ptr(), 1);
        }

        // Perform the effect operation
        unsafe {
            let args: [i64; 1] = [21];
            let result = blood_perform(100, 0, args.as_ptr(), 1, 0);
            assert_eq!(result, 42);
        }

        // Cleanup
        unsafe { blood_evidence_destroy(ev); }
    }

    #[test]
    fn test_evidence_state() {
        let ev = blood_evidence_create();

        // Register a handler
        unsafe {
            extern "C" fn noop(_state: *mut c_void, _args: *const i64, _arg_count: i64, _continuation: i64) -> i64 { 0 }
            let ops: [*const c_void; 1] = [noop as *const c_void];
            blood_evidence_register(ev, 50, ops.as_ptr(), 1);
        }

        // Set state
        let state_value: i64 = 999;
        unsafe {
            blood_evidence_set_state(ev, &state_value as *const i64 as *mut c_void);
        }

        // Get state back
        unsafe {
            let state = blood_evidence_get_state(ev, 0);
            assert!(!state.is_null());
            let retrieved = *(state as *const i64);
            assert_eq!(retrieved, 999);
        }

        // Cleanup
        unsafe { blood_evidence_destroy(ev); }
    }

    #[test]
    fn test_snapshot_create_destroy() {
        let snapshot = blood_snapshot_create();
        assert!(!snapshot.is_null());

        unsafe {
            assert_eq!(blood_snapshot_len(snapshot), 0);
            blood_snapshot_destroy(snapshot);
        }
    }

    #[test]
    fn test_snapshot_add_entries() {
        let snapshot = blood_snapshot_create();
        assert!(!snapshot.is_null());

        unsafe {
            // Add some entries
            blood_snapshot_add_entry(snapshot, 0x1000, 1);
            blood_snapshot_add_entry(snapshot, 0x2000, 2);
            blood_snapshot_add_entry(snapshot, 0x3000, 3);

            // Should have 3 entries
            assert_eq!(blood_snapshot_len(snapshot), 3);

            // Persistent pointers (gen = 0xFFFFFFFF) should be skipped
            blood_snapshot_add_entry(snapshot, 0x4000, 0xFFFFFFFF);
            assert_eq!(blood_snapshot_len(snapshot), 3); // Still 3

            blood_snapshot_destroy(snapshot);
        }
    }

    #[test]
    fn test_snapshot_validate_with_registry() {
        // Register allocations in the global slot registry
        let addr1 = 0xABCD_1000u64;
        let addr2 = 0xABCD_2000u64;

        let gen1 = blood_register_allocation(addr1, 64);
        let gen2 = blood_register_allocation(addr2, 128);

        let snapshot = blood_snapshot_create();
        assert!(!snapshot.is_null());

        unsafe {
            // Add entries with correct generations
            blood_snapshot_add_entry(snapshot, addr1, gen1);
            blood_snapshot_add_entry(snapshot, addr2, gen2);

            // Validation should succeed
            let result = blood_snapshot_validate(snapshot);
            assert_eq!(result, 0, "Snapshot with valid generations should validate");

            blood_snapshot_destroy(snapshot);
        }

        // Cleanup
        blood_unregister_allocation(addr1);
        blood_unregister_allocation(addr2);
    }

    #[test]
    fn test_snapshot_validate_stale() {
        // Register and then unregister to create a stale reference scenario
        let addr = 0xDEAD_1000u64;
        let gen = blood_register_allocation(addr, 32);

        let snapshot = blood_snapshot_create();
        assert!(!snapshot.is_null());

        unsafe {
            // Add entry with the current generation
            blood_snapshot_add_entry(snapshot, addr, gen);

            // Unregister (free) the allocation - this increments generation
            blood_unregister_allocation(addr);

            // Validation should fail - generation mismatch
            let result = blood_snapshot_validate(snapshot);
            assert_eq!(result, 1, "Stale reference should be detected at index 1");

            blood_snapshot_destroy(snapshot);
        }
    }

    #[test]
    fn test_snapshot_validate_persistent() {
        let snapshot = blood_snapshot_create();
        assert!(!snapshot.is_null());

        unsafe {
            // Persistent references (gen = 0xFFFFFFFF) should be skipped during add
            blood_snapshot_add_entry(snapshot, 0x9999_0000, generation::PERSISTENT);

            // Length should be 0 since persistent entries are skipped
            assert_eq!(blood_snapshot_len(snapshot), 0);

            // Empty snapshot validates successfully
            let result = blood_snapshot_validate(snapshot);
            assert_eq!(result, 0);

            blood_snapshot_destroy(snapshot);
        }
    }

    #[test]
    fn test_snapshot_get_stale_entry() {
        let addr = 0xBEEF_2000u64;
        let gen = blood_register_allocation(addr, 16);

        let snapshot = blood_snapshot_create();
        assert!(!snapshot.is_null());

        unsafe {
            blood_snapshot_add_entry(snapshot, addr, gen);
            blood_unregister_allocation(addr);

            let result = blood_snapshot_validate(snapshot);
            assert!(result > 0, "Should detect stale reference");

            // Get stale entry details
            let mut out_addr: u64 = 0;
            let mut out_gen: u32 = 0;
            let status = blood_snapshot_get_stale_entry(
                snapshot,
                result,
                &mut out_addr,
                &mut out_gen,
            );

            assert_eq!(status, 0, "Should successfully get stale entry");
            assert_eq!(out_addr, addr);
            assert_eq!(out_gen, gen);

            blood_snapshot_destroy(snapshot);
        }
    }

    #[test]
    fn test_validate_generation_ffi() {
        let addr = 0xCAFE_3000u64;
        let gen = blood_register_allocation(addr, 64);

        // Valid generation
        assert_eq!(blood_validate_generation(addr, gen), 0);

        // Wrong generation
        assert_eq!(blood_validate_generation(addr, gen + 1), 1);

        // After free
        blood_unregister_allocation(addr);
        assert_eq!(blood_validate_generation(addr, gen), 1);
    }

    /// Test the complete use-after-free detection flow via blood_alloc_or_abort
    ///
    /// This test verifies the end-to-end flow that compiled Blood programs use:
    /// 1. Allocate via blood_alloc_or_abort (stores generation)
    /// 2. Validate generation on dereference (blood_validate_generation returns 0)
    /// 3. Free the memory (blood_unregister_allocation)
    /// 4. Attempt to validate stale generation (blood_validate_generation returns 1)
    #[test]
    fn test_use_after_free_detection_full_flow() {
        // Step 1: Allocate memory via blood_alloc_or_abort
        let mut generation: u32 = 0;
        let address = unsafe { blood_alloc_or_abort(64, &mut generation) };

        assert!(address != 0, "Allocation should succeed");
        assert!(generation >= generation::FIRST, "Generation should be valid");

        // Step 2: Validate - should succeed (generation matches)
        assert_eq!(
            blood_validate_generation(address, generation),
            0,
            "Validation should succeed immediately after allocation"
        );

        // Step 3: Simulate free by unregistering
        blood_unregister_allocation(address);

        // Step 4: Validate again - should FAIL (stale reference detected)
        assert_eq!(
            blood_validate_generation(address, generation),
            1,
            "Validation should fail after deallocation - USE AFTER FREE DETECTED"
        );

        // Clean up the actual memory (since blood_alloc_or_abort uses std::alloc::alloc)
        unsafe {
            let layout = std::alloc::Layout::from_size_align(64, 16).unwrap();
            std::alloc::dealloc(address as *mut u8, layout);
        }
    }

    /// Test that reallocating the same address gets a new generation
    #[test]
    fn test_generation_increment_on_realloc() {
        // First allocation
        let mut gen1: u32 = 0;
        let addr1 = unsafe { blood_alloc_or_abort(32, &mut gen1) };
        assert!(addr1 != 0);

        // Free it
        blood_unregister_allocation(addr1);

        // Deallocate and reallocate to potentially get same address
        unsafe {
            let layout = std::alloc::Layout::from_size_align(32, 16).unwrap();
            std::alloc::dealloc(addr1 as *mut u8, layout);
        }

        // Second allocation
        let mut gen2: u32 = 0;
        let addr2 = unsafe { blood_alloc_or_abort(32, &mut gen2) };
        assert!(addr2 != 0);

        // Even if we got the same address, generation should be different
        // (or at least the old generation should be invalid)
        assert_eq!(
            blood_validate_generation(addr1, gen1),
            1,
            "Old generation should be invalid regardless of address reuse"
        );

        // New generation should be valid
        assert_eq!(
            blood_validate_generation(addr2, gen2),
            0,
            "New generation should be valid"
        );

        // Clean up
        blood_unregister_allocation(addr2);
        unsafe {
            let layout = std::alloc::Layout::from_size_align(32, 16).unwrap();
            std::alloc::dealloc(addr2 as *mut u8, layout);
        }
    }

    /// Test effect context snapshot get/set functions
    #[test]
    fn test_effect_context_snapshot_functions() {
        // Begin effect context (non-tail-resumptive)
        blood_effect_context_begin(false);

        // Initially no snapshot
        let initial = blood_effect_context_get_snapshot();
        assert!(initial.is_null(), "Initial snapshot should be null");

        // Create and set a snapshot
        let snapshot = blood_snapshot_create();
        assert!(!snapshot.is_null());

        blood_effect_context_set_snapshot(snapshot);

        // Get should return the snapshot
        let retrieved = blood_effect_context_get_snapshot();
        assert_eq!(retrieved, snapshot, "Get should return the set snapshot");

        // Take should return and clear
        let taken = blood_effect_context_take_snapshot();
        assert_eq!(taken, snapshot, "Take should return the snapshot");

        // After take, get should return null
        let after_take = blood_effect_context_get_snapshot();
        assert!(after_take.is_null(), "After take, get should return null");

        // Clean up
        unsafe { blood_snapshot_destroy(taken); }
        blood_effect_context_end();
    }

    /// Test snapshot validation through effect context
    #[test]
    fn test_effect_context_snapshot_validation() {
        // Set up: allocate, create snapshot, free, then validate
        let addr = 0xABCD_1000u64;
        let gen = blood_register_allocation(addr, 64);

        blood_effect_context_begin(false);

        // Create snapshot with the allocation
        let snapshot = blood_snapshot_create();
        unsafe { blood_snapshot_add_entry(snapshot, addr, gen); }
        blood_effect_context_set_snapshot(snapshot);

        // Free the allocation (making reference stale)
        blood_unregister_allocation(addr);

        // Get snapshot and validate - should detect stale
        let snap = blood_effect_context_get_snapshot();
        let result = unsafe { blood_snapshot_validate(snap) };
        assert_eq!(result, 1, "Should detect stale reference at index 1");

        // Clean up
        let snap = blood_effect_context_take_snapshot();
        unsafe { blood_snapshot_destroy(snap); }
        blood_effect_context_end();
    }

    /// Test that blood_get_generation returns actual values (not placeholder)
    #[test]
    fn test_blood_get_generation_not_placeholder() {
        // Allocate and register
        let addr = 0x7777_0000u64;
        let gen = blood_register_allocation(addr, 32);

        // blood_get_generation should return the registered generation
        let retrieved_gen = unsafe { blood_get_generation(addr) };
        assert_eq!(retrieved_gen, gen, "blood_get_generation should return registered generation");

        // After unregister, generation should be incremented
        blood_unregister_allocation(addr);

        let new_gen = unsafe { blood_get_generation(addr) };
        // The address may or may not still be in registry depending on implementation
        // But if it is, the generation should be incremented
        // If not, it returns FIRST (1) which is different from gen (since gen was incremented on register)
        assert_ne!(new_gen, gen, "Generation should change after unregister");

        // Re-register should get different generation
        let gen2 = blood_register_allocation(addr, 32);
        let retrieved_gen2 = unsafe { blood_get_generation(addr) };
        assert_eq!(retrieved_gen2, gen2, "Should get new generation after re-register");

        // Clean up
        blood_unregister_allocation(addr);
    }

    // ========================================================================
    // Snapshot Sharing Tests (Nested Handlers)
    // ========================================================================

    #[test]
    fn test_snapshot_create_child() {
        let parent = blood_snapshot_create();
        assert!(!parent.is_null());

        let child = blood_snapshot_create_child(parent);
        assert!(!child.is_null());

        unsafe {
            // Child should have parent set
            assert!(blood_snapshot_has_parent(child));
            assert_eq!(blood_snapshot_get_parent(child), parent);

            // Parent should not have parent
            assert!(!blood_snapshot_has_parent(parent));
            assert!(blood_snapshot_get_parent(parent).is_null());

            // Chain depth
            assert_eq!(blood_snapshot_chain_depth(parent), 1);
            assert_eq!(blood_snapshot_chain_depth(child), 2);

            // Clean up (child first, then parent)
            blood_snapshot_destroy(child);
            blood_snapshot_destroy(parent);
        }
    }

    #[test]
    fn test_snapshot_chain_depth() {
        let root = blood_snapshot_create();
        let child1 = blood_snapshot_create_child(root);
        let child2 = blood_snapshot_create_child(child1);
        let child3 = blood_snapshot_create_child(child2);

        unsafe {
            assert_eq!(blood_snapshot_chain_depth(root), 1);
            assert_eq!(blood_snapshot_chain_depth(child1), 2);
            assert_eq!(blood_snapshot_chain_depth(child2), 3);
            assert_eq!(blood_snapshot_chain_depth(child3), 4);

            // Clean up deepest first
            blood_snapshot_destroy(child3);
            blood_snapshot_destroy(child2);
            blood_snapshot_destroy(child1);
            blood_snapshot_destroy(root);
        }
    }

    #[test]
    fn test_snapshot_total_len() {
        let parent = blood_snapshot_create();
        let child = blood_snapshot_create_child(parent);

        unsafe {
            // Add entries to parent
            blood_snapshot_add_entry(parent, 0x1000, 1);
            blood_snapshot_add_entry(parent, 0x2000, 2);

            // Add entries to child
            blood_snapshot_add_entry(child, 0x3000, 3);

            // Local lengths
            assert_eq!(blood_snapshot_len(parent), 2);
            assert_eq!(blood_snapshot_len(child), 1);

            // Total lengths
            assert_eq!(blood_snapshot_total_len(parent), 2);
            assert_eq!(blood_snapshot_total_len(child), 3); // 1 + 2 from parent

            blood_snapshot_destroy(child);
            blood_snapshot_destroy(parent);
        }
    }

    #[test]
    fn test_snapshot_chain_validate_success() {
        // Register allocations
        let addr1 = 0xAAAA_1000u64;
        let addr2 = 0xAAAA_2000u64;
        let addr3 = 0xAAAA_3000u64;

        let gen1 = blood_register_allocation(addr1, 64);
        let gen2 = blood_register_allocation(addr2, 64);
        let gen3 = blood_register_allocation(addr3, 64);

        let parent = blood_snapshot_create();
        let child = blood_snapshot_create_child(parent);

        unsafe {
            // Add parent entries
            blood_snapshot_add_entry(parent, addr1, gen1);
            blood_snapshot_add_entry(parent, addr2, gen2);

            // Add child entry
            blood_snapshot_add_entry(child, addr3, gen3);

            // Validation should succeed for entire chain
            let result = blood_snapshot_validate(child);
            assert_eq!(result, 0, "Chain validation should succeed");

            blood_snapshot_destroy(child);
            blood_snapshot_destroy(parent);
        }

        // Clean up
        blood_unregister_allocation(addr1);
        blood_unregister_allocation(addr2);
        blood_unregister_allocation(addr3);
    }

    #[test]
    fn test_snapshot_chain_validate_failure_in_child() {
        // Register allocations
        let addr1 = 0xBBBB_1000u64;
        let addr2 = 0xBBBB_2000u64;

        let gen1 = blood_register_allocation(addr1, 64);
        let gen2 = blood_register_allocation(addr2, 64);

        let parent = blood_snapshot_create();
        let child = blood_snapshot_create_child(parent);

        unsafe {
            // Add parent entry (valid)
            blood_snapshot_add_entry(parent, addr1, gen1);

            // Add child entry (will become stale)
            blood_snapshot_add_entry(child, addr2, gen2);

            // Free the child's reference
            blood_unregister_allocation(addr2);

            // Validation should fail at child's entry (index 1 in child)
            let result = blood_snapshot_validate(child);
            assert_eq!(result, 1, "Should detect stale reference in child at index 1");

            blood_snapshot_destroy(child);
            blood_snapshot_destroy(parent);
        }

        // Clean up
        blood_unregister_allocation(addr1);
    }

    #[test]
    fn test_snapshot_chain_validate_failure_in_parent() {
        // Register allocations
        let addr1 = 0xCCCC_1000u64;
        let addr2 = 0xCCCC_2000u64;

        let gen1 = blood_register_allocation(addr1, 64);
        let gen2 = blood_register_allocation(addr2, 64);

        let parent = blood_snapshot_create();
        let child = blood_snapshot_create_child(parent);

        unsafe {
            // Add parent entry (will become stale)
            blood_snapshot_add_entry(parent, addr1, gen1);

            // Add child entry (valid)
            blood_snapshot_add_entry(child, addr2, gen2);

            // Free the parent's reference
            blood_unregister_allocation(addr1);

            // Validation walks child first (1 entry), then parent
            // Parent's stale entry is at offset 1 (child's len) + 1 = 2
            let result = blood_snapshot_validate(child);
            assert!(result > 0, "Should detect stale reference in parent");

            blood_snapshot_destroy(child);
            blood_snapshot_destroy(parent);
        }

        // Clean up
        blood_unregister_allocation(addr2);
    }

    #[test]
    fn test_snapshot_validate_local_only() {
        // Register allocations
        let addr1 = 0xDDDD_1000u64;
        let addr2 = 0xDDDD_2000u64;

        let gen1 = blood_register_allocation(addr1, 64);
        let gen2 = blood_register_allocation(addr2, 64);

        let parent = blood_snapshot_create();
        let child = blood_snapshot_create_child(parent);

        unsafe {
            // Add parent entry (will become stale)
            blood_snapshot_add_entry(parent, addr1, gen1);

            // Add child entry (valid)
            blood_snapshot_add_entry(child, addr2, gen2);

            // Free the parent's reference
            blood_unregister_allocation(addr1);

            // Local validation of child should succeed (parent's stale entry not checked)
            let result = blood_snapshot_validate_local(child);
            assert_eq!(result, 0, "Local validation of child should succeed");

            // Full chain validation should fail
            let chain_result = blood_snapshot_validate(child);
            assert!(chain_result > 0, "Chain validation should fail due to parent");

            blood_snapshot_destroy(child);
            blood_snapshot_destroy(parent);
        }

        // Clean up
        blood_unregister_allocation(addr2);
    }

    #[test]
    fn test_snapshot_set_parent_after_creation() {
        let parent = blood_snapshot_create();
        let orphan = blood_snapshot_create();

        unsafe {
            // Initially no parent
            assert!(!blood_snapshot_has_parent(orphan));
            assert_eq!(blood_snapshot_chain_depth(orphan), 1);

            // Set parent after creation
            blood_snapshot_set_parent(orphan, parent);

            // Now has parent
            assert!(blood_snapshot_has_parent(orphan));
            assert_eq!(blood_snapshot_get_parent(orphan), parent);
            assert_eq!(blood_snapshot_chain_depth(orphan), 2);

            blood_snapshot_destroy(orphan);
            blood_snapshot_destroy(parent);
        }
    }

    #[test]
    fn test_snapshot_deeply_nested_chain() {
        // Create a chain of 5 nested snapshots
        let mut snapshots: Vec<SnapshotHandle> = Vec::new();

        let root = blood_snapshot_create();
        snapshots.push(root);

        for _ in 0..4 {
            let child = blood_snapshot_create_child(*snapshots.last().unwrap());
            snapshots.push(child);
        }

        unsafe {
            // Verify depths
            for (i, snap) in snapshots.iter().enumerate() {
                assert_eq!(blood_snapshot_chain_depth(*snap), i + 1);
            }

            // Add one entry to each snapshot
            for (i, snap) in snapshots.iter().enumerate() {
                let addr = 0xEEEE_0000u64 + (i as u64 * 0x100);
                blood_snapshot_add_entry(*snap, addr, (i + 1) as u32);
            }

            // Total length of deepest should be 5
            assert_eq!(blood_snapshot_total_len(*snapshots.last().unwrap()), 5);

            // Clean up from deepest to root
            for snap in snapshots.iter().rev() {
                blood_snapshot_destroy(*snap);
            }
        }
    }

    // ========================================================================
    // Region Suspension Tests
    // ========================================================================

    #[test]
    fn test_region_create_destroy() {
        let region_id = blood_region_create(1024, 1024 * 1024);
        assert!(region_id != 0, "Region creation should succeed");

        // Check initial state
        assert_eq!(blood_region_get_suspend_count(region_id), 0);
        assert_eq!(blood_region_get_status(region_id), 0); // Active

        blood_region_destroy(region_id);
    }

    #[test]
    fn test_region_suspend_resume() {
        let region_id = blood_region_create(1024, 1024 * 1024);
        assert!(region_id != 0);

        // Initially not suspended
        assert_eq!(blood_region_is_suspended(region_id), 0);
        assert_eq!(blood_region_get_suspend_count(region_id), 0);

        // Suspend once
        let count1 = blood_region_suspend(region_id);
        assert_eq!(count1, 1);
        assert_eq!(blood_region_is_suspended(region_id), 1);
        assert_eq!(blood_region_get_status(region_id), 1); // Suspended

        // Suspend again
        let count2 = blood_region_suspend(region_id);
        assert_eq!(count2, 2);

        // Resume once - still suspended
        let result1 = blood_region_resume(region_id);
        let new_count = result1 & 0xFFFF;
        let should_dealloc = (result1 >> 16) != 0;
        assert_eq!(new_count, 1);
        assert!(!should_dealloc);
        assert_eq!(blood_region_is_suspended(region_id), 1);

        // Resume again - no longer suspended
        let result2 = blood_region_resume(region_id);
        let new_count2 = result2 & 0xFFFF;
        let should_dealloc2 = (result2 >> 16) != 0;
        assert_eq!(new_count2, 0);
        assert!(!should_dealloc2);
        assert_eq!(blood_region_is_suspended(region_id), 0);
        assert_eq!(blood_region_get_status(region_id), 0); // Back to Active

        blood_region_destroy(region_id);
    }

    #[test]
    fn test_region_exit_scope_immediate() {
        let region_id = blood_region_create(1024, 1024 * 1024);
        assert!(region_id != 0);

        // Exit scope when not suspended should return 1 (deallocate immediately)
        let should_dealloc = blood_region_exit_scope(region_id);
        assert_eq!(should_dealloc, 1);

        blood_region_destroy(region_id);
    }

    #[test]
    fn test_region_exit_scope_deferred() {
        let region_id = blood_region_create(1024, 1024 * 1024);
        assert!(region_id != 0);

        // Suspend the region
        blood_region_suspend(region_id);

        // Exit scope when suspended should return 0 (deferred)
        let should_dealloc = blood_region_exit_scope(region_id);
        assert_eq!(should_dealloc, 0);
        assert_eq!(blood_region_is_pending_deallocation(region_id), 1);
        assert_eq!(blood_region_get_status(region_id), 2); // PendingDeallocation

        // Resume - should now indicate deallocation needed
        let result = blood_region_resume(region_id);
        let new_count = result & 0xFFFF;
        let should_dealloc_now = (result >> 16) != 0;
        assert_eq!(new_count, 0);
        assert!(should_dealloc_now, "Should deallocate after last resume");

        blood_region_destroy(region_id);
    }

    #[test]
    fn test_region_alloc() {
        let region_id = blood_region_create(1024, 1024 * 1024);
        assert!(region_id != 0);

        // Allocate some memory
        let addr = blood_region_alloc(region_id, 64, 8);
        assert!(addr != 0, "Region allocation should succeed");

        // Allocate more
        let addr2 = blood_region_alloc(region_id, 128, 16);
        assert!(addr2 != 0, "Second allocation should succeed");
        assert_ne!(addr, addr2, "Allocations should be at different addresses");

        blood_region_destroy(region_id);
    }

    #[test]
    fn test_system_alloc_header_roundtrip() {
        use super::ALLOC_HEADER_SIZE;

        // Allocate memory via system allocator (no active region)
        let ptr = blood_alloc_simple(100);
        assert!(ptr != 0, "Allocation should succeed");

        // Verify we can read the size from the header
        unsafe {
            let header_ptr = (ptr as usize - ALLOC_HEADER_SIZE) as *const usize;
            assert_eq!(*header_ptr, 100, "Header should contain allocation size");
        }

        // Realloc should preserve data and update header
        let new_ptr = blood_realloc(ptr, 200);
        assert!(new_ptr != 0, "Realloc should succeed");

        unsafe {
            let header_ptr = (new_ptr as usize - ALLOC_HEADER_SIZE) as *const usize;
            assert_eq!(*header_ptr, 200, "Header should contain new size after realloc");
        }

        // Free should work correctly with header
        blood_free_simple(new_ptr);
    }

    #[test]
    fn test_system_alloc_statistics() {
        use crate::memory::system_alloc_stats;

        // Get baseline stats
        let (alloc_before, free_before, count_before, free_count_before) = system_alloc_stats();

        // Allocate and free
        let ptr = blood_alloc_simple(256);
        assert!(ptr != 0);

        let (alloc_after, _, count_after, _) = system_alloc_stats();
        assert!(alloc_after >= alloc_before + 256, "Alloc bytes should increase");
        assert!(count_after >= count_before + 1, "Alloc count should increase");

        blood_free_simple(ptr);

        let (_, free_after, _, free_count_after) = system_alloc_stats();
        assert!(free_after >= free_before + 256, "Free bytes should increase");
        assert!(free_count_after >= free_count_before + 1, "Free count should increase");
    }

    #[test]
    fn test_continuation_suspended_regions() {
        let region_id = blood_region_create(1024, 1024 * 1024);
        let continuation_id = 12345u64; // Fake continuation ID

        // Add suspended region
        blood_continuation_add_suspended_region(continuation_id, region_id);

        // Region should be suspended
        assert_eq!(blood_region_is_suspended(region_id), 1);
        assert_eq!(blood_region_get_suspend_count(region_id), 1);

        // Take suspended regions
        let mut regions = [0u64; 16];
        let count = unsafe {
            blood_continuation_take_suspended_regions(
                continuation_id,
                regions.as_mut_ptr(),
                regions.len(),
            )
        };

        assert_eq!(count, 1);
        assert_eq!(regions[0], region_id);

        // Region should be resumed
        assert_eq!(blood_region_is_suspended(region_id), 0);

        blood_region_destroy(region_id);
    }

    #[test]
    fn test_continuation_multiple_suspended_regions() {
        let region1 = blood_region_create(1024, 1024 * 1024);
        let region2 = blood_region_create(1024, 1024 * 1024);
        let region3 = blood_region_create(1024, 1024 * 1024);
        let continuation_id = 67890u64;

        // Add all regions
        blood_continuation_add_suspended_region(continuation_id, region1);
        blood_continuation_add_suspended_region(continuation_id, region2);
        blood_continuation_add_suspended_region(continuation_id, region3);

        // All regions should be suspended
        assert_eq!(blood_region_is_suspended(region1), 1);
        assert_eq!(blood_region_is_suspended(region2), 1);
        assert_eq!(blood_region_is_suspended(region3), 1);

        // Take suspended regions
        let mut regions = [0u64; 16];
        let count = unsafe {
            blood_continuation_take_suspended_regions(
                continuation_id,
                regions.as_mut_ptr(),
                regions.len(),
            )
        };

        assert_eq!(count, 3);

        // All regions should be resumed
        assert_eq!(blood_region_is_suspended(region1), 0);
        assert_eq!(blood_region_is_suspended(region2), 0);
        assert_eq!(blood_region_is_suspended(region3), 0);

        blood_region_destroy(region1);
        blood_region_destroy(region2);
        blood_region_destroy(region3);
    }

    #[test]
    fn test_deferred_deallocation_via_continuation() {
        let region_id = blood_region_create(1024, 1024 * 1024);
        let continuation_id = 11111u64;

        // Suspend via continuation
        blood_continuation_add_suspended_region(continuation_id, region_id);

        // Exit scope - should be deferred
        let immediate = blood_region_exit_scope(region_id);
        assert_eq!(immediate, 0, "Should defer deallocation");
        assert_eq!(blood_region_is_pending_deallocation(region_id), 1);

        // Taking regions should handle deallocation
        let mut regions = [0u64; 16];
        let count = unsafe {
            blood_continuation_take_suspended_regions(
                continuation_id,
                regions.as_mut_ptr(),
                regions.len(),
            )
        };

        assert_eq!(count, 1);
        // Region should have been destroyed by take_suspended_regions
        // since it was pending deallocation
    }
}

/// Debug function to print vec index information
///
/// # Safety
///
/// This function only prints its arguments and has no pointer validity requirements.
#[no_mangle]
pub unsafe extern "C" fn debug_vec_index(idx: i64, byte_offset: i64) {
    eprintln!("[debug_vec_index] idx={} byte_offset={}", idx, byte_offset);
}

/// Debug function to print data ptr and element ptr
///
/// # Safety
///
/// Both `data_ptr` and `elem_ptr` must be valid (or null) pointers. They are only
/// printed as addresses and not dereferenced.
#[no_mangle]
pub unsafe extern "C" fn debug_vec_ptrs(data_ptr: *const u8, elem_ptr: *const u8) {
    eprintln!("[debug_vec_ptrs] data_ptr={:p} elem_ptr={:p} offset={}", 
        data_ptr, elem_ptr, elem_ptr as usize - data_ptr as usize);
}

/// Debug function to read enum at pointer and print its contents
///
/// # Safety
///
/// `ptr` must be a valid pointer to at least 16 bytes of readable memory
/// containing a Blood enum layout (i32 tag at offset 0, i64 payload at offset 8).
#[no_mangle]
pub unsafe extern "C" fn debug_read_enum_at(ptr: *const u8) {
    let tag = std::ptr::read(ptr as *const i32);
    let payload = std::ptr::read(ptr.add(8) as *const i64);
    eprintln!("[debug_read_enum_at] ptr={:p} tag={} payload={}", ptr, tag, payload);
}

/// Debug function to read and print enum at local storage
///
/// # Safety
///
/// - `ptr` must be a valid pointer to at least 16 bytes of readable memory
///   containing a Blood enum layout (i32 tag at offset 0, i64 payload at offset 8).
/// - `name` must be a valid pointer to `name_len` bytes of valid UTF-8 data.
/// - `name_len` must be non-negative and accurately reflect the length of the name string.
#[no_mangle]
pub unsafe extern "C" fn debug_local_enum(ptr: *const u8, name: *const u8, name_len: i64) {
    let name_slice = std::slice::from_raw_parts(name, name_len as usize);
    let name_str = String::from_utf8_lossy(name_slice);
    let tag = std::ptr::read(ptr as *const i32);
    let payload = std::ptr::read(ptr.add(8) as *const i64);
    eprintln!("[debug_local_enum] {} at {:p}: tag={} payload={}", name_str, ptr, tag, payload);
}

// ============================================================================
// Simple I/O Functions
// ============================================================================

/// Print a boolean value without newline.
#[no_mangle]
pub extern "C" fn print_bool(val: bool) {
    print!("{}", val);
    let _ = io::stdout().flush();
}

/// Print a boolean value with newline.
#[no_mangle]
pub extern "C" fn println_bool(val: bool) {
    println!("{}", val);
}

/// Print a character without newline.
///
/// The character is represented as an i32 (Unicode code point).
#[no_mangle]
pub extern "C" fn print_char(c: i32) {
    if let Some(ch) = char::from_u32(c as u32) {
        print!("{}", ch);
        let _ = io::stdout().flush();
    }
}

/// Print a character with newline.
///
/// The character is represented as an i32 (Unicode code point).
#[no_mangle]
pub extern "C" fn println_char(c: i32) {
    if let Some(ch) = char::from_u32(c as u32) {
        println!("{}", ch);
    }
}

/// Print a 32-bit float without newline.
#[no_mangle]
pub extern "C" fn print_f32(val: f32) {
    print!("{}", val);
    let _ = io::stdout().flush();
}

/// Print a 32-bit float with newline.
#[no_mangle]
pub extern "C" fn println_f32(val: f32) {
    println!("{}", val);
}

/// Print a 64-bit float without newline.
#[no_mangle]
pub extern "C" fn print_f64(val: f64) {
    print!("{}", val);
    let _ = io::stdout().flush();
}

/// Print a 64-bit float with newline.
#[no_mangle]
pub extern "C" fn println_f64(val: f64) {
    println!("{}", val);
}

/// Print a 32-bit float with specified decimal precision, without newline.
#[no_mangle]
pub extern "C" fn print_f32_prec(val: f32, prec: i32) {
    print!("{:.1$}", val, prec as usize);
    let _ = io::stdout().flush();
}

/// Print a 32-bit float with specified decimal precision, with newline.
#[no_mangle]
pub extern "C" fn println_f32_prec(val: f32, prec: i32) {
    println!("{:.1$}", val, prec as usize);
}

/// Print a 64-bit float with specified decimal precision, without newline.
#[no_mangle]
pub extern "C" fn print_f64_prec(val: f64, prec: i32) {
    print!("{:.1$}", val, prec as usize);
    let _ = io::stdout().flush();
}

/// Print a 64-bit float with specified decimal precision, with newline.
#[no_mangle]
pub extern "C" fn println_f64_prec(val: f64, prec: i32) {
    println!("{:.1$}", val, prec as usize);
}

/// Print an unsigned 64-bit integer without newline.
///
/// In the Blood ABI, u64 is passed as i64. We reinterpret the bits.
#[no_mangle]
pub extern "C" fn print_u64(val: i64) {
    print!("{}", val as u64);
    let _ = io::stdout().flush();
}

/// Print an unsigned 64-bit integer with newline.
///
/// In the Blood ABI, u64 is passed as i64. We reinterpret the bits.
#[no_mangle]
pub extern "C" fn println_u64(val: i64) {
    println!("{}", val as u64);
}

/// Print a 64-bit integer without newline.
#[no_mangle]
pub extern "C" fn print_i64(val: i64) {
    print!("{}", val);
    let _ = io::stdout().flush();
}

// ============================================================================
// Simple Memory Allocation (for Vec/collections)
// ============================================================================

/// Size of the header prepended to non-region allocations to store the size.
/// This enables correct `Layout` reconstruction at deallocation time, which is
/// required by Rust's `GlobalAlloc` contract.
const ALLOC_HEADER_SIZE: usize = 8;

/// Allocate memory of the given size with default alignment (8).
///
/// This function is region-aware: if a region is active, it allocates from the region
/// (enabling memory reuse via the slab allocator). Otherwise, it uses the system allocator.
///
/// Returns the address as i64, or 0 on failure.
#[no_mangle]
pub extern "C" fn blood_alloc_simple(size: i64) -> i64 {
    if size <= 0 {
        return 0;
    }

    let size_usize = size as usize;

    // Check if a region is active — if so, allocate from it
    let region_id = current_active_region();
    if region_id != 0 {
        let addr = blood_region_alloc(region_id, size_usize, 8);
        if addr != 0 {
            // Register in slot registry with region info for proper cleanup
            register_allocation_with_region(addr, size_usize, region_id);
        }
        return addr as i64;
    }

    // No active region — use system allocator with inline size header.
    // We prepend an 8-byte header containing the allocation size, which allows
    // us to reconstruct the correct Layout at deallocation time (required by
    // Rust's GlobalAlloc contract). This adds 8 bytes overhead per allocation,
    // much less than the 72+ bytes per SlotEntry if we tracked in the registry.
    unsafe {
        let total_size = size_usize + ALLOC_HEADER_SIZE;
        let layout = std::alloc::Layout::from_size_align(total_size, 8)
            .expect("blood_alloc_simple: invalid layout");
        let ptr = runtime_alloc(layout);
        if ptr.is_null() {
            0
        } else {
            // Record allocation for statistics
            record_system_alloc(size_usize);
            // Store size in header, return pointer past header
            *(ptr as *mut usize) = size_usize;
            (ptr as usize + ALLOC_HEADER_SIZE) as i64
        }
    }
}

/// Reallocate memory at the given address to a new size.
///
/// This function is region-aware. For region allocations, it allocates a new buffer,
/// copies data, and returns the old buffer to the free list. For system allocations,
/// it uses the system realloc.
///
/// Returns the new address as i64, or 0 on failure.
/// If ptr is 0, behaves like blood_alloc_simple.
#[no_mangle]
pub extern "C" fn blood_realloc(ptr: i64, new_size: i64) -> i64 {
    if new_size <= 0 {
        return 0;
    }

    // If ptr is null, just allocate new memory
    if ptr == 0 {
        return blood_alloc_simple(new_size);
    }

    let old_addr = ptr as u64;
    let new_size_usize = new_size as usize;

    // Check if old allocation is in a region by looking up slot registry
    let old_info = get_slot_info(old_addr);

    match old_info {
        Some((old_size, region_id)) if region_id != 0 => {
            // --- Diagnostic counters ---
            REALLOC_COUNT.fetch_add(1, AtomicOrdering::Relaxed);

            // Compute slab-rounded old size for accurate waste tracking
            let class = crate::memory::size_class_for(old_size);
            let actual_old_size = if class != crate::memory::SIZE_CLASS_LARGE {
                let slot = crate::memory::slot_size_for_class(class);
                slot.max(old_size)
            } else {
                old_size
            };

            // Try in-place extension if growing
            if new_size_usize > old_size {
                let registry = get_region_registry();
                let reg = registry.lock();
                if let Some(region) = reg.get(&region_id) {
                    if region.try_realloc_in_place(old_addr as *mut u8, old_size, new_size_usize) {
                        REALLOC_INPLACE_ELIGIBLE.fetch_add(1, AtomicOrdering::Relaxed);
                        REALLOC_INPLACE_SAVED_BYTES.fetch_add(actual_old_size as u64, AtomicOrdering::Relaxed);
                        // Update slot registry with new size
                        drop(reg);
                        crate::memory::update_slot_size(old_addr, new_size_usize);
                        return ptr; // Same address, buffer extended in place
                    }
                }
            }

            // Fallback: allocate new, copy, free old
            REALLOC_WASTED_BYTES.fetch_add(actual_old_size as u64, AtomicOrdering::Relaxed);
            let new_ptr = blood_alloc_simple(new_size);
            if new_ptr == 0 {
                return 0;
            }

            // Copy old data to new buffer
            let copy_size = old_size.min(new_size_usize);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    old_addr as *const u8,
                    new_ptr as *mut u8,
                    copy_size,
                );
            }

            // Free old buffer (returns to region free list)
            blood_free_simple(ptr);

            new_ptr
        }
        _ => {
            // Non-region allocation: read old size from header, allocate new, copy, free old.
            // We can't use runtime_realloc directly because the pointer has a header offset.
            unsafe {
                let header_ptr = (ptr as usize - ALLOC_HEADER_SIZE) as *const usize;
                let old_size = *header_ptr;

                // Allocate new buffer (blood_alloc_simple handles the header)
                let new_ptr = blood_alloc_simple(new_size);
                if new_ptr == 0 {
                    return 0;
                }

                // Copy old data to new buffer
                let copy_size = old_size.min(new_size_usize);
                std::ptr::copy_nonoverlapping(
                    old_addr as *const u8,
                    new_ptr as *mut u8,
                    copy_size,
                );

                // Free old buffer (blood_free_simple handles the header)
                blood_free_simple(ptr);

                new_ptr
            }
        }
    }
}

/// Free memory allocated by blood_alloc_simple.
///
/// This function is region-aware: if the allocation belongs to a region, it adds
/// the address to the region's free list for reuse. Otherwise, it returns the
/// memory to the system allocator.
///
/// # Lock Ordering
///
/// Locks are acquired in consistent order to prevent deadlocks:
/// 1. Slot registry (via `unregister_allocation`) - released before step 2
/// 2. Region registry (if region allocation)
/// 3. Region's internal free list mutex
///
/// If ptr is 0, this is a no-op.
#[no_mangle]
pub extern "C" fn blood_free_simple(ptr: i64) {
    if ptr == 0 {
        return;
    }

    let addr = ptr as u64;

    // Step 1: Unregister from slot registry and get region/size info.
    // The slot registry lock is released when this returns.
    let info = unregister_allocation(addr);

    if let Some((region_id, size_class)) = info {
        if region_id != 0 {
            // Step 2 & 3: Region allocation - add to free list for reuse.
            let registry = get_region_registry();
            let reg = registry.lock();
            if let Some(region) = reg.get(&region_id) {
                let _ = region.deallocate(addr, size_class);
            }
            // Don't call system dealloc - region owns the memory
            return;
        }
    }

    // Non-region allocation: read size from header and return to system allocator.
    // The header is located 8 bytes before the user pointer.
    unsafe {
        let header_ptr = (ptr as usize - ALLOC_HEADER_SIZE) as *mut u8;
        let size = *(header_ptr as *const usize);
        // Record free for statistics
        record_system_free(size);
        let total_size = size + ALLOC_HEADER_SIZE;
        let layout = std::alloc::Layout::from_size_align(total_size, 8)
            .expect("blood_free_simple: invalid layout");
        runtime_dealloc(header_ptr, layout);
    }
}

/// Copy memory from src to dst (non-overlapping).
///
/// Returns dst. If size <= 0 or either pointer is 0, returns dst unchanged.
#[no_mangle]
pub extern "C" fn blood_memcpy(dst: i64, src: i64, size: i64) -> i64 {
    if size <= 0 || dst == 0 || src == 0 {
        return dst;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, size as usize);
    }
    dst
}

// ============================================================================
// Pointer Read/Write Intrinsics
// ============================================================================

/// Read an i32 value from the given memory address.
///
/// # Safety
/// The pointer must be valid and properly aligned for i32.
#[no_mangle]
pub extern "C" fn ptr_read_i32(ptr: i64) -> i32 {
    unsafe { *(ptr as *const i32) }
}

/// Write an i32 value to the given memory address.
///
/// # Safety
/// The pointer must be valid and properly aligned for i32.
#[no_mangle]
pub extern "C" fn ptr_write_i32(ptr: i64, val: i32) {
    unsafe { *(ptr as *mut i32) = val; }
}

/// Read an i64 value from the given memory address.
///
/// # Safety
/// The pointer must be valid and properly aligned for i64.
#[no_mangle]
pub extern "C" fn ptr_read_i64(ptr: i64) -> i64 {
    unsafe { *(ptr as *const i64) }
}

/// Write an i64 value to the given memory address.
///
/// # Safety
/// The pointer must be valid and properly aligned for i64.
#[no_mangle]
pub extern "C" fn ptr_write_i64(ptr: i64, val: i64) {
    unsafe { *(ptr as *mut i64) = val; }
}

/// Read a u64 value from the given memory address.
///
/// In the Blood ABI, u64 is represented as i64. The bits are reinterpreted.
///
/// # Safety
/// The pointer must be valid and properly aligned for u64/i64.
#[no_mangle]
pub extern "C" fn ptr_read_u64(ptr: i64) -> i64 {
    unsafe { *(ptr as *const i64) }
}

/// Write a u64 value to the given memory address.
///
/// In the Blood ABI, u64 is represented as i64. The bits are reinterpreted.
///
/// # Safety
/// The pointer must be valid and properly aligned for u64/i64.
#[no_mangle]
pub extern "C" fn ptr_write_u64(ptr: i64, val: i64) {
    unsafe { *(ptr as *mut i64) = val; }
}

/// Read a u8 value from the given memory address, returned as i8.
///
/// # Safety
/// The pointer must be valid.
#[no_mangle]
pub extern "C" fn ptr_read_u8(ptr: i64) -> i8 {
    unsafe { *(ptr as *const i8) }
}

/// Write a u8 value (as i8) to the given memory address.
///
/// # Safety
/// The pointer must be valid.
#[no_mangle]
pub extern "C" fn ptr_write_u8(ptr: i64, val: i8) {
    unsafe { *(ptr as *mut i8) = val; }
}

/// Read an f64 value from the given memory address.
///
/// # Safety
/// The pointer must be valid and properly aligned for f64.
#[no_mangle]
pub extern "C" fn ptr_read_f64(ptr: i64) -> f64 {
    unsafe { *(ptr as *const f64) }
}

/// Write an f64 value to the given memory address.
///
/// # Safety
/// The pointer must be valid and properly aligned for f64.
#[no_mangle]
pub extern "C" fn ptr_write_f64(ptr: i64, val: f64) {
    unsafe { *(ptr as *mut f64) = val; }
}

// ============================================================================
// Assertions
// ============================================================================

/// Assert that a condition is true (non-zero). Aborts if false.
#[no_mangle]
pub extern "C" fn blood_assert(cond: i32) {
    if cond == 0 {
        eprintln!("BLOOD RUNTIME PANIC: assertion failed");
        std::process::abort();
    }
}

/// Assert that two boolean values are equal. Aborts if not.
#[no_mangle]
pub extern "C" fn blood_assert_eq_bool(a: bool, b: bool) {
    if a != b {
        eprintln!("BLOOD RUNTIME PANIC: assertion failed: bool values not equal ({} != {})", a, b);
        std::process::abort();
    }
}

/// Assert that two integer values (i32) are equal. Aborts if not.
#[no_mangle]
pub extern "C" fn blood_assert_eq_int(a: i32, b: i32) {
    if a != b {
        eprintln!("BLOOD RUNTIME PANIC: assertion failed: {} != {}", a, b);
        std::process::abort();
    }
}

// ============================================================================
// Generation Management
// ============================================================================

/// Increment the generation counter at the given slot address.
///
/// This invalidates all existing references to the slot, causing stale
/// reference checks to fail.
///
/// # Safety
/// `slot_ptr` must be a valid pointer or null. If non-null, it must point
/// to a registered allocation whose generation can be incremented.
#[no_mangle]
pub extern "C" fn blood_increment_generation(slot_ptr: *mut u8) {
    if slot_ptr.is_null() {
        return;
    }
    // The address is the slot identifier in the allocation registry.
    // We look up the address and bump its generation.
    let addr = slot_ptr as u64;
    crate::memory::increment_generation(addr);
}

// ============================================================================
// Time Functions
// ============================================================================

/// Lazily initialized start time for monotonic clock.
static START_TIME: OnceLock<std::time::Instant> = OnceLock::new();

/// Get the start time, initializing it on first call.
fn get_start_time() -> std::time::Instant {
    *START_TIME.get_or_init(std::time::Instant::now)
}

/// Get nanoseconds elapsed since program start (or first call to a time function).
///
/// This provides high-precision monotonic timing suitable for performance measurement.
/// The value wraps after ~584 years, which should be sufficient.
#[no_mangle]
pub extern "C" fn blood_clock_nanos() -> u64 {
    get_start_time().elapsed().as_nanos() as u64
}

/// Get microseconds elapsed since program start (or first call to a time function).
///
/// Convenience function for microsecond-precision timing.
#[no_mangle]
pub extern "C" fn blood_clock_micros() -> u64 {
    get_start_time().elapsed().as_micros() as u64
}

/// Get milliseconds elapsed since program start (or first call to a time function).
///
/// Convenience function for coarse timing (e.g., phase timing in compilers).
#[no_mangle]
pub extern "C" fn blood_clock_millis() -> u64 {
    get_start_time().elapsed().as_millis() as u64
}

/// Get seconds elapsed since program start as a floating-point value.
///
/// Returns the elapsed time with sub-second precision as an f64.
#[no_mangle]
pub extern "C" fn blood_clock_secs_f64() -> f64 {
    get_start_time().elapsed().as_secs_f64()
}

/// Reinterpret an f64 value as its IEEE 754 bit representation (u64).
///
/// This is equivalent to `f64::to_bits()` — a bitwise reinterpretation,
/// not a numeric conversion. Used by the self-hosted compiler to avoid
/// raw pointer casts which are not yet supported in its codegen.
#[no_mangle]
pub extern "C" fn blood_float64_to_bits(value: f64) -> u64 {
    value.to_bits()
}

// ============================================================================
// OS Thread Primitives
// ============================================================================

/// Spawn an OS thread that calls `func_ptr(arg)`.
///
/// `func_ptr` is a raw function pointer (the fn_ptr field of a Blood fat pointer,
/// cast to u64 by the Blood program). The function signature must be `fn(u64) -> u64`.
/// `arg` is an opaque u64 passed to the function.
///
/// Returns an opaque thread handle (Box<JoinHandle> leaked as u64), or 0 on failure.
///
/// # Safety
/// The function pointer must be a valid `extern "C" fn(u64) -> u64`.
#[no_mangle]
pub unsafe extern "C" fn blood_thread_spawn(func_ptr: u64, arg: u64) -> u64 {
    let f: extern "C" fn(u64) -> u64 = std::mem::transmute(func_ptr);

    match std::thread::Builder::new().spawn(move || f(arg)) {
        Ok(handle) => Box::into_raw(Box::new(handle)) as u64,
        Err(_) => 0,
    }
}

/// Join a thread previously spawned by `blood_thread_spawn`.
///
/// `handle` is the opaque handle returned by `blood_thread_spawn`.
/// Returns 0 on success, 1 on failure (invalid handle or thread panicked).
///
/// # Safety
/// `handle` must be a valid handle returned by `blood_thread_spawn`, and must
/// not be joined more than once.
#[no_mangle]
pub unsafe extern "C" fn blood_thread_join(handle: u64) -> u64 {
    if handle == 0 {
        return 1;
    }
    let boxed: Box<std::thread::JoinHandle<u64>> =
        Box::from_raw(handle as *mut std::thread::JoinHandle<u64>);
    match boxed.join() {
        Ok(_) => 0,
        Err(_) => 1,
    }
}
