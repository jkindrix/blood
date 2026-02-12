//! # Continuation Support for Effect Handlers
//!
//! This module implements delimited continuations for algebraic effect handlers.
//!
//! ## Design
//!
//! Blood uses a hybrid approach to continuations:
//!
//! 1. **Tail-Resumptive Optimization**: For effects where `resume` is called exactly
//!    once in tail position (State, Reader, Writer), no continuation capture is needed.
//!    The handler is called directly and returns to the caller.
//!
//! 2. **Closure-Based Continuations**: For effects that need to suspend and resume
//!    (Async, Error), we use closure-based continuations. The "rest of the computation"
//!    is captured as a boxed closure.
//!
//! ## Technical References
//!
//! - [Retrofitting Effect Handlers onto OCaml](https://dl.acm.org/doi/10.1145/3453483.3454039) (PLDI'21)
//! - [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
//! - [Effect Handlers for C via Coroutines](https://dl.acm.org/doi/10.1145/3649818) (OOPSLA'24)
//!
//! ## One-Shot vs Multi-Shot
//!
//! Blood continuations are **one-shot** by default, meaning each continuation can
//! only be resumed once. This is sufficient for most use cases and is much cheaper
//! than multi-shot continuations (which require copying).
//!
//! Multi-shot continuations can be added later using explicit `clone` operations.

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use parking_lot::Mutex;

/// Unique identifier for a continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContinuationId(u64);

impl ContinuationId {
    /// Create a new continuation ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Global continuation ID counter.
static NEXT_CONTINUATION_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a new unique continuation ID.
fn next_continuation_id() -> ContinuationId {
    ContinuationId(NEXT_CONTINUATION_ID.fetch_add(1, Ordering::Relaxed))
}

/// Type alias for the complex continuation closure signature.
type ContinuationClosure = Box<dyn FnOnce(Box<dyn Any + Send>) -> Box<dyn Any + Send> + Send>;

/// A captured continuation.
///
/// This represents the "rest of the computation" after an effect is performed.
/// It can be invoked with a value to continue execution.
pub struct Continuation {
    /// Unique identifier.
    id: ContinuationId,
    /// The boxed continuation closure.
    /// Takes the resume value and returns the final result.
    closure: Option<ContinuationClosure>,
    /// Whether this continuation has been consumed (one-shot enforcement).
    consumed: bool,
    /// Regions that were suspended when this continuation was captured.
    ///
    /// These regions have their suspend_count incremented and will have
    /// it decremented when this continuation is resumed or dropped.
    /// Stored as raw u64 IDs to avoid circular dependency with memory module.
    suspended_regions: Vec<u64>,
}

impl Continuation {
    /// Create a new continuation from a closure.
    pub fn new<F, T, R>(f: F) -> Self
    where
        F: FnOnce(T) -> R + Send + 'static,
        T: Any + Send + 'static,
        R: Any + Send + 'static,
    {
        Self {
            id: next_continuation_id(),
            closure: Some(Box::new(move |val: Box<dyn Any + Send>| {
                let t = *val.downcast::<T>().expect("continuation type mismatch");
                let r = f(t);
                Box::new(r) as Box<dyn Any + Send>
            })),
            consumed: false,
            suspended_regions: Vec::new(),
        }
    }

    /// Create a new continuation with suspended regions.
    pub fn with_suspended_regions<F, T, R>(f: F, suspended_regions: Vec<u64>) -> Self
    where
        F: FnOnce(T) -> R + Send + 'static,
        T: Any + Send + 'static,
        R: Any + Send + 'static,
    {
        Self {
            id: next_continuation_id(),
            closure: Some(Box::new(move |val: Box<dyn Any + Send>| {
                let t = *val.downcast::<T>().expect("continuation type mismatch");
                let r = f(t);
                Box::new(r) as Box<dyn Any + Send>
            })),
            consumed: false,
            suspended_regions,
        }
    }

    /// Get the continuation ID.
    pub fn id(&self) -> ContinuationId {
        self.id
    }

    /// Check if this continuation has been consumed.
    pub fn is_consumed(&self) -> bool {
        self.consumed
    }

    /// Get the list of suspended region IDs.
    pub fn suspended_regions(&self) -> &[u64] {
        &self.suspended_regions
    }

    /// Add a suspended region to this continuation.
    pub fn add_suspended_region(&mut self, region_id: u64) {
        self.suspended_regions.push(region_id);
    }

    /// Take the suspended regions, leaving an empty vector.
    pub fn take_suspended_regions(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.suspended_regions)
    }

    /// Check if this continuation has any suspended regions.
    pub fn has_suspended_regions(&self) -> bool {
        !self.suspended_regions.is_empty()
    }

