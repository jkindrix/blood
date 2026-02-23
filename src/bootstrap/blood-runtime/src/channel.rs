//! # Channel Implementation
//!
//! Multi-producer multi-consumer (MPMC) channels for fiber communication.
//!
//! ## Design
//!
//! Based on crossbeam-channel for the core implementation:
//! - Bounded channels with backpressure
//! - Unbounded channels for async messaging
//! - Selection across multiple channels
//!
//! ## Technical References
//!
//! - [crossbeam-channel](https://docs.rs/crossbeam-channel)
//! - [Go Channels Design](https://golang.org/ref/mem)

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{
    self as cc,
    RecvTimeoutError as CcRecvTimeoutError,
    SendTimeoutError as CcSendTimeoutError,
    TryRecvError as CcTryRecvError,
    TrySendError as CcTrySendError,
};

use crate::cancellation::CancellationToken;

/// Unique channel identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId(pub u64);

impl ChannelId {
    /// Get the raw ID value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Channel({})", self.0)
    }
}

/// Global channel ID counter.
static NEXT_CHANNEL_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a new unique channel ID.
fn next_channel_id() -> ChannelId {
    ChannelId(NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed))
}

/// Error returned when sending fails because the channel is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendError<T>(pub T);

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sending on a closed channel")
    }
}

impl<T: fmt::Debug> std::error::Error for SendError<T> {}

/// Error returned when receiving fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "receiving from an empty and closed channel")
    }
}

impl std::error::Error for RecvError {}

/// Error returned by try_send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrySendError<T> {
    /// The channel is full.
    Full(T),
    /// The channel is closed.
    Disconnected(T),
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrySendError::Full(_) => write!(f, "sending on a full channel"),
            TrySendError::Disconnected(_) => write!(f, "sending on a closed channel"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for TrySendError<T> {}

/// Error returned by try_recv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    /// The channel is empty.
    Empty,
    /// The channel is closed.
    Disconnected,
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryRecvError::Empty => write!(f, "receiving from an empty channel"),
            TryRecvError::Disconnected => write!(f, "receiving from a closed channel"),
        }
    }
}

impl std::error::Error for TryRecvError {}

/// Error returned by send_timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendTimeoutError<T> {
    /// Timed out waiting for space.
    Timeout(T),
    /// The channel is closed.
    Disconnected(T),
}

impl<T> fmt::Display for SendTimeoutError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendTimeoutError::Timeout(_) => write!(f, "timed out waiting on send"),
            SendTimeoutError::Disconnected(_) => write!(f, "sending on a closed channel"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for SendTimeoutError<T> {}

/// Error returned by recv_timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvTimeoutError {
    /// Timed out waiting for a message.
    Timeout,
    /// The channel is closed.
    Disconnected,
}

impl fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvTimeoutError::Timeout => write!(f, "timed out waiting on recv"),
            RecvTimeoutError::Disconnected => write!(f, "receiving from a closed channel"),
        }
    }
}

impl std::error::Error for RecvTimeoutError {}

/// Error returned when send is cancelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendCancelledError<T> {
    /// The operation was cancelled.
    Cancelled(T),
    /// The channel is closed.
    Disconnected(T),
}

impl<T> fmt::Display for SendCancelledError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendCancelledError::Cancelled(_) => write!(f, "send operation cancelled"),
            SendCancelledError::Disconnected(_) => write!(f, "sending on a closed channel"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for SendCancelledError<T> {}

impl<T> SendCancelledError<T> {
    /// Extract the value from the error.
    pub fn into_inner(self) -> T {
        match self {
            SendCancelledError::Cancelled(v) => v,
            SendCancelledError::Disconnected(v) => v,
        }
    }

    /// Check if the error is due to cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, SendCancelledError::Cancelled(_))
    }
}

/// Error returned when recv is cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvCancelledError {
    /// The operation was cancelled.
    Cancelled,
    /// The channel is closed.
    Disconnected,
}

impl fmt::Display for RecvCancelledError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvCancelledError::Cancelled => write!(f, "recv operation cancelled"),
            RecvCancelledError::Disconnected => write!(f, "receiving from a closed channel"),
        }
    }
}

