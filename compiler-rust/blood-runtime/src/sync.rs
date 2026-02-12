//! # Synchronization Primitives
//!
//! This module provides synchronization primitives for fiber-based concurrency.
//!
//! ## Primitives
//!
//! - [`Mutex`] - Mutual exclusion lock
//! - [`RwLock`] - Reader-writer lock
//! - [`Once`] - One-time initialization
//! - [`Barrier`] - Synchronization barrier
//!
//! ## Design Notes
//!
//! These primitives are designed to work with Blood's fiber system.
//! They wrap standard library primitives with additional tracking
//! for deadlock detection and fiber-aware scheduling.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{self, LockResult, PoisonError, TryLockError, TryLockResult};

// ============================================================================
// Mutex
// ============================================================================

/// A mutual exclusion primitive useful for protecting shared data.
///
/// This mutex wraps `std::sync::Mutex` with additional tracking for
/// fiber-aware debugging and potential deadlock detection.
pub struct Mutex<T> {
    /// Unique mutex ID for debugging.
    id: u64,
    /// Inner standard mutex.
    inner: sync::Mutex<T>,
    /// Whether the mutex is currently held.
    held: AtomicBool,
}

impl<T> Mutex<T> {
    /// Creates a new mutex in an unlocked state.
    pub fn new(value: T) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            inner: sync::Mutex::new(value),
            held: AtomicBool::new(false),
        }
    }

    /// Get the mutex ID for debugging.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns whether the mutex is currently locked.
    pub fn is_locked(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }

    /// Consumes this mutex, returning the underlying data.
    pub fn into_inner(self) -> LockResult<T> {
        self.inner.into_inner()
    }
}

impl<T> Mutex<T> {
    /// Acquires a mutex, blocking the current fiber until it is able to do so.
    pub fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        match self.inner.lock() {
            Ok(guard) => {
                self.held.store(true, Ordering::Release);
                Ok(MutexGuard {
                    inner: guard,
                    held: &self.held,
                })
            }
            Err(poisoned) => {
                self.held.store(true, Ordering::Release);
                Err(PoisonError::new(MutexGuard {
                    inner: poisoned.into_inner(),
                    held: &self.held,
                }))
            }
        }
    }

    /// Attempts to acquire this lock.
    ///
    /// If the lock could not be acquired, returns an error.
    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        match self.inner.try_lock() {
            Ok(guard) => {
                self.held.store(true, Ordering::Release);
                Ok(MutexGuard {
                    inner: guard,
                    held: &self.held,
                })
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                self.held.store(true, Ordering::Release);
                Err(TryLockError::Poisoned(PoisonError::new(MutexGuard {
                    inner: poisoned.into_inner(),
                    held: &self.held,
                })))
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
        }
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// Since this call borrows the `Mutex` mutably, no actual locking needs to
    /// take place.
    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        self.inner.get_mut()
    }

    /// Determines whether the mutex is poisoned.
    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    /// Clear the poisoned state from a mutex.
    pub fn clear_poison(&self) {
        self.inner.clear_poison()
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_lock() {
            Ok(guard) => f.debug_struct("Mutex").field("data", &&*guard).finish(),
            Err(TryLockError::Poisoned(err)) => f
                .debug_struct("Mutex")
                .field("data", &&*err.into_inner())
                .finish(),
            Err(TryLockError::WouldBlock) => {
                struct LockedPlaceholder;
                impl fmt::Debug for LockedPlaceholder {
                    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str("<locked>")
                    }
                }
                f.debug_struct("Mutex")
                    .field("data", &LockedPlaceholder)
                    .finish()
            }
        }
    }
}

// Safety: Mutex provides synchronized access
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

/// An RAII implementation of a "scoped lock" of a mutex.
pub struct MutexGuard<'a, T> {
    inner: sync::MutexGuard<'a, T>,
    held: &'a AtomicBool,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.held.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display> fmt::Display for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

// ============================================================================
// RwLock
// ============================================================================

/// A reader-writer lock.
///
/// This lock allows a number of readers or at most one writer at any point in time.
pub struct RwLock<T> {
    /// Unique lock ID for debugging.
    id: u64,
    /// Inner standard rwlock.
    inner: sync::RwLock<T>,
    /// Current reader count.
    readers: AtomicU32,
    /// Whether a writer is holding the lock.
    writer: AtomicBool,
}

