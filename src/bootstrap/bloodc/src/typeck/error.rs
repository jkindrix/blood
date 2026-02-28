//! Type checking errors.

use std::fmt;
use std::path::PathBuf;

use crate::hir::Type;
use crate::span::Span;
use crate::diagnostics::Diagnostic;

/// Result type alias for type checking operations.
///
/// `TypeError` is boxed to reduce stack frame size (184+ bytes → 8 bytes on the Err path).
pub type TypeResult<T> = Result<T, Box<TypeError>>;

/// A type error.
#[derive(Debug, Clone)]
pub struct TypeError {
    /// The kind of error.
    pub kind: TypeErrorKind,
    /// The source span.
    pub span: Span,
    /// Optional help message.
    pub help: Option<String>,
    /// Source file path (for errors in imported modules).
    pub source_file: Option<PathBuf>,
    /// Source content (for errors in imported modules).
    pub source_content: Option<String>,
}

impl TypeError {
    /// Create a new type error.
    pub fn new(kind: TypeErrorKind, span: Span) -> Self {
        Self {
            kind,
            span,
            help: None,
            source_file: None,
            source_content: None,
        }
    }

    /// Wrap this error in a `Box` and return as `Err`.
    pub fn into_err<T>(self) -> TypeResult<T> {
        Err(Box::new(self))
    }

    /// Add a help message.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Set the source file for this error (for errors in imported modules).
    pub fn with_source_file(mut self, path: PathBuf, content: String) -> Self {
        self.source_file = Some(path);
        self.source_content = Some(content);
        self
    }

