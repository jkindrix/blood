//! # Work-Stealing Fiber Scheduler
//!
//! M:N cooperative scheduler with work-stealing.
//!
//! ## Design
//!
//! Based on the Blood runtime specification (ROADMAP.md §9.2):
//! - Multiple worker threads each with local deque
//! - Global queue for overflow
//! - Work-stealing for load balancing
//!
//! ## Technical References
//!
//! - [Chase-Lev Deque](https://doi.org/10.1145/1073970.1073974)
//! - [crossbeam-deque](https://docs.rs/crossbeam-deque)
//! - [Tokio Scheduler](https://tokio.rs/blog/2019-10-scheduler)

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_deque::{Injector, Stealer, Worker as Deque};
use parking_lot::Mutex;

use crate::cancellation::{CancellationSource, CancellationToken};
use crate::fiber::{Fiber, FiberConfig, FiberId, FiberState, WakeCondition};
use crate::signal::{self, SignalHandler};
use crate::SchedulerConfig;

// ============================================================================
// CONFIGURED SCHEDULER VALUES
// ============================================================================

/// Get the configured number of worker threads from the runtime configuration.
///
/// Returns the CPU count if no runtime config is available.
pub fn configured_worker_count() -> usize {
    crate::runtime_config()
        .map(|c| c.scheduler.num_workers)
        .unwrap_or_else(|| std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4))
}

/// Get the configured work stealing setting from the runtime configuration.
///
/// Returns true if no runtime config is available.
pub fn configured_work_stealing() -> bool {
    crate::runtime_config()
        .map(|c| c.scheduler.work_stealing)
        .unwrap_or(true)
}

/// Create a SchedulerConfig from the global runtime configuration.
///
/// This allows the scheduler to be configured from environment variables
/// via RuntimeConfig.
pub fn scheduler_config_from_runtime() -> SchedulerConfig {
    if let Some(config) = crate::runtime_config() {
        // Convert from config::SchedulerConfig to crate::SchedulerConfig
        SchedulerConfig {
            num_workers: config.scheduler.num_workers,
            initial_stack_size: config.scheduler.initial_stack_size,
            max_stack_size: config.scheduler.max_stack_size,
            work_stealing: config.scheduler.work_stealing,
        }
    } else {
        SchedulerConfig::default()
    }
}

/// Work-stealing scheduler for fibers.
pub struct Scheduler {
    /// Configuration.
    config: SchedulerConfig,
    /// Worker threads.
    workers: Vec<WorkerHandle>,
    /// Global injection queue.
    global_queue: Arc<Injector<FiberId>>,
    /// All fibers indexed by ID.
    fibers: Arc<Mutex<HashMap<FiberId, Fiber>>>,
    /// Stealers for work-stealing.
    stealers: Vec<Stealer<FiberId>>,
    /// Shutdown flag (kept for backwards compatibility).
    shutdown: Arc<AtomicBool>,
    /// Number of active workers.
    active_workers: Arc<AtomicUsize>,
    /// Global cancellation source for shutdown.
    cancellation_source: CancellationSource,
    /// Global cancellation token.
    cancellation_token: CancellationToken,
    /// Signal handler for graceful shutdown.
    signal_handler: Option<SignalHandler>,
}

impl Scheduler {
    /// Create a new scheduler from the global runtime configuration.
    ///
    /// This reads scheduler settings from RuntimeConfig, which can be
    /// configured via environment variables (BLOOD_NUM_WORKERS, etc.).
    pub fn from_runtime_config() -> Self {
        Self::new(scheduler_config_from_runtime())
    }

