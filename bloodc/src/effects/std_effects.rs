//! # Standard Effects Library
//!
//! This module implements Blood's standard effect library, providing
//! commonly used effects following established patterns from Koka and
//! other effect-typed languages.
//!
//! ## Design References
//!
//! - [Koka std/core/exn](https://koka-lang.github.io/koka/doc/std_core_exn.html)
//! - [Koka std/core types](https://koka-lang.github.io/koka/doc/std_core_types.html)
//! - [Effect Handlers, Evidently](https://dl.acm.org/doi/10.1145/3408981) (ICFP 2020)
//!
//! ## Standard Effects
//!
//! | Effect | Operations | Tail-Resumptive |
//! |--------|------------|-----------------|
//! | State  | get, put, modify | Yes |
//! | Error  | throw, try/catch | No (final ctl) |
//! | IO     | print, read, write | Yes |
//!
//! ## Implementation Notes
//!
//! All standard effects are defined as built-in effects with well-known
//! DefIds. This allows the compiler to recognize and optimize them.

use crate::hir::{DefId, Type};

// ============================================================================
// Well-Known Effect IDs
// ============================================================================

/// Well-known DefId for the State effect.
pub const STATE_EFFECT_ID: u32 = 0x1000;

/// Well-known DefId for the Error effect.
pub const ERROR_EFFECT_ID: u32 = 0x1001;

/// Well-known DefId for the IO effect.
pub const IO_EFFECT_ID: u32 = 0x1002;

/// Well-known DefId for the Console effect.
pub const CONSOLE_EFFECT_ID: u32 = 0x1003;

/// Well-known DefId for the StaleReference effect.
/// This effect is raised when a generational pointer validation fails.
pub const STALE_REFERENCE_EFFECT_ID: u32 = 0x1004;

// ============================================================================
// State Effect (WI-018)
// ============================================================================

/// State effect definition.
///
/// The State effect provides mutable state operations following the
/// pattern from Koka's `st<h>` effect.
///
/// ```text
/// effect State<S> {
///     fn get() -> S
///     fn put(s: S) -> ()
///     fn modify(f: fn(S) -> S) -> ()
/// }
/// ```
///
/// All State operations are tail-resumptive, meaning they can be
/// compiled without continuation capture.
///
/// Reference: [Koka State](https://koka-lang.github.io/koka/doc/std_core_types.html)
/// "The effects `alloc⟨h⟩`, `read⟨h⟩` and `write⟨h⟩` are used for stateful functions"
#[derive(Debug, Clone)]
pub struct StateEffect {
    /// The state type parameter.
    pub state_type: Type,
}

impl StateEffect {
    /// Create a new State effect with the given state type.
    pub fn new(state_type: Type) -> Self {
        Self { state_type }
    }

    /// Get the DefId for this effect.
    pub fn def_id() -> DefId {
        DefId::new(STATE_EFFECT_ID)
    }

    /// Get operation index for `get`.
    pub const GET_OP: u32 = 0;

    /// Get operation index for `put`.
    pub const PUT_OP: u32 = 1;

    /// Get operation index for `modify`.
    pub const MODIFY_OP: u32 = 2;

    /// Check if this effect is tail-resumptive.
    ///
    /// State is always tail-resumptive: all operations resume immediately.
    pub fn is_tail_resumptive() -> bool {
        true
    }
}

/// State effect handler implementation.
///
/// Provides a default handler that maintains state in a local variable.
#[derive(Debug, Clone)]
pub struct StateHandler {
    /// Initial state value.
    pub initial_state: Option<Box<crate::hir::Expr>>,
    /// The state type.
    pub state_type: Type,
}

impl StateHandler {
    /// Create a new State handler.
    pub fn new(state_type: Type) -> Self {
        Self {
            initial_state: None,
            state_type,
        }
    }

    /// Create a handler with an initial state.
    pub fn with_initial(state_type: Type, initial: crate::hir::Expr) -> Self {
        Self {
            initial_state: Some(Box::new(initial)),
            state_type,
        }
    }
}

// ============================================================================
// Error Effect (WI-019)
// ============================================================================

/// Error effect definition.
///
/// The Error effect provides exception-like error handling following
/// the pattern from Koka's `exn` effect.
///
/// ```text
/// effect Error<E> {
///     fn throw(e: E) -> !   // never returns (final ctl)
/// }
/// ```
///
/// The `throw` operation is a `final ctl` - it never resumes.
/// This means Error handlers don't need continuation capture for throw,
/// but they do need to set up try/catch boundaries.
///
/// Reference: [Koka exn](https://koka-lang.github.io/koka/doc/std_core_exn.html)
/// "If a function can raise an exception the effect is `exn`"
#[derive(Debug, Clone)]
pub struct ErrorEffect {
    /// The error type parameter.
    pub error_type: Type,
}

impl ErrorEffect {
    /// Create a new Error effect with the given error type.
    pub fn new(error_type: Type) -> Self {
        Self { error_type }
    }

    /// Get the DefId for this effect.
    pub fn def_id() -> DefId {
        DefId::new(ERROR_EFFECT_ID)
    }

