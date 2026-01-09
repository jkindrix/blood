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
}

impl ErrorCode {
    /// Get the formatted error code string (e.g., "E0001").
    pub fn as_str(&self) -> String {
        format!("E{:04}", *self as u16)
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
            _ => {}
        }

        diagnostic
    }
}
