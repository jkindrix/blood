//! # Blood Runtime Library
//!
//! The Blood runtime provides:
//!
//! - **Fiber Scheduler**: M:N cooperative scheduling with work-stealing
//! - **Channels**: Multi-producer multi-consumer communication
//! - **I/O Reactor**: Platform-native async I/O (io_uring, kqueue, IOCP)
//! - **FFI Support**: Safe foreign function interface
//! - **Standard Library**: Core types and effects
//!
//! ## Technical Standards
//!
//! Implementation follows these standards:
//!
//! - **Work Stealing**: Based on Chase-Lev deque per
//!   [crossbeam-deque](https://docs.rs/crossbeam-deque)
//! - **Channels**: MPMC channels per
//!   [crossbeam-channel](https://docs.rs/crossbeam-channel)
//! - **I/O**: Platform-native (io_uring on Linux 5.6+)
//! - **FFI**: Per Rust 2024 Edition with `unsafe extern`
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       BLOOD RUNTIME                              │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                  │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
//! │  │   Scheduler  │  │   Channels   │  │  I/O Reactor │          │
//! │  │  (fiber.rs)  │  │ (channel.rs) │  │   (io.rs)    │          │
//! │  └──────────────┘  └──────────────┘  └──────────────┘          │
//! │         │                 │                 │                   │
//! │         └─────────────────┼─────────────────┘                   │
//! │                           │                                     │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
//! │  │     FFI      │  │   Stdlib     │  │    Memory    │          │
//! │  │   (ffi.rs)   │  │ (stdlib.rs)  │  │ (memory.rs)  │          │
//! │  └──────────────┘  └──────────────┘  └──────────────┘          │
//! │                                                                  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod cancellation;
pub mod channel;
pub mod config;
pub mod continuation;
pub mod fiber;
pub mod fiber_local;
pub mod ffi;
pub mod ffi_exports;
pub mod io;
pub mod log;
pub mod memory;
pub mod net;
pub mod observability;
pub mod panic;
pub mod process;
pub mod scheduler;
pub mod serialize;
pub mod signal;
pub mod simd;
pub mod stdlib;
pub mod sync;
pub mod timeout;

// Re-exports
pub use cancellation::{CancellationToken, CancellationSource, CancellationError, CancellableScope};
pub use channel::{bounded, unbounded, Receiver, Sender};
pub use config::{RuntimeConfig, RuntimeConfigBuilder, LogLevel, ConfigError};
pub use fiber::{Fiber, FiberId, FiberHandle, FiberState};
pub use fiber_local::{FiberLocal, FiberLocalKey, FiberLocalStorage, FiberContext, TraceContext, RequestContext, PropagationMode};
pub use panic::{BloodPanicInfo, Location as PanicLocation};
pub use scheduler::{Scheduler, Worker};
pub use signal::{Signal, SignalHandler};
pub use sync::{Mutex, RwLock, Once, Barrier, Semaphore, OnceLock};
pub use timeout::{Timeout, TimeoutGuard, TimeoutError, TimeoutBuilder, Deadline, with_timeout, with_timeout_and_parent};

/// Runtime version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the runtime with default configuration.
pub fn init() -> Scheduler {
    Scheduler::new(SchedulerConfig::default())
}

/// Initialize the runtime with custom scheduler configuration.
///
/// This is the legacy initialization method. For more comprehensive
/// configuration, use `init_with_runtime_config`.
pub fn init_with(config: SchedulerConfig) -> Scheduler {
    Scheduler::new(config)
}

/// Initialize the runtime with full runtime configuration.
///
/// This method supports all runtime configuration options including
/// memory limits, timeout settings, and logging configuration.
///
/// # Example
///
/// ```rust,ignore
/// use blood_runtime::{init_with_runtime_config, RuntimeConfig};
///
/// let config = RuntimeConfig::builder()
///     .num_workers(4)
///     .max_heap_size(1024 * 1024 * 1024)
///     .build()
///     .unwrap();
///
/// let scheduler = init_with_runtime_config(config);
/// ```
pub fn init_with_runtime_config(config: RuntimeConfig) -> Scheduler {
    // Store the runtime config globally for other components to access
    let _ = RUNTIME_CONFIG.set(config.clone());

    // Set memory limits
    memory::set_max_heap_size(config.memory.max_heap_size as u64);

    Scheduler::new(config.to_scheduler_config())
}

/// Initialize the runtime from environment variables.
///
/// Reads configuration from `BLOOD_*` environment variables.
/// See `RuntimeConfig::from_env()` for the full list of supported variables.
pub fn init_from_env() -> Scheduler {
    let config = RuntimeConfig::from_env();
    init_with_runtime_config(config)
}

/// Global runtime configuration.
static RUNTIME_CONFIG: std::sync::OnceLock<RuntimeConfig> = std::sync::OnceLock::new();

/// Get the current runtime configuration.
///
/// Returns `None` if the runtime was not initialized with `init_with_runtime_config`
/// or `init_from_env`.
pub fn runtime_config() -> Option<&'static RuntimeConfig> {
    RUNTIME_CONFIG.get()
}

/// Configuration for the runtime scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Number of worker threads (default: number of CPUs).
    pub num_workers: usize,
    /// Initial fiber stack size in bytes (default: 8KB).
    pub initial_stack_size: usize,
    /// Maximum fiber stack size in bytes (default: 1MB).
    pub max_stack_size: usize,
    /// Enable work stealing (default: true).
    pub work_stealing: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            num_workers: num_cpus(),
            initial_stack_size: 8 * 1024,      // 8 KB
            max_stack_size: 1024 * 1024,       // 1 MB
            work_stealing: true,
        }
    }
}

/// Get the number of CPUs.
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_config_default() {
        let config = SchedulerConfig::default();
        assert!(config.num_workers >= 1);
        assert_eq!(config.initial_stack_size, 8 * 1024);
        assert_eq!(config.max_stack_size, 1024 * 1024);
        assert!(config.work_stealing);
    }
}
