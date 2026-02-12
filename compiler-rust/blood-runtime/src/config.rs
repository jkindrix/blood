//! Runtime Configuration
//!
//! This module provides comprehensive configuration for the Blood runtime.
//! Configuration can be set programmatically or loaded from environment variables.
//!
//! # Environment Variables
//!
//! All environment variables use the `BLOOD_` prefix:
//!
//! | Variable | Description | Default |
//! |----------|-------------|---------|
//! | `BLOOD_NUM_WORKERS` | Number of worker threads | CPU count |
//! | `BLOOD_INITIAL_STACK_SIZE` | Initial fiber stack size in bytes | 8192 (8KB) |
//! | `BLOOD_MAX_STACK_SIZE` | Maximum fiber stack size in bytes | 1048576 (1MB) |
//! | `BLOOD_WORK_STEALING` | Enable work stealing ("true"/"false") | true |
//! | `BLOOD_MAX_HEAP_SIZE` | Maximum heap size in bytes (0 = unlimited) | 0 |
//! | `BLOOD_MAX_REGION_SIZE` | Maximum region size in bytes | 16777216 (16MB) |
//! | `BLOOD_GC_THRESHOLD` | GC cycle collection threshold | 1000 |
//! | `BLOOD_DEFAULT_TIMEOUT_MS` | Default operation timeout in milliseconds (0 = none) | 0 |
//! | `BLOOD_GRACEFUL_SHUTDOWN_MS` | Graceful shutdown timeout in milliseconds | 5000 |
//! | `BLOOD_LOG_LEVEL` | Log level (off/error/warn/info/debug/trace) | info |
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::config::RuntimeConfig;
//!
//! // Load from environment with defaults
//! let config = RuntimeConfig::from_env();
//!
//! // Or use the builder pattern
//! let config = RuntimeConfig::builder()
//!     .num_workers(4)
//!     .max_heap_size(1024 * 1024 * 1024) // 1GB
//!     .build();
//! ```

use std::env;
use std::time::Duration;

/// Log level for runtime logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    /// No logging.
    Off,
    /// Error messages only.
    Error,
    /// Warnings and errors.
    Warn,
    /// Informational messages (default).
    #[default]
    Info,
    /// Debug messages.
    Debug,
    /// Trace-level messages.
    Trace,
}

impl LogLevel {
    /// Parse a log level from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "off" | "none" | "0" => Some(LogLevel::Off),
            "error" | "err" | "1" => Some(LogLevel::Error),
            "warn" | "warning" | "2" => Some(LogLevel::Warn),
            "info" | "3" => Some(LogLevel::Info),
            "debug" | "4" => Some(LogLevel::Debug),
            "trace" | "5" => Some(LogLevel::Trace),
            _ => None,
        }
    }

    /// Convert to a string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

/// Scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Number of worker threads.
    /// Default: number of available CPUs.
    pub num_workers: usize,

    /// Initial fiber stack size in bytes.
    /// Default: 8KB (8192 bytes).
    pub initial_stack_size: usize,

    /// Maximum fiber stack size in bytes.
    /// Default: 1MB (1048576 bytes).
    pub max_stack_size: usize,

    /// Enable work stealing between worker threads.
    /// Default: true.
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

/// Memory configuration.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Maximum heap size in bytes.
    /// 0 means unlimited (default).
    pub max_heap_size: usize,

    /// Maximum size for a single region in bytes.
    /// Default: 16MB.
    pub max_region_size: usize,

    /// Threshold for triggering cycle collection.
    /// Default: 1000 allocations.
    pub gc_threshold: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_heap_size: 0,                  // Unlimited
            max_region_size: 16 * 1024 * 1024, // 16 MB
            gc_threshold: 1000,
        }
    }
}

/// Timeout and deadline configuration.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Default operation timeout.
    /// None means no timeout (default).
    pub default_timeout: Option<Duration>,

    /// Default timeout for general operations.
    /// Default: 30 seconds.
    pub default_operation_timeout: Duration,

    /// Timeout for network operations.
    /// Default: 60 seconds.
    pub network_timeout: Duration,

    /// Timeout for I/O operations.
    /// Default: 30 seconds.
    pub io_timeout: Duration,

    /// Timeout for compute-bound operations.
    /// Default: 300 seconds (5 minutes).
    pub compute_timeout: Duration,

    /// Graceful shutdown timeout.
    /// Default: 5 seconds.
    pub graceful_shutdown: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            default_timeout: None,
            default_operation_timeout: Duration::from_secs(30),
            network_timeout: Duration::from_secs(60),
            io_timeout: Duration::from_secs(30),
            compute_timeout: Duration::from_secs(300),
            graceful_shutdown: Duration::from_secs(5),
        }
    }
}

