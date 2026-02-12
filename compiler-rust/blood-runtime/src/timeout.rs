//! Timeout Enforcement
//!
//! This module provides timeout enforcement for operations, integrating with
//! the cancellation mechanism to actually cancel operations when timeouts expire.
//!
//! # Design
//!
//! Timeouts in Blood are enforced through the cancellation system:
//! - Each timeout creates a cancellation source with a timer
//! - When the timer expires, the cancellation is triggered
//! - Operations must check the cancellation token to respond to timeouts
//!
//! # Components
//!
//! - `Timeout`: Wraps a duration with timeout semantics
//! - `TimeoutGuard`: RAII guard that enforces a timeout within a scope
//! - `with_timeout`: Run an operation with a timeout
//! - `TimeoutError`: Error returned when a timeout expires
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::timeout::{with_timeout, TimeoutError};
//! use std::time::Duration;
//!
//! // Run an operation with a timeout
//! let result = with_timeout(Duration::from_secs(5), || {
//!     // Long-running operation...
//!     do_work()
//! });
//!
//! match result {
//!     Ok(value) => println!("Operation completed: {:?}", value),
//!     Err(TimeoutError::Expired) => println!("Operation timed out"),
//!     Err(TimeoutError::Cancelled(e)) => println!("Operation cancelled: {}", e),
//! }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::cancellation::{CancellationError, CancellationSource, CancellationToken};

/// Error type for timeout operations.
#[derive(Debug, Clone)]
pub enum TimeoutError {
    /// The timeout expired before the operation completed.
    Expired {
        /// The timeout duration that was exceeded.
        duration: Duration,
        /// When the timeout was started.
        started_at: Instant,
    },
    /// The operation was cancelled (possibly by a parent cancellation).
    Cancelled(CancellationError),
}

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimeoutError::Expired { duration, .. } => {
                write!(f, "operation timed out after {:?}", duration)
            }
            TimeoutError::Cancelled(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for TimeoutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TimeoutError::Expired { .. } => None,
            TimeoutError::Cancelled(e) => Some(e),
        }
    }
}

impl From<CancellationError> for TimeoutError {
    fn from(err: CancellationError) -> Self {
        TimeoutError::Cancelled(err)
    }
}

/// A timeout duration with metadata.
#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    /// The timeout duration.
    duration: Duration,
    /// Whether the timeout should propagate to child operations.
    propagate: bool,
}

impl Timeout {
    /// Create a new timeout with the specified duration.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            propagate: true,
        }
    }

    /// Create a timeout from seconds.
    pub fn from_secs(secs: u64) -> Self {
        Self::new(Duration::from_secs(secs))
    }

    /// Create a timeout from milliseconds.
    pub fn from_millis(millis: u64) -> Self {
        Self::new(Duration::from_millis(millis))
    }

    /// Create a timeout from microseconds.
    pub fn from_micros(micros: u64) -> Self {
        Self::new(Duration::from_micros(micros))
    }

    /// Set whether the timeout should propagate to child operations.
    pub fn propagate(mut self, propagate: bool) -> Self {
        self.propagate = propagate;
        self
    }

    /// Get the timeout duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Check if this timeout should propagate to children.
    pub fn should_propagate(&self) -> bool {
        self.propagate
    }
}

impl From<Duration> for Timeout {
    fn from(duration: Duration) -> Self {
        Self::new(duration)
    }
}

/// RAII guard that enforces a timeout within a scope.
///
/// When the guard is created, a timer is started. If the guard is not
/// dropped before the timeout expires, the associated cancellation token
/// will be triggered.
///
/// # Example
///
/// ```rust,ignore
/// use blood_runtime::timeout::TimeoutGuard;
/// use std::time::Duration;
///
/// let guard = TimeoutGuard::new(Duration::from_secs(5));
/// let token = guard.token();
///
/// // Do work, checking the token periodically
/// while !token.is_cancelled() {
///     // Work...
/// }
///
/// // Mark as complete to prevent cancellation
/// guard.complete();
/// ```
#[derive(Debug)]
pub struct TimeoutGuard {
    /// The cancellation source for this timeout.
    source: CancellationSource,
    /// When the timeout was started.
    started_at: Instant,
    /// The timeout duration.
    duration: Duration,
    /// Whether the operation completed successfully.
    completed: AtomicBool,
    /// Handle to the timer thread — stored to prevent thread detach on drop.
    _timer_handle: Option<std::thread::JoinHandle<()>>,
}

impl TimeoutGuard {
    /// Create a new timeout guard with the specified duration.
    pub fn new(duration: Duration) -> Self {
        Self::with_parent(duration, None)
    }

