//! Type checking errors.

use std::fmt;

use crate::hir::Type;
use crate::span::Span;
use crate::diagnostics::Diagnostic;

/// A type error.
#[derive(Debug, Clone)]
pub struct TypeError {
    /// The kind of error.
    pub kind: TypeErrorKind,
    /// The source span.
    pub span: Span,
    /// Optional help message.
    pub help: Option<String>,
}

impl TypeError {
    /// Create a new type error.
    pub fn new(kind: TypeErrorKind, span: Span) -> Self {
        Self {
            kind,
            span,
            help: None,
        }
    }

    /// Add a help message.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Convert to a diagnostic.
    pub fn to_diagnostic(&self) -> Diagnostic {
        let (code, message) = match &self.kind {
            TypeErrorKind::Mismatch { expected, found } => (
                "E1001",
                format!("type mismatch: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::NotFound { name } => (
                "E1002",
                format!("cannot find value `{name}` in this scope"),
            ),
            TypeErrorKind::NotAType { name } => (
                "E1003",
                format!("`{name}` is not a type"),
            ),
            TypeErrorKind::NotAFunction { ty } => (
                "E1004",
                format!("expected function, found `{ty}`"),
            ),
            TypeErrorKind::WrongArity { expected, found } => (
                "E1005",
                format!("this function takes {expected} argument(s) but {found} were supplied"),
            ),
            TypeErrorKind::NotAStruct { ty } => (
                "E1006",
                format!("`{ty}` is not a struct"),
            ),
            TypeErrorKind::NoField { ty, field } => (
                "E1007",
                format!("no field `{field}` on type `{ty}`"),
            ),
            TypeErrorKind::CannotInfer => (
                "E1008",
                "type annotations needed".to_string(),
            ),
            TypeErrorKind::DuplicateDefinition { name } => (
                "E1009",
                format!("the name `{name}` is defined multiple times"),
            ),
            TypeErrorKind::MutableBorrow { reason } => (
                "E1010",
                format!("cannot borrow as mutable: {reason}"),
            ),
            TypeErrorKind::CannotDeref { ty } => (
                "E1011",
                format!("type `{ty}` cannot be dereferenced"),
            ),
            TypeErrorKind::InvalidBinaryOp { op, left, right } => (
                "E1012",
                format!("cannot apply `{op}` to `{left}` and `{right}`"),
            ),
            TypeErrorKind::InvalidUnaryOp { op, ty } => (
                "E1013",
                format!("cannot apply `{op}` to `{ty}`"),
            ),
            TypeErrorKind::NotIndexable { ty } => (
                "E1014",
                format!("cannot index into a value of type `{ty}`"),
            ),
            TypeErrorKind::TypeNotFound { name } => (
                "E1015",
                format!("cannot find type `{name}` in this scope"),
            ),
            TypeErrorKind::InfiniteType => (
                "E1016",
                "cannot construct the infinite type".to_string(),
            ),
            TypeErrorKind::MainSignature => (
                "E1017",
                "`main` function has wrong signature".to_string(),
            ),
            TypeErrorKind::ReturnTypeMismatch { expected, found } => (
                "E1018",
                format!("return type mismatch: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::NoMainFunction => (
                "E1019",
                "`main` function not found".to_string(),
            ),
            TypeErrorKind::RecursiveType { name } => (
                "E1020",
                format!("recursive type `{name}` has infinite size"),
            ),
            TypeErrorKind::MismatchedTypes { expected, found } => (
                "E1021",
                format!("mismatched types: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::BreakOutsideLoop => (
                "E1022",
                "`break` outside of loop".to_string(),
            ),
            TypeErrorKind::ContinueOutsideLoop => (
                "E1023",
                "`continue` outside of loop".to_string(),
            ),
            TypeErrorKind::ReturnOutsideFunction => (
                "E1024",
                "`return` outside of function".to_string(),
            ),
            TypeErrorKind::MissingField { ty, field } => (
                "E1025",
                format!("missing field `{field}` in initializer of `{ty}`"),
            ),
            TypeErrorKind::PatternMismatch { expected, pattern } => (
                "E1026",
                format!("pattern `{pattern}` not covered by type `{expected}`"),
            ),
            TypeErrorKind::NotATuple { ty } => (
                "E1027",
                format!("`{ty}` is not a tuple"),
            ),
            TypeErrorKind::UnsupportedFeature { feature } => (
                "E1028",
                format!("unsupported feature: {feature}"),
            ),
            TypeErrorKind::UnresolvedName { name } => (
                "E1029",
                format!("cannot find `{name}` in this scope"),
            ),
            TypeErrorKind::UnknownField { ty, field } => (
                "E1030",
                format!("unknown field `{field}` on type `{ty}`"),
            ),
            TypeErrorKind::EffectMismatch { expected, found } => (
                "E1031",
                format!("effect mismatch: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::UnhandledEffect { effect } => (
                "E1032",
                format!("unhandled effect `{effect}`"),
            ),
            TypeErrorKind::InvalidHandler { reason } => (
                "E1033",
                format!("invalid effect handler: {reason}"),
            ),
        };

        let mut diag = Diagnostic::error(message, self.span).with_code(code);

        if let Some(help) = &self.help {
            diag = diag.with_suggestion(help.clone());
        }

        diag
    }
}

/// The kind of type error.
#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// Type mismatch.
    Mismatch {
        expected: Type,
        found: Type,
    },
    /// Name not found.
    NotFound {
        name: String,
    },
    /// Type not found.
    TypeNotFound {
        name: String,
    },
    /// Not a type.
    NotAType {
        name: String,
    },
    /// Not a function.
    NotAFunction {
        ty: Type,
    },
    /// Wrong number of arguments.
    WrongArity {
        expected: usize,
        found: usize,
    },
    /// Not a struct.
    NotAStruct {
        ty: Type,
    },
    /// No such field.
    NoField {
        ty: Type,
        field: String,
    },
    /// Missing field in struct initializer.
    MissingField {
        ty: Type,
        field: String,
    },
    /// Cannot infer type.
    CannotInfer,
    /// Duplicate definition.
    DuplicateDefinition {
        name: String,
    },
    /// Cannot borrow as mutable.
    MutableBorrow {
        reason: String,
    },
    /// Cannot dereference.
    CannotDeref {
        ty: Type,
    },
    /// Invalid binary operator.
    InvalidBinaryOp {
        op: String,
        left: Type,
        right: Type,
    },
    /// Invalid unary operator.
    InvalidUnaryOp {
        op: String,
        ty: Type,
    },
    /// Not indexable.
    NotIndexable {
        ty: Type,
    },
    /// Infinite type (occurs check).
    InfiniteType,
    /// Invalid main function signature.
    MainSignature,
    /// Return type mismatch.
    ReturnTypeMismatch {
        expected: Type,
        found: Type,
    },
    /// No main function.
    NoMainFunction,
    /// Recursive type.
    RecursiveType {
        name: String,
    },
    /// Mismatched types (generic).
    MismatchedTypes {
        expected: Type,
        found: Type,
    },
    /// Break outside loop.
    BreakOutsideLoop,
    /// Continue outside loop.
    ContinueOutsideLoop,
    /// Return outside function.
    ReturnOutsideFunction,
    /// Pattern doesn't match type.
    PatternMismatch {
        expected: Type,
        pattern: String,
    },
    /// Not a tuple.
    NotATuple {
        ty: Type,
    },
    /// Unsupported feature.
    UnsupportedFeature {
        feature: String,
    },
    /// Unresolved name.
    UnresolvedName {
        name: String,
    },
    /// Unknown field on struct.
    UnknownField {
        ty: Type,
        field: String,
    },
    /// Effect row mismatch.
    EffectMismatch {
        expected: String,
        found: String,
    },
    /// Unhandled effect.
    UnhandledEffect {
        effect: String,
    },
    /// Invalid effect handler.
    InvalidHandler {
        reason: String,
    },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_diagnostic().message)
    }
}

impl std::error::Error for TypeError {}