    /// Resume the continuation with a value.
    ///
    /// # Panics
    /// Panics if the continuation has already been consumed (one-shot violation).
    pub fn resume<T: Any + Send + 'static, R: Any + Send + 'static>(mut self, value: T) -> R {
        if self.consumed {
            panic!("Continuation already resumed (one-shot violation)");
        }
        self.consumed = true;

        let closure = self.closure.take().expect("continuation closure missing");
        let result = closure(Box::new(value));
        *result.downcast::<R>().expect("continuation return type mismatch")
    }

    /// Try to resume the continuation with a value.
    ///
    /// Returns `None` if the continuation has already been consumed.
    pub fn try_resume<T: Any + Send + 'static, R: Any + Send + 'static>(
        mut self,
        value: T,
    ) -> Option<R> {
        if self.consumed {
            return None;
        }
        self.consumed = true;

        let closure = self.closure.take()?;
        let result = closure(Box::new(value));
        result.downcast::<R>().ok().map(|r| *r)
    }
}

impl std::fmt::Debug for Continuation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Continuation")
            .field("id", &self.id)
            .field("consumed", &self.consumed)
            .field("suspended_regions", &self.suspended_regions.len())
            .finish()
    }
}

/// A continuation reference for FFI.
///
/// This is a handle that can be passed across FFI boundaries and used
/// to resume a continuation from generated code.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ContinuationRef {
    /// The continuation ID.
    pub id: u64,
}

impl ContinuationRef {
    /// Create a null reference.
    pub const fn null() -> Self {
        Self { id: 0 }
    }

    /// Check if this is a null reference.
    pub fn is_null(&self) -> bool {
        self.id == 0
    }
}

/// Global continuation registry.
///
/// Stores captured continuations by ID for later resumption.
static CONTINUATION_REGISTRY: OnceLock<Mutex<HashMap<u64, Continuation>>> = OnceLock::new();

/// Get or initialize the continuation registry.
fn get_continuation_registry() -> &'static Mutex<HashMap<u64, Continuation>> {
    CONTINUATION_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a continuation and get its reference.
pub fn register_continuation(k: Continuation) -> ContinuationRef {
    let id = k.id().as_u64();
    let mut registry = get_continuation_registry().lock();
    registry.insert(id, k);
    ContinuationRef { id }
}

/// Take a continuation from the registry.
///
/// Removes and returns the continuation, or `None` if not found.
pub fn take_continuation(r: ContinuationRef) -> Option<Continuation> {
    let mut registry = get_continuation_registry().lock();
    registry.remove(&r.id)
}

/// Check if a continuation exists in the registry.
pub fn has_continuation(r: ContinuationRef) -> bool {
    let registry = get_continuation_registry().lock();
    registry.contains_key(&r.id)
}

// ============================================================================
// Effect Continuation Context
// ============================================================================

/// Opaque handle to a generation snapshot (matches FFI type).
pub type SnapshotHandle = *mut std::ffi::c_void;

/// Context for an effect operation that may need to suspend.
///
/// This is passed to handler operations and provides methods for:
/// - Resuming immediately (tail-resumptive path)
/// - Capturing a continuation for later resumption
/// - Managing generation snapshots for stale reference detection
///
/// ## Generation Snapshots
///
/// When an effect operation captures a continuation, any generational references
/// in the captured environment must be validated when the continuation resumes.
/// The snapshot captures the expected generations at capture time, and validation
/// on resume detects stale references (freed memory that was reallocated).
///
/// Based on:
/// - [Vale's Generational References](https://verdagon.dev/blog/generational-references)
/// - [Generational Arena Pattern](https://crates.io/crates/generational-arena)
#[derive(Debug)]
pub struct EffectContext {
    /// Whether we're in a tail-resumptive handler.
    pub is_tail_resumptive: bool,
    /// The captured continuation (if any).
    pub continuation: Option<ContinuationRef>,
    /// The resume value (if already provided).
    pub resume_value: Option<i64>,
    /// Generation snapshot for validating captured references.
    ///
    /// This is captured when a continuation is created and validated
    /// when the continuation is resumed. If validation fails, a
    /// StaleReference effect is raised.
    pub snapshot: Option<SnapshotHandle>,
}

impl EffectContext {
    /// Create a new effect context for tail-resumptive handlers.
    ///
    /// Tail-resumptive handlers don't need snapshots because they resume
    /// immediately without capturing a continuation.
    pub fn tail_resumptive() -> Self {
        Self {
            is_tail_resumptive: true,
            continuation: None,
            resume_value: None,
            snapshot: None,
        }
    }