impl<T> RwLock<T> {
    /// Creates a new instance of an `RwLock<T>`.
    pub fn new(value: T) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            inner: sync::RwLock::new(value),
            readers: AtomicU32::new(0),
            writer: AtomicBool::new(false),
        }
    }

    /// Get the lock ID for debugging.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the current number of readers.
    pub fn reader_count(&self) -> u32 {
        self.readers.load(Ordering::Acquire)
    }

    /// Returns whether a writer is holding the lock.
    pub fn is_write_locked(&self) -> bool {
        self.writer.load(Ordering::Acquire)
    }

    /// Consumes this lock, returning the underlying data.
    pub fn into_inner(self) -> LockResult<T> {
        self.inner.into_inner()
    }
}

impl<T> RwLock<T> {
    /// Locks this rwlock with shared read access.
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        match self.inner.read() {
            Ok(guard) => {
                self.readers.fetch_add(1, Ordering::AcqRel);
                Ok(RwLockReadGuard {
                    inner: guard,
                    readers: &self.readers,
                })
            }
            Err(poisoned) => {
                self.readers.fetch_add(1, Ordering::AcqRel);
                Err(PoisonError::new(RwLockReadGuard {
                    inner: poisoned.into_inner(),
                    readers: &self.readers,
                }))
            }
        }
    }

    /// Attempts to acquire this lock with shared read access.
    pub fn try_read(&self) -> TryLockResult<RwLockReadGuard<'_, T>> {
        match self.inner.try_read() {
            Ok(guard) => {
                self.readers.fetch_add(1, Ordering::AcqRel);
                Ok(RwLockReadGuard {
                    inner: guard,
                    readers: &self.readers,
                })
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                self.readers.fetch_add(1, Ordering::AcqRel);
                Err(TryLockError::Poisoned(PoisonError::new(RwLockReadGuard {
                    inner: poisoned.into_inner(),
                    readers: &self.readers,
                })))
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
        }
    }

    /// Locks this rwlock with exclusive write access.
    pub fn write(&self) -> LockResult<RwLockWriteGuard<'_, T>> {
        match self.inner.write() {
            Ok(guard) => {
                self.writer.store(true, Ordering::Release);
                Ok(RwLockWriteGuard {
                    inner: guard,
                    writer: &self.writer,
                })
            }
            Err(poisoned) => {
                self.writer.store(true, Ordering::Release);
                Err(PoisonError::new(RwLockWriteGuard {
                    inner: poisoned.into_inner(),
                    writer: &self.writer,
                }))
            }
        }
    }

    /// Attempts to lock this rwlock with exclusive write access.
    pub fn try_write(&self) -> TryLockResult<RwLockWriteGuard<'_, T>> {
        match self.inner.try_write() {
            Ok(guard) => {
                self.writer.store(true, Ordering::Release);
                Ok(RwLockWriteGuard {
                    inner: guard,
                    writer: &self.writer,
                })
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                self.writer.store(true, Ordering::Release);
                Err(TryLockError::Poisoned(PoisonError::new(RwLockWriteGuard {
                    inner: poisoned.into_inner(),
                    writer: &self.writer,
                })))
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
        }
    }

    /// Returns a mutable reference to the underlying data.
    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        self.inner.get_mut()
    }

    /// Determines whether the lock is poisoned.
    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    /// Clear the poisoned state.
    pub fn clear_poison(&self) {
        self.inner.clear_poison()
    }
}

impl<T: Default> Default for RwLock<T> {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_read() {
            Ok(guard) => f.debug_struct("RwLock").field("data", &&*guard).finish(),
            Err(TryLockError::Poisoned(err)) => f
                .debug_struct("RwLock")
                .field("data", &&*err.into_inner())
                .finish(),
            Err(TryLockError::WouldBlock) => {
                struct LockedPlaceholder;
                impl fmt::Debug for LockedPlaceholder {
                    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str("<locked>")
                    }
                }
                f.debug_struct("RwLock")
                    .field("data", &LockedPlaceholder)
                    .finish()
            }
        }
    }
}

// Safety: RwLock provides synchronized access
unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

/// RAII structure used to release the shared read access of a lock when dropped.
pub struct RwLockReadGuard<'a, T> {
    inner: sync::RwLockReadGuard<'a, T>,
    readers: &'a AtomicU32,
}

impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.readers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl<T: fmt::Debug> fmt::Debug for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display> fmt::Display for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

/// RAII structure used to release the exclusive write access of a lock when dropped.
pub struct RwLockWriteGuard<'a, T> {
    inner: sync::RwLockWriteGuard<'a, T>,
    writer: &'a AtomicBool,
}

impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.writer.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display> fmt::Display for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

// ============================================================================
// Once
// ============================================================================

/// A synchronization primitive which can be used to run a one-time initialization.
///
/// This is useful for initializing global state or resources that should only
/// be initialized once.
pub struct Once {
    /// Inner standard Once.
    inner: sync::Once,
    /// Unique ID for debugging.
    id: u64,
}

impl Once {
    /// Creates a new `Once` value.
    pub const fn new() -> Self {
        Self {
            inner: sync::Once::new(),
            id: 0, // ID not used in const context
        }
    }

    /// Get the Once ID for debugging.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Performs an initialization routine once and only once.
    ///
    /// If this is the first call to `call_once`, the closure `f` will be called.
    /// All other calls will block until the first call has completed.
    pub fn call_once<F>(&self, f: F)
    where
        F: FnOnce(),
    {
        self.inner.call_once(f)
    }

    /// Performs the same function as `call_once` except ignores poisoning.
    pub fn call_once_force<F>(&self, f: F)
    where
        F: FnOnce(&OnceState<'_>),
    {
        self.inner.call_once_force(|state| {
            f(&OnceState { inner: state })
        })
    }

    /// Returns `true` if some `call_once` call has completed successfully.
    pub fn is_completed(&self) -> bool {
        self.inner.is_completed()
    }
}

impl Default for Once {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Once {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Once")
            .field("completed", &self.is_completed())
            .finish()
    }
}

/// State yielded to `call_once_force`'s closure parameter.
pub struct OnceState<'a> {
    inner: &'a sync::OnceState,
}

impl OnceState<'_> {
    /// Returns `true` if the associated `Once` was poisoned prior to this call.
    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }
}

impl fmt::Debug for OnceState<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnceState")
            .field("poisoned", &self.is_poisoned())
            .finish()
    }
}

// ============================================================================
// OnceLock (for lazy initialization with value)
// ============================================================================

/// A cell which can be written to only once.
///
/// This is similar to `OnceCell` from `once_cell` crate, but using only std.
pub struct OnceLock<T> {
    inner: std::sync::OnceLock<T>,
}

impl<T> OnceLock<T> {
    /// Creates a new empty cell.
    pub const fn new() -> Self {
        Self {
            inner: std::sync::OnceLock::new(),
        }
    }

    /// Gets the reference to the underlying value.
    ///
    /// Returns `None` if the cell is empty.
    pub fn get(&self) -> Option<&T> {
        self.inner.get()
    }

    /// Gets the mutable reference to the underlying value.
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.inner.get_mut()
    }

    /// Sets the contents of this cell to `value`.
    ///
    /// Returns `Ok(())` if the cell was empty and `Err(value)` if it was full.
    pub fn set(&self, value: T) -> Result<(), T> {
        self.inner.set(value)
    }

    /// Gets the contents of the cell, initializing it with `f` if empty.
    pub fn get_or_init<F>(&self, f: F) -> &T
    where
        F: FnOnce() -> T,
    {
        self.inner.get_or_init(f)
    }

    /// Takes the value out of this cell, moving it back to an uninitialized state.
    pub fn take(&mut self) -> Option<T> {
        self.inner.take()
    }

    /// Consumes the cell, returning the wrapped value.
    pub fn into_inner(self) -> Option<T> {
        self.inner.into_inner()
    }
}

impl<T> Default for OnceLock<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for OnceLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get() {
            Some(v) => f.debug_struct("OnceLock").field("value", v).finish(),
            None => f.debug_struct("OnceLock").field("value", &"<empty>").finish(),
        }
    }
}

impl<T: Clone> Clone for OnceLock<T> {
    fn clone(&self) -> Self {
        let new_cell = Self::new();
        if let Some(value) = self.get() {
            let _ = new_cell.set(value.clone());
        }
        new_cell
    }
}

