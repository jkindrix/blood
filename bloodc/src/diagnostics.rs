//! Diagnostic reporting infrastructure.
//!
//! This module provides error reporting with source locations,
//! suggestions, and pretty-printed output.
//!
//! # Error Codes
//!
//! Blood compiler error codes are organized by category:
//!
//! - **E0001-E0099**: Lexer errors (unexpected characters, unclosed comments, etc.)
//! - **E0100-E0199**: Syntax/parser errors (unexpected tokens, missing items, etc.)
//! - **E0200-E0299**: Name resolution errors (future)
//! - **E0300-E0399**: Type errors (future)
//! - **E0400-E0499**: Effect/handler errors (future)
//! - **E9000-E9999**: Internal compiler errors (ICE)
//!
//! # Internal Compiler Errors (ICE)
//!
//! ICEs indicate bugs in the compiler itself, not user code errors. Use the
//! [`ice!`] macro to report these with consistent formatting and helpful
//! debugging information.

use crate::span::Span;
use ariadne::{Color, Label, Report, ReportKind, Source};
use thiserror::Error;

/// Compiler error codes.
///
/// Error codes follow this organization:
/// - E0001-E0099: Lexer errors
/// - E0100-E0199: Parser errors
/// - E0200-E0299: Name resolution errors
/// - E0300-E0399: Type errors
/// - E0400-E0499: Effect/handler errors
/// - W0001-W0099: Memory/pointer warnings
/// - W0100-W0199: Performance warnings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorCode {
    // ============================================================
    // Lexer errors (E0001-E0099)
    // ============================================================
    /// Unexpected character in source.
    UnexpectedCharacter = 1,
    /// Unclosed block comment.
    UnclosedBlockComment = 2,
    /// Unclosed string literal.
    UnclosedString = 3,
    /// Invalid escape sequence.
    InvalidEscape = 4,
    /// Invalid integer literal.
    InvalidInteger = 5,
    /// Invalid float literal.
    InvalidFloat = 6,
    /// Unclosed character literal.
    UnclosedChar = 7,
    /// Empty character literal.
    EmptyChar = 8,

    // ============================================================
    // Parser errors (E0100-E0199)
    // ============================================================
    /// Unexpected token.
    UnexpectedToken = 100,
    /// Unexpected end of file.
    UnexpectedEof = 101,
    /// Missing expected item (e.g., missing `;`).
    MissingItem = 102,
    /// Invalid item in context.
    InvalidItem = 103,
    /// Missing closing delimiter.
    UnclosedDelimiter = 104,
    /// Missing type annotation.
    MissingType = 105,
    /// Invalid pattern.
    InvalidPattern = 106,
    /// Invalid expression.
    InvalidExpression = 107,
    /// Expected identifier.
    ExpectedIdentifier = 108,
    /// Expected type.
    ExpectedType = 109,
    /// Expected expression.
    ExpectedExpression = 110,
    /// Expected pattern.
    ExpectedPattern = 111,
    /// Duplicate modifier.
    DuplicateModifier = 112,
    /// Invalid visibility.
    InvalidVisibility = 113,
    /// Missing function body.
    MissingFunctionBody = 114,
    /// Invalid match arm.
    InvalidMatchArm = 115,

    // ============================================================
    // Pointer/Memory Warnings (W0001-W0099)
    // Warning codes use 1000+ to distinguish from errors
    // ============================================================
    /// Deeply nested box types (e.g., Box<Box<Box<T>>>).
    /// This causes multiple generation checks per access.
    DeeplyNestedBox = 1001,

    /// Struct with high pointer density (>75% pointer fields).
    /// Such structures have significant memory overhead with 128-bit pointers.
    PointerHeavyStruct = 1002,

    /// Linked list or self-referential type detected.
    /// These have poor cache behavior with 128-bit pointers.
    LinkedListPattern = 1003,

    /// Array of pointers detected.
    /// Consider using indices into a Vec instead for better cache locality.
    PointerArrayPattern = 1004,

    /// Excessive indirection (>3 dereferences to access data).
    ExcessiveIndirection = 1005,

    // ============================================================
    // Effect/Handler Warnings (W0100-W0199)
    // ============================================================
    /// Deeply nested effect handlers (>5 levels).
    /// Each nesting level adds lookup overhead.
    DeeplyNestedHandlers = 1100,

    /// Handler with many operations may be better split.
    LargeHandlerDefinition = 1101,
}