    /// Create a new effect context with a captured continuation.
    pub fn with_continuation(k: ContinuationRef) -> Self {
        Self {
            is_tail_resumptive: false,
            continuation: Some(k),
            resume_value: None,
            snapshot: None,
        }
    }

    /// Create a new effect context with a continuation and snapshot.
    ///
    /// The snapshot contains captured generation values that will be
    /// validated when the continuation is resumed.
    pub fn with_continuation_and_snapshot(k: ContinuationRef, snapshot: SnapshotHandle) -> Self {
        Self {
            is_tail_resumptive: false,
            continuation: Some(k),
            resume_value: None,
            snapshot: if snapshot.is_null() { None } else { Some(snapshot) },
        }
    }

    /// Set the resume value (for immediate resumption).
    pub fn set_resume_value(&mut self, value: i64) {
        self.resume_value = Some(value);
    }

    /// Set the generation snapshot.
    pub fn set_snapshot(&mut self, snapshot: SnapshotHandle) {
        self.snapshot = if snapshot.is_null() { None } else { Some(snapshot) };
    }

    /// Take the snapshot, leaving None in its place.
    ///
    /// This transfers ownership of the snapshot to the caller.
    pub fn take_snapshot(&mut self) -> Option<SnapshotHandle> {
        self.snapshot.take()
    }

    /// Check if this context has a snapshot attached.
    pub fn has_snapshot(&self) -> bool {
        self.snapshot.is_some()
    }
}

impl Default for EffectContext {
    fn default() -> Self {
        Self::tail_resumptive()
    }
}

// ============================================================================
// Continuation with Snapshot
// ============================================================================

/// A continuation bundled with its generation snapshot.
///
/// This type ensures that when a continuation is resumed, the snapshot
/// is validated to detect stale references.
pub struct ContinuationWithSnapshot {
    /// The continuation to resume.
    pub continuation: Continuation,
    /// The generation snapshot captured at suspension time.
    pub snapshot: SnapshotHandle,
}

impl ContinuationWithSnapshot {
    /// Create a new continuation with snapshot.
    pub fn new(continuation: Continuation, snapshot: SnapshotHandle) -> Self {
        Self { continuation, snapshot }
    }

    /// Get the continuation ID.
    pub fn id(&self) -> ContinuationId {
        self.continuation.id()
    }

    /// Check if the continuation has been consumed.
    pub fn is_consumed(&self) -> bool {
        self.continuation.is_consumed()
    }

    /// Get the snapshot handle.
    pub fn snapshot(&self) -> SnapshotHandle {
        self.snapshot
    }
}

