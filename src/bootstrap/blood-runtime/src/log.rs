//! Logging Infrastructure
//!
//! This module provides structured logging for the Blood runtime.
//! It supports log levels, structured key-value pairs, and multiple output formats.
//!
//! # Features
//!
//! - **Log Levels**: Trace, Debug, Info, Warn, Error
//! - **Structured Logging**: Key-value pairs for machine-readable output
//! - **Output Formats**: Plain text and JSON
//! - **Thread-Safe**: Safe for concurrent use from multiple fibers
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::log::{info, warn, error, LogBuilder};
//!
//! // Simple logging
//! info!("Server started");
//! warn!("Connection timeout");
//! error!("Failed to connect");
//!
//! // Structured logging
//! LogBuilder::new(LogLevel::Info)
//!     .message("Request completed")
//!     .field("method", "GET")
//!     .field("path", "/api/users")
//!     .field("status", 200)
//!     .field("duration_ms", 42)
//!     .emit();
//! ```

use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicU8, AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Log level enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum LogLevel {
    /// Trace level (most verbose).
    Trace = 0,
    /// Debug level.
    Debug = 1,
    /// Info level.
    Info = 2,
    /// Warning level.
    Warn = 3,
    /// Error level.
    Error = 4,
    /// Off (no logging).
    Off = 5,
}

impl LogLevel {
    /// Get the level name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Off => "OFF",
        }
    }

    /// Get the level from a u8.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(LogLevel::Trace),
            1 => Some(LogLevel::Debug),
            2 => Some(LogLevel::Info),
            3 => Some(LogLevel::Warn),
            4 => Some(LogLevel::Error),
            5 => Some(LogLevel::Off),
            _ => None,
        }
    }

    /// Parse a log level from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "TRACE" => Some(LogLevel::Trace),
            "DEBUG" => Some(LogLevel::Debug),
            "INFO" => Some(LogLevel::Info),
            "WARN" | "WARNING" => Some(LogLevel::Warn),
            "ERROR" => Some(LogLevel::Error),
            "OFF" | "NONE" => Some(LogLevel::Off),
            _ => None,
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        LogLevel::Info
    }
}

/// Output format for log messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogFormat {
    /// Plain text format (human readable).
    Plain = 0,
    /// JSON format (machine readable).
    Json = 1,
}

impl LogFormat {
    /// Get the format from a u8.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(LogFormat::Plain),
            1 => Some(LogFormat::Json),
            _ => None,
        }
    }

    /// Parse a format from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "plain" | "text" => Some(LogFormat::Plain),
            "json" => Some(LogFormat::Json),
            _ => None,
        }
    }
}

impl Default for LogFormat {
    fn default() -> Self {
        LogFormat::Plain
    }
}

/// A key-value field in a structured log entry.
#[derive(Debug, Clone)]
pub struct LogField {
    /// Field key.
    pub key: String,
    /// Field value.
    pub value: LogValue,
}

/// A value in a structured log entry.
#[derive(Debug, Clone)]
pub enum LogValue {
    /// String value.
    String(String),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
    /// Boolean value.
    Bool(bool),
    /// Null value.
    Null,
}

impl fmt::Display for LogValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogValue::String(s) => write!(f, "{}", s),
            LogValue::Int(i) => write!(f, "{}", i),
            LogValue::Float(v) => write!(f, "{}", v),
            LogValue::Bool(b) => write!(f, "{}", b),
            LogValue::Null => write!(f, "null"),
        }
    }
}

impl LogValue {
    /// Format as JSON value.
    fn to_json(&self) -> String {
        match self {
            LogValue::String(s) => format!("\"{}\"", escape_json(s)),
            LogValue::Int(i) => i.to_string(),
            LogValue::Float(v) => v.to_string(),
            LogValue::Bool(b) => b.to_string(),
            LogValue::Null => "null".to_string(),
        }
    }
}

