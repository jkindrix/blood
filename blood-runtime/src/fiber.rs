//! # Fiber Implementation
//!
//! Lightweight, cooperatively-scheduled units of execution.
//!
//! ## Design
//!
//! Based on the Blood concurrency specification (CONCURRENCY.md):
//! - Stackful coroutines with growable stacks
//! - M:N scheduling (M fibers to N OS threads)
//! - Cooperative scheduling with explicit yield points
//!
//! ## Technical References
//!
//! - [Tokio Scheduler Design](https://tokio.rs/blog/2019-10-scheduler)
//! - [crossbeam-deque](https://docs.rs/crossbeam-deque) for work-stealing

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique identifier for a fiber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FiberId(pub u64);

impl FiberId {
    /// Create a new fiber ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for FiberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fiber({})", self.0)
    }
}

/// Global fiber ID counter.
static NEXT_FIBER_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a new unique fiber ID.
pub fn next_fiber_id() -> FiberId {
    FiberId(NEXT_FIBER_ID.fetch_add(1, Ordering::Relaxed))
}

/// Fiber execution state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FiberState {
    /// Ready to run.
    #[default]
    Runnable,
    /// Currently executing on a worker thread.
    Running,
    /// Waiting for an event.
    Suspended,
    /// Waiting for child fibers.
    Joining,
    /// Completed successfully.
    Completed,
    /// Failed with error.
    Failed,
    /// Cancelled.
    Cancelled,
}

/// Priority level for fibers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Priority {
    /// Low priority (background tasks).
    Low = 0,
    /// Normal priority (default).
    #[default]
    Normal = 1,
    /// High priority (latency-sensitive).
    High = 2,
    /// Critical priority (system tasks).
    Critical = 3,
}

/// Configuration for fiber creation.
#[derive(Debug, Clone)]
pub struct FiberConfig {
    /// Optional name for debugging.
    pub name: Option<String>,
    /// Initial stack size in bytes.
    pub stack_size: usize,
    /// Scheduling priority.
    pub priority: Priority,
}

impl Default for FiberConfig {
    fn default() -> Self {
        Self {
            name: None,
            stack_size: 8 * 1024, // 8 KB
            priority: Priority::Normal,
        }
    }
}

impl FiberConfig {
    /// Create a new fiber config with a name.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Default::default()
        }
    }

    /// Set the stack size.
    pub fn with_stack_size(mut self, size: usize) -> Self {
        self.stack_size = size;
        self
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
}

/// Condition for waking a suspended fiber.
#[derive(Debug, Clone, PartialEq)]
pub enum WakeCondition {
    /// Channel has data available.
    ChannelReadable(u64),
    /// Channel has space available.
    ChannelWritable(u64),
    /// Timer expired.
    Timeout(std::time::Instant),
    /// I/O ready.
    IoReady {
        /// File descriptor.
        fd: i32,
        /// Interest flags.
        interest: IoInterest,
    },
    /// Effect resumed.
    EffectResumed,
    /// Any of these conditions.
    Any(Vec<WakeCondition>),
}

/// I/O interest flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoInterest(u8);

impl IoInterest {
    /// Interested in read readiness.
    pub const READABLE: Self = Self(0b01);
    /// Interested in write readiness.
    pub const WRITABLE: Self = Self(0b10);
    /// Interested in both.
    pub const BOTH: Self = Self(0b11);

    /// Create from raw bits.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Get raw bits.
    pub const fn bits(&self) -> u8 {
        self.0
    }

    /// Check if readable interest is set.
    pub fn is_readable(&self) -> bool {
        self.0 & 0b01 != 0
    }

    /// Check if writable interest is set.
    pub fn is_writable(&self) -> bool {
        self.0 & 0b10 != 0
    }
}

/// A fiber's saved execution context.
#[derive(Debug, Default)]
pub struct FiberContext {
    // Platform-specific saved registers would go here.
    // For now, we use a simplified representation.
    /// Instruction pointer (program counter).
    pub ip: usize,
    /// Stack pointer.
    pub sp: usize,
    /// Base pointer (frame pointer).
    pub bp: usize,
}