/// Signal handling configuration.
#[derive(Debug, Clone)]
pub struct SignalConfig {
    /// Handle SIGTERM for graceful shutdown.
    /// Default: true.
    pub handle_sigterm: bool,

    /// Handle SIGINT (Ctrl+C) for graceful shutdown.
    /// Default: true.
    pub handle_sigint: bool,

    /// Handle SIGHUP for configuration reload.
    /// Default: false.
    pub handle_sighup: bool,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            handle_sigterm: true,
            handle_sigint: true,
            handle_sighup: false,
        }
    }
}

/// Logging configuration.
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Log level.
    /// Default: Info.
    pub level: LogLevel,

    /// Include timestamps in log output.
    /// Default: true.
    pub timestamps: bool,

    /// Include source location in log output.
    /// Default: false (only in debug builds).
    pub source_location: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            timestamps: true,
            source_location: cfg!(debug_assertions),
        }
    }
}

/// Complete runtime configuration.
///
/// This struct contains all configuration for the Blood runtime.
/// Use `RuntimeConfig::default()` for sensible defaults, or
/// `RuntimeConfig::from_env()` to load from environment variables.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    /// Scheduler configuration.
    pub scheduler: SchedulerConfig,

    /// Memory configuration.
    pub memory: MemoryConfig,

    /// Timeout configuration.
    pub timeout: TimeoutConfig,

    /// Signal handling configuration.
    pub signals: SignalConfig,

    /// Logging configuration.
    pub log: LogConfig,
}

impl RuntimeConfig {
    /// Create a new builder for RuntimeConfig.
    pub fn builder() -> RuntimeConfigBuilder {
        RuntimeConfigBuilder::new()
    }

    /// Load configuration from environment variables.
    ///
    /// Environment variables that are not set will use default values.
    /// Invalid values will be logged as warnings and default values will be used.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Scheduler configuration
        if let Some(val) = parse_env_usize("BLOOD_NUM_WORKERS") {
            if val > 0 {
                config.scheduler.num_workers = val;
            }
        }

        if let Some(val) = parse_env_usize("BLOOD_INITIAL_STACK_SIZE") {
            if val >= 4096 {
                config.scheduler.initial_stack_size = val;
            }
        }

        if let Some(val) = parse_env_usize("BLOOD_MAX_STACK_SIZE") {
            if val >= config.scheduler.initial_stack_size {
                config.scheduler.max_stack_size = val;
            }
        }

        if let Some(val) = parse_env_bool("BLOOD_WORK_STEALING") {
            config.scheduler.work_stealing = val;
        }

        // Memory configuration
        if let Some(val) = parse_env_usize("BLOOD_MAX_HEAP_SIZE") {
            config.memory.max_heap_size = val;
        }

        if let Some(val) = parse_env_usize("BLOOD_MAX_REGION_SIZE") {
            if val >= 4096 {
                config.memory.max_region_size = val;
            }
        }

        if let Some(val) = parse_env_usize("BLOOD_GC_THRESHOLD") {
            if val > 0 {
                config.memory.gc_threshold = val;
            }
        }

        // Timeout configuration
        if let Some(val) = parse_env_usize("BLOOD_DEFAULT_TIMEOUT_MS") {
            config.timeout.default_timeout = if val > 0 {
                Some(Duration::from_millis(val as u64))
            } else {
                None
            };
        }

        if let Some(val) = parse_env_usize("BLOOD_GRACEFUL_SHUTDOWN_MS") {
            config.timeout.graceful_shutdown = Duration::from_millis(val as u64);
        }

        if let Some(val) = parse_env_usize("BLOOD_OPERATION_TIMEOUT_MS") {
            config.timeout.default_operation_timeout = Duration::from_millis(val as u64);
        }

        if let Some(val) = parse_env_usize("BLOOD_NETWORK_TIMEOUT_MS") {
            config.timeout.network_timeout = Duration::from_millis(val as u64);
        }

        if let Some(val) = parse_env_usize("BLOOD_IO_TIMEOUT_MS") {
            config.timeout.io_timeout = Duration::from_millis(val as u64);
        }

        if let Some(val) = parse_env_usize("BLOOD_COMPUTE_TIMEOUT_MS") {
            config.timeout.compute_timeout = Duration::from_millis(val as u64);
        }