    /// Get operation index for `throw`.
    pub const THROW_OP: u32 = 0;

    /// Check if this effect is tail-resumptive.
    ///
    /// Error is NOT tail-resumptive: throw never resumes (final ctl).
    pub fn is_tail_resumptive() -> bool {
        false
    }
}

/// Exception information categories (following Koka's exception-info).
///
/// Reference: [Koka exception-info](https://koka-lang.github.io/koka/doc/std_core_exn.html)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExceptionInfo {
    /// Generic assertion failure.
    Assert,
    /// General error.
    Error,
    /// Internal system error with name.
    Internal(String),
    /// Pattern matching failure with location.
    Pattern { location: String, definition: String },
    /// Range/boundary violation.
    Range,
    /// System-level error with errno.
    System(i32),
    /// Unimplemented functionality.
    Todo,
}

/// Standard exception type.
#[derive(Debug, Clone)]
pub struct Exception {
    /// Error message.
    pub message: String,
    /// Exception category.
    pub info: ExceptionInfo,
}

impl Exception {
    /// Create a new exception.
    pub fn new(message: String, info: ExceptionInfo) -> Self {
        Self { message, info }
    }

    /// Create a simple error exception.
    pub fn error(message: String) -> Self {
        Self::new(message, ExceptionInfo::Error)
    }

    /// Create an assertion failure.
    pub fn assert(message: String) -> Self {
        Self::new(message, ExceptionInfo::Assert)
    }

    /// Create a pattern match failure.
    pub fn pattern(location: String, definition: String) -> Self {
        Self::new(
            format!("Pattern match failed at {} in {}", location, definition),
            ExceptionInfo::Pattern { location, definition },
        )
    }
}

/// Error effect handler implementation.
///
/// Provides try/catch semantics for error handling.
#[derive(Debug, Clone)]
pub struct ErrorHandler {
    /// The error type being handled.
    pub error_type: Type,
}

impl ErrorHandler {
    /// Create a new Error handler.
    pub fn new(error_type: Type) -> Self {
        Self { error_type }
    }
}

// ============================================================================
// IO Effect (WI-020)
// ============================================================================

/// IO effect definition.
///
/// The IO effect provides input/output operations following the
/// pattern from Koka's `io` effect.
///
/// ```text
/// effect IO {
///     fn print(s: String) -> ()
///     fn println(s: String) -> ()
///     fn read_line() -> String
/// }
/// ```
///
/// IO operations are tail-resumptive in the sense that they always
/// resume after completing the I/O operation.
///
/// Reference: [Koka io](https://koka-lang.github.io/koka/doc/std_core_types.html)
/// "On top of pure effects, we find mutability (as `st`) up to full
/// non-deterministic side effects in `io`."
#[derive(Debug, Clone)]
pub struct IOEffect;

impl IOEffect {
    /// Create a new IO effect.
    pub fn new() -> Self {
        Self
    }

    /// Get the DefId for this effect.
    pub fn def_id() -> DefId {
        DefId::new(IO_EFFECT_ID)
    }

    /// Get operation index for `print`.
    pub const PRINT_OP: u32 = 0;

    /// Get operation index for `println`.
    pub const PRINTLN_OP: u32 = 1;

    /// Get operation index for `read_line`.
    pub const READ_LINE_OP: u32 = 2;

    /// Check if this effect is tail-resumptive.
    ///
    /// IO is tail-resumptive: all operations resume after I/O completes.
    pub fn is_tail_resumptive() -> bool {
        true
    }
}

impl Default for IOEffect {
    fn default() -> Self {
        Self::new()
    }
}

/// Console effect for basic console I/O.
///
/// This is a subset of IO focused on console operations.
#[derive(Debug, Clone)]
pub struct ConsoleEffect;

impl ConsoleEffect {
    /// Create a new Console effect.
    pub fn new() -> Self {
        Self
    }

    /// Get the DefId for this effect.
    pub fn def_id() -> DefId {
        DefId::new(CONSOLE_EFFECT_ID)
    }

    /// Get operation index for `print`.
    pub const PRINT_OP: u32 = 0;

    /// Get operation index for `println`.
    pub const PRINTLN_OP: u32 = 1;

    /// Get operation index for `read_line`.
    pub const READ_LINE_OP: u32 = 2;

    /// Get operation index for `read_char`.
    pub const READ_CHAR_OP: u32 = 3;
}

impl Default for ConsoleEffect {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Effect Registry
// ============================================================================

/// Registry of standard effects.
///
/// This provides a way to look up standard effects by name or DefId.
#[derive(Debug, Default)]
pub struct StandardEffects {
    /// Whether State is available.
    pub has_state: bool,
    /// Whether Error is available.
    pub has_error: bool,
    /// Whether IO is available.
    pub has_io: bool,
}

impl StandardEffects {
    /// Create a new registry with all standard effects.
    pub fn all() -> Self {
        Self {
            has_state: true,
            has_error: true,
            has_io: true,
        }
    }

    /// Create a registry with only pure effects (none).
    pub fn pure() -> Self {
        Self::default()
    }