impl ErrorCode {
    /// Get the formatted error code string (e.g., "E0001" or "W0001").
    pub fn as_str(&self) -> String {
        let code = *self as u16;
        if code >= 1000 {
            // Warnings use W prefix
            format!("W{:04}", code - 1000)
        } else {
            // Errors use E prefix
            format!("E{:04}", code)
        }
    }

    /// Returns true if this is a warning code (not an error).
    pub fn is_warning(&self) -> bool {
        (*self as u16) >= 1000
    }

    /// Get a human-readable description of the error.
    pub fn description(&self) -> &'static str {
        match self {
            // Lexer errors
            ErrorCode::UnexpectedCharacter => "unexpected character in source",
            ErrorCode::UnclosedBlockComment => "unclosed block comment",
            ErrorCode::UnclosedString => "unclosed string literal",
            ErrorCode::InvalidEscape => "invalid escape sequence",
            ErrorCode::InvalidInteger => "invalid integer literal",
            ErrorCode::InvalidFloat => "invalid float literal",
            ErrorCode::UnclosedChar => "unclosed character literal",
            ErrorCode::EmptyChar => "empty character literal",
            // Parser errors
            ErrorCode::UnexpectedToken => "unexpected token",
            ErrorCode::UnexpectedEof => "unexpected end of file",
            ErrorCode::MissingItem => "missing required item",
            ErrorCode::InvalidItem => "invalid item in this context",
            ErrorCode::UnclosedDelimiter => "unclosed delimiter",
            ErrorCode::MissingType => "missing type annotation",
            ErrorCode::InvalidPattern => "invalid pattern",
            ErrorCode::InvalidExpression => "invalid expression",
            ErrorCode::ExpectedIdentifier => "expected identifier",
            ErrorCode::ExpectedType => "expected type",
            ErrorCode::ExpectedExpression => "expected expression",
            ErrorCode::ExpectedPattern => "expected pattern",
            ErrorCode::DuplicateModifier => "duplicate modifier",
            ErrorCode::InvalidVisibility => "invalid visibility specifier",
            ErrorCode::MissingFunctionBody => "missing function body",
            ErrorCode::InvalidMatchArm => "invalid match arm",
            // Pointer/Memory warnings
            ErrorCode::DeeplyNestedBox => "deeply nested box type causes multiple generation checks",
            ErrorCode::PointerHeavyStruct => "struct has high pointer density (>75% pointer fields)",
            ErrorCode::LinkedListPattern => "linked list pattern detected",
            ErrorCode::PointerArrayPattern => "array of pointers detected",
            ErrorCode::ExcessiveIndirection => "excessive pointer indirection (>3 levels)",
            // Effect/Handler warnings
            ErrorCode::DeeplyNestedHandlers => "deeply nested effect handlers add lookup overhead",
            ErrorCode::LargeHandlerDefinition => "handler with many operations may impact readability",
        }
    }

    /// Get a help message suggesting how to fix the error.
    pub fn help(&self) -> Option<&'static str> {
        match self {
            ErrorCode::UnclosedBlockComment => Some("add `*/` to close the block comment"),
            ErrorCode::UnclosedString => Some("add a closing `\"` to complete the string"),
            ErrorCode::UnclosedChar => Some("add a closing `'` to complete the character literal"),
            ErrorCode::EmptyChar => Some("character literals must contain exactly one character"),
            ErrorCode::InvalidEscape => {
                Some("valid escape sequences are: \\n, \\r, \\t, \\\\, \\', \\\", \\0, \\x##, \\u{####}")
            }
            ErrorCode::UnclosedDelimiter => {
                Some("check for matching opening and closing delimiters")
            }
            ErrorCode::MissingType => Some("add a type annotation after the `:`"),
            ErrorCode::MissingFunctionBody => Some("add a function body `{ ... }` or use `;` for a declaration"),
            // Pointer/Memory warning help
            ErrorCode::DeeplyNestedBox => {
                Some("consider using `Box<T>` directly instead of `Box<Box<T>>`; each nesting adds ~4 cycles per access")
            }
            ErrorCode::PointerHeavyStruct => {
                Some("consider storing values directly or using indices into a Vec; 128-bit pointers double memory for pointer-heavy structures")
            }
            ErrorCode::LinkedListPattern => {
                Some("linked lists have poor cache locality with 128-bit pointers; consider using Vec<T> or indices")
            }
            ErrorCode::PointerArrayPattern => {
                Some("arrays of pointers have 2x memory overhead; consider storing values directly or using indices")
            }
            ErrorCode::ExcessiveIndirection => {
                Some("each pointer dereference costs ~4 cycles; flatten the data structure to reduce indirection")
            }
            // Effect/Handler warning help
            ErrorCode::DeeplyNestedHandlers => {
                Some("each nested handler adds ~2.6 cycles lookup overhead; consider combining handlers or reducing nesting depth")
            }
            ErrorCode::LargeHandlerDefinition => {
                Some("handlers with many operations may be harder to maintain; consider splitting into focused handlers")
            }
            _ => None,
        }
    }
}