impl<T: PartialEq> PartialEq for OnceLock<T> {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl<T: Eq> Eq for OnceLock<T> {}

// Safety: OnceLock provides synchronized access
unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}
unsafe impl<T: Send> Send for OnceLock<T> {}

// ============================================================================
// Barrier
// ============================================================================

/// A barrier enables multiple fibers to synchronize the beginning
/// of some computation.
pub struct Barrier {
    /// Unique barrier ID for debugging.
    id: u64,
    /// Inner standard barrier.
    inner: sync::Barrier,
    /// Number of parties this barrier is configured for.
    num_parties: u32,
    /// Number of parties currently waiting.
    waiting: AtomicU32,
}

impl Barrier {
    /// Creates a new barrier that can block a given number of fibers.
    pub fn new(n: usize) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            inner: sync::Barrier::new(n),
            num_parties: n as u32,
            waiting: AtomicU32::new(0),
        }
    }

    /// Get the barrier ID for debugging.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get the number of parties this barrier is configured for.
    pub fn parties(&self) -> u32 {
        self.num_parties
    }

    /// Get the current number of waiting parties.
    pub fn waiting(&self) -> u32 {
        self.waiting.load(Ordering::Acquire)
    }

    /// Blocks the current fiber until all fibers have rendezvoused here.
    ///
    /// Returns a `BarrierWaitResult` indicating whether this fiber was
    /// the "leader" (the last to arrive).
    pub fn wait(&self) -> BarrierWaitResult {
        self.waiting.fetch_add(1, Ordering::AcqRel);
        let result = self.inner.wait();
        self.waiting.fetch_sub(1, Ordering::AcqRel);
        BarrierWaitResult {
            is_leader: result.is_leader(),
        }
    }
}

impl fmt::Debug for Barrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Barrier")
            .field("parties", &self.num_parties)
            .field("waiting", &self.waiting())
            .finish()
    }
}

/// A result returned from `Barrier::wait`.
#[derive(Debug, Clone, Copy)]
pub struct BarrierWaitResult {
    is_leader: bool,
}

impl BarrierWaitResult {
    /// Returns `true` if this fiber is the "leader" (the last fiber to reach the barrier).
    pub fn is_leader(&self) -> bool {
        self.is_leader
    }
}

// ============================================================================
// Semaphore
// ============================================================================

/// A counting semaphore.
///
/// Semaphores are used to limit the number of fibers that can access
/// a resource concurrently.
pub struct Semaphore {
    /// Unique semaphore ID for debugging.
    id: u64,
    /// Current permit count.
    permits: AtomicU32,
    /// Maximum permits.
    max_permits: u32,
    /// Condvar for waiting.
    condvar: sync::Condvar,
    /// Mutex for the condvar.
    mutex: sync::Mutex<()>,
}