impl std::fmt::Debug for ContinuationWithSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContinuationWithSnapshot")
            .field("continuation", &self.continuation)
            .field("has_snapshot", &!self.snapshot.is_null())
            .finish()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_continuation_id_generation() {
        let id1 = next_continuation_id();
        let id2 = next_continuation_id();
        assert_ne!(id1, id2);
        assert!(id2.0 > id1.0);
    }

    #[test]
    fn test_continuation_resume() {
        let k = Continuation::new(|x: i32| x + 1);
        let result: i32 = k.resume(41);
        assert_eq!(result, 42);
    }

    #[test]
    fn test_continuation_one_shot_via_registry() {
        // Test that a continuation can only be taken from the registry once
        let k = Continuation::new(|x: i32| x + 1);
        let k_ref = register_continuation(k);

        // First take should succeed
        assert!(has_continuation(k_ref));
        let k1 = take_continuation(k_ref);
        assert!(k1.is_some());

        // Second take should return None (one-shot enforcement)
        assert!(!has_continuation(k_ref));
        let k2 = take_continuation(k_ref);
        assert!(k2.is_none());

        // Resume the first continuation to ensure it works
        let result: i32 = k1.unwrap().resume(41);
        assert_eq!(result, 42);
    }

    #[test]
    fn test_continuation_is_consumed() {
        // Test that is_consumed correctly tracks state
        let k = Continuation::new(|x: i32| x + 1);
        assert!(!k.is_consumed());
        // After resuming, the continuation is consumed
        let _result: i32 = k.resume(1);
        // Note: We can't check is_consumed after resume because
        // resume takes ownership. The ownership model enforces one-shot.
    }

    #[test]
    fn test_continuation_registry() {
        let k = Continuation::new(|x: i32| x * 2);
        let r = register_continuation(k);

        assert!(has_continuation(r));

        let k2 = take_continuation(r).expect("continuation should exist");
        let result: i32 = k2.resume(21);
        assert_eq!(result, 42);

        assert!(!has_continuation(r));
    }

    #[test]
    fn test_effect_context_default() {
        let ctx = EffectContext::default();
        assert!(ctx.is_tail_resumptive);
        assert!(ctx.continuation.is_none());
        assert!(ctx.resume_value.is_none());
        assert!(ctx.snapshot.is_none());
    }

    #[test]
    fn test_effect_context_with_snapshot() {
        let k = Continuation::new(|x: i32| x);
        let k_ref = register_continuation(k);

        // Create a fake snapshot handle (just for testing the context)
        let fake_snapshot = 0x1234 as SnapshotHandle;

        let ctx = EffectContext::with_continuation_and_snapshot(k_ref, fake_snapshot);
        assert!(!ctx.is_tail_resumptive);
        assert!(ctx.continuation.is_some());
        assert!(ctx.has_snapshot());
        assert_eq!(ctx.snapshot.unwrap(), fake_snapshot);

        // Clean up
        let _ = take_continuation(k_ref);
    }

    #[test]
    fn test_effect_context_null_snapshot() {
        let k = Continuation::new(|x: i32| x);
        let k_ref = register_continuation(k);

        // Null snapshot should result in None
        let ctx = EffectContext::with_continuation_and_snapshot(k_ref, std::ptr::null_mut());
        assert!(!ctx.has_snapshot());

        // Clean up
        let _ = take_continuation(k_ref);
    }

    #[test]
    fn test_effect_context_take_snapshot() {
        let k = Continuation::new(|x: i32| x);
        let k_ref = register_continuation(k);

        let fake_snapshot = 0x5678 as SnapshotHandle;
        let mut ctx = EffectContext::with_continuation_and_snapshot(k_ref, fake_snapshot);

        // Take the snapshot
        let taken = ctx.take_snapshot();
        assert!(taken.is_some());
        assert_eq!(taken.unwrap(), fake_snapshot);

        // Snapshot should now be None
        assert!(!ctx.has_snapshot());
        assert!(ctx.take_snapshot().is_none());

        // Clean up
        let _ = take_continuation(k_ref);
    }

    #[test]
    fn test_continuation_with_snapshot_struct() {
        let k = Continuation::new(|x: i32| x + 1);
        let fake_snapshot = 0xABCD as SnapshotHandle;

        let k_with_snap = ContinuationWithSnapshot::new(k, fake_snapshot);
        assert!(!k_with_snap.is_consumed());
        assert_eq!(k_with_snap.snapshot(), fake_snapshot);
    }

    #[test]
    fn test_try_resume_success() {
        let k = Continuation::new(|x: i32| x * 3);
        let result: Option<i32> = k.try_resume(14);
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_continuation_ref_null() {
        let null_ref = ContinuationRef::null();
        assert!(null_ref.is_null());
        assert_eq!(null_ref.id, 0);

        let valid_ref = ContinuationRef { id: 42 };
        assert!(!valid_ref.is_null());
    }

    #[test]
    fn test_continuation_with_string() {
        let k = Continuation::new(|s: String| format!("Hello, {}!", s));
        let result: String = k.resume("World".to_string());
        assert_eq!(result, "Hello, World!");
    }

    #[test]
    fn test_continuation_with_complex_type() {
        #[derive(Debug, PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        let k = Continuation::new(|p: Point| Point {
            x: p.x * 2,
            y: p.y * 2,
        });
        let result: Point = k.resume(Point { x: 10, y: 20 });
        assert_eq!(result, Point { x: 20, y: 40 });
    }

    #[test]
    fn test_effect_context_with_continuation() {
        let k = Continuation::new(|x: i32| x);
        let k_ref = register_continuation(k);

        let ctx = EffectContext::with_continuation(k_ref);
        assert!(!ctx.is_tail_resumptive);
        assert!(ctx.continuation.is_some());
        assert_eq!(ctx.continuation.unwrap().id, k_ref.id);

        // Clean up
        let _ = take_continuation(k_ref);
    }

    #[test]
    fn test_effect_context_resume_value() {
        let mut ctx = EffectContext::tail_resumptive();
        assert!(ctx.resume_value.is_none());

        ctx.set_resume_value(42);
        assert_eq!(ctx.resume_value, Some(42));
    }

    #[test]
    fn test_continuation_id_uniqueness() {
        let ids: Vec<ContinuationId> = (0..100).map(|_| next_continuation_id()).collect();

        // All IDs should be unique
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j]);
            }
        }
    }

    #[test]
    fn test_continuation_debug_format() {
        let k = Continuation::new(|x: i32| x);
        let debug_str = format!("{:?}", k);
        assert!(debug_str.contains("Continuation"));
        assert!(debug_str.contains("consumed: false"));
    }
}