    /// Convert to a diagnostic.
    pub fn to_diagnostic(&self) -> Diagnostic {
        // Error code categories per DIAGNOSTICS.md spec:
        // - E02xx: Type errors
        // - E03xx: Effect errors
        // - E04xx: Ownership errors
        let (code, message) = match &self.kind {
            // Type errors (E02xx)
            TypeErrorKind::Mismatch { expected, found } => (
                "E0201",
                format!("type mismatch: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::CannotInfer => (
                "E0202",
                "type annotations needed".to_string(),
            ),
            TypeErrorKind::CannotInferWithContext { ty, hint } => (
                "E0202",
                format!("cannot infer type `{ty}`; {hint}"),
            ),
            TypeErrorKind::TypeNotFound { name } => (
                "E0203",
                format!("cannot find type `{name}` in this scope"),
            ),
            TypeErrorKind::GenericArgsMismatch { expected, found } => (
                "E0204",
                format!("wrong number of type arguments: expected {expected}, found {found}"),
            ),
            TypeErrorKind::RecursiveType { name } => (
                "E0205",
                format!("recursive type `{name}` has infinite size"),
            ),
            TypeErrorKind::InfiniteType => (
                "E0205",
                "cannot construct the infinite type".to_string(),
            ),
            TypeErrorKind::TraitBoundNotSatisfied { ty, trait_name } => (
                "E0206",
                format!("the trait bound `{ty}: {trait_name}` is not satisfied"),
            ),
            TypeErrorKind::TraitNotFound { name } => (
                "E0207",
                format!("cannot find trait `{name}` in this scope"),
            ),
            TypeErrorKind::NotFound { name } => (
                "E0208",
                format!("cannot find value `{name}` in this scope"),
            ),
            TypeErrorKind::NotAType { name } => (
                "E0209",
                format!("`{name}` is not a type"),
            ),
            TypeErrorKind::NotAFunction { ty } => (
                "E0210",
                format!("expected function, found `{ty}`"),
            ),
            TypeErrorKind::WrongArity { expected, found } => (
                "E0211",
                format!("this function takes {expected} argument(s) but {found} were supplied"),
            ),
            TypeErrorKind::NotAStruct { ty } => (
                "E0212",
                format!("`{ty}` is not a struct"),
            ),
            TypeErrorKind::NotAStructName { name } => (
                "E0212",
                format!("`{name}` is not a struct type"),
            ),
            TypeErrorKind::NoField { ty, field } => (
                "E0213",
                format!("no field `{field}` on type `{ty}`"),
            ),
            TypeErrorKind::DuplicateDefinition { name } => (
                "E0214",
                format!("the name `{name}` is defined multiple times"),
            ),
            TypeErrorKind::CannotDeref { ty } => (
                "E0215",
                format!("type `{ty}` cannot be dereferenced"),
            ),
            TypeErrorKind::InvalidBinaryOp { op, left, right } => (
                "E0216",
                format!("cannot apply `{op}` to `{left}` and `{right}`"),
            ),
            TypeErrorKind::InvalidUnaryOp { op, ty } => (
                "E0217",
                format!("cannot apply `{op}` to `{ty}`"),
            ),
            TypeErrorKind::NotIndexable { ty } => (
                "E0218",
                format!("cannot index into a value of type `{ty}`"),
            ),
            TypeErrorKind::MainSignature => (
                "E0219",
                "`main` function has wrong signature".to_string(),
            ),
            TypeErrorKind::ReturnTypeMismatch { expected, found } => (
                "E0220",
                format!("return type mismatch: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::NoMainFunction => (
                "E0221",
                "`main` function not found".to_string(),
            ),
            TypeErrorKind::MismatchedTypes { expected, found } => (
                "E0201",
                format!("mismatched types: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::BreakOutsideLoop => (
                "E0222",
                "`break` outside of loop".to_string(),
            ),
            TypeErrorKind::ContinueOutsideLoop => (
                "E0223",
                "`continue` outside of loop".to_string(),
            ),
            TypeErrorKind::ReturnOutsideFunction => (
                "E0224",
                "`return` outside of function".to_string(),
            ),
            TypeErrorKind::MissingField { ty, field } => (
                "E0225",
                format!("missing field `{field}` in initializer of `{ty}`"),
            ),
            TypeErrorKind::PatternMismatch { expected, pattern } => (
                "E0226",
                format!("pattern `{pattern}` not covered by type `{expected}`"),
            ),
            TypeErrorKind::NotATuple { ty } => (
                "E0227",
                format!("`{ty}` is not a tuple"),
            ),
            TypeErrorKind::UnsupportedFeature { feature } => (
                "E0228",
                format!("unsupported feature: {feature}"),
            ),
            TypeErrorKind::UnresolvedName { name } => (
                "E0229",
                format!("cannot find `{name}` in this scope"),
            ),
            TypeErrorKind::UnknownField { ty, field } => (
                "E0230",
                format!("unknown field `{field}` on type `{ty}`"),
            ),
            TypeErrorKind::TypeAnnotationRequired => (
                "E0202",
                "type annotations needed for parameter".to_string(),
            ),

            // Effect errors (E03xx)
            TypeErrorKind::UnhandledEffect { effect } => (
                "E0301",
                format!("unhandled effect `{effect}`"),
            ),
            TypeErrorKind::EffectMismatch { expected, found } => (
                "E0302",
                format!("effect mismatch: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::ResumeTypeMismatch { expected, found } => (
                "E0303",
                format!("resume type mismatch: expected `{expected}`, found `{found}`"),
            ),
            TypeErrorKind::InvalidHandler { reason } => (
                "E0305",
                format!("invalid effect handler: {reason}"),
            ),
            TypeErrorKind::NotAnEffect { name } => (
                "E0306",
                format!("`{name}` is not an effect"),
            ),
            TypeErrorKind::InvalidEffectType { found } => (
                "E0307",
                format!("invalid effect type syntax: expected a named effect like `State<T>`, found {found}"),
            ),
            TypeErrorKind::UndeclaredEffects { effects } => (
                "E0308",
                format!(
                    "function performs undeclared effects: {}",
                    effects.join(", ")
                ),
            ),
            TypeErrorKind::MultipleResumesInShallowHandler { operation, count } => (
                "E0309",
                format!(
                    "shallow handler operation `{operation}` calls resume {count} times, but shallow handlers require single-shot semantics (at most 1 resume)"
                ),
            ),
            TypeErrorKind::LinearValueInMultiShotHandler { operation, field_name, field_type } => (
                "E0304",
                format!(
                    "linear value `{field_name}: {field_type}` captured in multi-shot handler operation `{operation}`"
                ),
            ),
            TypeErrorKind::AffineValueInMultiShotHandler { operation, field_name, field_type } => (
                "E0310",
                format!(
                    "affine value `{field_name}: {field_type}` captured in multi-shot handler operation `{operation}`"
                ),
            ),

            // Ownership errors (E04xx)
            TypeErrorKind::MutableBorrow { reason } => (
                "E0401",
                format!("cannot borrow as mutable: {reason}"),
            ),

            // Method errors
            TypeErrorKind::MethodNotFound { ty, method } => (
                "E0231",
                format!("no method named `{method}` found for type `{ty}` in the current scope"),
            ),

            // Trait impl errors
            TypeErrorKind::MissingTraitMethod { trait_name, method } => (
                "E0232",
                format!("not all trait items implemented, missing: `{method}` from trait `{trait_name}`"),
            ),
            TypeErrorKind::MissingAssocType { trait_name, type_name } => (
                "E0233",
                format!("not all trait items implemented, missing: `type {type_name}` from trait `{trait_name}`"),
            ),
            TypeErrorKind::OverlappingImpls { trait_name, ty_a, ty_b } => (
                "E0234",
                format!("conflicting implementations of trait `{trait_name}` for type `{ty_a}` and `{ty_b}`"),
            ),
            TypeErrorKind::ConstEvalError { reason } => (
                "E0235",
                format!("const evaluation failed: {reason}"),
            ),
            TypeErrorKind::NonExhaustivePatterns { missing } => (
                "E0236",
                format!("non-exhaustive patterns: {} not covered", missing.join(", ")),
            ),
            TypeErrorKind::UnreachablePattern => (
                "E0237",
                "unreachable pattern".to_string(),
            ),
            TypeErrorKind::ImmutableAssign { name } => (
                "E0238",
                format!("cannot assign twice to immutable variable `{name}`"),
            ),
            TypeErrorKind::InvalidCast { source, target } => (
                "E0239",
                format!("casting `{source}` as `{target}` is invalid"),
            ),

            // Linearity errors (E08xx)
            TypeErrorKind::UnusedLinearValue { name, ty } => (
                "E0801",
                format!("linear value `{name}: {ty}` was never consumed. Linear types must be used exactly once."),
            ),
            TypeErrorKind::MultipleLinearUse { name, ty, .. } => (
                "E0802",
                format!("linear value `{name}: {ty}` used more than once. Linear types must be used exactly once."),
            ),
            TypeErrorKind::MultipleAffineUse { name, ty, .. } => (
                "E0803",
                format!("affine value `{name}: {ty}` used more than once. Affine types may be used at most once."),
            ),
            TypeErrorKind::LinearConsumedInLoop { name, ty } => (
                "E0804",
                format!("linear value `{name}: {ty}` consumed inside loop. Linear values defined outside loops cannot be consumed inside."),
            ),
            TypeErrorKind::InconsistentLinearBranches { name, ty, consumed_count, branch_count } => (
                "E0805",
                format!(
                    "linear value `{name}: {ty}` consumed in {consumed_count} of {branch_count} branches. Linear values must be consumed in all branches or none."
                ),
            ),
            TypeErrorKind::LinearCaptureByRef { name, ty } => (
                "E0806",
                format!(
                    "linear value `{name}: {ty}` cannot be captured by reference. Use `move` to capture by value."
                ),
            ),

            // FFI errors (E05xx)
            TypeErrorKind::FfiUnsafeType { ty, reason, context } => (
                "E0501",
                format!("type `{ty}` in {context} is not FFI-safe: {reason}"),
            ),
            TypeErrorKind::FfiPortabilityWarning { ty, reason, context } => (
                "E0502",
                format!("type `{ty}` in {context} may have FFI portability issues: {reason}"),
            ),

            // Multiple dispatch errors (E06xx)
            TypeErrorKind::NoApplicableMethod { name, arg_types } => (
                "E0601",
                format!(
                    "no applicable method `{name}` found for argument types ({})",
                    arg_types.join(", ")
                ),
            ),
            TypeErrorKind::AmbiguousDispatch { name, candidates } => (
                "E0602",
                format!(
                    "ambiguous dispatch for `{name}`: multiple methods match with equal specificity: {}",
                    candidates.join(", ")
                ),
            ),

            // Module errors (E07xx)
            TypeErrorKind::ModuleNotFound { name, searched_paths } => (
                "E0701",
                format!(
                    "cannot find module `{name}`. Searched in:\n  - {}",
                    searched_paths.join("\n  - ")
                ),
            ),
            TypeErrorKind::ImportError { message } => (
                "E0704",
                format!("import error: {message}"),
            ),
            TypeErrorKind::IoError { message } => (
                "E0702",
                message.clone(),
            ),
            TypeErrorKind::ParseError { message } => (
                "E0703",
                format!("parse error in module: {message}"),
            ),

            // Security limits (E07xx)
            TypeErrorKind::TypeDepthExceeded { depth, limit } => (
                "E0701",
                format!("type recursion depth exceeded: depth {depth} exceeds limit {limit}"),
            ),

        };

        let mut diag = Diagnostic::error(message, self.span).with_code(code);

        if let Some(help) = &self.help {
            diag = diag.with_suggestion(help.clone());
        }

        // Pass source file info if this error is from an imported module
        if let (Some(path), Some(content)) = (&self.source_file, &self.source_content) {
            diag = diag.with_source_file(path.clone(), content.clone());
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
    /// Wrong number of generic type arguments.
    GenericArgsMismatch {
        expected: usize,
        found: usize,
    },
    /// Not a struct (when we have a type).
    NotAStruct {
        ty: Type,
    },
    /// Not a struct (when we only have a name, no resolved type).
    NotAStructName {
        name: String,
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
    /// Cannot infer type (with context about what couldn't be inferred).
    CannotInferWithContext {
        ty: String,
        hint: String,
    },
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
    /// Resume type mismatch - value passed to resume doesn't match expected type.
    ResumeTypeMismatch {
        expected: String,
        found: String,
    },
    /// Invalid effect handler.
    InvalidHandler {
        reason: String,
    },
    /// Not an effect.
    NotAnEffect {
        name: String,
    },
    /// Invalid effect type syntax (expected a named effect like `State<T>`).
    InvalidEffectType {
        found: String,
    },
    /// Function performs effects that were not declared in its signature.
    UndeclaredEffects {
        effects: Vec<String>,
    },
    /// Multiple resume calls in a shallow handler operation.
    /// Shallow handlers enforce single-shot continuation semantics.
    MultipleResumesInShallowHandler {
        operation: String,
        count: usize,
    },
    /// Linear value captured in a multi-shot (deep) handler operation.
    /// Linear values must be used exactly once, but multi-shot handlers can
    /// resume multiple times, which would duplicate the linear value.
    LinearValueInMultiShotHandler {
        operation: String,
        field_name: String,
        field_type: String,
    },
    /// Affine value captured in a multi-shot (deep) handler operation.
    /// Affine values may be used at most once, but multi-shot handlers can
    /// resume multiple times, which would duplicate the affine value.
    AffineValueInMultiShotHandler {
        operation: String,
        field_name: String,
        field_type: String,
    },
    /// Trait not found.
    TraitNotFound {
        name: String,
    },
    /// Type annotation required.
    TypeAnnotationRequired,
    /// Trait bound not satisfied.
    TraitBoundNotSatisfied {
        ty: Type,
        trait_name: String,
    },
    /// Method not found on type.
    MethodNotFound {
        ty: Type,
        method: String,
    },
    /// Missing trait method in impl.
    MissingTraitMethod {
        trait_name: String,
        method: String,
    },
    /// Missing associated type in impl.
    MissingAssocType {
        trait_name: String,
        type_name: String,
    },
    /// Overlapping impl blocks.
    OverlappingImpls {
        trait_name: String,
        ty_a: Type,
        ty_b: Type,
    },
    /// Const evaluation error.
    ConstEvalError {
        reason: String,
    },
    /// Non-exhaustive pattern match.
    NonExhaustivePatterns {
        missing: Vec<String>,
    },
    /// Unreachable pattern (dead code).
    UnreachablePattern,
    /// Assignment to immutable variable.
    ImmutableAssign {
        name: String,
    },
    /// Invalid cast expression.
    InvalidCast {
        source: Type,
        target: Type,
    },

    // Linearity errors (E08xx)
    /// Linear value was never consumed.
    /// Linear types must be used exactly once.
    UnusedLinearValue {
        name: String,
        ty: Type,
    },
    /// Linear value was used more than once.
    /// Linear types must be used exactly once.
    MultipleLinearUse {
        name: String,
        ty: Type,
        first_use: Span,
        second_use: Span,
    },
    /// Affine value was used more than once.
    /// Affine types may be used at most once.
    MultipleAffineUse {
        name: String,
        ty: Type,
        first_use: Span,
        second_use: Span,
    },
    /// Linear value defined outside loop was consumed inside loop.
    /// Loops may execute zero or many times, violating linear semantics.
    LinearConsumedInLoop {
        name: String,
        ty: Type,
    },
    /// Linear value consumed in some branches but not all.
    /// Linear values must be consumed consistently across all control flow paths.
    InconsistentLinearBranches {
        name: String,
        ty: Type,
        consumed_count: usize,
        branch_count: usize,
    },
    /// Linear/affine value captured by reference in a closure.
    /// Linear values can only be captured by-move (with `move` keyword).
    LinearCaptureByRef {
        name: String,
        ty: Type,
    },

    // FFI errors (E05xx)
    /// Type is not FFI-safe.
    FfiUnsafeType {
        ty: Type,
        reason: String,
        context: String,
    },
    /// FFI type has portability warning.
    FfiPortabilityWarning {
        ty: Type,
        reason: String,
        context: String,
    },
    /// No applicable method for multiple dispatch call.
    NoApplicableMethod {
        name: String,
        arg_types: Vec<String>,
    },
    /// Ambiguous dispatch - multiple methods match with equal specificity.
    AmbiguousDispatch {
        name: String,
        candidates: Vec<String>,
    },

    // Module errors (E06xx)
    /// External module file not found.
    ModuleNotFound {
        name: String,
        searched_paths: Vec<String>,
    },
    /// Import resolution error.
    ImportError {
        message: String,
    },
    /// I/O error reading module file.
    IoError {
        message: String,
    },
    /// Parse error in module file.
    ParseError {
        message: String,
    },

    // Security limits (E07xx)
    /// Type recursion depth exceeded during unification.
    /// This prevents DoS attacks via pathologically nested types.
    TypeDepthExceeded {
        depth: usize,
        limit: usize,
    },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_diagnostic().message)
    }
}

impl std::error::Error for TypeError {}