        // Logging configuration
        if let Ok(val) = env::var("BLOOD_LOG_LEVEL") {
            if let Some(level) = LogLevel::from_str(&val) {
                config.log.level = level;
            }
        }

        config
    }

    /// Convert to the legacy SchedulerConfig for backward compatibility.
    pub fn to_scheduler_config(&self) -> crate::SchedulerConfig {
        crate::SchedulerConfig {
            num_workers: self.scheduler.num_workers,
            initial_stack_size: self.scheduler.initial_stack_size,
            max_stack_size: self.scheduler.max_stack_size,
            work_stealing: self.scheduler.work_stealing,
        }
    }

    /// Validate the configuration and return any errors.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate scheduler config
        if self.scheduler.num_workers == 0 {
            return Err(ConfigError::InvalidValue {
                field: "scheduler.num_workers".into(),
                message: "must be at least 1".into(),
            });
        }

        if self.scheduler.initial_stack_size < 4096 {
            return Err(ConfigError::InvalidValue {
                field: "scheduler.initial_stack_size".into(),
                message: "must be at least 4096 bytes".into(),
            });
        }

        if self.scheduler.max_stack_size < self.scheduler.initial_stack_size {
            return Err(ConfigError::InvalidValue {
                field: "scheduler.max_stack_size".into(),
                message: "must be at least initial_stack_size".into(),
            });
        }

        // Validate memory config
        if self.memory.max_region_size < 4096 {
            return Err(ConfigError::InvalidValue {
                field: "memory.max_region_size".into(),
                message: "must be at least 4096 bytes".into(),
            });
        }

        if self.memory.gc_threshold == 0 {
            return Err(ConfigError::InvalidValue {
                field: "memory.gc_threshold".into(),
                message: "must be at least 1".into(),
            });
        }

        Ok(())
    }
}

/// Configuration error.
#[derive(Debug, Clone)]
pub enum ConfigError {
    /// Invalid configuration value.
    InvalidValue {
        /// Field name.
        field: String,
        /// Error message.
        message: String,
    },
    /// Environment variable parse error.
    EnvParseError {
        /// Variable name.
        var: String,
        /// Error message.
        message: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::InvalidValue { field, message } => {
                write!(f, "invalid configuration for '{}': {}", field, message)
            }
            ConfigError::EnvParseError { var, message } => {
                write!(f, "failed to parse environment variable '{}': {}", var, message)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Builder for RuntimeConfig.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfigBuilder {
    config: RuntimeConfig,
}

impl RuntimeConfigBuilder {
    /// Create a new builder with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the number of worker threads.
    pub fn num_workers(mut self, n: usize) -> Self {
        self.config.scheduler.num_workers = n;
        self
    }

    /// Set the initial fiber stack size in bytes.
    pub fn initial_stack_size(mut self, size: usize) -> Self {
        self.config.scheduler.initial_stack_size = size;
        self
    }

    /// Set the maximum fiber stack size in bytes.
    pub fn max_stack_size(mut self, size: usize) -> Self {
        self.config.scheduler.max_stack_size = size;
        self
    }

    /// Enable or disable work stealing.
    pub fn work_stealing(mut self, enabled: bool) -> Self {
        self.config.scheduler.work_stealing = enabled;
        self
    }

    /// Set the maximum heap size in bytes (0 = unlimited).
    pub fn max_heap_size(mut self, size: usize) -> Self {
        self.config.memory.max_heap_size = size;
        self
    }

    /// Set the maximum region size in bytes.
    pub fn max_region_size(mut self, size: usize) -> Self {
        self.config.memory.max_region_size = size;
        self
    }

    /// Set the GC cycle collection threshold.
    pub fn gc_threshold(mut self, threshold: usize) -> Self {
        self.config.memory.gc_threshold = threshold;
        self
    }

    /// Set the default operation timeout.
    pub fn default_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.config.timeout.default_timeout = timeout;
        self
    }

    /// Set the graceful shutdown timeout.
    pub fn graceful_shutdown(mut self, timeout: Duration) -> Self {
        self.config.timeout.graceful_shutdown = timeout;
        self
    }

    /// Set the default operation timeout.
    pub fn default_operation_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout.default_operation_timeout = timeout;
        self
    }

    /// Set the network timeout.
    pub fn network_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout.network_timeout = timeout;
        self
    }

