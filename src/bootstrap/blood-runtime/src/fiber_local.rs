//! Fiber-Local Storage
//!
//! This module provides fiber-local storage (FLS), similar to thread-local
//! storage but scoped to individual fibers rather than OS threads.
//!
//! # Overview
//!
//! Fiber-local storage allows each fiber to have its own copy of data,
//! with automatic propagation to child fibers. This is essential for:
//!
//! - Distributed tracing (propagating trace/span IDs)
//! - Request-scoped data (auth context, request ID)
//! - Cancellation tokens (hierarchical cancellation)
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::fiber_local::{FiberLocal, fiber_local};
//!
//! // Define a fiber-local value
//! fiber_local! {
//!     static REQUEST_ID: FiberLocal<String> = FiberLocal::new();
//! }
//!
//! // In a request handler:
//! REQUEST_ID.set("req-12345".to_string());
//!
//! // Later, in nested code:
//! if let Some(id) = REQUEST_ID.get() {
//!     println!("Request: {}", id);
//! }
//! ```

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique key for a fiber-local variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FiberLocalKey(u64);

impl FiberLocalKey {
    /// Generate a new unique key.
    pub fn new() -> Self {
        static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_KEY.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for FiberLocalKey {
    fn default() -> Self {
        Self::new()
    }
}

/// Storage for fiber-local values.
///
/// This is stored per-fiber and contains all fiber-local values for that fiber.
#[derive(Debug, Default)]
pub struct FiberLocalStorage {
    /// Map from key to boxed value.
    values: HashMap<FiberLocalKey, Box<dyn Any + Send>>,
}

impl FiberLocalStorage {
    /// Create new empty storage.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Get a value by key.
    pub fn get<T: 'static + Send>(&self, key: FiberLocalKey) -> Option<&T> {
        self.values.get(&key)?.downcast_ref::<T>()
    }

    /// Get a mutable value by key.
    pub fn get_mut<T: 'static + Send>(&mut self, key: FiberLocalKey) -> Option<&mut T> {
        self.values.get_mut(&key)?.downcast_mut::<T>()
    }

    /// Set a value by key.
    pub fn set<T: 'static + Send>(&mut self, key: FiberLocalKey, value: T) {
        self.values.insert(key, Box::new(value));
    }

    /// Remove a value by key.
    pub fn remove(&mut self, key: FiberLocalKey) -> bool {
        self.values.remove(&key).is_some()
    }

    /// Check if a key exists.
    pub fn contains(&self, key: FiberLocalKey) -> bool {
        self.values.contains_key(&key)
    }

    /// Clear all values.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Clone values that implement Clone (for propagation to child fibers).
    ///
    /// This creates a new storage with cloned values for types that implement Clone.
    /// Values that don't implement Clone are not propagated.
    pub fn clone_propagated(&self) -> Self {
        // For now, we can't easily clone arbitrary boxed values.
        // In production, we'd use a registration system where types
        // explicitly opt into propagation.
        Self::new()
    }

    /// Merge another storage into this one (for context inheritance).
    pub fn merge_from(&mut self, other: &FiberLocalStorage) {
        // For types we can clone, copy them over.
        // This is a simplified implementation.
        for (&key, _value) in &other.values {
            if !self.values.contains_key(&key) {
                // In a full implementation, we'd clone the value if possible
                // For now, skip non-cloneable values
            }
        }
    }
}

/// A fiber-local variable.
///
/// This provides access to a value that is unique to each fiber.
/// The value is lazily initialized on first access.
pub struct FiberLocal<T> {
    /// The unique key for this variable.
    #[allow(dead_code)]
    key: FiberLocalKey,
    /// Default value initializer.
    #[allow(dead_code)]
    init: fn() -> T,
    /// Phantom data for T.
    _marker: std::marker::PhantomData<T>,
}

impl<T: 'static + Send> FiberLocal<T> {
    /// Create a new fiber-local variable with a default initializer.
    pub const fn new(init: fn() -> T) -> Self {
        Self {
            key: FiberLocalKey(0), // Will be assigned at runtime
            init,
            _marker: std::marker::PhantomData,
        }
    }

    /// Get the key for this variable, initializing it if needed.
    #[allow(dead_code)]
    fn get_key(&self) -> FiberLocalKey {
        // In a real implementation, we'd use lazy initialization.
        // For now, generate a new key based on the address of self.
        let ptr = self as *const _ as u64;
        FiberLocalKey(ptr)
    }
}