    /// Create a new scheduler with the given configuration.
    pub fn new(config: SchedulerConfig) -> Self {
        let global_queue = Arc::new(Injector::new());
        let fibers = Arc::new(Mutex::new(HashMap::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let active_workers = Arc::new(AtomicUsize::new(0));

        // Create global cancellation source for shutdown
        let cancellation_source = CancellationSource::new();
        let cancellation_token = cancellation_source.token();

        // Create worker deques and collect stealers
        let mut workers = Vec::with_capacity(config.num_workers);
        let mut stealers = Vec::with_capacity(config.num_workers);

        for id in 0..config.num_workers {
            let deque = Deque::new_fifo();
            stealers.push(deque.stealer());

            workers.push(WorkerHandle {
                id,
                deque,
                thread: None,
            });
        }

        Self {
            config,
            workers,
            global_queue,
            fibers,
            stealers,
            shutdown,
            active_workers,
            cancellation_source,
            cancellation_token,
            signal_handler: None,
        }
    }

    /// Get the number of workers.
    pub fn num_workers(&self) -> usize {
        self.config.num_workers
    }

    /// Spawn a new fiber with a task.
    pub fn spawn<F>(&self, task: F) -> FiberId
    where
        F: FnOnce() + Send + 'static,
    {
        self.spawn_with_config(task, FiberConfig::default())
    }

    /// Spawn a new fiber with configuration.
    ///
    /// If the config doesn't have a cancellation token, the fiber will
    /// inherit the scheduler's global cancellation token.
    pub fn spawn_with_config<F>(&self, task: F, mut config: FiberConfig) -> FiberId
    where
        F: FnOnce() + Send + 'static,
    {
        // Inherit scheduler's cancellation token if not specified
        if config.cancellation_token.is_none() {
            config.cancellation_token = Some(self.cancellation_token.clone());
        }

        let fiber = Fiber::new(task, config);
        let id = fiber.id;

        // Add to fiber registry
        {
            let mut fibers = self.fibers.lock();
            fibers.insert(id, fiber);
        }

        // Push to global queue
        self.global_queue.push(id);

        id
    }

    /// Spawn a fiber with a specific parent cancellation token.
    ///
    /// The fiber will be cancelled when the parent token is cancelled.
    pub fn spawn_with_cancellation<F>(
        &self,
        task: F,
        parent_token: CancellationToken,
    ) -> FiberId
    where
        F: FnOnce() + Send + 'static,
    {
        let config = FiberConfig::default().with_cancellation(parent_token);
        self.spawn_with_config(task, config)
    }

    /// Get the scheduler's cancellation token.
    ///
    /// This token is cancelled when the scheduler shuts down.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    /// Run the scheduler, blocking until shutdown.
    pub fn run(&mut self) {
        // Start worker threads
        let worker_count = self.workers.len();

        for i in 0..worker_count {
            let worker = Worker::new(
                i,
                self.global_queue.clone(),
                self.fibers.clone(),
                self.stealers.clone(),
                self.shutdown.clone(),
                self.active_workers.clone(),
                self.cancellation_token.clone(),
            );

            // Take ownership of the deque
            let deque = std::mem::replace(
                &mut self.workers[i].deque,
                Deque::new_fifo(),
            );

            let handle = thread::Builder::new()
                .name(format!("blood-worker-{}", i))
                .spawn(move || worker.run_loop(deque))
                .expect("failed to spawn worker thread");

            self.workers[i].thread = Some(handle);
        }

        // Wait for all workers to finish
        for worker in &mut self.workers {
            if let Some(handle) = worker.thread.take() {
                let _ = handle.join();
            }
        }
    }

    /// Request scheduler shutdown.
    ///
    /// This triggers the global cancellation token, which propagates
    /// cancellation to all fibers and their child operations.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.cancellation_source.cancel_with_reason(Some("scheduler shutdown".into()));
    }

    /// Request scheduler shutdown with a reason.
    pub fn shutdown_with_reason(&self, reason: &str) {
        self.shutdown.store(true, Ordering::Release);
        self.cancellation_source.cancel_with_reason(Some(reason.into()));
    }

    /// Cancel a specific fiber by ID.
    ///
    /// Returns true if the fiber was found and cancelled.
    pub fn cancel_fiber(&self, fiber_id: FiberId) -> bool {
        let mut fibers = self.fibers.lock();
        if let Some(fiber) = fibers.get_mut(&fiber_id) {
            fiber.cancel();
            true
        } else {
            false
        }
    }

    // ========================================================================
    // Signal Handling Integration
    // ========================================================================

    /// Install signal handlers for graceful shutdown.
    ///
    /// When SIGTERM or SIGINT is received, the scheduler will trigger
    /// cancellation of all fibers and begin graceful shutdown.
    ///
    /// Returns true if handlers were installed, false if already installed.
    pub fn install_signal_handlers(&mut self) -> bool {
        if self.signal_handler.is_some() {
            return false; // Already installed
        }

        let handler = SignalHandler::new();
        let installed = handler.install();
        self.signal_handler = Some(handler);
        installed
    }

    /// Check if a shutdown signal has been received.
    pub fn signal_shutdown_requested(&self) -> bool {
        signal::shutdown_requested()
    }

    /// Run the scheduler with signal handling enabled.
    ///
    /// This method:
    /// 1. Installs signal handlers if not already installed
    /// 2. Starts workers with signal monitoring
    /// 3. When a signal is received, triggers the cancellation tree
    /// 4. Runs until all fibers complete or shutdown is complete
    pub fn run_with_signal_handling(&mut self) {
        // Install signal handlers
        self.install_signal_handlers();

        // Start worker threads (same as run())
        let worker_count = self.workers.len();

        for i in 0..worker_count {
            let worker = Worker::new(
                i,
                self.global_queue.clone(),
                self.fibers.clone(),
                self.stealers.clone(),
                self.shutdown.clone(),
                self.active_workers.clone(),
                self.cancellation_token.clone(),
            );

            let deque = std::mem::replace(
                &mut self.workers[i].deque,
                Deque::new_fifo(),
            );

            let handle = thread::Builder::new()
                .name(format!("blood-worker-{}", i))
                .spawn(move || worker.run_loop(deque))
                .expect("failed to spawn worker thread");

            self.workers[i].thread = Some(handle);
        }

        // Monitor for signals in the main thread
        while !self.shutdown.load(Ordering::Acquire) && !self.cancellation_token.is_cancelled() {
            // Check for OS signal
            if signal::shutdown_requested() {
                self.shutdown_with_reason("signal received");
                break;
            }

            // Check if all fibers are done
            if self.active_fiber_count() == 0 && self.runnable_fiber_count() == 0 {
                self.shutdown();
                break;
            }

            // Small sleep to avoid busy-waiting
            thread::sleep(std::time::Duration::from_millis(10));
        }

        // Wait for all workers to finish
        for worker in &mut self.workers {
            if let Some(handle) = worker.thread.take() {
                let _ = handle.join();
            }
        }
    }

    /// Get the signal handler, if installed.
    pub fn signal_handler(&self) -> Option<&SignalHandler> {
        self.signal_handler.as_ref()
    }

    /// Check if the scheduler is shutting down.
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Get the number of active fibers.
    pub fn active_fiber_count(&self) -> usize {
        self.fibers.lock().len()
    }

    /// Get the number of runnable fibers.
    pub fn runnable_fiber_count(&self) -> usize {
        self.fibers
            .lock()
            .values()
            .filter(|f| f.is_runnable())
            .count()
    }
}

/// Handle to a worker thread.
struct WorkerHandle {
    /// Worker ID (reserved for debugging).
    #[allow(dead_code)] // Worker ID — used for debugging and diagnostics
    id: usize,
    /// Local work deque.
    deque: Deque<FiberId>,
    /// Thread handle.
    thread: Option<JoinHandle<()>>,
}

/// A worker thread in the scheduler.
pub struct Worker {
    /// Worker ID (reserved for debugging and metrics).
    #[allow(dead_code)] // Worker ID — used for debugging, metrics, and tracing
    id: usize,
    /// Global injection queue.
    global_queue: Arc<Injector<FiberId>>,
    /// All fibers.
    fibers: Arc<Mutex<HashMap<FiberId, Fiber>>>,
    /// Stealers from other workers.
    stealers: Vec<Stealer<FiberId>>,
    /// Shutdown flag.
    shutdown: Arc<AtomicBool>,
    /// Active worker count.
    active_workers: Arc<AtomicUsize>,
    /// Cancellation token for shutdown detection.
    cancellation_token: CancellationToken,
}

impl Worker {
    /// Create a new worker.
    fn new(
        id: usize,
        global_queue: Arc<Injector<FiberId>>,
        fibers: Arc<Mutex<HashMap<FiberId, Fiber>>>,
        stealers: Vec<Stealer<FiberId>>,
        shutdown: Arc<AtomicBool>,
        active_workers: Arc<AtomicUsize>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            id,
            global_queue,
            fibers,
            stealers,
            shutdown,
            active_workers,
            cancellation_token,
        }
    }

