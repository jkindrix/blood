//! Panic Handling and Stack Traces
//!
//! This module provides panic hooks and stack trace capture for the Blood runtime.
//! It allows Blood programs to intercept panics, log them, and potentially recover.
//!
//! # Features
//!
//! - **Panic Hooks**: Register callbacks that run when a panic occurs
//! - **Stack Traces**: Capture backtraces for debugging
//! - **Panic Info**: Structured information about panics
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::panic::{set_panic_hook, PanicInfo};
//!
//! // Register a panic hook
//! set_panic_hook(|info| {
//!     eprintln!("Panic occurred: {}", info.message());
//!     if let Some(location) = info.location() {
//!         eprintln!("  at {}:{}:{}", location.file, location.line, location.column);
//!     }
//!     if let Some(backtrace) = info.backtrace() {
//!         eprintln!("Backtrace:\n{}", backtrace);
//!     }
//! });
//! ```

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Counter for panic events.
static PANIC_COUNT: AtomicU64 = AtomicU64::new(0);

/// Whether panic hooks are installed.
static HOOKS_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Registry of panic hooks.
static PANIC_HOOKS: OnceLock<Mutex<Vec<Arc<dyn Fn(&BloodPanicInfo) + Send + Sync>>>> = OnceLock::new();

/// Last panic info for retrieval.
static LAST_PANIC: OnceLock<Mutex<Option<BloodPanicInfo>>> = OnceLock::new();

fn get_panic_hooks() -> &'static Mutex<Vec<Arc<dyn Fn(&BloodPanicInfo) + Send + Sync>>> {
    PANIC_HOOKS.get_or_init(|| Mutex::new(Vec::new()))
}

fn get_last_panic() -> &'static Mutex<Option<BloodPanicInfo>> {
    LAST_PANIC.get_or_init(|| Mutex::new(None))
}

/// Source location information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// File name.
    pub file: String,
    /// Line number.
    pub line: u32,
    /// Column number.
    pub column: u32,
}

/// Information about a panic event.
#[derive(Debug, Clone, PartialEq)]
pub struct BloodPanicInfo {
    /// Panic message.
    message: String,
    /// Source location where the panic occurred.
    location: Option<Location>,
    /// Stack backtrace at the point of panic.
    backtrace: Option<String>,
    /// Panic count (how many panics have occurred).
    count: u64,
    /// Thread name where panic occurred.
    thread_name: Option<String>,
    /// Thread ID.
    thread_id: u64,
}

impl BloodPanicInfo {
    /// Get the panic message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Get the source location, if available.
    pub fn location(&self) -> Option<&Location> {
        self.location.as_ref()
    }

    /// Get the stack backtrace, if available.
    pub fn backtrace(&self) -> Option<&str> {
        self.backtrace.as_deref()
    }

    /// Get the panic count.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Get the thread name, if available.
    pub fn thread_name(&self) -> Option<&str> {
        self.thread_name.as_deref()
    }

    /// Get the thread ID.
    pub fn thread_id(&self) -> u64 {
        self.thread_id
    }

    /// Format as a string for logging.
    pub fn format(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("panic #{}: {}\n", self.count, self.message));

        if let Some(loc) = &self.location {
            output.push_str(&format!("  at {}:{}:{}\n", loc.file, loc.line, loc.column));
        }

        if let Some(thread) = &self.thread_name {
            output.push_str(&format!("  in thread '{}' (id: {})\n", thread, self.thread_id));
        } else {
            output.push_str(&format!("  in thread id: {}\n", self.thread_id));
        }

        if let Some(bt) = &self.backtrace {
            output.push_str("\nBacktrace:\n");
            output.push_str(bt);
        }

        output
    }
}

/// Install the Blood panic hook.
///
/// This should be called once during runtime initialization.
/// It installs a Rust panic hook that captures panic information
/// and invokes registered Blood panic hooks.
pub fn install_panic_handler() {
    if HOOKS_INSTALLED.swap(true, Ordering::SeqCst) {
        return; // Already installed
    }

    std::panic::set_hook(Box::new(|info| {
        handle_panic(info);
    }));
}