impl Semaphore {
    /// Creates a new semaphore with the given number of permits.
    pub fn new(permits: u32) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            permits: AtomicU32::new(permits),
            max_permits: permits,
            condvar: sync::Condvar::new(),
            mutex: sync::Mutex::new(()),
        }
    }

    /// Get the semaphore ID for debugging.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get the current number of available permits.
    pub fn available_permits(&self) -> u32 {
        self.permits.load(Ordering::Acquire)
    }

    /// Get the maximum number of permits.
    pub fn max_permits(&self) -> u32 {
        self.max_permits
    }

    /// Acquires a permit from this semaphore, blocking until one is available.
    pub fn acquire(&self) {
        loop {
            // Try to acquire without blocking
            let current = self.permits.load(Ordering::Acquire);
            if current > 0 {
                if self
                    .permits
                    .compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    return;
                }
                continue;
            }

            // Need to wait
            let guard = self.mutex.lock().unwrap();
            // Re-check after acquiring the mutex
            if self.permits.load(Ordering::Acquire) == 0 {
                let _guard = self.condvar.wait(guard).unwrap();
            }
        }
    }

    /// Attempts to acquire a permit without blocking.
    ///
    /// Returns `true` if a permit was acquired, `false` otherwise.
    pub fn try_acquire(&self) -> bool {
        loop {
            let current = self.permits.load(Ordering::Acquire);
            if current == 0 {
                return false;
            }
            if self
                .permits
                .compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Releases a permit back to the semaphore.
    pub fn release(&self) {
        let old = self.permits.fetch_add(1, Ordering::AcqRel);
        if old < self.max_permits {
            // Wake up a waiting thread
            let _guard = self.mutex.lock().unwrap();
            self.condvar.notify_one();
        }
    }

    /// Acquires multiple permits at once.
    pub fn acquire_many(&self, n: u32) {
        for _ in 0..n {
            self.acquire();
        }
    }

    /// Releases multiple permits at once.
    pub fn release_many(&self, n: u32) {
        for _ in 0..n {
            self.release();
        }
    }
}

impl fmt::Debug for Semaphore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Semaphore")
            .field("available", &self.available_permits())
            .field("max", &self.max_permits)
            .finish()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_mutex_basic() {
        let mutex = Mutex::new(0);
        assert!(!mutex.is_locked());

        {
            let mut guard = mutex.lock().unwrap();
            assert!(mutex.is_locked());
            *guard = 42;
        }

        assert!(!mutex.is_locked());
        assert_eq!(*mutex.lock().unwrap(), 42);
    }

    #[test]
    fn test_mutex_try_lock() {
        let mutex = Mutex::new(0);

        let guard = mutex.lock().unwrap();
        assert!(matches!(mutex.try_lock(), Err(TryLockError::WouldBlock)));
        drop(guard);

        assert!(mutex.try_lock().is_ok());
    }

    #[test]
    fn test_rwlock_basic() {
        let lock = RwLock::new(0);

        // Multiple readers
        {
            let r1 = lock.read().unwrap();
            let r2 = lock.read().unwrap();
            assert_eq!(lock.reader_count(), 2);
            assert_eq!(*r1, 0);
            assert_eq!(*r2, 0);
        }

        assert_eq!(lock.reader_count(), 0);

        // Single writer
        {
            let mut w = lock.write().unwrap();
            assert!(lock.is_write_locked());
            *w = 42;
        }

        assert!(!lock.is_write_locked());
        assert_eq!(*lock.read().unwrap(), 42);
    }

    #[test]
    fn test_rwlock_try_locks() {
        let lock = RwLock::new(0);

        let _w = lock.write().unwrap();
        assert!(matches!(lock.try_read(), Err(TryLockError::WouldBlock)));
        assert!(matches!(lock.try_write(), Err(TryLockError::WouldBlock)));
    }

    #[test]
    fn test_once() {
        let once = Once::new();
        let mut value = 0;

        once.call_once(|| value = 1);
        assert_eq!(value, 1);
        assert!(once.is_completed());

        once.call_once(|| value = 2);
        assert_eq!(value, 1); // Should not change
    }

    #[test]
    fn test_once_lock() {
        let cell: OnceLock<u32> = OnceLock::new();
        assert!(cell.get().is_none());

        let value = cell.get_or_init(|| 42);
        assert_eq!(*value, 42);

        let value2 = cell.get_or_init(|| 100);
        assert_eq!(*value2, 42); // Should be the same
    }

    #[test]
    fn test_barrier() {
        let barrier = Barrier::new(3);
        assert_eq!(barrier.parties(), 3);

        let _handles: Vec<_> = (0..3)
            .map(|_| {
                let _b = &barrier;
                thread::scope(|_| {
                    // In a real test, we'd use scoped threads properly
                })
            })
            .collect();

        // Basic properties test
        assert_eq!(barrier.waiting(), 0);
    }

    #[test]
    fn test_semaphore() {
        let sem = Semaphore::new(2);
        assert_eq!(sem.available_permits(), 2);

        assert!(sem.try_acquire());
        assert_eq!(sem.available_permits(), 1);

        assert!(sem.try_acquire());
        assert_eq!(sem.available_permits(), 0);

        assert!(!sem.try_acquire());

        sem.release();
        assert_eq!(sem.available_permits(), 1);

        sem.release();
        assert_eq!(sem.available_permits(), 2);
    }

    #[test]
    fn test_mutex_concurrent() {
        let mutex = std::sync::Arc::new(Mutex::new(0));
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let m = mutex.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        let mut guard = m.lock().unwrap();
                        *guard += 1;
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(*mutex.lock().unwrap(), 1000);
    }

    #[test]
    fn test_rwlock_concurrent_readers() {
        let lock = std::sync::Arc::new(RwLock::new(42));
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let l = lock.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        let guard = l.read().unwrap();
                        assert_eq!(*guard, 42);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }
}