/// The kind of diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// An error that prevents compilation.
    Error,
    /// A warning that doesn't prevent compilation.
    Warning,
    /// An informational note.
    Note,
    /// A hint for fixing the issue.
    Help,
}

impl DiagnosticKind {
    fn to_report_kind(self) -> ReportKind<'static> {
        match self {
            DiagnosticKind::Error => ReportKind::Error,
            DiagnosticKind::Warning => ReportKind::Warning,
            DiagnosticKind::Note => ReportKind::Advice,
            DiagnosticKind::Help => ReportKind::Advice,
        }
    }

    fn color(self) -> Color {
        match self {
            DiagnosticKind::Error => Color::Red,
            DiagnosticKind::Warning => Color::Yellow,
            DiagnosticKind::Note => Color::Cyan,
            DiagnosticKind::Help => Color::Green,
        }
    }
}

/// A compiler diagnostic.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// The kind of diagnostic.
    pub kind: DiagnosticKind,
    /// The error code (e.g., "E0001").
    pub code: Option<String>,
    /// The main error message.
    pub message: String,
    /// The primary span where the error occurred.
    pub span: Span,
    /// Additional labels pointing to relevant code.
    pub labels: Vec<DiagnosticLabel>,
    /// Suggestions for fixing the error.
    pub suggestions: Vec<String>,
}

impl Diagnostic {
    /// Create a new error diagnostic.
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            kind: DiagnosticKind::Error,
            code: None,
            message: message.into(),
            span,
            labels: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    /// Create a new warning diagnostic.
    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            kind: DiagnosticKind::Warning,
            code: None,
            message: message.into(),
            span,
            labels: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    /// Set the error code from a string.
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Set the error code from an ErrorCode enum.
    /// Automatically adds the help message if available.
    pub fn with_error_code(mut self, code: ErrorCode) -> Self {
        self.code = Some(code.as_str());
        if let Some(help) = code.help() {
            self.suggestions.push(help.to_string());
        }
        self
    }

    /// Create an error diagnostic from an ErrorCode with automatic message and help.
    pub fn from_error_code(code: ErrorCode, span: Span) -> Self {
        let mut diag = Self::error(code.description(), span);
        diag.code = Some(code.as_str());
        if let Some(help) = code.help() {
            diag.suggestions.push(help.to_string());
        }
        diag
    }

    /// Add a note to help explain the error.
    pub fn with_note(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(DiagnosticLabel::secondary(span, message));
        self
    }

    /// Add a primary label with a custom message.
    pub fn with_primary_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(DiagnosticLabel::primary(span, message));
        self
    }

    /// Add a label.
    pub fn with_label(mut self, label: DiagnosticLabel) -> Self {
        self.labels.push(label);
        self
    }

    /// Add a suggestion.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestions.push(suggestion.into());
        self
    }
}

/// A secondary label in a diagnostic.
#[derive(Debug, Clone)]
pub struct DiagnosticLabel {
    /// The span this label points to.
    pub span: Span,
    /// The label message.
    pub message: String,
    /// Whether this is the primary label.
    pub primary: bool,
}

impl DiagnosticLabel {
    pub fn primary(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            primary: true,
        }
    }

    pub fn secondary(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            primary: false,
        }
    }
}