    /// Create a new timeout guard with a parent cancellation token.
    ///
    /// The timeout will be triggered if either:
    /// - The specified duration expires
    /// - The parent token is cancelled
    pub fn with_parent(duration: Duration, parent: Option<CancellationToken>) -> Self {
        let source = match parent {
            Some(parent_token) => CancellationSource::with_parent(parent_token),
            None => CancellationSource::new(),
        };

        let started_at = Instant::now();

        // Start the timeout timer
        let state = Arc::new((AtomicBool::new(false), std::sync::Mutex::new(None::<Waker>)));
        let timer_state = state.clone();
        let timer_source_token = source.token();

        let timer_handle = std::thread::spawn(move || {
            std::thread::sleep(duration);

            // Check if already completed or cancelled
            if !timer_state.0.load(Ordering::SeqCst) && !timer_source_token.is_cancelled() {
                // Timeout expired - this will be handled by the source's cancel_after
            }
        });

        // Schedule cancellation after the duration
        source.cancel_after(duration);

        Self {
            source,
            started_at,
            duration,
            completed: AtomicBool::new(false),
            _timer_handle: Some(timer_handle),
        }
    }

    /// Get the cancellation token for this timeout.
    pub fn token(&self) -> CancellationToken {
        self.source.token()
    }

    /// Check if the timeout has expired.
    pub fn is_expired(&self) -> bool {
        self.source.is_cancelled() && !self.completed.load(Ordering::SeqCst)
    }

    /// Check if the operation is still within the timeout.
    pub fn is_active(&self) -> bool {
        !self.source.is_cancelled()
    }

    /// Get the time remaining before timeout.
    ///
    /// Returns `None` if the timeout has already expired.
    pub fn remaining(&self) -> Option<Duration> {
        let elapsed = self.started_at.elapsed();
        if elapsed >= self.duration {
            None
        } else {
            Some(self.duration - elapsed)
        }
    }

    /// Get the elapsed time since the timeout started.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Mark the operation as complete.
    ///
    /// This prevents the timeout from being reported as expired
    /// if it fires after the operation finishes.
    pub fn complete(self) {
        self.completed.store(true, Ordering::SeqCst);
        // Don't cancel on drop since we completed successfully
    }

    /// Cancel the operation manually.
    ///
    /// This is useful for early termination.
    pub fn cancel(&self) {
        self.source.cancel_with_reason(Some("manually cancelled".into()));
    }

    /// Check the timeout and return an error if expired.
    pub fn check(&self) -> Result<(), TimeoutError> {
        if self.source.is_cancelled() {
            Err(TimeoutError::Expired {
                duration: self.duration,
                started_at: self.started_at,
            })
        } else {
            Ok(())
        }
    }
}

impl Drop for TimeoutGuard {
    fn drop(&mut self) {
        // Mark as completed to prevent false timeout reports
        self.completed.store(true, Ordering::SeqCst);
    }
}

/// Run an operation with a timeout.
///
/// This function executes the provided closure and enforces a timeout.
/// If the operation takes longer than the specified duration, the
/// cancellation token will be triggered.
///
/// # Note
///
/// The operation must check the cancellation token to respond to
/// the timeout. This function does not forcibly terminate the operation.
///
/// # Example
///
/// ```rust,ignore
/// use blood_runtime::timeout::with_timeout;
/// use std::time::Duration;
///
/// let result = with_timeout(Duration::from_secs(5), || {
///     // This closure should check for cancellation
///     expensive_operation()
/// });
/// ```
pub fn with_timeout<F, T>(duration: Duration, f: F) -> Result<T, TimeoutError>
where
    F: FnOnce(CancellationToken) -> Result<T, CancellationError>,
{
    let guard = TimeoutGuard::new(duration);
    let token = guard.token();

    match f(token.clone()) {
        Ok(value) => {
            guard.complete();
            Ok(value)
        }
        Err(e) => {
            if guard.is_expired() || token.is_cancelled() {
                Err(TimeoutError::Expired {
                    duration,
                    started_at: guard.started_at,
                })
            } else {
                Err(TimeoutError::Cancelled(e))
            }
        }
    }
}