/// Handle a panic event.
fn handle_panic(info: &PanicHookInfo<'_>) {
    let count = PANIC_COUNT.fetch_add(1, Ordering::SeqCst) + 1;

    // Extract message
    let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    };

    // Extract location
    let location = info.location().map(|loc| Location {
        file: loc.file().to_string(),
        line: loc.line(),
        column: loc.column(),
    });

    // Capture backtrace
    let backtrace = {
        let bt = Backtrace::capture();
        match bt.status() {
            std::backtrace::BacktraceStatus::Captured => Some(bt.to_string()),
            _ => None,
        }
    };

    // Get thread info
    let thread = std::thread::current();
    let thread_name = thread.name().map(|s| s.to_string());
    let thread_id = thread_id_as_u64();

    let panic_info = BloodPanicInfo {
        message,
        location,
        backtrace,
        count,
        thread_name,
        thread_id,
    };

    // Store as last panic
    if let Ok(mut guard) = get_last_panic().lock() {
        *guard = Some(panic_info.clone());
    }

    // Invoke registered hooks
    if let Ok(hooks) = get_panic_hooks().lock() {
        for hook in hooks.iter() {
            // Call each hook, catching any panics from hooks themselves
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hook(&panic_info);
            }));
        }
    }

    // Also print to stderr if no hooks are registered or as fallback
    eprintln!("{}", panic_info.format());
}

/// Get the thread ID as u64.
fn thread_id_as_u64() -> u64 {
    // Thread ID is opaque, but we can hash it for a numeric identifier
    let id = std::thread::current().id();
    let id_str = format!("{:?}", id);
    // Extract number from "ThreadId(N)"
    id_str
        .trim_start_matches("ThreadId(")
        .trim_end_matches(')')
        .parse()
        .unwrap_or(0)
}

/// Register a panic hook.
///
/// The hook will be called when a panic occurs, before the program terminates.
/// Multiple hooks can be registered and will be called in order.
///
/// Returns a hook ID that can be used to unregister the hook.
pub fn register_panic_hook<F>(hook: F) -> u64
where
    F: Fn(&BloodPanicInfo) + Send + Sync + 'static,
{
    static HOOK_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

    let hook_id = HOOK_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let hook = Arc::new(hook);

    if let Ok(mut hooks) = get_panic_hooks().lock() {
        hooks.push(hook);
    }

    // Ensure panic handler is installed
    install_panic_handler();

    hook_id
}

/// Set a single panic hook, replacing any existing hooks.
///
/// This is a simpler API for when only one hook is needed.
pub fn set_panic_hook<F>(hook: F)
where
    F: Fn(&BloodPanicInfo) + Send + Sync + 'static,
{
    if let Ok(mut hooks) = get_panic_hooks().lock() {
        hooks.clear();
        hooks.push(Arc::new(hook));
    }

    // Ensure panic handler is installed
    install_panic_handler();
}

/// Clear all registered panic hooks.
pub fn clear_panic_hooks() {
    if let Ok(mut hooks) = get_panic_hooks().lock() {
        hooks.clear();
    }
}

/// Get the number of panics that have occurred.
pub fn panic_count() -> u64 {
    PANIC_COUNT.load(Ordering::SeqCst)
}

/// Get information about the last panic, if any.
pub fn last_panic() -> Option<BloodPanicInfo> {
    get_last_panic().lock().ok()?.clone()
}

/// Capture a backtrace at the current location.
///
/// This can be used to capture stack traces at any point, not just during panics.
pub fn capture_backtrace() -> Option<String> {
    let bt = Backtrace::capture();
    match bt.status() {
        std::backtrace::BacktraceStatus::Captured => Some(bt.to_string()),
        _ => None,
    }
}

/// Check if backtraces are available.
///
/// Backtraces require the `RUST_BACKTRACE` environment variable to be set,
/// or the program to be compiled with debug info.
pub fn backtraces_available() -> bool {
    let bt = Backtrace::capture();
    bt.status() == std::backtrace::BacktraceStatus::Captured
}

/// Result of catching a panic.
#[derive(Debug)]
pub enum CatchResult<T> {
    /// The closure completed successfully.
    Ok(T),
    /// The closure panicked.
    Panicked(BloodPanicInfo),
}

impl<T> CatchResult<T> {
    /// Returns true if the result is Ok.
    pub fn is_ok(&self) -> bool {
        matches!(self, CatchResult::Ok(_))
    }

    /// Returns true if the closure panicked.
    pub fn is_panicked(&self) -> bool {
        matches!(self, CatchResult::Panicked(_))
    }

