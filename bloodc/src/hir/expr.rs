//! HIR expressions and function bodies.
//!
//! This module defines the typed expression representation used after
//! type checking. All expressions have a resolved type attached.

use std::collections::HashMap;
use std::fmt;

use crate::ast::{BinOp, UnaryOp, LiteralKind};
use crate::span::Span;
use super::{DefId, LocalId, Type};

/// A unique identifier for a function/closure body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyId(pub u32);

impl BodyId {
    /// Create a new BodyId.
    pub const fn new(id: u32) -> Self {
        BodyId(id)
    }
}

/// A function or closure body.
///
/// Bodies contain:
/// - A list of locals (parameters and temporaries)
/// - The body expression
#[derive(Debug, Clone)]
pub struct Body {
    /// The parameters and local variables.
    pub locals: Vec<Local>,
    /// The number of parameters (first N locals are params).
    pub param_count: usize,
    /// The body expression.
    pub expr: Expr,
    /// The span of the body.
    pub span: Span,
    /// Tuple destructuring info: maps the hidden tuple local to the element locals.
    /// Used by MIR lowering to decompose tuple patterns.
    pub tuple_destructures: HashMap<LocalId, Vec<LocalId>>,
}

impl Body {
    /// Get the return type (type of local 0).
    pub fn return_type(&self) -> &Type {
        &self.locals[0].ty
    }

    /// Iterate over parameters.
    pub fn params(&self) -> impl Iterator<Item = &Local> {
        self.locals.iter().skip(1).take(self.param_count)
    }

    /// Get a local by ID.
    ///
    /// Note: LocalIds may not be contiguous (e.g., when closures share the
    /// outer function's local ID space), so we search by ID rather than
    /// using the ID as a direct index.
    pub fn get_local(&self, id: LocalId) -> Option<&Local> {
        self.locals.iter().find(|l| l.id == id)
    }
}

/// A local variable (parameter or temporary).
#[derive(Debug, Clone)]
pub struct Local {
    /// The local ID.
    pub id: LocalId,
    /// The type of this local.
    pub ty: Type,
    /// Whether this local is mutable.
    pub mutable: bool,
    /// The name of this local (for debugging/errors).
    pub name: Option<String>,
    /// The span where this local was declared.
    pub span: Span,
}

/// A typed expression.
#[derive(Debug, Clone)]
pub struct Expr {
    /// The expression kind.
    pub kind: ExprKind,
    /// The type of this expression.
    pub ty: Type,
    /// The source span.
    pub span: Span,
}

impl Expr {
    /// Create a new expression.
    pub fn new(kind: ExprKind, ty: Type, span: Span) -> Self {
        Self { kind, ty, span }
    }

    /// Create an error expression.
    pub fn error(span: Span) -> Self {
        Self {
            kind: ExprKind::Error,
            ty: Type::error(),
            span,
        }
    }

    /// Check if this is an error expression.
    pub fn is_error(&self) -> bool {
        matches!(self.kind, ExprKind::Error)
    }
}

/// The kind of an expression.
#[derive(Debug, Clone)]
pub enum ExprKind {
    /// A literal value.
    Literal(LiteralValue),

    /// A local variable reference.
    Local(LocalId),

    /// A reference to a definition (function, constant, etc.).
    Def(DefId),

