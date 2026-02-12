//! Arena allocation infrastructure.
//!
//! This module provides arena allocators for efficient memory management
//! during compilation. Arena allocation offers several benefits:
//!
//! - **Fast allocation**: O(1) bump allocation
//! - **Cache-friendly**: Contiguous memory layout
//! - **Simple deallocation**: All memory freed at once
//! - **Reduced fragmentation**: No per-object overhead
//!
//! # Usage
//!
//! The arena is primarily designed for AST nodes during parsing:
//!
//! ```rust,ignore
//! use bloodc::arena::Arena;
//!
//! let arena = Arena::new();
//! let node = arena.alloc(MyAstNode { ... });
//! // 'node' is valid for the lifetime of 'arena'
//! ```
//!
//! # Architecture
//!
//! The arena uses a typed bump allocator strategy:
//! - New chunks are allocated as needed (8KB default)
//! - Objects are bump-allocated within chunks
//! - All memory is freed when the arena is dropped
//!
//! # Future Work
//!
//! This module currently provides a simple typed arena. Future improvements:
//! - Interned strings arena
//! - AST node arena with type-erased storage
//! - Span interning for deduplication

use std::alloc::{alloc, dealloc, Layout};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::mem;
use std::ptr::NonNull;

/// Default chunk size for arena allocation (8KB).
const DEFAULT_CHUNK_SIZE: usize = 8 * 1024;

/// A chunk of memory in the arena.
struct Chunk {
    /// Pointer to the allocated memory.
    data: NonNull<u8>,
    /// Layout used for allocation (for deallocation).
    layout: Layout,
    /// Current offset into the chunk.
    offset: usize,
    /// Capacity of this chunk.
    capacity: usize,
}

impl Chunk {
    /// Create a new chunk with the given capacity.
    fn new(capacity: usize) -> Self {
        let layout = Layout::from_size_align(capacity, 16)
            .expect("BUG: invalid layout - capacity must be within valid bounds for 16-byte alignment");

        // SAFETY: Layout is valid and non-zero sized
        let data = unsafe {
            let ptr = alloc(layout);
            NonNull::new(ptr).expect("FATAL: memory allocation failed - system out of memory")
        };

        Self {
            data,
            layout,
            offset: 0,
            capacity,
        }
    }

    /// Try to allocate within this chunk.
    fn try_alloc(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        // Align the offset
        let aligned_offset = (self.offset + layout.align() - 1) & !(layout.align() - 1);
        let new_offset = aligned_offset + layout.size();

        if new_offset > self.capacity {
            return None;
        }

        // SAFETY: We just verified the offset is within bounds
        let ptr = unsafe {
            NonNull::new_unchecked(self.data.as_ptr().add(aligned_offset))
        };

        self.offset = new_offset;
        Some(ptr)
    }

    /// Reset the chunk for reuse.
    fn reset(&mut self) {
        self.offset = 0;
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        // SAFETY: We allocated with this layout
        unsafe {
            dealloc(self.data.as_ptr(), self.layout);
        }
    }
}

/// A typed arena allocator.
///
/// The arena provides fast bump allocation for a single type. All allocated
/// objects share the arena's lifetime and are freed when the arena is dropped.
///
/// # Example
///
/// ```rust,ignore
/// use bloodc::arena::TypedArena;
///
/// let arena: TypedArena<i32> = TypedArena::new();
/// let x = arena.alloc(42);
/// let y = arena.alloc(100);
/// assert_eq!(*x, 42);
/// ```
pub struct TypedArena<T> {
    /// The chunks of memory.
    chunks: RefCell<Vec<Chunk>>,
    /// Marker for the type.
    _marker: PhantomData<T>,
}

impl<T> TypedArena<T> {
    /// Create a new empty arena.
    pub fn new() -> Self {
        Self {
            chunks: RefCell::new(Vec::new()),
            _marker: PhantomData,
        }
    }