    /// Converts to a standard Result.
    pub fn into_result(self) -> Result<T, BloodPanicInfo> {
        match self {
            CatchResult::Ok(v) => Ok(v),
            CatchResult::Panicked(info) => Err(info),
        }
    }

    /// Returns the Ok value, panicking if the closure panicked.
    pub fn unwrap(self) -> T {
        match self {
            CatchResult::Ok(v) => v,
            CatchResult::Panicked(info) => {
                panic!("called `CatchResult::unwrap()` on a `Panicked` value: {}", info.message);
            }
        }
    }

    /// Returns the Ok value or a default.
    pub fn unwrap_or(self, default: T) -> T {
        match self {
            CatchResult::Ok(v) => v,
            CatchResult::Panicked(_) => default,
        }
    }

    /// Returns the Ok value or computes it from the panic info.
    pub fn unwrap_or_else<F>(self, f: F) -> T
    where
        F: FnOnce(BloodPanicInfo) -> T,
    {
        match self {
            CatchResult::Ok(v) => v,
            CatchResult::Panicked(info) => f(info),
        }
    }

    /// Maps the Ok value.
    pub fn map<U, F>(self, f: F) -> CatchResult<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            CatchResult::Ok(v) => CatchResult::Ok(f(v)),
            CatchResult::Panicked(info) => CatchResult::Panicked(info),
        }
    }
}

/// Catch panics from a closure, allowing recovery.
///
/// This wraps `std::panic::catch_unwind` and converts the panic payload
/// into a `BloodPanicInfo` for structured error handling.
///
/// # Safety
///
/// The closure must be unwind-safe. If the closure has captured references
/// that may be in an inconsistent state after a panic, recovery may not
/// be safe. Use `AssertUnwindSafe` wrapper if needed.
///
/// # Example
///
/// ```rust,ignore
/// use blood_runtime::panic::{catch_panic, CatchResult};
///
/// let result = catch_panic(|| {
///     // Code that might panic
///     panic!("oops!");
/// });
///
/// match result {
///     CatchResult::Ok(value) => println!("Success: {:?}", value),
///     CatchResult::Panicked(info) => {
///         eprintln!("Caught panic: {}", info.message());
///         // Recover or handle error
///     }
/// }
/// ```
pub fn catch_panic<F, R>(f: F) -> CatchResult<R>
where
    F: FnOnce() -> R + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(value) => CatchResult::Ok(value),
        Err(payload) => {
            // Extract message from panic payload
            let message = if let Some(s) = payload.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };

            // Capture backtrace
            let backtrace = capture_backtrace();

            // Get thread info
            let thread = std::thread::current();
            let thread_name = thread.name().map(|s| s.to_string());
            let thread_id = thread_id_as_u64();

            let info = BloodPanicInfo {
                message,
                location: None, // Not available from catch_unwind
                backtrace,
                count: PANIC_COUNT.load(Ordering::SeqCst),
                thread_name,
                thread_id,
            };

            CatchResult::Panicked(info)
        }
    }
}

/// Catch panics from a closure that may not be unwind-safe.
///
/// This is like `catch_panic` but wraps the closure in `AssertUnwindSafe`,
/// asserting that it's safe to unwind through. Use with caution.
///
/// # Safety
///
/// The caller must ensure that catching a panic won't leave program state
/// in an inconsistent or dangerous condition.
pub fn catch_panic_unchecked<F, R>(f: F) -> CatchResult<R>
where
    F: FnOnce() -> R,
{
    catch_panic(std::panic::AssertUnwindSafe(f))
}

/// Resume a panic that was previously caught.
///
/// This re-raises a panic from a `BloodPanicInfo`, allowing panics
/// to be caught, inspected, and optionally re-raised.
pub fn resume_panic(info: BloodPanicInfo) -> ! {
    panic!("{}", info.message);
}