    /// Run the worker loop.
    fn run_loop(self, local: Deque<FiberId>) {
        self.active_workers.fetch_add(1, Ordering::AcqRel);

        loop {
            // Check for shutdown (both via flag and cancellation token)
            if self.shutdown.load(Ordering::Acquire) || self.cancellation_token.is_cancelled() {
                break;
            }

            // Try to get work
            if let Some(fiber_id) = self.find_work(&local) {
                self.run_fiber(fiber_id);

                // Check cancellation between fiber executions for responsive shutdown
                if self.cancellation_token.is_cancelled() {
                    break;
                }
            } else {
                // No work available, yield to OS
                thread::yield_now();
            }
        }

        self.active_workers.fetch_sub(1, Ordering::AcqRel);
    }

    /// Find work using work-stealing.
    fn find_work(&self, local: &Deque<FiberId>) -> Option<FiberId> {
        // 1. Try local queue first
        if let Some(id) = local.pop() {
            return Some(id);
        }

        // 2. Try global queue
        loop {
            match self.global_queue.steal() {
                crossbeam_deque::Steal::Success(id) => return Some(id),
                crossbeam_deque::Steal::Empty => break,
                crossbeam_deque::Steal::Retry => continue,
            }
        }

        // 3. Try stealing from other workers
        for stealer in &self.stealers {
            loop {
                match stealer.steal() {
                    crossbeam_deque::Steal::Success(id) => return Some(id),
                    crossbeam_deque::Steal::Empty => break,
                    crossbeam_deque::Steal::Retry => continue,
                }
            }
        }

        None
    }