impl std::error::Error for RecvCancelledError {}

impl RecvCancelledError {
    /// Check if the error is due to cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, RecvCancelledError::Cancelled)
    }
}

/// The sending half of a channel.
#[derive(Debug)]
pub struct Sender<T> {
    /// Channel ID.
    id: ChannelId,
    /// Inner crossbeam sender.
    inner: cc::Sender<T>,
}

impl<T> Sender<T> {
    /// Get the channel ID.
    pub fn id(&self) -> ChannelId {
        self.id
    }

    /// Send a message, blocking if the channel is full.
    pub fn send(&self, msg: T) -> Result<(), SendError<T>> {
        self.inner.send(msg).map_err(|e| SendError(e.0))
    }

    /// Try to send a message without blocking.
    pub fn try_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        self.inner.try_send(msg).map_err(|e| match e {
            CcTrySendError::Full(v) => TrySendError::Full(v),
            CcTrySendError::Disconnected(v) => TrySendError::Disconnected(v),
        })
    }

    /// Send a message with a timeout.
    pub fn send_timeout(&self, msg: T, timeout: Duration) -> Result<(), SendTimeoutError<T>> {
        self.inner.send_timeout(msg, timeout).map_err(|e| match e {
            CcSendTimeoutError::Timeout(v) => SendTimeoutError::Timeout(v),
            CcSendTimeoutError::Disconnected(v) => SendTimeoutError::Disconnected(v),
        })
    }

    /// Check if the channel is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Check if the channel is full.
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    /// Get the number of messages in the channel.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Get the channel capacity, or None for unbounded.
    pub fn capacity(&self) -> Option<usize> {
        self.inner.capacity()
    }

    /// Send a message with cancellation support.
    ///
    /// Polls the cancellation token while waiting for space in the channel.
    /// Returns `Err(SendCancelledError::Cancelled)` if cancelled before sending.
    pub fn send_cancellable(
        &self,
        msg: T,
        token: &CancellationToken,
    ) -> Result<(), SendCancelledError<T>> {
        // Fast path: check if channel has space (for bounded) or is open
        match self.inner.try_send(msg) {
            Ok(()) => return Ok(()),
            Err(CcTrySendError::Disconnected(v)) => {
                return Err(SendCancelledError::Disconnected(v));
            }
            Err(CcTrySendError::Full(v)) => {
                // Need to wait for space, with cancellation checking
                let mut value = v;
                loop {
                    // Check for cancellation
                    if token.is_cancelled() {
                        return Err(SendCancelledError::Cancelled(value));
                    }

                    // Try to send with short timeout
                    match self.inner.send_timeout(value, Duration::from_millis(10)) {
                        Ok(()) => return Ok(()),
                        Err(CcSendTimeoutError::Disconnected(v)) => {
                            return Err(SendCancelledError::Disconnected(v));
                        }
                        Err(CcSendTimeoutError::Timeout(v)) => {
                            value = v;
                            // Continue loop, will check cancellation again
                        }
                    }
                }
            }
        }
    }

    /// Send a message with both cancellation and timeout support.
    ///
    /// Returns error if cancelled, timeout expires, or channel disconnected.
    pub fn send_cancellable_timeout(
        &self,
        msg: T,
        timeout: Duration,
        token: &CancellationToken,
    ) -> Result<(), SendCancelledError<T>> {
        let deadline = std::time::Instant::now() + timeout;
        let mut value = msg;

        loop {
            // Check for cancellation
            if token.is_cancelled() {
                return Err(SendCancelledError::Cancelled(value));
            }

            // Check for timeout
            let now = std::time::Instant::now();
            if now >= deadline {
                return Err(SendCancelledError::Cancelled(value));
            }

            // Calculate remaining time with minimum poll interval
            let remaining = deadline.saturating_duration_since(now);
            let poll_timeout = remaining.min(Duration::from_millis(10));

            match self.inner.send_timeout(value, poll_timeout) {
                Ok(()) => return Ok(()),
                Err(CcSendTimeoutError::Disconnected(v)) => {
                    return Err(SendCancelledError::Disconnected(v));
                }
                Err(CcSendTimeoutError::Timeout(v)) => {
                    value = v;
                    // Continue loop
                }
            }
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            inner: self.inner.clone(),
        }
    }
}