    /// Check if a DefId is a standard effect.
    pub fn is_standard_effect(def_id: DefId) -> bool {
        let id = def_id.index;
        id == STATE_EFFECT_ID
            || id == ERROR_EFFECT_ID
            || id == IO_EFFECT_ID
            || id == CONSOLE_EFFECT_ID
            || id == STALE_REFERENCE_EFFECT_ID
    }

    /// Get the name of a standard effect by DefId.
    pub fn effect_name(def_id: DefId) -> Option<&'static str> {
        match def_id.index {
            x if x == STATE_EFFECT_ID => Some("State"),
            x if x == ERROR_EFFECT_ID => Some("Error"),
            x if x == IO_EFFECT_ID => Some("IO"),
            x if x == CONSOLE_EFFECT_ID => Some("Console"),
            x if x == STALE_REFERENCE_EFFECT_ID => Some("StaleReference"),
            _ => None,
        }
    }

    /// Check if an effect is tail-resumptive by DefId.
    /// StaleReference is not tail-resumptive because stale_error never returns.
    pub fn is_tail_resumptive(def_id: DefId) -> Option<bool> {
        match def_id.index {
            x if x == STATE_EFFECT_ID => Some(true),
            x if x == ERROR_EFFECT_ID => Some(false),
            x if x == IO_EFFECT_ID => Some(true),
            x if x == CONSOLE_EFFECT_ID => Some(true),
            x if x == STALE_REFERENCE_EFFECT_ID => Some(false),
            _ => None,
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
    fn test_state_effect_def_id() {
        assert_eq!(StateEffect::def_id().index, STATE_EFFECT_ID);
    }

    #[test]
    fn test_state_is_tail_resumptive() {
        assert!(StateEffect::is_tail_resumptive());
    }

    #[test]
    fn test_error_effect_def_id() {
        assert_eq!(ErrorEffect::def_id().index, ERROR_EFFECT_ID);
    }

    #[test]
    fn test_error_not_tail_resumptive() {
        assert!(!ErrorEffect::is_tail_resumptive());
    }

    #[test]
    fn test_io_effect_def_id() {
        assert_eq!(IOEffect::def_id().index, IO_EFFECT_ID);
    }

    #[test]
    fn test_io_is_tail_resumptive() {
        assert!(IOEffect::is_tail_resumptive());
    }

    #[test]
    fn test_standard_effects_all() {
        let effects = StandardEffects::all();
        assert!(effects.has_state);
        assert!(effects.has_error);
        assert!(effects.has_io);
    }

    #[test]
    fn test_standard_effects_pure() {
        let effects = StandardEffects::pure();
        assert!(!effects.has_state);
        assert!(!effects.has_error);
        assert!(!effects.has_io);
    }

    #[test]
    fn test_is_standard_effect() {
        assert!(StandardEffects::is_standard_effect(StateEffect::def_id()));
        assert!(StandardEffects::is_standard_effect(ErrorEffect::def_id()));
        assert!(StandardEffects::is_standard_effect(IOEffect::def_id()));
        assert!(!StandardEffects::is_standard_effect(DefId::new(999)));
    }

    #[test]
    fn test_effect_name() {
        assert_eq!(StandardEffects::effect_name(StateEffect::def_id()), Some("State"));
        assert_eq!(StandardEffects::effect_name(ErrorEffect::def_id()), Some("Error"));
        assert_eq!(StandardEffects::effect_name(IOEffect::def_id()), Some("IO"));
        assert_eq!(StandardEffects::effect_name(DefId::new(999)), None);
    }

    #[test]
    fn test_tail_resumptive_by_def_id() {
        assert_eq!(StandardEffects::is_tail_resumptive(StateEffect::def_id()), Some(true));
        assert_eq!(StandardEffects::is_tail_resumptive(ErrorEffect::def_id()), Some(false));
        assert_eq!(StandardEffects::is_tail_resumptive(IOEffect::def_id()), Some(true));
    }

    #[test]
    fn test_exception_creation() {
        let e = Exception::error("test error".to_string());
        assert_eq!(e.message, "test error");
        assert_eq!(e.info, ExceptionInfo::Error);
    }

    #[test]
    fn test_exception_assert() {
        let e = Exception::assert("assertion failed".to_string());
        assert_eq!(e.info, ExceptionInfo::Assert);
    }

    #[test]
    fn test_exception_pattern() {
        let e = Exception::pattern("line 10".to_string(), "foo".to_string());
        assert!(matches!(e.info, ExceptionInfo::Pattern { .. }));
    }

    #[test]
    fn test_state_operation_indices() {
        assert_eq!(StateEffect::GET_OP, 0);
        assert_eq!(StateEffect::PUT_OP, 1);
        assert_eq!(StateEffect::MODIFY_OP, 2);
    }

    #[test]
    fn test_error_operation_indices() {
        assert_eq!(ErrorEffect::THROW_OP, 0);
    }

    #[test]
    fn test_io_operation_indices() {
        assert_eq!(IOEffect::PRINT_OP, 0);
        assert_eq!(IOEffect::PRINTLN_OP, 1);
        assert_eq!(IOEffect::READ_LINE_OP, 2);
    }
}