    /// Run a fiber to completion or suspension.
    fn run_fiber(&self, fiber_id: FiberId) {
        // Get the fiber
        let mut fiber = {
            let mut fibers = self.fibers.lock();
            match fibers.remove(&fiber_id) {
                Some(f) => f,
                None => return, // Fiber was removed
            }
        };

        // Check if runnable
        if !fiber.is_runnable() {
            // Put it back
            let mut fibers = self.fibers.lock();
            fibers.insert(fiber_id, fiber);
            return;
        }

        // Run the fiber
        fiber.run();

        // Handle result based on state
        match fiber.state {
            FiberState::Completed | FiberState::Failed | FiberState::Cancelled => {
                // Fiber is done, don't re-add
            }
            FiberState::Suspended => {
                // Fiber is suspended, add back to registry (will be woken later)
                let mut fibers = self.fibers.lock();
                fibers.insert(fiber_id, fiber);
            }
            FiberState::Runnable => {
                // Fiber yielded, reschedule
                {
                    let mut fibers = self.fibers.lock();
                    fibers.insert(fiber_id, fiber);
                }
                self.global_queue.push(fiber_id);
            }
            _ => {
                // Other states, put back
                let mut fibers = self.fibers.lock();
                fibers.insert(fiber_id, fiber);
            }
        }
    }
}

