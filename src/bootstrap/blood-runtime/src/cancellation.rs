//! Cooperative Cancellation
//!
//! This module provides cancellation tokens and sources for cooperative
//! cancellation of fibers and long-running operations.
//!
//! # Design
//!
//! Cancellation in Blood is cooperative, meaning operations must check
//! for cancellation at appropriate points (typically at effect boundaries
//! or suspension points). This is similar to Go's context cancellation.
//!
//! # Components
//!
//! - `CancellationToken`: A read-only token that can be checked for cancellation
//! - `CancellationSource`: Creates and controls cancellation tokens
//! - `CancellableScope`: RAII guard for scoped cancellation
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::cancellation::{CancellationSource, CancellationToken};
//!
//! // Create a cancellation source and token
//! let source = CancellationSource::new();
//! let token = source.token();
//!
//! // Spawn work with the token
//! std::thread::spawn(move || {
//!     while !token.is_cancelled() {
//!         // Do work...
//!     }
//!     // Cleanup on cancellation
//! });
//!
//! // Cancel after some event
//! source.cancel();
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

/// Counter for generating unique cancellation token IDs.
static TOKEN_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Shared state for cancellation tokens.
#[derive(Debug)]
struct CancellationState {
    /// Whether cancellation has been requested.
    cancelled: AtomicBool,
    /// Parent token (for hierarchical cancellation).
    parent: Option<CancellationToken>,
    /// Condition variable for waiting on cancellation.
    notify: (Mutex<bool>, Condvar),
    /// Cancellation reason (optional message).
    reason: Mutex<Option<String>>,
    /// Timestamp when cancellation was requested.
    cancelled_at: Mutex<Option<Instant>>,
}

impl CancellationState {
    fn new(parent: Option<CancellationToken>) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            parent,
            notify: (Mutex::new(false), Condvar::new()),
            reason: Mutex::new(None),
            cancelled_at: Mutex::new(None),
        }
    }
}

/// A cancellation token that can be checked for cancellation.
///
/// Tokens are created by `CancellationSource` and can be cloned cheaply.
/// Multiple tokens can share the same underlying cancellation state.
#[derive(Debug, Clone)]
pub struct CancellationToken {
    /// Unique token ID.
    id: u64,
    /// Shared cancellation state.
    state: Arc<CancellationState>,
}

impl CancellationToken {
    /// Create a new token with no parent.
    fn new() -> Self {
        Self {
            id: TOKEN_ID_COUNTER.fetch_add(1, Ordering::SeqCst),
            state: Arc::new(CancellationState::new(None)),
        }
    }

    /// Create a new token with a parent token.
    ///
    /// The child token is cancelled when either:
    /// - The child is directly cancelled
    /// - The parent is cancelled
    fn with_parent(parent: CancellationToken) -> Self {
        Self {
            id: TOKEN_ID_COUNTER.fetch_add(1, Ordering::SeqCst),
            state: Arc::new(CancellationState::new(Some(parent))),
        }
    }

    /// Get the token ID.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Check if cancellation has been requested.
    ///
    /// This also checks parent tokens recursively.
    pub fn is_cancelled(&self) -> bool {
        // Check our own state
        if self.state.cancelled.load(Ordering::SeqCst) {
            return true;
        }

        // Check parent recursively
        if let Some(ref parent) = self.state.parent {
            return parent.is_cancelled();
        }

        false
    }

    /// Get the cancellation reason, if any.
    pub fn reason(&self) -> Option<String> {
        self.state.reason.lock().ok()?.clone()
    }

    /// Get the time when cancellation was requested.
    pub fn cancelled_at(&self) -> Option<Instant> {
        *self.state.cancelled_at.lock().ok()?
    }

    /// Wait for cancellation.
    ///
    /// Blocks until cancellation is requested or the timeout expires.
    /// Returns true if cancelled, false if timeout expired.
    pub fn wait(&self, timeout: Duration) -> bool {
        // Fast path: already cancelled
        if self.is_cancelled() {
            return true;
        }

        let (lock, cvar) = &self.state.notify;
        let guard = lock.lock().unwrap();

        let result = cvar.wait_timeout_while(guard, timeout, |cancelled| {
            !*cancelled && !self.is_cancelled()
        });

        match result {
            Ok((guard, timeout_result)) => *guard || self.is_cancelled() && !timeout_result.timed_out(),
            Err(_) => self.is_cancelled(),
        }
    }

    /// Create a child token.
    ///
    /// The child token is cancelled when either the child or parent is cancelled.
    pub fn child(&self) -> CancellationToken {
        CancellationToken::with_parent(self.clone())
    }