/// Diagnostic emitter that prints diagnostics to stderr.
pub struct DiagnosticEmitter<'a> {
    filename: &'a str,
    source: &'a str,
}

impl<'a> DiagnosticEmitter<'a> {
    pub fn new(filename: &'a str, source: &'a str) -> Self {
        Self { filename, source }
    }

    /// Emit a diagnostic to stderr.
    pub fn emit(&self, diagnostic: &Diagnostic) {
        let mut builder = Report::build(
            diagnostic.kind.to_report_kind(),
            self.filename,
            diagnostic.span.start,
        );

        // Add main message
        let message = if let Some(code) = &diagnostic.code {
            format!("[{}] {}", code, diagnostic.message)
        } else {
            diagnostic.message.clone()
        };
        builder = builder.with_message(&message);

        // Add primary label
        builder = builder.with_label(
            Label::new((self.filename, diagnostic.span.start..diagnostic.span.end))
                .with_color(diagnostic.kind.color())
                .with_message(&diagnostic.message),
        );

        // Add secondary labels
        for label in &diagnostic.labels {
            let color = if label.primary {
                diagnostic.kind.color()
            } else {
                Color::Blue
            };
            builder = builder.with_label(
                Label::new((self.filename, label.span.start..label.span.end))
                    .with_color(color)
                    .with_message(&label.message),
            );
        }

        // Add suggestions
        if !diagnostic.suggestions.is_empty() {
            let help = diagnostic.suggestions.join("\n");
            builder = builder.with_help(help);
        }

        let report = builder.finish();

        // Write to stderr
        report
            .eprint((self.filename, Source::from(self.source)))
            .expect("BUG: failed to write diagnostic to stderr - terminal may be in an invalid state");
    }
}

/// Common parse errors.
#[derive(Debug, Clone, Error)]
pub enum ParseError {
    #[error("unexpected token: expected {expected}, found {found}")]
    UnexpectedToken {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("unexpected end of file")]
    UnexpectedEof { span: Span },

    #[error("invalid integer literal")]
    InvalidInteger { span: Span },

    #[error("invalid float literal")]
    InvalidFloat { span: Span },

    #[error("unclosed string literal")]
    UnclosedString { span: Span },

    #[error("unclosed block comment")]
    UnclosedBlockComment { span: Span },

    #[error("invalid escape sequence")]
    InvalidEscape { span: Span },

    #[error("{message}")]
    Custom { message: String, span: Span },
}

impl ParseError {
    pub fn span(&self) -> Span {
        match self {
            ParseError::UnexpectedToken { span, .. } => *span,
            ParseError::UnexpectedEof { span } => *span,
            ParseError::InvalidInteger { span } => *span,
            ParseError::InvalidFloat { span } => *span,
            ParseError::UnclosedString { span } => *span,
            ParseError::UnclosedBlockComment { span } => *span,
            ParseError::InvalidEscape { span } => *span,
            ParseError::Custom { span, .. } => *span,
        }
    }
}

impl From<ParseError> for Diagnostic {
    fn from(error: ParseError) -> Self {
        let span = error.span();
        let message = error.to_string();

        let mut diagnostic = Diagnostic::error(message, span);

        // Add suggestions based on error type
        match &error {
            ParseError::UnexpectedToken { expected, .. } => {
                diagnostic = diagnostic.with_suggestion(format!("expected {}", expected));
            }
            ParseError::UnclosedString { .. } => {
                diagnostic = diagnostic.with_suggestion("add a closing `\"` to end the string");
            }
            ParseError::UnclosedBlockComment { .. } => {
                diagnostic = diagnostic.with_suggestion("add `*/` to close the block comment");
            }
            // These error types don't have specific suggestions beyond the error message
            ParseError::UnexpectedEof { .. }
            | ParseError::InvalidInteger { .. }
            | ParseError::InvalidFloat { .. }
            | ParseError::InvalidEscape { .. }
            | ParseError::Custom { .. } => {}
        }

        diagnostic
    }
}

// ============================================================================
// Internal Compiler Error (ICE) Infrastructure
// ============================================================================