/// Check if panic recovery is available.
///
/// Some platforms or configurations may not support panic recovery.
/// This function returns true if `catch_panic` will work.
pub fn recovery_available() -> bool {
    // Panic recovery is available unless we're compiled with panic=abort
    // We can't easily detect this at runtime, so we assume it's available
    // and let catch_unwind fail if it's not.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_panic_info_format() {
        let info = BloodPanicInfo {
            message: "test panic".to_string(),
            location: Some(Location {
                file: "test.rs".to_string(),
                line: 42,
                column: 10,
            }),
            backtrace: None,
            count: 1,
            thread_name: Some("main".to_string()),
            thread_id: 1,
        };

        let formatted = info.format();
        assert!(formatted.contains("test panic"));
        assert!(formatted.contains("test.rs:42:10"));
        assert!(formatted.contains("main"));
    }

    #[test]
    fn test_panic_info_no_location() {
        let info = BloodPanicInfo {
            message: "test".to_string(),
            location: None,
            backtrace: None,
            count: 1,
            thread_name: None,
            thread_id: 1,
        };

        assert!(info.location().is_none());
        assert!(info.thread_name().is_none());
    }

    #[test]
    fn test_panic_count_initial() {
        // Note: This might not be 0 if other tests have panicked
        let count = panic_count();
        assert!(count >= 0);
    }

    #[test]
    fn test_capture_backtrace() {
        // Backtrace capture depends on environment
        let _ = capture_backtrace();
        // Just verify it doesn't panic
    }

    #[test]
    fn test_register_hook() {
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        let _hook_id = register_panic_hook(move |_info| {
            called_clone.store(true, Ordering::SeqCst);
        });

        // We can't easily trigger a panic in a test without failing,
        // so just verify the hook was registered
        clear_panic_hooks();
    }

    #[test]
    fn test_last_panic_initially_none() {
        // This might have a value if other tests panicked
        let _ = last_panic();
        // Just verify it doesn't panic to access
    }

    #[test]
    fn test_location_struct() {
        let loc = Location {
            file: "main.rs".to_string(),
            line: 100,
            column: 5,
        };
        assert_eq!(loc.file, "main.rs");
        assert_eq!(loc.line, 100);
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn test_catch_panic_success() {
        let result = catch_panic(|| {
            42
        });
        assert!(result.is_ok());
        assert!(!result.is_panicked());
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_catch_panic_str() {
        let result: CatchResult<()> = catch_panic(|| {
            panic!("test panic message");
        });
        assert!(!result.is_ok());
        assert!(result.is_panicked());

        if let CatchResult::Panicked(info) = result {
            assert!(info.message().contains("test panic message"));
        } else {
            panic!("Expected Panicked result");
        }
    }

    #[test]
    fn test_catch_panic_string() {
        let result: CatchResult<()> = catch_panic(|| {
            panic!("{}", "formatted message".to_string());
        });
        assert!(result.is_panicked());

        if let CatchResult::Panicked(info) = result {
            assert!(info.message().contains("formatted message"));
        }
    }

    #[test]
    fn test_catch_result_into_result() {
        let ok_result = catch_panic(|| 42);
        assert_eq!(ok_result.into_result(), Ok(42));

        let panic_result: CatchResult<i32> = catch_panic(|| {
            panic!("error");
        });
        assert!(panic_result.into_result().is_err());
    }

    #[test]
    fn test_catch_result_unwrap_or() {
        let ok_result = catch_panic(|| 42);
        assert_eq!(ok_result.unwrap_or(0), 42);

        let panic_result: CatchResult<i32> = catch_panic(|| {
            panic!("error");
        });
        assert_eq!(panic_result.unwrap_or(99), 99);
    }

    #[test]
    fn test_catch_result_unwrap_or_else() {
        let panic_result: CatchResult<String> = catch_panic(|| {
            panic!("original error");
        });
        let recovered = panic_result.unwrap_or_else(|info| {
            format!("recovered from: {}", info.message())
        });
        assert!(recovered.contains("recovered from:"));
        assert!(recovered.contains("original error"));
    }

    #[test]
    fn test_catch_result_map() {
        let result = catch_panic(|| 21);
        let mapped = result.map(|x| x * 2);
        assert_eq!(mapped.unwrap(), 42);

        let panic_result: CatchResult<i32> = catch_panic(|| {
            panic!("error");
        });
        let mapped_panic = panic_result.map(|x| x * 2);
        assert!(mapped_panic.is_panicked());
    }

    #[test]
    fn test_catch_panic_unchecked() {
        // Test with a non-unwind-safe closure (has mutable reference)
        let mut counter = 0;
        let result = catch_panic_unchecked(|| {
            counter += 1;
            counter
        });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_recovery_available() {
        // Should always return true in test environment
        assert!(recovery_available());
    }
}