/// Run an operation with a timeout, using a parent cancellation token.
///
/// The operation will be cancelled if either:
/// - The timeout expires
/// - The parent token is cancelled
pub fn with_timeout_and_parent<F, T>(
    duration: Duration,
    parent: CancellationToken,
    f: F,
) -> Result<T, TimeoutError>
where
    F: FnOnce(CancellationToken) -> Result<T, CancellationError>,
{
    let guard = TimeoutGuard::with_parent(duration, Some(parent));
    let token = guard.token();
    let started_at = guard.started_at;

    match f(token.clone()) {
        Ok(value) => {
            guard.complete();
            Ok(value)
        }
        Err(e) => {
            if guard.is_expired() {
                Err(TimeoutError::Expired {
                    duration,
                    started_at,
                })
            } else {
                Err(TimeoutError::Cancelled(e))
            }
        }
    }
}

/// A deadline represents an absolute point in time by which an operation
/// must complete.
///
/// Unlike `Timeout` which is a duration, `Deadline` is an absolute time point.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    /// The instant by which the operation must complete.
    deadline: Instant,
}

impl Deadline {
    /// Create a deadline at a specific instant.
    pub fn at(instant: Instant) -> Self {
        Self { deadline: instant }
    }

    /// Create a deadline that expires after a duration from now.
    pub fn after(duration: Duration) -> Self {
        Self {
            deadline: Instant::now() + duration,
        }
    }

    /// Get the underlying instant.
    pub fn instant(&self) -> Instant {
        self.deadline
    }

    /// Check if the deadline has passed.
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Get the time remaining until the deadline.
    ///
    /// Returns `None` if the deadline has already passed.
    pub fn remaining(&self) -> Option<Duration> {
        let now = Instant::now();
        if now >= self.deadline {
            None
        } else {
            Some(self.deadline - now)
        }
    }

    /// Convert to a duration from now.
    ///
    /// Returns `Duration::ZERO` if the deadline has already passed.
    pub fn to_duration(&self) -> Duration {
        self.remaining().unwrap_or(Duration::ZERO)
    }
}

impl From<Instant> for Deadline {
    fn from(instant: Instant) -> Self {
        Self::at(instant)
    }
}

impl From<Duration> for Deadline {
    fn from(duration: Duration) -> Self {
        Self::after(duration)
    }
}

/// A future that completes when a timeout expires.
///
/// This can be used with async code to implement timeout semantics.
pub struct TimeoutFuture {
    /// When the timeout was started.
    started_at: Instant,
    /// The timeout duration.
    duration: Duration,
    /// Whether the timeout has been polled at least once.
    polled: bool,
}

impl TimeoutFuture {
    /// Create a new timeout future.
    pub fn new(duration: Duration) -> Self {
        Self {
            started_at: Instant::now(),
            duration,
            polled: false,
        }
    }

    /// Create a timeout future from a deadline.
    pub fn until(deadline: Deadline) -> Self {
        Self {
            started_at: Instant::now(),
            duration: deadline.to_duration(),
            polled: false,
        }
    }
}

impl Future for TimeoutFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.started_at.elapsed() >= self.duration {
            Poll::Ready(())
        } else {
            // Schedule a wake-up when the timeout expires
            if !self.polled {
                self.polled = true;
                let remaining = self.duration - self.started_at.elapsed();
                let waker = cx.waker().clone();

                std::thread::spawn(move || {
                    std::thread::sleep(remaining);
                    waker.wake();
                });
            }
            Poll::Pending
        }
    }
}

/// Extension trait for adding timeout functionality to operations.
pub trait TimeoutExt {
    /// The output type of the operation.
    type Output;

    /// Run this operation with a timeout.
    fn with_timeout(self, duration: Duration) -> Result<Self::Output, TimeoutError>;
}

/// Builder for complex timeout configurations.
#[derive(Debug)]
pub struct TimeoutBuilder {
    /// The timeout duration.
    duration: Duration,
    /// Parent cancellation token.
    parent: Option<CancellationToken>,
    /// Custom reason for timeout.
    reason: Option<String>,
    /// Whether to propagate timeout to children.
    propagate: bool,
}

impl TimeoutBuilder {
    /// Create a new timeout builder with the specified duration.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            parent: None,
            reason: None,
            propagate: true,
        }
    }

    /// Set a parent cancellation token.
    pub fn parent(mut self, parent: CancellationToken) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Set a custom timeout reason.
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Set whether the timeout should propagate to children.
    pub fn propagate(mut self, propagate: bool) -> Self {
        self.propagate = propagate;
        self
    }

    /// Build a timeout guard with the configured settings.
    pub fn build(self) -> TimeoutGuard {
        TimeoutGuard::with_parent(self.duration, self.parent)
    }

    /// Run an operation with the configured timeout.
    pub fn run<F, T>(self, f: F) -> Result<T, TimeoutError>
    where
        F: FnOnce(CancellationToken) -> Result<T, CancellationError>,
    {
        match self.parent {
            Some(parent) => with_timeout_and_parent(self.duration, parent, f),
            None => with_timeout(self.duration, f),
        }
    }
}