/// The receiving half of a channel.
#[derive(Debug)]
pub struct Receiver<T> {
    /// Channel ID.
    id: ChannelId,
    /// Inner crossbeam receiver.
    inner: cc::Receiver<T>,
}

impl<T> Receiver<T> {
    /// Get the channel ID.
    pub fn id(&self) -> ChannelId {
        self.id
    }

    /// Receive a message, blocking if the channel is empty.
    pub fn recv(&self) -> Result<T, RecvError> {
        self.inner.recv().map_err(|_| RecvError)
    }

    /// Try to receive a message without blocking.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.inner.try_recv().map_err(|e| match e {
            CcTryRecvError::Empty => TryRecvError::Empty,
            CcTryRecvError::Disconnected => TryRecvError::Disconnected,
        })
    }

    /// Receive a message with a timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        self.inner.recv_timeout(timeout).map_err(|e| match e {
            CcRecvTimeoutError::Timeout => RecvTimeoutError::Timeout,
            CcRecvTimeoutError::Disconnected => RecvTimeoutError::Disconnected,
        })
    }

    /// Check if the channel is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Check if the channel is full.
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    /// Get the number of messages in the channel.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Get the channel capacity, or None for unbounded.
    pub fn capacity(&self) -> Option<usize> {
        self.inner.capacity()
    }

    /// Create an iterator over received messages.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.inner.iter()
    }

    /// Create a try-iterator over received messages.
    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        self.inner.try_iter()
    }

    /// Receive a message with cancellation support.
    ///
    /// Polls the cancellation token while waiting for a message.
    /// Returns `Err(RecvCancelledError::Cancelled)` if cancelled before receiving.
    pub fn recv_cancellable(&self, token: &CancellationToken) -> Result<T, RecvCancelledError> {
        // Fast path: check if message available
        match self.inner.try_recv() {
            Ok(v) => return Ok(v),
            Err(CcTryRecvError::Disconnected) => {
                return Err(RecvCancelledError::Disconnected);
            }
            Err(CcTryRecvError::Empty) => {
                // Need to wait for message, with cancellation checking
                loop {
                    // Check for cancellation
                    if token.is_cancelled() {
                        return Err(RecvCancelledError::Cancelled);
                    }

                    // Try to receive with short timeout
                    match self.inner.recv_timeout(Duration::from_millis(10)) {
                        Ok(v) => return Ok(v),
                        Err(CcRecvTimeoutError::Disconnected) => {
                            return Err(RecvCancelledError::Disconnected);
                        }
                        Err(CcRecvTimeoutError::Timeout) => {
                            // Continue loop, will check cancellation again
                        }
                    }
                }
            }
        }
    }

    /// Receive a message with both cancellation and timeout support.
    ///
    /// Returns error if cancelled, timeout expires, or channel disconnected.
    pub fn recv_cancellable_timeout(
        &self,
        timeout: Duration,
        token: &CancellationToken,
    ) -> Result<T, RecvCancelledError> {
        let deadline = std::time::Instant::now() + timeout;

        loop {
            // Check for cancellation
            if token.is_cancelled() {
                return Err(RecvCancelledError::Cancelled);
            }

            // Check for timeout
            let now = std::time::Instant::now();
            if now >= deadline {
                return Err(RecvCancelledError::Cancelled);
            }

            // Calculate remaining time with minimum poll interval
            let remaining = deadline.saturating_duration_since(now);
            let poll_timeout = remaining.min(Duration::from_millis(10));

            match self.inner.recv_timeout(poll_timeout) {
                Ok(v) => return Ok(v),
                Err(CcRecvTimeoutError::Disconnected) => {
                    return Err(RecvCancelledError::Disconnected);
                }
                Err(CcRecvTimeoutError::Timeout) => {
                    // Continue loop
                }
            }
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            inner: self.inner.clone(),
        }
    }
}