    /// Binary operation: `a + b`
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// Unary operation: `-x`, `!x`, `*x`, `&x`
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },

    /// Function call: `f(x, y)`
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },

    /// Method call: `x.method(y)`
    /// Note: In HIR, this is resolved to a function call with receiver.
    MethodCall {
        receiver: Box<Expr>,
        method: DefId,
        args: Vec<Expr>,
    },

    /// Field access: `x.field`
    Field {
        base: Box<Expr>,
        field_idx: u32,
    },

    /// Array/slice indexing: `x[i]`
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },

    /// Tuple expression: `(a, b, c)`
    Tuple(Vec<Expr>),

    /// Array expression: `[a, b, c]`
    Array(Vec<Expr>),

    /// Array repeat: `[x; N]`
    Repeat {
        value: Box<Expr>,
        count: u64,
    },

    /// Struct construction: `Point { x: 1, y: 2 }`
    Struct {
        def_id: DefId,
        fields: Vec<FieldExpr>,
        base: Option<Box<Expr>>,
    },

    /// Anonymous record construction: `{ x: 1, y: 2 }`
    Record {
        fields: Vec<RecordFieldExpr>,
    },

    /// Enum variant construction: `Some(x)`
    Variant {
        def_id: DefId,
        variant_idx: u32,
        fields: Vec<Expr>,
    },

    /// Type cast: `x as T`
    Cast {
        expr: Box<Expr>,
        target_ty: Type,
    },

    /// Assignment: `x = y`
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },

    /// Block: `{ stmts; expr }`
    Block {
        stmts: Vec<Stmt>,
        expr: Option<Box<Expr>>,
    },

    /// If expression: `if cond { then } else { else }`
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },

    /// Match expression: `match x { patterns }`
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    /// Loop: `loop { body }`
    Loop {
        body: Box<Expr>,
        /// Label for break/continue.
        label: Option<LoopId>,
    },

    /// While loop: `while cond { body }`
    /// Desugared to `loop { if !cond { break }; body }`
    While {
        condition: Box<Expr>,
        body: Box<Expr>,
        label: Option<LoopId>,
    },

    /// Return: `return x`
    Return(Option<Box<Expr>>),

    /// Break: `break x`
    Break {
        label: Option<LoopId>,
        value: Option<Box<Expr>>,
    },

    /// Continue: `continue`
    Continue {
        label: Option<LoopId>,
    },

    /// Closure: `|x| x + 1`
    Closure {
        body_id: BodyId,
        captures: Vec<Capture>,
    },

    /// Borrow: `&x` or `&mut x`
    Borrow {
        expr: Box<Expr>,
        mutable: bool,
    },

    /// Dereference: `*x`
    Deref(Box<Expr>),

    /// Address of: for raw pointers
    AddrOf {
        expr: Box<Expr>,
        mutable: bool,
    },

    /// Let binding within an expression (let-else)
    Let {
        pattern: Pattern,
        init: Box<Expr>,
    },

    /// Unsafe block: `unsafe { ... }`
    Unsafe(Box<Expr>),

    /// Effect operation: `perform Effect.op(args)`
    ///
    /// After evidence translation, this becomes a call through the evidence vector.
    /// See: [Generalized Evidence Passing](https://dl.acm.org/doi/10.1145/3473576)
    Perform {
        /// The effect being performed.
        effect_id: DefId,
        /// The operation index within the effect.
        op_index: u32,
        /// Arguments to the operation.
        args: Vec<Expr>,
    },

    /// Resume continuation in a handler: `resume(value)`
    ///
    /// Resumes the suspended computation with the given value.
    /// For tail-resumptive handlers, this compiles to a direct return.
    Resume {
        /// The value to resume with.
        value: Option<Box<Expr>>,
    },

    /// Handle expression: `handle { body } with { handlers }`
    ///
    /// Runs the body with the given effect handlers installed.
    Handle {
        /// The body expression to run.
        body: Box<Expr>,
        /// The handler definition.
        handler_id: DefId,
        /// The handler instance expression (initializes handler state).
        handler_instance: Box<Expr>,
    },

    /// Inline handle expression: `try { body } with { Effect::op(x) => { ... } }`
    ///
    /// Runs the body with inline effect handlers. Unlike `Handle`, the handlers
    /// are defined inline rather than referencing a pre-declared handler.
    InlineHandle {
        /// The body expression to run.
        body: Box<Expr>,
        /// The inline operation handlers.
        handlers: Vec<InlineOpHandler>,
    },

    /// Range expression: `start..end` or `start..=end`
    ///
    /// Creates a Range or RangeInclusive value for iteration or slicing.
    Range {
        /// Start of range (None for `..end` ranges).
        start: Option<Box<Expr>>,
        /// End of range (None for `start..` ranges).
        end: Option<Box<Expr>>,
        /// Whether this is an inclusive range (`..=`).
        inclusive: bool,
    },

    /// A method family for multiple dispatch resolution.
    ///
    /// This represents a call site where multiple function overloads exist.
    /// The actual method to call is determined by matching argument types
    /// against the candidates' parameter types.
    MethodFamily {
        /// The method family name.
        name: String,
        /// Candidate function DefIds.
        candidates: Vec<DefId>,
    },

    /// Default value: `default`
    ///
    /// Creates a default instance of the inferred type.
    /// The type must implement the Default trait.
    Default,

    // =========================================================================
    // Macro Expansion Results
    // =========================================================================

    /// Expanded format macro (format!, println!, print!, etc.)
    MacroExpansion {
        macro_name: String,
        format_str: String,
        args: Vec<Expr>,
    },

    /// Vec literal: `vec![1, 2, 3]`
    VecLiteral(Vec<Expr>),

    /// Vec repeat: `vec![0; 10]`
    VecRepeat {
        value: Box<Expr>,
        count: Box<Expr>,
    },

    /// Assert expression: `assert!(cond)` or `assert!(cond, msg)`
    Assert {
        condition: Box<Expr>,
        message: Option<Box<Expr>>,
    },

    /// Debug expression: `dbg!(expr)`
    Dbg(Box<Expr>),

    /// Error placeholder (for error recovery).
    Error,
}

/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    /// Integer literal.
    Int(i128),
    /// Unsigned integer literal.
    Uint(u128),
    /// Floating-point literal.
    Float(f64),
    /// Boolean literal.
    Bool(bool),
    /// Character literal.
    Char(char),
    /// Byte character literal (b'a').
    Byte(u8),
    /// String literal.
    String(String),
    /// Byte string literal.
    ByteString(Vec<u8>),
}