    /// Check cancellation and return an error if cancelled.
    ///
    /// This is a convenience method for use in loops and operations.
    pub fn check(&self) -> Result<(), CancellationError> {
        if self.is_cancelled() {
            Err(CancellationError {
                reason: self.reason(),
            })
        } else {
            Ok(())
        }
    }
}

/// A token that represents no cancellation.
///
/// This token is never cancelled and can be used as a default.
static NONE_TOKEN: std::sync::OnceLock<CancellationToken> = std::sync::OnceLock::new();

impl Default for CancellationToken {
    /// Returns a token that is never cancelled.
    fn default() -> Self {
        NONE_TOKEN.get_or_init(CancellationToken::new).clone()
    }
}

/// A cancellation source that creates and controls cancellation tokens.
///
/// The source owns the ability to trigger cancellation.
/// Tokens derived from the source can only check for cancellation.
#[derive(Debug)]
pub struct CancellationSource {
    /// The token associated with this source.
    token: CancellationToken,
}

impl CancellationSource {
    /// Create a new cancellation source.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Create a new cancellation source with a parent token.
    ///
    /// Tokens from this source are cancelled when either:
    /// - This source is cancelled
    /// - The parent token is cancelled
    pub fn with_parent(parent: CancellationToken) -> Self {
        Self {
            token: CancellationToken::with_parent(parent),
        }
    }

    /// Get a token from this source.
    ///
    /// The token can be cloned and shared with other components.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Cancel all tokens from this source.
    pub fn cancel(&self) {
        self.cancel_with_reason(None);
    }

    /// Cancel all tokens with a reason.
    pub fn cancel_with_reason(&self, reason: Option<String>) {
        // Set the cancelled flag
        self.token.state.cancelled.store(true, Ordering::SeqCst);

        // Store the reason and timestamp
        if let Ok(mut r) = self.token.state.reason.lock() {
            *r = reason;
        }
        if let Ok(mut t) = self.token.state.cancelled_at.lock() {
            *t = Some(Instant::now());
        }

        // Notify waiters
        let (lock, cvar) = &self.token.state.notify;
        if let Ok(mut guard) = lock.lock() {
            *guard = true;
            cvar.notify_all();
        }
    }

    /// Check if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Cancel after a delay.
    ///
    /// Spawns a thread that cancels after the specified duration.
    /// Returns immediately.
    pub fn cancel_after(&self, delay: Duration) {
        let state = Arc::downgrade(&self.token.state);

        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if let Some(state) = state.upgrade() {
                state.cancelled.store(true, Ordering::SeqCst);
                if let Ok(mut t) = state.cancelled_at.lock() {
                    *t = Some(Instant::now());
                }
                let (lock, cvar) = &state.notify;
                if let Ok(mut guard) = lock.lock() {
                    *guard = true;
                    cvar.notify_all();
                }
            }
        });
    }
}

impl Default for CancellationSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when an operation is cancelled.
#[derive(Debug, Clone)]
pub struct CancellationError {
    /// The cancellation reason, if provided.
    pub reason: Option<String>,
}

impl std::fmt::Display for CancellationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.reason {
            Some(reason) => write!(f, "operation cancelled: {}", reason),
            None => write!(f, "operation cancelled"),
        }
    }
}

impl std::error::Error for CancellationError {}

/// RAII guard for scoped cancellation.
///
/// Automatically cancels when dropped.
#[derive(Debug)]
pub struct CancellableScope {
    source: CancellationSource,
    cancel_on_drop: bool,
}

impl CancellableScope {
    /// Create a new cancellable scope.
    pub fn new() -> Self {
        Self {
            source: CancellationSource::new(),
            cancel_on_drop: true,
        }
    }

    /// Create a new cancellable scope with a parent token.
    pub fn with_parent(parent: CancellationToken) -> Self {
        Self {
            source: CancellationSource::with_parent(parent),
            cancel_on_drop: true,
        }
    }

    /// Get a token for this scope.
    pub fn token(&self) -> CancellationToken {
        self.source.token()
    }

    /// Cancel the scope manually.
    pub fn cancel(&self) {
        self.source.cancel();
    }

    /// Prevent cancellation on drop.
    ///
    /// Call this if the scope completed successfully and
    /// cancellation should not be triggered.
    pub fn complete(mut self) {
        self.cancel_on_drop = false;
    }
}

impl Default for CancellableScope {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CancellableScope {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.source.cancel();
        }
    }
}