/// Growable stack for fiber execution.
pub struct FiberStack {
    /// Stack memory.
    memory: Vec<u8>,
    /// Current stack size.
    size: usize,
    /// Maximum stack size.
    max_size: usize,
}

impl FiberStack {
    /// Create a new stack with the given initial and maximum sizes.
    pub fn new(initial_size: usize, max_size: usize) -> Self {
        Self {
            memory: vec![0u8; initial_size],
            size: initial_size,
            max_size,
        }
    }

    /// Get the stack top address.
    pub fn top(&self) -> *const u8 {
        unsafe { self.memory.as_ptr().add(self.size) }
    }

    /// Get the stack bottom address.
    pub fn bottom(&self) -> *const u8 {
        self.memory.as_ptr()
    }

    /// Get the current stack size.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Grow the stack if possible.
    pub fn grow(&mut self) -> bool {
        let new_size = self.size * 2;
        if new_size > self.max_size {
            return false;
        }
        self.memory.resize(new_size, 0);
        self.size = new_size;
        true
    }
}

impl fmt::Debug for FiberStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FiberStack")
            .field("size", &self.size)
            .field("max_size", &self.max_size)
            .finish()
    }
}

/// A fiber (lightweight thread).
pub struct Fiber {
    /// Unique identifier.
    pub id: FiberId,
    /// Parent fiber ID.
    pub parent: Option<FiberId>,
    /// Current state.
    pub state: FiberState,
    /// Execution stack.
    pub stack: FiberStack,
    /// Saved context.
    pub context: FiberContext,
    /// Scheduling priority.
    pub priority: Priority,
    /// Wake condition (if suspended).
    pub wake_condition: Option<WakeCondition>,
    /// Optional name for debugging.
    pub name: Option<String>,
    /// Creation timestamp.
    pub created_at: std::time::Instant,
    /// The fiber's task (boxed closure).
    task: Option<Box<dyn FnOnce() + Send>>,
    /// Result value (if completed).
    /// Reserved for future use when fiber join returns results.
    #[allow(dead_code)]
    result: Option<Box<dyn std::any::Any + Send>>,
}

impl Fiber {
    /// Create a new fiber with a task.
    pub fn new<F>(task: F, config: FiberConfig) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            id: next_fiber_id(),
            parent: None,
            state: FiberState::Runnable,
            stack: FiberStack::new(config.stack_size, 1024 * 1024),
            context: FiberContext::default(),
            priority: config.priority,
            wake_condition: None,
            name: config.name,
            created_at: std::time::Instant::now(),
            task: Some(Box::new(task)),
            result: None,
        }
    }

    /// Create a new fiber with a parent.
    pub fn with_parent<F>(task: F, config: FiberConfig, parent: FiberId) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let mut fiber = Self::new(task, config);
        fiber.parent = Some(parent);
        fiber
    }

    /// Check if the fiber is runnable.
    pub fn is_runnable(&self) -> bool {
        self.state == FiberState::Runnable
    }

    /// Check if the fiber is completed.
    pub fn is_completed(&self) -> bool {
        matches!(
            self.state,
            FiberState::Completed | FiberState::Failed | FiberState::Cancelled
        )
    }

    /// Suspend the fiber with a wake condition.
    pub fn suspend(&mut self, condition: WakeCondition) {
        self.state = FiberState::Suspended;
        self.wake_condition = Some(condition);
    }

    /// Resume the fiber.
    pub fn resume(&mut self) {
        self.state = FiberState::Runnable;
        self.wake_condition = None;
    }

    /// Mark the fiber as completed.
    pub fn complete(&mut self) {
        self.state = FiberState::Completed;
    }

    /// Mark the fiber as failed.
    pub fn fail(&mut self) {
        self.state = FiberState::Failed;
    }

    /// Mark the fiber as cancelled.
    pub fn cancel(&mut self) {
        self.state = FiberState::Cancelled;
    }

    /// Run the fiber's task.
    pub fn run(&mut self) {
        if let Some(task) = self.task.take() {
            self.state = FiberState::Running;
            task();
            if self.state == FiberState::Running {
                self.complete();
            }
        }
    }
}