    /// Create a new arena with a pre-allocated chunk.
    pub fn with_capacity(capacity: usize) -> Self {
        let chunk_size = capacity * mem::size_of::<T>();
        let arena = Self::new();
        arena.chunks.borrow_mut().push(Chunk::new(chunk_size.max(DEFAULT_CHUNK_SIZE)));
        arena
    }

    /// Allocate a value in the arena.
    ///
    /// Returns a reference with the arena's lifetime.
    pub fn alloc(&self, value: T) -> &T {
        let layout = Layout::new::<T>();
        let ptr = self.alloc_raw(layout);

        // SAFETY: ptr is properly aligned and we have exclusive access
        unsafe {
            ptr.as_ptr().cast::<T>().write(value);
            &*ptr.as_ptr().cast::<T>()
        }
    }

    /// Allocate a slice of values in the arena.
    pub fn alloc_slice(&self, values: &[T]) -> &[T]
    where
        T: Copy,
    {
        if values.is_empty() {
            return &[];
        }

        let layout = Layout::array::<T>(values.len())
            .expect("BUG: invalid array layout - array size exceeds addressable memory");
        let ptr = self.alloc_raw(layout);

        // SAFETY: ptr is properly aligned and sized
        unsafe {
            std::ptr::copy_nonoverlapping(
                values.as_ptr(),
                ptr.as_ptr().cast::<T>(),
                values.len(),
            );
            std::slice::from_raw_parts(ptr.as_ptr().cast::<T>(), values.len())
        }
    }

    /// Raw allocation returning a pointer.
    fn alloc_raw(&self, layout: Layout) -> NonNull<u8> {
        let mut chunks = self.chunks.borrow_mut();

        // Try to allocate in the last chunk
        if let Some(chunk) = chunks.last_mut() {
            if let Some(ptr) = chunk.try_alloc(layout) {
                return ptr;
            }
        }

        // Need a new chunk
        let chunk_size = layout.size().max(DEFAULT_CHUNK_SIZE);
        let mut new_chunk = Chunk::new(chunk_size);
        let ptr = new_chunk.try_alloc(layout)
            .expect("BUG: fresh chunk must have space - allocation size exceeds chunk capacity");
        chunks.push(new_chunk);
        ptr
    }

    /// Clear all allocations, keeping the memory for reuse.
    ///
    /// # Safety
    ///
    /// All references to arena-allocated values become invalid.
    /// The caller must ensure no references are used after calling this.
    pub unsafe fn clear(&self) {
        for chunk in self.chunks.borrow_mut().iter_mut() {
            chunk.reset();
        }
    }
}

impl<T> Default for TypedArena<T> {
    fn default() -> Self {
        Self::new()
    }
}

// TypedArena is not Send/Sync because it uses interior mutability
// and the allocated references are tied to the arena's lifetime.

/// An arena for strings with interning support.
///
/// This arena stores strings and provides deduplication through interning.
/// Equal strings will return the same reference.
pub struct StringArena {
    /// The chunks storing string data.
    chunks: RefCell<Vec<Chunk>>,
    /// Interning map for deduplication.
    interned: RefCell<rustc_hash::FxHashMap<&'static str, &'static str>>,
}

impl StringArena {
    /// Create a new string arena.
    pub fn new() -> Self {
        Self {
            chunks: RefCell::new(Vec::new()),
            interned: RefCell::new(rustc_hash::FxHashMap::default()),
        }
    }