/// A log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Log level.
    pub level: LogLevel,
    /// Log message.
    pub message: String,
    /// Structured fields.
    pub fields: Vec<LogField>,
    /// Timestamp (Unix milliseconds).
    pub timestamp: u64,
    /// Thread name (if available).
    pub thread_name: Option<String>,
    /// Module/target name.
    pub target: Option<String>,
}

impl LogEntry {
    /// Create a new log entry.
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let thread_name = std::thread::current().name().map(|s| s.to_string());

        Self {
            level,
            message: message.into(),
            fields: Vec::new(),
            timestamp,
            thread_name,
            target: None,
        }
    }

    /// Add a field.
    pub fn with_field(mut self, key: impl Into<String>, value: LogValue) -> Self {
        self.fields.push(LogField {
            key: key.into(),
            value,
        });
        self
    }

    /// Set the target.
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Format as plain text.
    pub fn format_plain(&self) -> String {
        let mut output = String::new();

        // Timestamp
        let secs = self.timestamp / 1000;
        let millis = self.timestamp % 1000;
        output.push_str(&format!("[{}.{:03}] ", secs, millis));

        // Level
        output.push_str(&format!("{:<5} ", self.level.as_str()));

        // Target
        if let Some(target) = &self.target {
            output.push_str(&format!("[{}] ", target));
        }

        // Thread
        if let Some(thread) = &self.thread_name {
            output.push_str(&format!("({}) ", thread));
        }

        // Message
        output.push_str(&self.message);

        // Fields
        if !self.fields.is_empty() {
            output.push_str(" {");
            for (i, field) in self.fields.iter().enumerate() {
                if i > 0 {
                    output.push_str(", ");
                }
                output.push_str(&format!("{}={}", field.key, field.value));
            }
            output.push('}');
        }

        output
    }

    /// Format as JSON.
    pub fn format_json(&self) -> String {
        let mut output = String::from("{");

        // Timestamp
        output.push_str(&format!("\"timestamp\":{}", self.timestamp));

        // Level
        output.push_str(&format!(",\"level\":\"{}\"", self.level.as_str()));

        // Target
        if let Some(target) = &self.target {
            output.push_str(&format!(",\"target\":\"{}\"", escape_json(target)));
        }

        // Thread
        if let Some(thread) = &self.thread_name {
            output.push_str(&format!(",\"thread\":\"{}\"", escape_json(thread)));
        }

        // Message
        output.push_str(&format!(",\"message\":\"{}\"", escape_json(&self.message)));

        // Fields
        if !self.fields.is_empty() {
            output.push_str(",\"fields\":{");
            for (i, field) in self.fields.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                output.push_str(&format!("\"{}\":{}", escape_json(&field.key), field.value.to_json()));
            }
            output.push('}');
        }

        output.push('}');
        output
    }

    /// Format according to the given format.
    pub fn format(&self, format: LogFormat) -> String {
        match format {
            LogFormat::Plain => self.format_plain(),
            LogFormat::Json => self.format_json(),
        }
    }
}

/// Escape a string for JSON output.
fn escape_json(s: &str) -> String {
    let mut output = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c.is_control() => output.push_str(&format!("\\u{:04x}", c as u32)),
            c => output.push(c),
        }
    }
    output
}

/// Global logger state.
static LOGGER: OnceLock<Mutex<LoggerConfig>> = OnceLock::new();

/// Minimum log level (atomic for fast checking).
static MIN_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// Whether logging is enabled.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Logger configuration.
#[derive(Debug)]
struct LoggerConfig {
    /// Output format.
    format: LogFormat,
    /// Whether to write to stderr (vs stdout).
    use_stderr: bool,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::Plain,
            use_stderr: true,
        }
    }
}

fn get_logger() -> &'static Mutex<LoggerConfig> {
    LOGGER.get_or_init(|| Mutex::new(LoggerConfig::default()))
}

/// Initialize the logger with default settings.
pub fn init() {
    let _ = get_logger(); // Initialize if not already
}

/// Initialize the logger with a specific level.
pub fn init_with_level(level: LogLevel) {
    set_level(level);
    init();
}