/// Internal Compiler Error (ICE) - indicates a bug in the compiler.
///
/// ICEs should never be triggered by user code. If one occurs, it means
/// there's a bug in the compiler that needs to be fixed.
#[derive(Debug, Clone)]
pub struct Ice {
    /// A message describing what went wrong.
    pub message: String,
    /// The file where the ICE occurred (compiler source file).
    pub file: &'static str,
    /// The line number where the ICE occurred.
    pub line: u32,
    /// Optional additional context about the error.
    pub context: Vec<String>,
}

impl Ice {
    /// Create a new ICE with file and line information.
    pub fn new(message: impl Into<String>, file: &'static str, line: u32) -> Self {
        Self {
            message: message.into(),
            file,
            line,
            context: Vec::new(),
        }
    }

    /// Add context information to the ICE.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context.push(context.into());
        self
    }

    /// Format the ICE as an error message for display.
    pub fn format(&self) -> String {
        let mut msg = format!(
            "internal compiler error: {}\n\
             --> {}:{}\n\
             \n\
             This is a bug in the Blood compiler. Please report it at:\n\
             https://github.com/blood-lang/blood/issues\n",
            self.message, self.file, self.line
        );

        if !self.context.is_empty() {
            msg.push_str("\nContext:\n");
            for ctx in &self.context {
                msg.push_str(&format!("  - {}\n", ctx));
            }
        }

        msg
    }

    /// Print the ICE to stderr.
    pub fn emit(&self) {
        eprintln!("\n{}", self.format());
    }

    /// Convert to a Diagnostic for integration with the diagnostic system.
    pub fn to_diagnostic(&self, span: Span) -> Diagnostic {
        let mut diag = Diagnostic::error(format!("internal compiler error: {}", self.message), span)
            .with_code("E9000");

        diag = diag.with_suggestion(format!(
            "This is a bug in the Blood compiler ({}:{}). Please report it.",
            self.file, self.line
        ));

        for ctx in &self.context {
            diag = diag.with_suggestion(format!("Context: {}", ctx));
        }

        diag
    }
}

impl std::fmt::Display for Ice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format())
    }
}

impl std::error::Error for Ice {}

/// Report an Internal Compiler Error (ICE) with file and line information.
///
/// This macro should be used when the compiler encounters an internal
/// inconsistency that should never happen with valid code. ICEs indicate
/// bugs in the compiler itself.
///
/// # Examples
///
/// ```ignore
/// // Simple ICE
/// ice!("type variable should have been resolved before codegen");
///
/// // ICE with context
/// ice!("unexpected type kind"; "type" => format!("{:?}", ty));
///
/// // ICE with multiple context items
/// ice!("mismatched field count";
///      "expected" => expected_count,
///      "found" => found_count);
/// ```
#[macro_export]
macro_rules! ice {
    ($msg:expr) => {{
        let ice = $crate::diagnostics::Ice::new($msg, file!(), line!());
        ice.emit();
        ice
    }};
    ($msg:expr; $($key:expr => $value:expr),+ $(,)?) => {{
        let mut ice = $crate::diagnostics::Ice::new($msg, file!(), line!());
        $(
            ice = ice.with_context(format!("{}: {:?}", $key, $value));
        )+
        ice.emit();
        ice
    }};
}

/// Report an ICE and panic.
///
/// Use this when the ICE is unrecoverable and the compiler cannot continue.
#[macro_export]
macro_rules! ice_panic {
    ($msg:expr) => {{
        let ice = $crate::ice!($msg);
        panic!("{}", ice.message);
    }};
    ($msg:expr; $($key:expr => $value:expr),+ $(,)?) => {{
        let ice = $crate::ice!($msg; $($key => $value),+);
        panic!("{}", ice.message);
    }};
}

/// Report an ICE and return an error.
///
/// Use this when the ICE should be converted to a Diagnostic error for
/// integration with the error reporting system.
#[macro_export]
macro_rules! ice_err {
    ($span:expr, $msg:expr) => {{
        let ice = $crate::diagnostics::Ice::new($msg, file!(), line!());
        ice.emit();
        ice.to_diagnostic($span)
    }};
    ($span:expr, $msg:expr; $($key:expr => $value:expr),+ $(,)?) => {{
        let mut ice = $crate::diagnostics::Ice::new($msg, file!(), line!());
        $(
            ice = ice.with_context(format!("{}: {:?}", $key, $value));
        )+
        ice.emit();
        ice.to_diagnostic($span)
    }};
}