    /// Allocate a string in the arena.
    ///
    /// Returns a reference with static lifetime (valid as long as arena lives).
    /// Note: The 'static lifetime is a lie - the reference is actually tied
    /// to the arena's lifetime. This is safe as long as the arena outlives
    /// all uses of the returned reference.
    pub fn alloc(&self, s: &str) -> &str {
        // Check if already interned
        if let Some(&existing) = self.interned.borrow().get(s) {
            // SAFETY: We're returning a reference that was allocated in this arena
            return unsafe { std::mem::transmute::<&str, &str>(existing) };
        }

        // Allocate new string
        let layout = Layout::from_size_align(s.len(), 1)
            .expect("BUG: invalid string layout - string length exceeds addressable memory");
        let ptr = self.alloc_raw(layout);

        // SAFETY: We just allocated this memory
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), ptr.as_ptr(), s.len());
            let allocated = std::str::from_utf8_unchecked(
                std::slice::from_raw_parts(ptr.as_ptr(), s.len())
            );

            // Store in intern map with 'static lifetime (safe within arena)
            let static_ref: &'static str = std::mem::transmute(allocated);
            self.interned.borrow_mut().insert(static_ref, static_ref);

            allocated
        }
    }

    /// Raw allocation.
    fn alloc_raw(&self, layout: Layout) -> NonNull<u8> {
        let mut chunks = self.chunks.borrow_mut();

        if let Some(chunk) = chunks.last_mut() {
            if let Some(ptr) = chunk.try_alloc(layout) {
                return ptr;
            }
        }

        let chunk_size = layout.size().max(DEFAULT_CHUNK_SIZE);
        let mut new_chunk = Chunk::new(chunk_size);
        let ptr = new_chunk.try_alloc(layout)
            .expect("BUG: fresh chunk must have space - allocation size exceeds chunk capacity");
        chunks.push(new_chunk);
        ptr
    }

    /// Get statistics about the arena.
    pub fn stats(&self) -> ArenaStats {
        let chunks = self.chunks.borrow();
        ArenaStats {
            num_chunks: chunks.len(),
            total_capacity: chunks.iter().map(|c| c.capacity).sum(),
            total_used: chunks.iter().map(|c| c.offset).sum(),
            num_interned: self.interned.borrow().len(),
        }
    }
}

impl Default for StringArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about arena usage.
#[derive(Debug, Clone, Copy)]
pub struct ArenaStats {
    /// Number of chunks allocated.
    pub num_chunks: usize,
    /// Total capacity in bytes.
    pub total_capacity: usize,
    /// Total used bytes.
    pub total_used: usize,
    /// Number of interned strings (for StringArena).
    pub num_interned: usize,
}

impl ArenaStats {
    /// Calculate utilization as a percentage.
    pub fn utilization(&self) -> f64 {
        if self.total_capacity == 0 {
            0.0
        } else {
            (self.total_used as f64 / self.total_capacity as f64) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_arena_basic() {
        let arena: TypedArena<i32> = TypedArena::new();
        let a = arena.alloc(1);
        let b = arena.alloc(2);
        let c = arena.alloc(3);

        assert_eq!(*a, 1);
        assert_eq!(*b, 2);
        assert_eq!(*c, 3);
    }

    #[test]
    fn test_typed_arena_many_allocations() {
        let arena: TypedArena<u64> = TypedArena::new();
        let mut refs = Vec::new();

        for i in 0..10000 {
            refs.push(arena.alloc(i));
        }

        for (i, r) in refs.iter().enumerate() {
            assert_eq!(**r, i as u64);
        }
    }

    #[test]
    fn test_typed_arena_slice() {
        let arena: TypedArena<i32> = TypedArena::new();
        let slice = arena.alloc_slice(&[1, 2, 3, 4, 5]);

        assert_eq!(slice, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_string_arena_basic() {
        let arena = StringArena::new();
        let a = arena.alloc("hello");
        let b = arena.alloc("world");

        assert_eq!(a, "hello");
        assert_eq!(b, "world");
    }

    #[test]
    fn test_string_arena_interning() {
        let arena = StringArena::new();
        let a = arena.alloc("hello");
        let b = arena.alloc("hello");

        // Same content should give same pointer (interning)
        assert!(std::ptr::eq(a.as_ptr(), b.as_ptr()));
    }

    #[test]
    fn test_arena_stats() {
        let arena = StringArena::new();
        arena.alloc("hello");
        arena.alloc("world");
        arena.alloc("hello"); // Should be interned

        let stats = arena.stats();
        assert_eq!(stats.num_interned, 2);
        assert!(stats.total_used > 0);
    }
}
