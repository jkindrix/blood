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

/// Context for an effect operation that may need to suspend.
///
/// This is passed to handler operations and provides methods for:
/// - Resuming immediately (tail-resumptive path)
/// - Capturing a continuation for later resumption
#[derive(Debug)]
pub struct EffectContext {
    /// Whether we're in a tail-resumptive handler.
    pub is_tail_resumptive: bool,
    /// The captured continuation (if any).
    pub continuation: Option<ContinuationRef>,
    /// The resume value (if already provided).
    pub resume_value: Option<i64>,
}

impl EffectContext {
    /// Create a new effect context for tail-resumptive handlers.
    pub fn tail_resumptive() -> Self {
        Self {
            is_tail_resumptive: true,
            continuation: None,
            resume_value: None,
        }
    }

    /// Create a new effect context with a captured continuation.
    pub fn with_continuation(k: ContinuationRef) -> Self {
        Self {
            is_tail_resumptive: false,
            continuation: Some(k),
            resume_value: None,
        }
    }

    /// Set the resume value (for immediate resumption).
    pub fn set_resume_value(&mut self, value: i64) {
        self.resume_value = Some(value);
    }
}

impl Default for EffectContext {
    fn default() -> Self {
        Self::tail_resumptive()
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