// Thread-local storage for the current fiber's local storage.
// This is used when we don't have access to the fiber directly.
thread_local! {
    static CURRENT_FIBER_STORAGE: RefCell<Option<FiberLocalStorage>> = RefCell::new(None);
}

/// Context for the current fiber.
///
/// This provides access to fiber-local storage without needing
/// a direct reference to the fiber.
pub struct FiberContext;

impl FiberContext {
    /// Run code with fiber-local storage available.
    pub fn with_storage<F, R>(_storage: &mut FiberLocalStorage, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        // Save the current storage and install the new one
        CURRENT_FIBER_STORAGE.with(|cell| {
            let old = cell.borrow_mut().take();
            // Note: This is a simplified implementation.
            // In production, we'd properly swap the storage.
            let result = f();
            *cell.borrow_mut() = old;
            result
        })
    }

    /// Get a value from the current fiber's storage.
    pub fn get<T: 'static + Send + Clone>(key: FiberLocalKey) -> Option<T> {
        CURRENT_FIBER_STORAGE.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|storage| storage.get::<T>(key).cloned())
        })
    }

    /// Set a value in the current fiber's storage.
    pub fn set<T: 'static + Send>(key: FiberLocalKey, value: T) {
        CURRENT_FIBER_STORAGE.with(|cell| {
            if let Some(storage) = cell.borrow_mut().as_mut() {
                storage.set(key, value);
            }
        });
    }

    /// Initialize fiber-local storage for the current context.
    pub fn init_storage() {
        CURRENT_FIBER_STORAGE.with(|cell| {
            *cell.borrow_mut() = Some(FiberLocalStorage::new());
        });
    }

    /// Clear fiber-local storage for the current context.
    pub fn clear_storage() {
        CURRENT_FIBER_STORAGE.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// Propagation mode for fiber-local values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PropagationMode {
    /// Don't propagate to child fibers.
    #[default]
    None,
    /// Copy value to child fibers (value must implement Clone).
    Copy,
    /// Share reference with child fibers (requires Arc).
    Share,
}

/// A propagatable fiber-local value.
///
/// This extends FiberLocal with explicit propagation semantics.
pub struct PropagatedLocal<T> {
    /// The underlying storage key.
    key: FiberLocalKey,
    /// Propagation mode.
    mode: PropagationMode,
    /// Phantom data.
    _marker: std::marker::PhantomData<T>,
}

impl<T: 'static + Send + Clone> PropagatedLocal<T> {
    /// Create a new propagated local with copy semantics.
    pub fn new_copied() -> Self {
        Self {
            key: FiberLocalKey::new(),
            mode: PropagationMode::Copy,
            _marker: std::marker::PhantomData,
        }
    }

    /// Get the current value.
    pub fn get(&self) -> Option<T> {
        FiberContext::get(self.key)
    }

    /// Set the current value.
    pub fn set(&self, value: T) {
        FiberContext::set(self.key, value);
    }

    /// Get the propagation mode.
    pub fn propagation_mode(&self) -> PropagationMode {
        self.mode
    }
}

impl<T: 'static + Send + Sync> PropagatedLocal<std::sync::Arc<T>> {
    /// Create a new propagated local with share semantics.
    pub fn new_shared() -> Self {
        Self {
            key: FiberLocalKey::new(),
            mode: PropagationMode::Share,
            _marker: std::marker::PhantomData,
        }
    }
}

/// Macro for defining fiber-local variables.
///
/// This is similar to `thread_local!` but for fibers.
///
/// # Example
///
/// ```rust,ignore
/// fiber_local! {
///     static MY_VALUE: FiberLocal<i32> = FiberLocal::new(|| 0);
/// }
/// ```
#[macro_export]
macro_rules! fiber_local {
    ($(static $name:ident: FiberLocal<$ty:ty> = FiberLocal::new($init:expr);)*) => {
        $(
            static $name: $crate::fiber_local::FiberLocal<$ty> =
                $crate::fiber_local::FiberLocal::new($init);
        )*
    };
}

/// Trace context for distributed tracing.
///
/// This provides a standard way to propagate trace information
/// through fiber hierarchies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    /// Trace ID (identifies the entire trace).
    pub trace_id: u128,
    /// Span ID (identifies this specific operation).
    pub span_id: u64,
    /// Parent span ID (if any).
    pub parent_span_id: Option<u64>,
    /// Sampling decision.
    pub sampled: bool,
}

impl TraceContext {
    /// Create a new root trace context.
    pub fn new_root() -> Self {
        Self {
            trace_id: rand_u128(),
            span_id: rand_u64(),
            parent_span_id: None,
            sampled: true,
        }
    }