/// Set the minimum log level.
pub fn set_level(level: LogLevel) {
    MIN_LEVEL.store(level as u8, Ordering::SeqCst);
}

/// Get the current minimum log level.
pub fn level() -> LogLevel {
    LogLevel::from_u8(MIN_LEVEL.load(Ordering::SeqCst)).unwrap_or(LogLevel::Info)
}

/// Set the output format.
pub fn set_format(format: LogFormat) {
    if let Ok(mut config) = get_logger().lock() {
        config.format = format;
    }
}

/// Set whether to use stderr (default) or stdout.
pub fn set_use_stderr(use_stderr: bool) {
    if let Ok(mut config) = get_logger().lock() {
        config.use_stderr = use_stderr;
    }
}

/// Enable or disable logging.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
}

/// Check if logging is enabled.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Check if a log level would be logged.
pub fn would_log(level: LogLevel) -> bool {
    is_enabled() && level >= LogLevel::from_u8(MIN_LEVEL.load(Ordering::SeqCst)).unwrap_or(LogLevel::Info)
}

/// Emit a log entry.
pub fn emit(entry: &LogEntry) {
    if !would_log(entry.level) {
        return;
    }

    let output = {
        let config = match get_logger().lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        entry.format(config.format)
    };

    // Write to output
    let config = match get_logger().lock() {
        Ok(c) => c,
        Err(_) => return,
    };

    if config.use_stderr {
        let _ = writeln!(std::io::stderr(), "{}", output);
    } else {
        let _ = writeln!(std::io::stdout(), "{}", output);
    }
}

/// Builder for log entries.
#[derive(Debug)]
pub struct LogBuilder {
    entry: LogEntry,
}

impl LogBuilder {
    /// Create a new log builder.
    pub fn new(level: LogLevel) -> Self {
        Self {
            entry: LogEntry::new(level, ""),
        }
    }

    /// Set the message.
    pub fn message(mut self, msg: impl Into<String>) -> Self {
        self.entry.message = msg.into();
        self
    }

    /// Set the target.
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.entry.target = Some(target.into());
        self
    }

    /// Add a string field.
    pub fn field_str(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.entry.fields.push(LogField {
            key: key.into(),
            value: LogValue::String(value.into()),
        });
        self
    }

    /// Add an integer field.
    pub fn field_int(mut self, key: impl Into<String>, value: i64) -> Self {
        self.entry.fields.push(LogField {
            key: key.into(),
            value: LogValue::Int(value),
        });
        self
    }

    /// Add a float field.
    pub fn field_float(mut self, key: impl Into<String>, value: f64) -> Self {
        self.entry.fields.push(LogField {
            key: key.into(),
            value: LogValue::Float(value),
        });
        self
    }

    /// Add a boolean field.
    pub fn field_bool(mut self, key: impl Into<String>, value: bool) -> Self {
        self.entry.fields.push(LogField {
            key: key.into(),
            value: LogValue::Bool(value),
        });
        self
    }

    /// Emit the log entry.
    pub fn emit(self) {
        emit(&self.entry);
    }
}

/// Log a message at the given level.
pub fn log(level: LogLevel, message: impl Into<String>) {
    if !would_log(level) {
        return;
    }
    let entry = LogEntry::new(level, message);
    emit(&entry);
}

/// Log a trace message.
pub fn trace(message: impl Into<String>) {
    log(LogLevel::Trace, message);
}

/// Log a debug message.
pub fn debug(message: impl Into<String>) {
    log(LogLevel::Debug, message);
}

/// Log an info message.
pub fn info(message: impl Into<String>) {
    log(LogLevel::Info, message);
}

/// Log a warning message.
pub fn warn(message: impl Into<String>) {
    log(LogLevel::Warn, message);
}

