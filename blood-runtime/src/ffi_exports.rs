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

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::io::{self, Write};
use std::sync::OnceLock;

use parking_lot::Mutex;

use crate::memory::{BloodPtr, PointerMetadata, generation, get_slot_generation, Region};
use crate::continuation::{
    ContinuationRef, Continuation, EffectContext,
    register_continuation, take_continuation, has_continuation,
};

/// Fiber handle for continuation capture.
pub type FiberHandle = u64;

/// Continuation handle for FFI.
pub type ContinuationHandle = u64;

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

/// Blood str slice representation {ptr, len}.
#[repr(C)]
pub struct BloodStr {
    /// Pointer to the string data.
    pub ptr: *const u8,
    /// Length of the string in bytes.
    pub len: u64,
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
    let ptr = std::alloc::alloc(layout);
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

        match handle.read_until(b'\n', &mut *buf) {
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
    let ptr = unsafe { std::alloc::alloc(layout) };
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

    // Use 16-byte alignment for BloodPtr compatibility
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
    unregister_allocation(addr);

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
    let vec = &*(ev as *const Vec<EvidenceEntry>);

    let registry = get_effect_registry();
    let reg = registry.lock();

    // Search from most recent to oldest handler (reverse order)
    for ev_entry in vec.iter().rev() {
        let handler_index = ev_entry.registry_index;
        let instance_state = ev_entry.state;
        if let Some(registry_entry) = reg.get(handler_index as usize) {
            if registry_entry.effect_id == effect_id {
                // Found the handler for this effect
                if let Some(&op_fn) = registry_entry.operations.get(op_index as usize) {
                    if !op_fn.is_null() {
                        // Call the operation handler with INSTANCE state, not registry state
                        // The handler signature is: fn(state: *void, args: *i64, arg_count: i64, continuation: i64) -> i64
                        type OpHandler = unsafe extern "C" fn(*mut c_void, *const i64, i64, i64) -> i64;
                        let handler: OpHandler = std::mem::transmute(op_fn);
                        return handler(instance_state, args, arg_count, continuation);
                    }
                }
            }
        }
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

use crate::memory::{register_allocation, unregister_allocation};

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

/// Unregister an allocation from the slot registry.
///
/// This should be called by the runtime allocator when memory is freed.
/// The slot is marked as deallocated but retained for stale reference detection.
///
/// # Arguments
/// * `address` - The address being freed
#[no_mangle]
pub extern "C" fn blood_unregister_allocation(address: u64) {
    unregister_allocation(address)
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
                eprintln!("BLOOD RUNTIME ERROR: Failed to resume continuation {}", continuation);
                0
            }
        }
    } else {
        eprintln!("BLOOD RUNTIME ERROR: Continuation {} not found or already consumed", continuation);
        0
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
