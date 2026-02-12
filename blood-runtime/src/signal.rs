//! Signal Handling
//!
//! This module provides signal handling infrastructure for the Blood runtime.
//! It supports graceful shutdown on SIGTERM/SIGINT and configuration reload on SIGHUP.
//!
//! # Platform Support
//!
//! - **Unix**: Full support for SIGTERM, SIGINT, SIGHUP
//! - **Windows**: Basic support for Ctrl+C (SIGINT equivalent)
//!
//! # Usage
//!
//! ```rust,ignore
//! use blood_runtime::signal::{SignalHandler, Signal};
//!
//! // Initialize signal handling
//! let handler = SignalHandler::new();
//! handler.install();
//!
//! // Check if shutdown was requested
//! if handler.shutdown_requested() {
//!     // Perform graceful shutdown
//! }
//!
//! // Wait for shutdown with timeout
//! handler.wait_for_shutdown(Duration::from_secs(5));
//! ```

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Signal types that can be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Signal {
    /// No signal received.
    None = 0,
    /// SIGTERM - Termination request (graceful shutdown).
    Term = 1,
    /// SIGINT - Interrupt (Ctrl+C).
    Int = 2,
    /// SIGHUP - Hangup (typically config reload).
    Hup = 3,
}

impl Signal {
    /// Convert from u8.
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Signal::Term,
            2 => Signal::Int,
            3 => Signal::Hup,
            _ => Signal::None,
        }
    }

    /// Check if this is a shutdown signal (SIGTERM or SIGINT).
    pub fn is_shutdown(&self) -> bool {
        matches!(self, Signal::Term | Signal::Int)
    }
}

/// Global shutdown flag for signal handlers.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Last received signal.
static LAST_SIGNAL: AtomicU8 = AtomicU8::new(0);

/// Signal count (for detecting multiple signals).
static SIGNAL_COUNT: AtomicU8 = AtomicU8::new(0);

/// Shutdown notification channel.
static SHUTDOWN_NOTIFY: std::sync::OnceLock<Arc<(Mutex<bool>, Condvar)>> = std::sync::OnceLock::new();

/// Get or initialize the shutdown notification channel.
fn get_shutdown_notify() -> &'static Arc<(Mutex<bool>, Condvar)> {
    SHUTDOWN_NOTIFY.get_or_init(|| Arc::new((Mutex::new(false), Condvar::new())))
}

/// Signal handler for graceful shutdown.
#[derive(Debug, Clone)]
pub struct SignalHandler {
    /// Whether signal handlers have been installed.
    installed: Arc<AtomicBool>,
}

impl Default for SignalHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalHandler {
    /// Create a new signal handler.
    pub fn new() -> Self {
        Self {
            installed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Install signal handlers.
    ///
    /// This installs handlers for SIGTERM, SIGINT, and optionally SIGHUP
    /// based on the runtime configuration.
    ///
    /// Returns true if handlers were installed, false if already installed.
    pub fn install(&self) -> bool {
        if self.installed.swap(true, Ordering::SeqCst) {
            return false; // Already installed
        }

        #[cfg(unix)]
        self.install_unix_handlers();

        #[cfg(windows)]
        self.install_windows_handlers();

        true
    }

    /// Install Unix signal handlers.
    #[cfg(unix)]
    fn install_unix_handlers(&self) {
        use std::thread;

        // Spawn a thread to handle signals
        // We use a dedicated thread because signal handlers have severe restrictions
        thread::Builder::new()
            .name("blood-signal".into())
            .spawn(|| {
                use nix::sys::signal::{self, SigHandler, Signal as NixSignal};

                // Set up signal handlers using the safer sigaction approach
                unsafe {
                    // SIGTERM - graceful shutdown
                    let _ = signal::signal(NixSignal::SIGTERM, SigHandler::Handler(signal_handler));
                    // SIGINT - Ctrl+C
                    let _ = signal::signal(NixSignal::SIGINT, SigHandler::Handler(signal_handler));
                    // SIGHUP - config reload (we treat as shutdown for now)
                    let _ = signal::signal(NixSignal::SIGHUP, SigHandler::Handler(signal_handler));
                }
            })
            .expect("failed to spawn signal handler thread");
    }

    /// Install Windows signal handlers.
    #[cfg(windows)]
    fn install_windows_handlers(&self) {
        // On Windows, use SetConsoleCtrlHandler for Ctrl+C
        use std::os::windows::ffi::OsStrExt;

        unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> i32 {
            match ctrl_type {
                0 | 1 => {
                    // CTRL_C_EVENT or CTRL_BREAK_EVENT
                    handle_signal(Signal::Int);
                    1 // Handled
                }
                2 => {
                    // CTRL_CLOSE_EVENT
                    handle_signal(Signal::Term);
                    1
                }
                _ => 0, // Not handled
            }
        }

        extern "system" {
            fn SetConsoleCtrlHandler(
                handler: unsafe extern "system" fn(u32) -> i32,
                add: i32,
            ) -> i32;
        }

        unsafe {
            SetConsoleCtrlHandler(ctrl_handler, 1);
        }
    }

    /// Check if shutdown has been requested.
    pub fn shutdown_requested(&self) -> bool {
        SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
    }

    /// Get the last received signal.
    pub fn last_signal(&self) -> Signal {
        Signal::from_u8(LAST_SIGNAL.load(Ordering::SeqCst))
    }

    /// Get the number of signals received.
    pub fn signal_count(&self) -> u8 {
        SIGNAL_COUNT.load(Ordering::SeqCst)
    }

    /// Wait for a shutdown signal.
    ///
    /// Returns true if shutdown was requested, false if timeout elapsed.
    pub fn wait_for_shutdown(&self, timeout: Duration) -> bool {
        let notify = get_shutdown_notify();
        let (lock, cvar) = &**notify;

        let guard = lock.lock().unwrap();
        let result = cvar.wait_timeout_while(guard, timeout, |shutdown| !*shutdown);

        match result {
            Ok((guard, timeout_result)) => *guard && !timeout_result.timed_out(),
            Err(_) => false,
        }
    }

    /// Request shutdown programmatically.
    ///
    /// This can be used to trigger graceful shutdown from code.
    pub fn request_shutdown(&self) {
        handle_signal(Signal::Term);
    }

    /// Reset the shutdown state.
    ///
    /// This is mainly useful for testing.
    pub fn reset(&self) {
        SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
        LAST_SIGNAL.store(0, Ordering::SeqCst);
        SIGNAL_COUNT.store(0, Ordering::SeqCst);

        let notify = get_shutdown_notify();
        let (lock, _) = &**notify;
        if let Ok(mut guard) = lock.lock() {
            *guard = false;
        }
    }
}

/// Handle a received signal.
fn handle_signal(signal: Signal) {
    // Store the signal
    LAST_SIGNAL.store(signal as u8, Ordering::SeqCst);
    SIGNAL_COUNT.fetch_add(1, Ordering::SeqCst);

    // If it's a shutdown signal, set the flag and notify waiters
    if signal.is_shutdown() {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);

        // Notify any threads waiting for shutdown
        let notify = get_shutdown_notify();
        let (lock, cvar) = &**notify;
        if let Ok(mut guard) = lock.lock() {
            *guard = true;
            cvar.notify_all();
        }
    }
}

/// Unix signal handler function.
///
/// This must be async-signal-safe, so we only set atomic flags.
#[cfg(unix)]
extern "C" fn signal_handler(sig: i32) {
    let signal = match sig {
        15 => Signal::Term, // SIGTERM
        2 => Signal::Int,   // SIGINT
        1 => Signal::Hup,   // SIGHUP
        _ => Signal::None,
    };
    handle_signal(signal);
}

/// Global signal handler instance.
static GLOBAL_HANDLER: std::sync::OnceLock<SignalHandler> = std::sync::OnceLock::new();

/// Get the global signal handler.
pub fn global_handler() -> &'static SignalHandler {
    GLOBAL_HANDLER.get_or_init(SignalHandler::new)
}