    /// Set the I/O timeout.
    pub fn io_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout.io_timeout = timeout;
        self
    }

    /// Set the compute timeout.
    pub fn compute_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout.compute_timeout = timeout;
        self
    }

    /// Enable or disable SIGTERM handling.
    pub fn handle_sigterm(mut self, enabled: bool) -> Self {
        self.config.signals.handle_sigterm = enabled;
        self
    }

    /// Enable or disable SIGINT handling.
    pub fn handle_sigint(mut self, enabled: bool) -> Self {
        self.config.signals.handle_sigint = enabled;
        self
    }

    /// Enable or disable SIGHUP handling.
    pub fn handle_sighup(mut self, enabled: bool) -> Self {
        self.config.signals.handle_sighup = enabled;
        self
    }

    /// Set the log level.
    pub fn log_level(mut self, level: LogLevel) -> Self {
        self.config.log.level = level;
        self
    }

    /// Build the configuration.
    ///
    /// This validates the configuration and returns an error if invalid.
    pub fn build(self) -> Result<RuntimeConfig, ConfigError> {
        self.config.validate()?;
        Ok(self.config)
    }

    /// Build the configuration without validation.
    ///
    /// Use this only if you're certain the configuration is valid.
    pub fn build_unchecked(self) -> RuntimeConfig {
        self.config
    }
}

/// Parse an environment variable as usize.
fn parse_env_usize(name: &str) -> Option<usize> {
    env::var(name).ok().and_then(|s| s.parse().ok())
}

/// Parse an environment variable as bool.
fn parse_env_bool(name: &str) -> Option<bool> {
    env::var(name).ok().and_then(|s| {
        match s.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

/// Get the number of available CPUs.
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RuntimeConfig::default();
        assert!(config.scheduler.num_workers >= 1);
        assert_eq!(config.scheduler.initial_stack_size, 8 * 1024);
        assert_eq!(config.scheduler.max_stack_size, 1024 * 1024);
        assert!(config.scheduler.work_stealing);
        assert_eq!(config.memory.max_heap_size, 0);
        assert_eq!(config.memory.max_region_size, 16 * 1024 * 1024);
        assert!(config.timeout.default_timeout.is_none());
        assert_eq!(config.timeout.graceful_shutdown, Duration::from_secs(5));
    }

    #[test]
    fn test_builder() {
        let config = RuntimeConfig::builder()
            .num_workers(4)
            .max_heap_size(1024 * 1024 * 1024)
            .default_timeout(Some(Duration::from_secs(30)))
            .log_level(LogLevel::Debug)
            .build()
            .unwrap();

        assert_eq!(config.scheduler.num_workers, 4);
        assert_eq!(config.memory.max_heap_size, 1024 * 1024 * 1024);
        assert_eq!(config.timeout.default_timeout, Some(Duration::from_secs(30)));
        assert_eq!(config.log.level, LogLevel::Debug);
    }

    #[test]
    fn test_builder_validation() {
        let result = RuntimeConfig::builder()
            .num_workers(0)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str("off"), Some(LogLevel::Off));
        assert_eq!(LogLevel::from_str("ERROR"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("debug"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("TRACE"), Some(LogLevel::Trace));
        assert_eq!(LogLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_validation_initial_stack_too_small() {
        let result = RuntimeConfig::builder()
            .initial_stack_size(1024)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_validation_max_stack_less_than_initial() {
        let result = RuntimeConfig::builder()
            .initial_stack_size(16 * 1024)
            .max_stack_size(8 * 1024)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_to_scheduler_config() {
        let runtime_config = RuntimeConfig::builder()
            .num_workers(8)
            .initial_stack_size(16 * 1024)
            .max_stack_size(2 * 1024 * 1024)
            .work_stealing(false)
            .build()
            .unwrap();

        let scheduler_config = runtime_config.to_scheduler_config();
        assert_eq!(scheduler_config.num_workers, 8);
        assert_eq!(scheduler_config.initial_stack_size, 16 * 1024);
        assert_eq!(scheduler_config.max_stack_size, 2 * 1024 * 1024);
        assert!(!scheduler_config.work_stealing);
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::InvalidValue {
            field: "num_workers".into(),
            message: "must be positive".into(),
        };
        assert!(err.to_string().contains("num_workers"));
        assert!(err.to_string().contains("must be positive"));
    }

    #[test]
    fn test_from_env_with_no_vars() {
        // Clear any existing vars for this test
        env::remove_var("BLOOD_NUM_WORKERS");
        env::remove_var("BLOOD_MAX_HEAP_SIZE");

        let config = RuntimeConfig::from_env();
        assert!(config.scheduler.num_workers >= 1);
        assert_eq!(config.memory.max_heap_size, 0);
    }
}
