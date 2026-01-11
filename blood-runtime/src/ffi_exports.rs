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

use std::ffi::{c_char, c_int, c_void, CStr};
use std::io::{self, Write};

use crate::memory::{BloodPtr, PointerMetadata, generation, get_slot_generation};

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

/// Print a string (no newline).
///
/// # Safety
/// The pointer must be a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn print_str(s: *const c_char) {
    if !s.is_null() {
        if let Ok(cstr) = CStr::from_ptr(s).to_str() {
            print!("{cstr}");
            let _ = io::stdout().flush();
        }
    }
}

/// Print a string with newline.
///
/// # Safety
/// The pointer must be a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn println_str(s: *const c_char) {
    if !s.is_null() {
        if let Ok(cstr) = CStr::from_ptr(s).to_str() {
            println!("{cstr}");
        }
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
        blood_panic(b"blood_alloc_or_abort: null out_generation pointer\0".as_ptr() as *const i8);
    }

    // Use 16-byte alignment for BloodPtr compatibility
    let align = 16.max(std::mem::align_of::<usize>());
    let layout = match std::alloc::Layout::from_size_align(size, align) {
        Ok(l) => l,
        Err(_) => {
            blood_panic(b"blood_alloc_or_abort: invalid layout\0".as_ptr() as *const i8);
        }
    };

    let ptr = std::alloc::alloc(layout);
    if ptr.is_null() {
        blood_panic(b"blood_alloc_or_abort: out of memory\0".as_ptr() as *const i8);
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
/// # Safety
/// The address must point to a valid Blood allocation header.
#[no_mangle]
pub unsafe extern "C" fn blood_get_generation(addr: u64) -> u32 {
    if addr == 0 {
        return 0;
    }

    // In a real implementation, this would read from the slot header
    // For now, return a placeholder
    1
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
#[no_mangle]
pub extern "C" fn blood_evidence_create() -> EvidenceHandle {
    let ev = Box::new(Vec::<EvidenceEntry>::new());
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
                        // The handler signature is: fn(state: *void, args: *i64, arg_count: i64) -> i64
                        type OpHandler = unsafe extern "C" fn(*mut c_void, *const i64, i64) -> i64;
                        let handler: OpHandler = std::mem::transmute(op_fn);
                        return handler(instance_state, args, arg_count);
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
struct GenerationSnapshot {
    entries: Vec<SnapshotEntry>,
}

/// Opaque handle to a generation snapshot.
pub type SnapshotHandle = *mut c_void;

/// Create a new generation snapshot.
///
/// Returns a handle to an empty snapshot, or null on failure.
#[no_mangle]
pub extern "C" fn blood_snapshot_create() -> SnapshotHandle {
    let snapshot = Box::new(GenerationSnapshot {
        entries: Vec::new(),
    });
    Box::into_raw(snapshot) as SnapshotHandle
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
/// Returns 0 if all generations match (valid), or the 1-based index
/// of the first stale entry found. A return value > 0 indicates a
/// stale reference at entry (return_value - 1).
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
                    return (i + 1) as i64;
                }
            }
        }
    }

    0 // All valid
}

/// Get the number of entries in a snapshot.
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

/// Destroy a generation snapshot.
///
/// # Safety
/// The snapshot handle must have been created with blood_snapshot_create.
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
// ============================================================================
// Fiber/Continuation Support (for effect handlers)
// ============================================================================

use crate::continuation::{
    ContinuationRef, Continuation, EffectContext,
    register_continuation, take_continuation, has_continuation,
};

/// Fiber handle for continuation capture.
pub type FiberHandle = u64;

/// Continuation handle for FFI.
pub type ContinuationHandle = u64;

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

// ============================================================================
// Work-Stealing Scheduler FFI
// ============================================================================

use std::sync::OnceLock;
use parking_lot::Mutex;
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

/// Called when a stale reference is dereferenced.
///
/// By default, this aborts the program.
#[no_mangle]
pub extern "C" fn blood_stale_reference_panic(expected: u32, actual: u32) -> ! {
    eprintln!(
        "BLOOD RUNTIME ERROR: Stale reference detected!\n\
         Expected generation: {expected}, Actual: {actual}\n\
         This indicates use-after-free. Aborting."
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
            extern "C" fn mock_op(_state: *mut c_void, _args: *const i64, _arg_count: i64) -> i64 { 0 }
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
        unsafe {
            extern "C" fn double_op(_state: *mut c_void, args: *const i64, _arg_count: i64) -> i64 {
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
            let result = blood_perform(100, 0, args.as_ptr(), 1);
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
            extern "C" fn noop(_state: *mut c_void, _args: *const i64, _arg_count: i64) -> i64 { 0 }
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
}