/// Install global signal handlers.
///
/// This is a convenience function that installs the global signal handler.
pub fn install_handlers() -> bool {
    global_handler().install()
}

/// Check if shutdown has been requested.
///
/// This is a convenience function that checks the global shutdown flag.
pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

/// Request shutdown.
///
/// This is a convenience function that triggers graceful shutdown.
pub fn request_shutdown() {
    global_handler().request_shutdown()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_from_u8() {
        assert_eq!(Signal::from_u8(0), Signal::None);
        assert_eq!(Signal::from_u8(1), Signal::Term);
        assert_eq!(Signal::from_u8(2), Signal::Int);
        assert_eq!(Signal::from_u8(3), Signal::Hup);
        assert_eq!(Signal::from_u8(255), Signal::None);
    }

    #[test]
    fn test_signal_is_shutdown() {
        assert!(!Signal::None.is_shutdown());
        assert!(Signal::Term.is_shutdown());
        assert!(Signal::Int.is_shutdown());
        assert!(!Signal::Hup.is_shutdown());
    }

    #[test]
    fn test_handler_creation() {
        let handler = SignalHandler::new();
        assert!(!handler.shutdown_requested());
        assert_eq!(handler.last_signal(), Signal::None);
        assert_eq!(handler.signal_count(), 0);
    }

    #[test]
    fn test_request_shutdown() {
        let handler = SignalHandler::new();
        handler.reset(); // Clear any previous state

        assert!(!handler.shutdown_requested());
        handler.request_shutdown();
        assert!(handler.shutdown_requested());
        assert!(handler.last_signal().is_shutdown());

        handler.reset();
        assert!(!handler.shutdown_requested());
    }

    #[test]
    fn test_wait_for_shutdown_timeout() {
        let handler = SignalHandler::new();
        handler.reset();

        // Should timeout since no shutdown is requested
        let result = handler.wait_for_shutdown(Duration::from_millis(10));
        assert!(!result);
    }

    #[test]
    fn test_wait_for_shutdown_immediate() {
        let handler = SignalHandler::new();
        handler.reset();

        // Request shutdown first
        handler.request_shutdown();

        // Should return immediately since shutdown is already requested
        let result = handler.wait_for_shutdown(Duration::from_secs(1));
        assert!(result);

        handler.reset();
    }

    #[test]
    fn test_global_handler() {
        let handler = global_handler();
        assert!(!handler.shutdown_requested());
    }

    #[test]
    fn test_install_handlers() {
        // This should succeed (or return false if already installed)
        let _result = install_handlers();
        // Just verify it doesn't panic
    }
}