impl LiteralValue {
    /// Create from AST literal kind.
    pub fn from_ast(kind: &LiteralKind) -> Self {
        match kind {
            LiteralKind::Int { value, suffix } => {
                // For now, treat all as signed unless suffix says otherwise
                if matches!(
                    suffix,
                    Some(crate::ast::IntSuffix::U8)
                        | Some(crate::ast::IntSuffix::U16)
                        | Some(crate::ast::IntSuffix::U32)
                        | Some(crate::ast::IntSuffix::U64)
                        | Some(crate::ast::IntSuffix::U128)
                        | Some(crate::ast::IntSuffix::Usize)
                ) {
                    LiteralValue::Uint(*value)
                } else {
                    LiteralValue::Int(*value as i128)
                }
            }
            LiteralKind::Float { value, .. } => LiteralValue::Float(value.0),
            LiteralKind::Bool(b) => LiteralValue::Bool(*b),
            LiteralKind::Char(c) => LiteralValue::Char(*c),
            LiteralKind::String(s) => LiteralValue::String(s.clone()),
            LiteralKind::ByteString(bytes) => LiteralValue::ByteString(bytes.clone()),
            LiteralKind::Byte(b) => LiteralValue::Byte(*b),
        }
    }
}

/// A field in a struct expression.
#[derive(Debug, Clone)]
pub struct FieldExpr {
    /// Field index.
    pub field_idx: u32,
    /// Field value.
    pub value: Expr,
}

/// A field in an anonymous record expression.
#[derive(Debug, Clone)]
pub struct RecordFieldExpr {
    /// Field name.
    pub name: crate::ast::Symbol,
    /// Field value.
    pub value: Expr,
}

/// A statement in a block.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// Let binding: `let x = e;`
    Let {
        local_id: LocalId,
        init: Option<Expr>,
    },
    /// Expression statement: `e;`
    Expr(Expr),
    /// Item declaration (nested function, etc.)
    Item(DefId),
    /// Defer statement: execute body at scope exit.
    Defer {
        body: Expr,
    },
}

/// A match arm.
#[derive(Debug, Clone)]
pub struct MatchArm {
    /// The pattern to match.
    pub pattern: Pattern,
    /// Optional guard: `if cond`
    pub guard: Option<Expr>,
    /// The body expression.
    pub body: Expr,
}

/// A pattern in HIR.
#[derive(Debug, Clone)]
pub struct Pattern {
    /// The pattern kind.
    pub kind: PatternKind,
    /// The type this pattern matches.
    pub ty: Type,
    /// The source span.
    pub span: Span,
}

/// The kind of a pattern.
#[derive(Debug, Clone)]
pub enum PatternKind {
    /// Wildcard: `_`
    Wildcard,
    /// Binding: `x` or `mut x`
    Binding {
        local_id: LocalId,
        mutable: bool,
        /// Optional subpattern: `x @ pat`
        subpattern: Option<Box<Pattern>>,
    },
    /// Literal: `42`, `true`
    Literal(LiteralValue),
    /// Variant: `Some(x)`
    Variant {
        def_id: DefId,
        variant_idx: u32,
        fields: Vec<Pattern>,
    },
    /// Struct: `Point { x, y }`
    Struct {
        def_id: DefId,
        fields: Vec<FieldPattern>,
    },
    /// Tuple: `(a, b)`
    Tuple(Vec<Pattern>),
    /// Slice: `[first, .., last]`
    Slice {
        prefix: Vec<Pattern>,
        slice: Option<Box<Pattern>>,
        suffix: Vec<Pattern>,
    },
    /// Or pattern: `A | B`
    Or(Vec<Pattern>),
    /// Reference: `&x`
    Ref {
        mutable: bool,
        inner: Box<Pattern>,
    },
    /// Range: `0..10`, `'a'..='z'`
    Range {
        start: Option<Box<Pattern>>,
        end: Option<Box<Pattern>>,
        inclusive: bool,
    },
}

/// A field pattern.
#[derive(Debug, Clone)]
pub struct FieldPattern {
    /// Field index.
    pub field_idx: u32,
    /// The pattern for this field.
    pub pattern: Pattern,
}

/// A captured variable in a closure.
#[derive(Debug, Clone)]
pub struct Capture {
    /// The local being captured.
    pub local_id: LocalId,
    /// Whether captured by move or by reference.
    pub by_move: bool,
}

/// A loop identifier for break/continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoopId(pub u32);

impl LoopId {
    pub const fn new(id: u32) -> Self {
        LoopId(id)
    }
}

impl fmt::Display for LoopId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'loop{}", self.0)
    }
}

/// An inline operation handler clause.
///
/// Used in `try { ... } with { Effect::op(x) => { ... } }` expressions.
#[derive(Debug, Clone)]
pub struct InlineOpHandler {
    /// The effect being handled.
    pub effect_id: DefId,
    /// The operation name being handled.
    pub op_name: String,
    /// Parameter local IDs for the operation parameters.
    pub params: Vec<LocalId>,
    /// Parameter types for the operation.
    pub param_types: Vec<Type>,
    /// The return type of the operation.
    pub return_type: Type,
    /// The handler body.
    pub body: Expr,
}