/// Result of running a fiber.
#[derive(Debug, Clone, PartialEq)]
pub enum FiberResult {
    /// Fiber yielded control.
    Yielded,
    /// Fiber suspended waiting for condition.
    Suspended(WakeCondition),
    /// Fiber completed successfully.
    Completed,
    /// Fiber failed with error.
    Failed,
    /// Fiber was cancelled.
    Cancelled,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI32;
    use std::time::Duration;

    #[test]
    fn test_scheduler_creation() {
        let config = SchedulerConfig::default();
        let scheduler = Scheduler::new(config.clone());
        assert_eq!(scheduler.num_workers(), config.num_workers);
    }

    #[test]
    fn test_spawn_fiber() {
        let scheduler = Scheduler::new(SchedulerConfig::default());
        let id = scheduler.spawn(|| {});
        assert_eq!(scheduler.active_fiber_count(), 1);
        assert!(id.as_u64() > 0);
    }

    #[test]
    fn test_fiber_execution() {
        let counter = Arc::new(AtomicI32::new(0));
        let counter_clone = counter.clone();

        let scheduler = Scheduler::new(SchedulerConfig {
            num_workers: 1,
            ..Default::default()
        });

        scheduler.spawn(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Run in a separate thread with timeout
        let scheduler_arc = Arc::new(Mutex::new(scheduler));
        let scheduler_clone = scheduler_arc.clone();

        let handle = thread::spawn(move || {
            let mut sched = scheduler_clone.lock();

            // Spawn a shutdown task
            let shutdown_flag = sched.shutdown.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                shutdown_flag.store(true, Ordering::Release);
            });

            sched.run();
        });

        // Wait for completion
        let _ = handle.join();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_multiple_fibers() {
        let counter = Arc::new(AtomicI32::new(0));
        let mut scheduler = Scheduler::new(SchedulerConfig {
            num_workers: 2,
            ..Default::default()
        });

        for _ in 0..10 {
            let c = counter.clone();
            scheduler.spawn(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }

        assert_eq!(scheduler.active_fiber_count(), 10);

        // Shutdown after short delay
        let shutdown = scheduler.shutdown.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            shutdown.store(true, Ordering::Release);
        });

        scheduler.run();

        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn test_work_stealing() {
        // This is a basic test that the work-stealing mechanism works
        let counter = Arc::new(AtomicI32::new(0));
        let mut scheduler = Scheduler::new(SchedulerConfig {
            num_workers: 4,
            ..Default::default()
        });

        // Spawn many fibers
        for _ in 0..100 {
            let c = counter.clone();
            scheduler.spawn(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }

        let shutdown = scheduler.shutdown.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            shutdown.store(true, Ordering::Release);
        });

        scheduler.run();

        // All fibers should have run
        assert_eq!(counter.load(Ordering::SeqCst), 100);
    }

    #[test]
    fn test_configured_worker_count() {
        // Without runtime config, should return available CPUs or default to 4
        let count = configured_worker_count();
        assert!(count > 0);
    }

    #[test]
    fn test_configured_work_stealing() {
        // Without runtime config, should return true
        let enabled = configured_work_stealing();
        assert!(enabled);
    }

    #[test]
    fn test_scheduler_config_from_runtime() {
        // Without runtime config, should return default SchedulerConfig
        let config = scheduler_config_from_runtime();
        assert!(config.num_workers > 0);
        assert!(config.initial_stack_size >= 4096);
        assert!(config.work_stealing);
    }

    #[test]
    fn test_scheduler_from_runtime_config() {
        // Create scheduler using runtime configuration
        let scheduler = Scheduler::from_runtime_config();
        assert!(scheduler.num_workers() > 0);
    }
}