/// Default timeout for operations that don't specify one.
///
/// This is read from the runtime configuration or defaults to 30 seconds.
pub fn default_timeout() -> Duration {
    crate::runtime_config()
        .map(|c| c.timeout.default_operation_timeout)
        .unwrap_or(Duration::from_secs(30))
}

/// Default timeout for network operations.
pub fn network_timeout() -> Duration {
    crate::runtime_config()
        .map(|c| c.timeout.network_timeout)
        .unwrap_or(Duration::from_secs(60))
}

/// Default timeout for I/O operations.
pub fn io_timeout() -> Duration {
    crate::runtime_config()
        .map(|c| c.timeout.io_timeout)
        .unwrap_or(Duration::from_secs(30))
}

/// Default timeout for compute-bound operations.
pub fn compute_timeout() -> Duration {
    crate::runtime_config()
        .map(|c| c.timeout.compute_timeout)
        .unwrap_or(Duration::from_secs(300))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_timeout_guard_basic() {
        let guard = TimeoutGuard::new(Duration::from_secs(10));
        assert!(guard.is_active());
        assert!(!guard.is_expired());
        assert!(guard.remaining().is_some());
    }

    #[test]
    fn test_timeout_guard_expires() {
        let guard = TimeoutGuard::new(Duration::from_millis(10));
        thread::sleep(Duration::from_millis(50));
        assert!(guard.is_expired());
        assert!(guard.remaining().is_none());
    }

    #[test]
    fn test_timeout_guard_complete() {
        let guard = TimeoutGuard::new(Duration::from_millis(10));
        guard.complete();
        // After completing, the guard should be dropped without issues
    }

    #[test]
    fn test_with_timeout_success() {
        let result = with_timeout(Duration::from_secs(1), |_token| {
            // Fast operation
            Ok(42)
        });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_with_timeout_expires() {
        let result = with_timeout(Duration::from_millis(10), |token| {
            // Slow operation that doesn't check cancellation
            thread::sleep(Duration::from_millis(100));
            token.check()?;
            Ok(42)
        });
        assert!(result.is_err());
        match result {
            Err(TimeoutError::Expired { .. }) => {}
            _ => panic!("expected TimeoutError::Expired"),
        }
    }

    #[test]
    fn test_deadline_basic() {
        let deadline = Deadline::after(Duration::from_secs(10));
        assert!(!deadline.is_expired());
        assert!(deadline.remaining().is_some());
    }

    #[test]
    fn test_deadline_expired() {
        let deadline = Deadline::after(Duration::from_millis(1));
        thread::sleep(Duration::from_millis(10));
        assert!(deadline.is_expired());
        assert!(deadline.remaining().is_none());
    }

    #[test]
    fn test_timeout_error_display() {
        let err = TimeoutError::Expired {
            duration: Duration::from_secs(5),
            started_at: Instant::now(),
        };
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn test_timeout_builder() {
        let result = TimeoutBuilder::new(Duration::from_secs(1))
            .reason("test operation")
            .run(|_token| Ok(42));
        assert!(result.is_ok());
    }

    #[test]
    fn test_timeout_from_duration() {
        let timeout = Timeout::from_secs(5);
        assert_eq!(timeout.duration(), Duration::from_secs(5));
    }

    #[test]
    fn test_timeout_propagation() {
        let timeout = Timeout::new(Duration::from_secs(5)).propagate(false);
        assert!(!timeout.should_propagate());
    }

    #[test]
    fn test_with_timeout_and_parent_cancelled() {
        let parent_source = crate::cancellation::CancellationSource::new();
        let parent_token = parent_source.token();

        // Cancel the parent immediately
        parent_source.cancel();

        let result = with_timeout_and_parent(Duration::from_secs(10), parent_token, |token| {
            token.check()?;
            Ok(42)
        });

        // Should fail due to parent cancellation
        assert!(result.is_err());
    }

    #[test]
    fn test_timeout_guard_with_parent() {
        let parent_source = crate::cancellation::CancellationSource::new();
        let parent_token = parent_source.token();

        let guard = TimeoutGuard::with_parent(Duration::from_secs(10), Some(parent_token.clone()));
        assert!(guard.is_active());

        parent_source.cancel();

        // After parent cancellation, guard should reflect cancellation
        assert!(guard.token().is_cancelled());
    }
}