/// Registry for active cancellation tokens.
///
/// This allows the runtime to track and cancel all active operations.
static TOKEN_REGISTRY: std::sync::OnceLock<Mutex<std::collections::HashMap<u64, Weak<CancellationState>>>> =
    std::sync::OnceLock::new();

fn get_token_registry() -> &'static Mutex<std::collections::HashMap<u64, Weak<CancellationState>>> {
    TOKEN_REGISTRY.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Register a token in the global registry.
pub fn register_token(token: &CancellationToken) {
    let registry = get_token_registry();
    if let Ok(mut guard) = registry.lock() {
        guard.insert(token.id, Arc::downgrade(&token.state));
    }
}

/// Unregister a token from the global registry.
pub fn unregister_token(token_id: u64) {
    let registry = get_token_registry();
    if let Ok(mut guard) = registry.lock() {
        guard.remove(&token_id);
    }
}

/// Cancel all registered tokens.
///
/// This is typically called during shutdown.
pub fn cancel_all() {
    let registry = get_token_registry();
    if let Ok(guard) = registry.lock() {
        for (_, weak) in guard.iter() {
            if let Some(state) = weak.upgrade() {
                state.cancelled.store(true, Ordering::SeqCst);
                let (lock, cvar) = &state.notify;
                if let Ok(mut g) = lock.lock() {
                    *g = true;
                    cvar.notify_all();
                }
            }
        }
    }
}

/// Get the count of active tokens in the registry.
pub fn active_token_count() -> usize {
    let registry = get_token_registry();
    if let Ok(guard) = registry.lock() {
        guard.values().filter(|w| w.strong_count() > 0).count()
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation_source_basic() {
        let source = CancellationSource::new();
        let token = source.token();

        assert!(!token.is_cancelled());
        source.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_cancellation_with_reason() {
        let source = CancellationSource::new();
        let token = source.token();

        source.cancel_with_reason(Some("test reason".into()));
        assert!(token.is_cancelled());
        assert_eq!(token.reason(), Some("test reason".into()));
        assert!(token.cancelled_at().is_some());
    }

    #[test]
    fn test_hierarchical_cancellation() {
        let parent_source = CancellationSource::new();
        let parent_token = parent_source.token();

        let child_source = CancellationSource::with_parent(parent_token.clone());
        let child_token = child_source.token();

        assert!(!child_token.is_cancelled());

        // Cancelling parent should cancel child
        parent_source.cancel();
        assert!(child_token.is_cancelled());
    }

    #[test]
    fn test_child_cancellation_independent() {
        let parent_source = CancellationSource::new();
        let parent_token = parent_source.token();

        let child_source = CancellationSource::with_parent(parent_token.clone());
        let child_token = child_source.token();

        // Cancelling child should not cancel parent
        child_source.cancel();
        assert!(child_token.is_cancelled());
        assert!(!parent_token.is_cancelled());
    }

    #[test]
    fn test_token_check() {
        let source = CancellationSource::new();
        let token = source.token();

        assert!(token.check().is_ok());
        source.cancel();
        assert!(token.check().is_err());
    }

    #[test]
    fn test_cancellation_wait_immediate() {
        let source = CancellationSource::new();
        let token = source.token();

        source.cancel();

        // Should return immediately since already cancelled
        let result = token.wait(Duration::from_secs(1));
        assert!(result);
    }

    #[test]
    fn test_cancellation_wait_timeout() {
        let source = CancellationSource::new();
        let token = source.token();

        // Should timeout since not cancelled
        let result = token.wait(Duration::from_millis(10));
        assert!(!result);
    }

    #[test]
    fn test_cancellable_scope() {
        let token;
        {
            let scope = CancellableScope::new();
            token = scope.token();
            assert!(!token.is_cancelled());
            // scope drops here
        }
        // Token should be cancelled after scope drops
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_cancellable_scope_complete() {
        let token;
        {
            let scope = CancellableScope::new();
            token = scope.token();
            scope.complete(); // Mark as complete
        }
        // Token should NOT be cancelled since we called complete()
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_default_token() {
        let token = CancellationToken::default();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_token_clone() {
        let source = CancellationSource::new();
        let token1 = source.token();
        let token2 = token1.clone();

        assert!(!token1.is_cancelled());
        assert!(!token2.is_cancelled());

        source.cancel();

        assert!(token1.is_cancelled());
        assert!(token2.is_cancelled());
    }

    #[test]
    fn test_cancellation_error_display() {
        let err = CancellationError { reason: None };
        assert_eq!(err.to_string(), "operation cancelled");

        let err = CancellationError {
            reason: Some("timeout".into()),
        };
        assert_eq!(err.to_string(), "operation cancelled: timeout");
    }
}