/// Log an error message.
pub fn error(message: impl Into<String>) {
    log(LogLevel::Error, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
        assert!(LogLevel::Error < LogLevel::Off);
    }

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str("TRACE"), Some(LogLevel::Trace));
        assert_eq!(LogLevel::from_str("debug"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("Info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("WARN"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("WARNING"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("OFF"), Some(LogLevel::Off));
        assert_eq!(LogLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_log_level_display() {
        assert_eq!(format!("{}", LogLevel::Info), "INFO");
        assert_eq!(format!("{}", LogLevel::Error), "ERROR");
    }

    #[test]
    fn test_log_format_from_str() {
        assert_eq!(LogFormat::from_str("plain"), Some(LogFormat::Plain));
        assert_eq!(LogFormat::from_str("text"), Some(LogFormat::Plain));
        assert_eq!(LogFormat::from_str("json"), Some(LogFormat::Json));
        assert_eq!(LogFormat::from_str("invalid"), None);
    }

    #[test]
    fn test_log_entry_format_plain() {
        let entry = LogEntry::new(LogLevel::Info, "Test message")
            .with_field("key1", LogValue::String("value1".into()))
            .with_field("key2", LogValue::Int(42));

        let plain = entry.format_plain();
        assert!(plain.contains("INFO"));
        assert!(plain.contains("Test message"));
        assert!(plain.contains("key1=value1"));
        assert!(plain.contains("key2=42"));
    }

    #[test]
    fn test_log_entry_format_json() {
        let entry = LogEntry::new(LogLevel::Error, "Test error")
            .with_field("code", LogValue::Int(500))
            .with_field("retry", LogValue::Bool(true));

        let json = entry.format_json();
        assert!(json.contains("\"level\":\"ERROR\""));
        assert!(json.contains("\"message\":\"Test error\""));
        assert!(json.contains("\"code\":500"));
        assert!(json.contains("\"retry\":true"));
    }

    #[test]
    fn test_escape_json() {
        assert_eq!(escape_json("hello"), "hello");
        assert_eq!(escape_json("hello\"world"), "hello\\\"world");
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_json("path\\to\\file"), "path\\\\to\\\\file");
    }

    #[test]
    fn test_log_value_display() {
        assert_eq!(format!("{}", LogValue::String("hello".into())), "hello");
        assert_eq!(format!("{}", LogValue::Int(42)), "42");
        assert_eq!(format!("{}", LogValue::Float(3.14)), "3.14");
        assert_eq!(format!("{}", LogValue::Bool(true)), "true");
        assert_eq!(format!("{}", LogValue::Null), "null");
    }

    #[test]
    fn test_log_value_json() {
        assert_eq!(LogValue::String("hello".into()).to_json(), "\"hello\"");
        assert_eq!(LogValue::Int(42).to_json(), "42");
        assert_eq!(LogValue::Float(3.14).to_json(), "3.14");
        assert_eq!(LogValue::Bool(false).to_json(), "false");
        assert_eq!(LogValue::Null.to_json(), "null");
    }

    #[test]
    fn test_log_builder() {
        let builder = LogBuilder::new(LogLevel::Info)
            .message("Request completed")
            .target("http")
            .field_str("method", "GET")
            .field_int("status", 200)
            .field_float("duration", 0.042)
            .field_bool("cached", false);

        assert_eq!(builder.entry.level, LogLevel::Info);
        assert_eq!(builder.entry.message, "Request completed");
        assert_eq!(builder.entry.target, Some("http".to_string()));
        assert_eq!(builder.entry.fields.len(), 4);
    }

    #[test]
    fn test_set_level() {
        let original = level();
        set_level(LogLevel::Error);
        assert_eq!(level(), LogLevel::Error);
        set_level(original); // Restore
    }

    #[test]
    fn test_would_log() {
        let original = level();
        set_level(LogLevel::Warn);
        assert!(!would_log(LogLevel::Debug));
        assert!(!would_log(LogLevel::Info));
        assert!(would_log(LogLevel::Warn));
        assert!(would_log(LogLevel::Error));
        set_level(original); // Restore
    }

    #[test]
    fn test_is_enabled() {
        assert!(is_enabled()); // Default is enabled
        set_enabled(false);
        assert!(!is_enabled());
        set_enabled(true);
        assert!(is_enabled());
    }
}