    /// Create a child span from this context.
    pub fn child_span(&self) -> Self {
        Self {
            trace_id: self.trace_id,
            span_id: rand_u64(),
            parent_span_id: Some(self.span_id),
            sampled: self.sampled,
        }
    }

    /// Format as W3C traceparent header.
    pub fn to_traceparent(&self) -> String {
        format!(
            "00-{:032x}-{:016x}-{:02x}",
            self.trace_id,
            self.span_id,
            if self.sampled { 1 } else { 0 }
        )
    }

    /// Parse from W3C traceparent header.
    pub fn from_traceparent(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 4 || parts[0] != "00" {
            return None;
        }

        let trace_id = u128::from_str_radix(parts[1], 16).ok()?;
        let span_id = u64::from_str_radix(parts[2], 16).ok()?;
        let flags = u8::from_str_radix(parts[3], 16).ok()?;

        Some(Self {
            trace_id,
            span_id,
            parent_span_id: None,
            sampled: flags & 1 != 0,
        })
    }
}

impl Default for TraceContext {
    fn default() -> Self {
        Self::new_root()
    }
}

// Simple random number generation for trace IDs.
fn rand_u128() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos();
    let thread_id = std::thread::current().id();
    let thread_hash = format!("{:?}", thread_id).len() as u128;
    nanos ^ (thread_hash << 64)
}

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_nanos() as u64
}

/// Request context for web applications.
///
/// A common pattern for propagating request-scoped data.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Unique request ID.
    pub request_id: String,
    /// Trace context for distributed tracing.
    pub trace: TraceContext,
    /// User ID (if authenticated).
    pub user_id: Option<String>,
    /// Additional metadata.
    pub metadata: HashMap<String, String>,
}

impl RequestContext {
    /// Create a new request context.
    pub fn new() -> Self {
        Self {
            request_id: format!("req-{:016x}", rand_u64()),
            trace: TraceContext::new_root(),
            user_id: None,
            metadata: HashMap::new(),
        }
    }

    /// Create with a specific request ID.
    pub fn with_request_id(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            trace: TraceContext::new_root(),
            user_id: None,
            metadata: HashMap::new(),
        }
    }

    /// Set the user ID.
    pub fn with_user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fiber_local_key() {
        let key1 = FiberLocalKey::new();
        let key2 = FiberLocalKey::new();
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_fiber_local_storage() {
        let mut storage = FiberLocalStorage::new();
        let key = FiberLocalKey::new();

        // Initially empty
        assert!(!storage.contains(key));
        assert!(storage.get::<i32>(key).is_none());

        // Set and get
        storage.set(key, 42i32);
        assert!(storage.contains(key));
        assert_eq!(storage.get::<i32>(key), Some(&42));

        // Modify
        if let Some(val) = storage.get_mut::<i32>(key) {
            *val = 100;
        }
        assert_eq!(storage.get::<i32>(key), Some(&100));

        // Remove
        assert!(storage.remove(key));
        assert!(!storage.contains(key));
    }

    #[test]
    fn test_trace_context() {
        let root = TraceContext::new_root();
        assert!(root.sampled);
        assert!(root.parent_span_id.is_none());

        let child = root.child_span();
        assert_eq!(child.trace_id, root.trace_id);
        assert_eq!(child.parent_span_id, Some(root.span_id));
        assert_ne!(child.span_id, root.span_id);
    }

    #[test]
    fn test_traceparent_format() {
        let ctx = TraceContext {
            trace_id: 0x0102030405060708090a0b0c0d0e0f10,
            span_id: 0x1112131415161718,
            parent_span_id: None,
            sampled: true,
        };

        let traceparent = ctx.to_traceparent();
        assert!(traceparent.starts_with("00-"));

        let parsed = TraceContext::from_traceparent(&traceparent).unwrap();
        assert_eq!(parsed.trace_id, ctx.trace_id);
        assert_eq!(parsed.span_id, ctx.span_id);
        assert_eq!(parsed.sampled, ctx.sampled);
    }

    #[test]
    fn test_request_context() {
        let ctx = RequestContext::new()
            .with_user("user123")
            .with_metadata("tenant", "acme");

        assert!(ctx.request_id.starts_with("req-"));
        assert_eq!(ctx.user_id, Some("user123".to_string()));
        assert_eq!(ctx.metadata.get("tenant"), Some(&"acme".to_string()));
    }

    #[test]
    fn test_propagation_mode() {
        assert_eq!(PropagationMode::default(), PropagationMode::None);
    }
}