impl fmt::Debug for Fiber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fiber")
            .field("id", &self.id)
            .field("parent", &self.parent)
            .field("state", &self.state)
            .field("priority", &self.priority)
            .field("name", &self.name)
            .finish()
    }
}

/// A handle to a spawned fiber.
#[derive(Debug, Clone)]
pub struct FiberHandle<T> {
    /// The fiber ID.
    pub id: FiberId,
    /// Phantom type parameter.
    _marker: std::marker::PhantomData<T>,
}

impl<T> FiberHandle<T> {
    /// Create a new fiber handle.
    pub fn new(id: FiberId) -> Self {
        Self {
            id,
            _marker: std::marker::PhantomData,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fiber_id_generation() {
        let id1 = next_fiber_id();
        let id2 = next_fiber_id();
        assert_ne!(id1, id2);
        assert!(id2.0 > id1.0);
    }

    #[test]
    fn test_fiber_state_default() {
        assert_eq!(FiberState::default(), FiberState::Runnable);
    }

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Low < Priority::Normal);
        assert!(Priority::Normal < Priority::High);
        assert!(Priority::High < Priority::Critical);
    }

    #[test]
    fn test_fiber_config_default() {
        let config = FiberConfig::default();
        assert!(config.name.is_none());
        assert_eq!(config.stack_size, 8 * 1024);
        assert_eq!(config.priority, Priority::Normal);
    }

    #[test]
    fn test_fiber_config_builder() {
        let config = FiberConfig::named("test")
            .with_stack_size(16 * 1024)
            .with_priority(Priority::High);

        assert_eq!(config.name.as_deref(), Some("test"));
        assert_eq!(config.stack_size, 16 * 1024);
        assert_eq!(config.priority, Priority::High);
    }

    #[test]
    fn test_fiber_stack_grow() {
        let mut stack = FiberStack::new(1024, 4096);
        assert_eq!(stack.size(), 1024);

        assert!(stack.grow());
        assert_eq!(stack.size(), 2048);

        assert!(stack.grow());
        assert_eq!(stack.size(), 4096);

        // Can't grow beyond max
        assert!(!stack.grow());
        assert_eq!(stack.size(), 4096);
    }

    #[test]
    fn test_fiber_creation() {
        let fiber = Fiber::new(|| {}, FiberConfig::default());
        assert!(fiber.is_runnable());
        assert!(!fiber.is_completed());
    }

    #[test]
    fn test_fiber_suspend_resume() {
        let mut fiber = Fiber::new(|| {}, FiberConfig::default());

        fiber.suspend(WakeCondition::EffectResumed);
        assert_eq!(fiber.state, FiberState::Suspended);
        assert!(fiber.wake_condition.is_some());

        fiber.resume();
        assert_eq!(fiber.state, FiberState::Runnable);
        assert!(fiber.wake_condition.is_none());
    }

    #[test]
    fn test_fiber_run() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let ran = Arc::new(AtomicBool::new(false));
        let ran_clone = ran.clone();

        let mut fiber = Fiber::new(
            move || {
                ran_clone.store(true, Ordering::SeqCst);
            },
            FiberConfig::default(),
        );

        fiber.run();

        assert!(ran.load(Ordering::SeqCst));
        assert!(fiber.is_completed());
    }

    #[test]
    fn test_io_interest() {
        assert!(IoInterest::READABLE.is_readable());
        assert!(!IoInterest::READABLE.is_writable());
        assert!(IoInterest::WRITABLE.is_writable());
        assert!(!IoInterest::WRITABLE.is_readable());
        assert!(IoInterest::BOTH.is_readable());
        assert!(IoInterest::BOTH.is_writable());
    }
}