impl<T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = cc::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

/// Create a bounded channel with the given capacity.
pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let id = next_channel_id();
    let (tx, rx) = cc::bounded(capacity);
    (
        Sender { id, inner: tx },
        Receiver { id, inner: rx },
    )
}

/// Create an unbounded channel.
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let id = next_channel_id();
    let (tx, rx) = cc::unbounded();
    (
        Sender { id, inner: tx },
        Receiver { id, inner: rx },
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_bounded_channel() {
        let (tx, rx) = bounded::<i32>(10);
        assert_eq!(tx.capacity(), Some(10));
        assert_eq!(rx.capacity(), Some(10));
    }

    #[test]
    fn test_unbounded_channel() {
        let (tx, rx) = unbounded::<i32>();
        assert_eq!(tx.capacity(), None);
        assert_eq!(rx.capacity(), None);
    }

    #[test]
    fn test_send_recv() {
        let (tx, rx) = bounded(10);
        tx.send(42).unwrap();
        assert_eq!(rx.recv().unwrap(), 42);
    }

    #[test]
    fn test_try_send_recv() {
        let (tx, rx) = bounded(1);

        assert!(tx.try_send(1).is_ok());
        assert!(matches!(tx.try_send(2), Err(TrySendError::Full(_))));

        assert_eq!(rx.try_recv().unwrap(), 1);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn test_channel_id() {
        let (tx1, rx1) = bounded::<i32>(1);
        let (tx2, rx2) = bounded::<i32>(1);

        assert_eq!(tx1.id(), rx1.id());
        assert_eq!(tx2.id(), rx2.id());
        assert_ne!(tx1.id(), tx2.id());
    }

    #[test]
    fn test_multi_producer() {
        let (tx, rx) = bounded(100);

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let tx = tx.clone();
                thread::spawn(move || {
                    for j in 0..10 {
                        tx.send(i * 10 + j).unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        drop(tx);

        let mut received: Vec<i32> = rx.into_iter().collect();
        received.sort();

        assert_eq!(received.len(), 100);
        assert_eq!(received, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn test_multi_consumer() {
        let (tx, rx) = bounded(100);

        // Send messages
        for i in 0..100 {
            tx.send(i).unwrap();
        }
        drop(tx);

        // Multiple consumers
        let results: Vec<_> = (0..4)
            .map(|_| {
                let rx = rx.clone();
                thread::spawn(move || {
                    let mut received = Vec::new();
                    while let Ok(v) = rx.recv() {
                        received.push(v);
                    }
                    received
                })
            })
            .collect();

        let mut all_received = Vec::new();
        for handle in results {
            all_received.extend(handle.join().unwrap());
        }

        all_received.sort();
        assert_eq!(all_received.len(), 100);
        assert_eq!(all_received, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn test_timeout() {
        let (_tx, rx) = bounded::<i32>(1);

        let start = std::time::Instant::now();
        let result = rx.recv_timeout(Duration::from_millis(50));
        let elapsed = start.elapsed();

        assert!(matches!(result, Err(RecvTimeoutError::Timeout)));
        assert!(elapsed >= Duration::from_millis(45));
    }

    #[test]
    fn test_disconnect() {
        let (tx, rx) = bounded::<i32>(1);
        drop(tx);

        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
        assert!(matches!(rx.recv(), Err(RecvError)));
    }

    #[test]
    fn test_len_and_empty() {
        let (tx, rx) = bounded(10);

        assert!(tx.is_empty());
        assert_eq!(tx.len(), 0);

        tx.send(1).unwrap();
        tx.send(2).unwrap();

        assert!(!tx.is_empty());
        assert_eq!(tx.len(), 2);
        assert_eq!(rx.len(), 2);

        rx.recv().unwrap();
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn test_clone() {
        let (tx, rx) = bounded(10);

        let tx2 = tx.clone();
        let rx2 = rx.clone();

        tx.send(1).unwrap();
        tx2.send(2).unwrap();

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx2.recv().unwrap(), 2);
    }
}
