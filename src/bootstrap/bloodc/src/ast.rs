//! Abstract Syntax Tree for Blood.
//!
//! This module defines the AST data structures that represent parsed Blood
//! programs. The AST closely mirrors the surface syntax defined in GRAMMAR.md.
//!
//! # AST Structure
//!
//! The AST is hierarchical:
//!
//! - [`Program`] - Root node containing a module declaration, imports, and declarations
//! - [`Declaration`] - Top-level items (functions, structs, enums, effects, etc.)
//! - [`Expr`] - Expressions (literals, operations, control flow, etc.)
//! - [`Pattern`] - Patterns for destructuring (match, let bindings)
//! - [`Type`] - Type expressions (paths, references, generics, etc.)
//!
//! # Design Notes
//!
//! - All AST nodes derive `Debug`, `Clone`, `PartialEq`, and `Eq` for testing.
//! - Floating-point values use `OrderedFloat` wrapper for total ordering.
//! - Source locations are tracked via `Span` on each node.
//! - Identifiers are interned as `Symbol` for efficient comparison.
//!
//! # Example
//!
//! ```rust
//! use bloodc::Parser;
//! use bloodc::ast::Declaration;
//!
//! let source = "fn main() { let x = 42; x }";
//! let mut parser = Parser::new(source);
//! let program = parser.parse_program().expect("parse failed");
//!
//! // Access the function declaration
//! let Declaration::Function(func) = &program.declarations[0] else {
//!     panic!("expected function");
//! };
//!
//! // The function body is a Block with statements and optional trailing expr
//! if let Some(body) = &func.body {
//!     assert_eq!(body.statements.len(), 1); // let statement
//!     assert!(body.expr.is_some()); // trailing expression: x
//! }
//! ```

use crate::span::{Span, Spanned};
use serde::{Deserialize, Serialize};
use string_interner::DefaultSymbol;

/// A symbol representing an interned string.
pub type Symbol = DefaultSymbol;

/// Wrapper for f64 that provides total ordering and Eq.
///
/// This allows AST nodes containing floats to derive Eq for testing.
/// NaN values are considered equal to each other.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OrderedFloat(pub f64);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        // NaN == NaN for our purposes
        if self.0.is_nan() && other.0.is_nan() {
            return true;
        }
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl std::hash::Hash for OrderedFloat {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl From<f64> for OrderedFloat {
    fn from(f: f64) -> Self {
        OrderedFloat(f)
    }
}

impl From<OrderedFloat> for f64 {
    fn from(f: OrderedFloat) -> Self {
        f.0
    }
}

/// A program is a compilation unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// Optional module declaration.
    pub module: Option<ModuleDecl>,
    /// Import statements.
    pub imports: Vec<Import>,
    /// Top-level declarations.
    pub declarations: Vec<Declaration>,
    /// The span of the entire program.
    pub span: Span,
}

/// Module declaration: `module std.collections.vec;`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDecl {
    pub path: ModulePath,
    pub span: Span,
}

/// A module path like `std.collections.vec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePath {
    pub segments: Vec<Spanned<Symbol>>,
    pub span: Span,
}

/// Import statement.
///
/// Imports can be either private (default) or public (re-exports).
/// Public imports (`pub use ...`) make the imported item available
/// to external modules, enabling clean API surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    /// `use std.mem.allocate;` or `pub use std.mem.allocate;`
    Simple {
        path: ModulePath,
        alias: Option<Spanned<Symbol>>,
        visibility: Visibility,
        span: Span,
    },
    /// `use std.iter::{Iterator, IntoIterator};`
    Group {
        path: ModulePath,
        items: Vec<ImportItem>,
        visibility: Visibility,
        span: Span,
    },
    /// `use std.ops::*;`
    Glob {
        path: ModulePath,
        visibility: Visibility,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportItem {
    pub name: Spanned<Symbol>,
    pub alias: Option<Spanned<Symbol>>,
}

/// Top-level declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    Function(FnDecl),
    Type(TypeDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Effect(EffectDecl),
    Handler(HandlerDecl),
    Const(ConstDecl),
    Static(StaticDecl),
    Impl(ImplBlock),
    Trait(TraitDecl),
    Bridge(BridgeDecl),
    Module(ModItemDecl),
    /// Declarative macro definition: `macro name!(...) { ... }`
    Macro(MacroDecl),
    /// Use/import declaration: `use path;` or `pub use path;`
    /// This allows use statements to appear anywhere in the file,
    /// enabling imports after module declarations.
    Use(Import),
}

impl Declaration {
    pub fn span(&self) -> Span {
        match self {
            Declaration::Function(d) => d.span,
            Declaration::Type(d) => d.span,
            Declaration::Struct(d) => d.span,
            Declaration::Enum(d) => d.span,
            Declaration::Effect(d) => d.span,
            Declaration::Handler(d) => d.span,
            Declaration::Const(d) => d.span,
            Declaration::Static(d) => d.span,
            Declaration::Impl(d) => d.span,
            Declaration::Trait(d) => d.span,
            Declaration::Bridge(d) => d.span,
            Declaration::Module(d) => d.span,
            Declaration::Macro(d) => d.span,
            Declaration::Use(import) => match import {
                Import::Simple { span, .. } => *span,
                Import::Group { span, .. } => *span,
                Import::Glob { span, .. } => *span,
            },
        }
    }
}

/// Module item declaration: `mod foo;` or `mod foo { ... }`
///
/// This is distinct from `ModuleDecl` at the top of a file (`module std.collections;`).
/// `ModItemDecl` represents inline module definitions within a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModItemDecl {
    /// Attributes on the module
    pub attrs: Vec<Attribute>,
    /// Visibility
    pub vis: Visibility,
    /// Module name
    pub name: Spanned<Symbol>,
    /// Module body - None for `mod foo;` (external module), Some for `mod foo { ... }`
    pub body: Option<Vec<Declaration>>,
    /// Full span
    pub span: Span,
}

// ============================================================
// Visibility
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Private,
    Public,
    PublicCrate,
    PublicSuper,
    PublicSelf,
}

// ============================================================
// Attributes
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Whether this is an inner attribute (`#![...]`) vs outer (`#[...]`)
    pub is_inner: bool,
    pub path: Vec<Spanned<Symbol>>,
    pub args: Option<AttributeArgs>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeArgs {
    /// `#[attr = "value"]`
    Eq(Literal),
    /// `#[attr(key = "value", ...)]`
    List(Vec<AttributeArg>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeArg {
    /// Just an identifier: `#[attr(flag)]`
    Ident(Spanned<Symbol>),
    /// Key-value pair: `#[attr(key = "value")]`
    KeyValue(Spanned<Symbol>, Literal),
    /// Literal: `#[attr("value")]`
    Literal(Literal),
    /// Call-style nested attribute: `#[repr(align(N))]`
    Call(Spanned<Symbol>, Literal),
}

// ============================================================
// Function Declaration
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnDecl {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub qualifiers: FnQualifiers,
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub effects: Option<EffectRow>,
    pub spec_clauses: Vec<SpecClause>,
    pub where_clause: Option<WhereClause>,
    pub body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FnQualifiers {
    pub is_const: bool,
    pub is_async: bool,
    pub is_unsafe: bool,
}

/// The kind of specification clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecClauseKind {
    Requires,
    Ensures,
    Invariant,
    Decreases,
}

/// A specification clause on a function declaration.
///
/// Spec clauses are parsed and stored in the AST but not enforced at runtime.
/// They serve as documentation/annotations for future formal verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecClause {
    pub kind: SpecClauseKind,
    pub expr: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub qualifier: Option<ParamQualifier>,
    pub pattern: Pattern,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamQualifier {
    Linear,
    Affine,
    Mut,
}

// ============================================================
// Type Declarations
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub ty: Option<Type>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDecl {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub body: StructBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructBody {
    /// `struct Foo { x: i32, y: i32 }`
    Record(Vec<StructField>),
    /// `struct Foo(i32, i32);`
    Tuple(Vec<Type>),
    /// `struct Foo;`
    Unit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructField {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub name: Spanned<Symbol>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDecl {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariant {
    pub attrs: Vec<Attribute>,
    pub name: Spanned<Symbol>,
    pub body: StructBody,
    pub span: Span,
}

// ============================================================
// Effect and Handler Declarations
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDecl {
    pub attrs: Vec<Attribute>,
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub extends: Vec<Type>,
    pub operations: Vec<OperationDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationDecl {
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerDecl {
    pub attrs: Vec<Attribute>,
    pub kind: HandlerKind,
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub effect: Type,
    pub where_clause: Option<WhereClause>,
    pub state: Vec<HandlerState>,
    pub return_clause: Option<ReturnClause>,
    pub operations: Vec<OperationImpl>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandlerKind {
    #[default]
    Deep,
    Shallow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerState {
    pub is_mut: bool,
    pub name: Spanned<Symbol>,
    pub ty: Type,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnClause {
    pub param: Spanned<Symbol>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationImpl {
    pub name: Spanned<Symbol>,
    pub params: Vec<Pattern>,
    pub body: Block,
    pub span: Span,
}

/// Handler clause in a try-with expression.
/// Example: `Effect::op(x, y) => { body }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryWithHandler {
    /// The effect path, e.g., `LexerDiagnostic` in `LexerDiagnostic::error`
    pub effect: TypePath,
    /// The operation name, e.g., `error` in `LexerDiagnostic::error`
    pub operation: Spanned<Symbol>,
    /// Parameter patterns for the operation
    pub params: Vec<Pattern>,
    /// The handler body
    pub body: Block,
    pub span: Span,
}

/// Inline handler operation definition.
/// Used in inline handler expressions like `handle { } with Effect { op name(resume) -> T { } }`
/// Example: `op choose() -> bool { resume(true) }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineHandlerOp {
    /// The operation name (e.g., `choose`)
    pub name: Spanned<Symbol>,
    /// Parameter patterns including optional `resume` parameter
    pub params: Vec<Pattern>,
    /// Optional explicit return type
    pub return_type: Option<Type>,
    /// The operation body
    pub body: Block,
    pub span: Span,
}

// ============================================================
// Trait and Implementation
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitDecl {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub name: Spanned<Symbol>,
    pub type_params: Option<TypeParams>,
    pub supertraits: Vec<Type>,
    pub where_clause: Option<WhereClause>,
    pub items: Vec<TraitItem>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraitItem {
    Function(FnDecl),
    Type(TypeDecl),
    Const(ConstDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplBlock {
    pub attrs: Vec<Attribute>,
    pub type_params: Option<TypeParams>,
    pub trait_ty: Option<Type>,
    pub self_ty: Type,
    pub where_clause: Option<WhereClause>,
    pub items: Vec<ImplItem>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImplItem {
    Function(FnDecl),
    Type(TypeDecl),
    Const(ConstDecl),
}

// ============================================================
// Constants and Statics
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstDecl {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub name: Spanned<Symbol>,
    pub ty: Type,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticDecl {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub is_mut: bool,
    pub name: Spanned<Symbol>,
    pub ty: Type,
    pub value: Expr,
    pub span: Span,
}

// ============================================================
// Bridge Declaration (FFI)
// ============================================================

/// Foreign function interface bridge declaration.
/// `bridge "C" libc { ... }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDecl {
    /// Attributes on the bridge
    pub attrs: Vec<Attribute>,
    /// The language/ABI specifier ("C", "C++", "wasm")
    pub language: Spanned<String>,
    /// The bridge name (e.g., "libc")
    pub name: Spanned<Symbol>,
    /// Items within the bridge
    pub items: Vec<BridgeItem>,
    /// Full span
    pub span: Span,
}

/// An item within a bridge block
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeItem {
    /// Link specification: `#[link(name = "c")]`
    Link(LinkSpec),
    /// Foreign function: `fn malloc(size: usize) -> *mut u8;`
    Function(BridgeFn),
    /// Constant: `const STDIN_FILENO: i32 = 0;`
    Const(BridgeConst),
    /// Opaque type: `type FILE;`
    OpaqueType(BridgeOpaqueType),
    /// Type alias: `type MyInt = i32;`
    TypeAlias(BridgeTypeAlias),
    /// Struct with C layout: `#[repr(C)] struct Point { x: f64, y: f64 }`
    Struct(BridgeStruct),
    /// Enum with C layout: `#[repr(C)] enum Status { Ok = 0, Error = 1 }`
    Enum(BridgeEnum),
    /// Union with C layout: `#[repr(C)] union Value { i: i32, f: f32 }`
    Union(BridgeUnion),
    /// Callback type: `callback Comparator = fn(*const u8, *const u8) -> i32;`
    Callback(BridgeCallback),
}

/// Link specification for a bridge
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpec {
    /// Library name to link
    pub name: String,
    /// Link kind: dylib, static, framework
    pub kind: Option<LinkKind>,
    /// WASM import module name
    pub wasm_import_module: Option<String>,
    /// Full span
    pub span: Span,
}

/// Kind of library linking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// Dynamic library (default)
    Dylib,
    /// Static library
    Static,
    /// macOS framework
    Framework,
}

/// Foreign function declaration
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFn {
    /// Attributes (e.g., calling_convention, mangle)
    pub attrs: Vec<Attribute>,
    /// Function name
    pub name: Spanned<Symbol>,
    /// Parameters
    pub params: Vec<BridgeParam>,
    /// Is variadic (...)
    pub is_variadic: bool,
    /// Return type
    pub return_type: Option<Type>,
    /// Full span
    pub span: Span,
}

/// Parameter in a bridge function
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeParam {
    /// Parameter name
    pub name: Spanned<Symbol>,
    /// Parameter type
    pub ty: Type,
    /// Ownership annotation (#[borrow], #[transfer], #[acquire])
    pub ownership: Option<BridgeOwnership>,
    /// Full span
    pub span: Span,
}

/// Ownership annotation for bridge parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeOwnership {
    /// Caller retains ownership, callee borrows
    Borrow,
    /// Caller transfers ownership to callee
    Transfer,
    /// Callee returns ownership to caller
    Acquire,
}

/// Constant declaration in bridge
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConst {
    /// Constant name
    pub name: Spanned<Symbol>,
    /// Constant type
    pub ty: Type,
    /// Constant value
    pub value: Literal,
    /// Full span
    pub span: Span,
}

/// Opaque type declaration (type FILE;)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeOpaqueType {
    /// Type name
    pub name: Spanned<Symbol>,
    /// Full span
    pub span: Span,
}

/// Type alias declaration (type MyInt = i32;)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTypeAlias {
    /// Type name
    pub name: Spanned<Symbol>,
    /// The aliased type
    pub ty: Type,
    /// Full span
    pub span: Span,
}

/// C-compatible struct
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStruct {
    /// Attributes (repr, packed, align)
    pub attrs: Vec<Attribute>,
    /// Struct name
    pub name: Spanned<Symbol>,
    /// Fields
    pub fields: Vec<BridgeField>,
    /// Full span
    pub span: Span,
}

/// Field in a bridge struct/union
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeField {
    /// Field name
    pub name: Spanned<Symbol>,
    /// Field type
    pub ty: Type,
    /// Full span
    pub span: Span,
}

/// C-compatible enum
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEnum {
    /// Attributes (repr)
    pub attrs: Vec<Attribute>,
    /// Enum name
    pub name: Spanned<Symbol>,
    /// Variants
    pub variants: Vec<BridgeEnumVariant>,
    /// Full span
    pub span: Span,
}

/// Enum variant in bridge
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEnumVariant {
    /// Variant name
    pub name: Spanned<Symbol>,
    /// Discriminant value (if specified)
    pub discriminant: Option<Literal>,
    /// Full span
    pub span: Span,
}

/// C-compatible union
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeUnion {
    /// Attributes (repr)
    pub attrs: Vec<Attribute>,
    /// Union name
    pub name: Spanned<Symbol>,
    /// Fields
    pub fields: Vec<BridgeField>,
    /// Full span
    pub span: Span,
}

/// Callback type definition
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCallback {
    /// Callback name
    pub name: Spanned<Symbol>,
    /// Parameters
    pub params: Vec<Type>,
    /// Return type
    pub return_type: Option<Type>,
    /// Full span
    pub span: Span,
}

// ============================================================
// Type Parameters and Constraints
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParams {
    pub params: Vec<GenericParam>,
    pub span: Span,
}

/// A generic parameter (type, lifetime, or const).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenericParam {
    /// A type parameter: `T`, `T: Trait`, `T: Trait + Other`
    Type(TypeParam),
    /// A lifetime parameter: `'a`
    Lifetime(LifetimeParam),
    /// A const parameter: `const N: usize`
    Const(ConstParam),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParam {
    pub name: Spanned<Symbol>,
    pub bounds: Vec<Type>,
    pub span: Span,
}

/// A lifetime parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifetimeParam {
    /// The lifetime name (including the leading `'`).
    pub name: Spanned<Symbol>,
    /// Lifetime bounds (e.g., `'a: 'b`).
    pub bounds: Vec<Spanned<Symbol>>,
    pub span: Span,
}

/// A const generic parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstParam {
    /// The parameter name.
    pub name: Spanned<Symbol>,
    /// The type of the const (e.g., `usize`).
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhereClause {
    pub predicates: Vec<WherePredicate>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WherePredicate {
    TypeBound {
        ty: Type,
        bounds: Vec<Type>,
        span: Span,
    },
    Lifetime {
        lifetime: Spanned<Symbol>,
        bounds: Spanned<Symbol>,
        span: Span,
    },
}

// ============================================================
// Types
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    /// Type path: `i32`, `std::vec::Vec<T>`
    Path(TypePath),

    /// Reference: `&T`, `&mut T`, `&'a T`
    Reference {
        lifetime: Option<Spanned<Symbol>>,
        mutable: bool,
        inner: Box<Type>,
    },

    /// Pointer: `*const T`, `*mut T`
    Pointer {
        mutable: bool,
        inner: Box<Type>,
    },

    /// Array: `[T; N]`
    Array {
        element: Box<Type>,
        size: Box<Expr>,
    },

    /// Slice: `[T]`
    Slice {
        element: Box<Type>,
    },

    /// Tuple: `()`, `(T,)`, `(T, U)`
    Tuple(Vec<Type>),

    /// Function: `fn(T) -> U / E`
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
        effects: Option<EffectRow>,
    },

    /// Record: `{ x: T, y: U }` or `{ x: T | R }`
    Record {
        fields: Vec<RecordTypeField>,
        rest: Option<Spanned<Symbol>>,
    },

    /// Ownership qualifier: `linear T`, `affine T`
    Ownership {
        qualifier: OwnershipQualifier,
        inner: Box<Type>,
    },

    /// Higher-rank polymorphic type: `forall<T>. T -> T`
    ///
    /// Enables first-class polymorphism where types themselves can be polymorphic.
    /// Used for functions that need to work with polymorphic values directly.
    Forall {
        /// Type parameters bound by this forall
        params: Vec<Spanned<Symbol>>,
        /// The body type where params are in scope
        body: Box<Type>,
    },

    /// Never type: `!`
    Never,

    /// Inferred type: `_`
    Infer,

    /// Parenthesized: `(T)`
    Paren(Box<Type>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypePath {
    pub segments: Vec<TypePathSegment>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypePathSegment {
    pub name: Spanned<Symbol>,
    pub args: Option<TypeArgs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeArgs {
    pub args: Vec<TypeArg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeArg {
    Type(Type),
    Lifetime(Spanned<Symbol>),
    Const(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordTypeField {
    pub name: Spanned<Symbol>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipQualifier {
    Linear,
    Affine,
}

// ============================================================
// Effect Rows
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRow {
    pub kind: EffectRowKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectRowKind {
    /// `pure` or `{}`
    Pure,
    /// `{IO, Error<E>}` or `{IO | e}`
    Effects {
        effects: Vec<Type>,
        rest: Option<Spanned<Symbol>>,
    },
    /// Just a type variable: `e`
    Var(Spanned<Symbol>),
}

// ============================================================
// Expressions
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    /// Literal: `42`, `"hello"`, `true`
    Literal(Literal),

    /// Path: `x`, `std::vec::Vec`
    Path(ExprPath),

    /// Binary operation: `a + b`
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// Unary operation: `!x`, `-x`, `*x`, `&x`
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },

    /// Function call: `f(x, y)`
    Call {
        callee: Box<Expr>,
        args: Vec<CallArg>,
    },

    /// Method call: `x.foo(y)`
    MethodCall {
        receiver: Box<Expr>,
        method: Spanned<Symbol>,
        type_args: Option<TypeArgs>,
        args: Vec<CallArg>,
    },

    /// Field access: `x.field`, `x.0`
    Field {
        base: Box<Expr>,
        field: FieldAccess,
    },

    /// Index: `x[i]`
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },

    /// Tuple: `()`, `(x,)`, `(x, y)`
    Tuple(Vec<Expr>),

    /// Array: `[1, 2, 3]` or `[0; 10]`
    Array(ArrayExpr),

    /// Record: `Point { x: 1, y: 2 }` or `{ x, y }`
    Record {
        path: Option<TypePath>,
        fields: Vec<RecordExprField>,
        base: Option<Box<Expr>>,
    },

    /// Range: `a..b`, `a..=b`, `..b`, `a..`
    Range {
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool,
    },

    /// Cast: `x as T`
    Cast {
        expr: Box<Expr>,
        ty: Type,
    },

    /// Assignment: `x = y`
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },

    /// Compound assignment: `x += y`
    AssignOp {
        op: BinOp,
        target: Box<Expr>,
        value: Box<Expr>,
    },

    /// Block: `{ ... }`
    Block(Block),

    /// If: `if cond { } else { }`
    If {
        condition: Box<Expr>,
        then_branch: Block,
        else_branch: Option<ElseBranch>,
    },

    /// If-let: `if let PATTERN = EXPR { } else { }`
    IfLet {
        pattern: Pattern,
        scrutinee: Box<Expr>,
        then_branch: Block,
        else_branch: Option<ElseBranch>,
    },

    /// Match: `match x { ... }`
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    /// Loop: `loop { }`
    Loop {
        label: Option<Spanned<Symbol>>,
        body: Block,
    },

    /// While: `while cond { }`
    While {
        label: Option<Spanned<Symbol>>,
        condition: Box<Expr>,
        body: Block,
    },

    /// While-let: `while let PATTERN = EXPR { }`
    WhileLet {
        label: Option<Spanned<Symbol>>,
        pattern: Pattern,
        scrutinee: Box<Expr>,
        body: Block,
    },

    /// For: `for x in iter { }`
    For {
        label: Option<Spanned<Symbol>>,
        pattern: Pattern,
        iter: Box<Expr>,
        body: Block,
    },

    /// Return: `return x`
    Return(Option<Box<Expr>>),

    /// Break: `break 'label x`
    Break {
        label: Option<Spanned<Symbol>>,
        value: Option<Box<Expr>>,
    },

    /// Continue: `continue 'label`
    Continue {
        label: Option<Spanned<Symbol>>,
    },

    /// Closure: `|x| x + 1`, `move |x| { x }`
    Closure {
        is_move: bool,
        params: Vec<ClosureParam>,
        return_type: Option<Type>,
        effects: Option<EffectRow>,
        body: Box<Expr>,
    },

    /// With-handle: `with handler handle { }`
    WithHandle {
        handler: Box<Expr>,
        body: Box<Expr>,
    },

    /// Perform: `perform Effect.op(args)`
    Perform {
        effect: Option<TypePath>,
        operation: Spanned<Symbol>,
        args: Vec<Expr>,
    },

    /// Resume: `resume(x)`
    Resume(Box<Expr>),

    /// Try-with inline handler: `try { body } with { handlers }`
    TryWith {
        body: Block,
        handlers: Vec<TryWithHandler>,
    },

    /// Inline handle expression: `handle { body } with Effect { ops... }`
    /// Provides inline handler definitions without a named handler declaration.
    InlineHandle {
        /// The body expression to execute with the handler
        body: Box<Expr>,
        /// The effect being handled
        effect: TypePath,
        /// Inline operation definitions
        operations: Vec<InlineHandlerOp>,
    },

    /// Inline with-do expression: `with Effect { ops... } do { body }`
    /// Alternative syntax for inline handlers with effect first.
    InlineWithDo {
        /// The effect being handled
        effect: TypePath,
        /// Inline operation definitions
        operations: Vec<InlineHandlerOp>,
        /// The body expression to execute with the handler
        body: Box<Expr>,
    },

    /// Unsafe block: `@unsafe { }`
    Unsafe(Block),

    /// Region: `region 'a { }`
    Region {
        name: Option<Spanned<Symbol>>,
        body: Block,
    },

    /// Parenthesized: `(x)`
    Paren(Box<Expr>),

    /// Default value: `default`
    /// Creates a default instance of the type being assigned to.
    Default,

    /// Macro call: `name!(args)`, `name![args]`, `name!{args}`
    MacroCall {
        path: ExprPath,
        kind: MacroCallKind,
    },
}

/// The kind of macro call (built-in or user-defined).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroCallKind {
    /// Built-in format macro: `format!("...", args)`, `println!(...)`, etc.
    Format {
        format_str: Spanned<String>,
        args: Vec<Expr>,
        /// Named arguments: `name = expr`
        named_args: Vec<(String, Expr)>,
    },
    /// Built-in vec macro: `vec![1, 2, 3]` or `vec![0; 10]`
    Vec(VecMacroArgs),
    /// Built-in assert macro: `assert!(cond)` or `assert!(cond, "message")`
    Assert {
        condition: Box<Expr>,
        message: Option<Box<Expr>>,
    },
    /// Built-in dbg macro: `dbg!(expr)`
    Dbg(Box<Expr>),
    /// Built-in matches macro: `matches!(expr, pattern)` returns bool
    Matches {
        expr: Box<Expr>,
        pattern: Box<Pattern>,
    },
    /// User-defined macro (not yet supported - stored as raw tokens for future expansion)
    Custom {
        delim: MacroDelimiter,
        /// Raw token content between delimiters (as string for now)
        content: String,
    },
}

/// Delimiter type for macro invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroDelimiter {
    /// `()`
    Paren,
    /// `[]`
    Bracket,
    /// `{}`
    Brace,
}

/// Arguments to the vec! macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VecMacroArgs {
    /// `vec![1, 2, 3]`
    List(Vec<Expr>),
    /// `vec![0; 10]`
    Repeat { value: Box<Expr>, count: Box<Expr> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprPath {
    pub segments: Vec<ExprPathSegment>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprPathSegment {
    pub name: Spanned<Symbol>,
    pub args: Option<TypeArgs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallArg {
    pub name: Option<Spanned<Symbol>>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldAccess {
    Named(Spanned<Symbol>),
    Index(u32, Span),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrayExpr {
    /// `[1, 2, 3]`
    List(Vec<Expr>),
    /// `[0; 10]`
    Repeat { value: Box<Expr>, count: Box<Expr> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordExprField {
    pub name: Spanned<Symbol>,
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElseBranch {
    Block(Block),
    If(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureParam {
    pub pattern: Pattern,
    pub ty: Option<Type>,
    pub span: Span,
}

// ============================================================
// Operators
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Logical
    And,
    Or,
    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    // Pipe
    Pipe,
}

impl BinOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Pipe => "|>",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Not,
    Deref,
    Ref,
    RefMut,
}

impl UnaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
            UnaryOp::Deref => "*",
            UnaryOp::Ref => "&",
            UnaryOp::RefMut => "&mut",
        }
    }
}

// ============================================================
// Literals
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Literal {
    pub kind: LiteralKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiteralKind {
    Int {
        value: u128,
        suffix: Option<IntSuffix>,
    },
    Float {
        /// Float value wrapped for total ordering (Eq support).
        value: OrderedFloat,
        suffix: Option<FloatSuffix>,
    },
    String(String),
    /// Byte string literal (b"...")
    ByteString(Vec<u8>),
    Char(char),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntSuffix {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FloatSuffix {
    F32,
    F64,
}

// ============================================================
// Patterns
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternKind {
    /// Wildcard: `_`
    Wildcard,

    /// Rest: `..`
    Rest,

    /// Literal: `42`, `"hello"`
    Literal(Literal),

    /// Identifier: `x`, `ref x`, `mut x`, `x @ pat`
    Ident {
        by_ref: bool,
        mutable: bool,
        name: Spanned<Symbol>,
        subpattern: Option<Box<Pattern>>,
    },

    /// Reference pattern: `&x`, `&mut x`
    Ref {
        mutable: bool,
        inner: Box<Pattern>,
    },

    /// Struct pattern: `Point { x, y: 0 }`
    Struct {
        path: TypePath,
        fields: Vec<StructPatternField>,
        rest: bool,
    },

    /// Tuple struct: `Some(x)`
    TupleStruct {
        path: TypePath,
        fields: Vec<Pattern>,
        rest_pos: Option<usize>,
    },

    /// Tuple: `(x, y)`
    Tuple {
        fields: Vec<Pattern>,
        rest_pos: Option<usize>,
    },

    /// Slice: `[first, .., last]`
    Slice {
        elements: Vec<Pattern>,
        rest_pos: Option<usize>,
    },

    /// Or pattern: `A | B | C`
    Or(Vec<Pattern>),

    /// Range: `0..10`, `'a'..='z'`
    Range {
        start: Option<Box<Pattern>>,
        end: Option<Box<Pattern>>,
        inclusive: bool,
    },

    /// Path: `None`, `std::option::None`
    Path(TypePath),

    /// Parenthesized: `(pat)`
    Paren(Box<Pattern>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructPatternField {
    pub name: Spanned<Symbol>,
    pub pattern: Option<Pattern>,
    pub span: Span,
}

// ============================================================
// Statements and Blocks
// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub expr: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    /// Let binding: `let x = 1;`
    Let {
        pattern: Pattern,
        ty: Option<Type>,
        value: Option<Expr>,
        span: Span,
    },

    /// Expression statement: `foo();`
    Expr {
        expr: Expr,
        has_semi: bool,
    },

    /// Item (function, struct, etc. inside a block)
    Item(Declaration),
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Statement::Let { span, .. } => *span,
            Statement::Expr { expr, .. } => expr.span,
            Statement::Item(decl) => decl.span(),
        }
    }
}

// ============================================================
// Macro System Types
// ============================================================

/// A hygiene context identifier for macro expansion.
///
/// Each macro expansion gets a unique hygiene ID to prevent
/// accidental variable capture (hygienic macros).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(Default)]
pub struct HygieneId(pub u32);


/// A token with hygiene information for macro expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroToken {
    /// The token kind
    pub kind: crate::lexer::TokenKind,
    /// Source span
    pub span: Span,
    /// Hygiene context for macro hygiene
    pub hygiene: HygieneId,
}

/// A stream of tokens for macro processing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenStream {
    pub tokens: Vec<TokenTree>,
}

impl TokenStream {
    pub fn new() -> Self {
        Self { tokens: Vec::new() }
    }

    pub fn push(&mut self, tree: TokenTree) {
        self.tokens.push(tree);
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }
}

/// A single token or delimited group in a token stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenTree {
    /// A single token
    Token(MacroToken),
    /// A delimited group: `(...)`, `[...]`, `{...}`
    Group {
        delimiter: MacroDelimiter,
        stream: TokenStream,
        span: Span,
    },
}

impl TokenTree {
    pub fn span(&self) -> Span {
        match self {
            TokenTree::Token(t) => t.span,
            TokenTree::Group { span, .. } => *span,
        }
    }
}

// ============================================================
// Macro Declarations
// ============================================================

/// A declarative macro definition: `macro name!(...) { ... }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroDecl {
    /// Attributes on the macro
    pub attrs: Vec<Attribute>,
    /// Visibility
    pub vis: Visibility,
    /// Macro name (without the `!`)
    pub name: Spanned<Symbol>,
    /// Macro rules (pattern -> expansion pairs)
    pub rules: Vec<MacroRule>,
    /// Full span
    pub span: Span,
}

/// A single macro rule: `(pattern) => { expansion }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroRule {
    /// The pattern to match
    pub pattern: MacroPattern,
    /// The expansion template
    pub expansion: MacroExpansion,
    /// Full span
    pub span: Span,
}

/// A macro pattern (what to match in macro invocation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroPattern {
    pub parts: Vec<MacroPatternPart>,
    pub span: Span,
}

/// A part of a macro pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroPatternPart {
    /// Literal token to match
    Token(crate::lexer::TokenKind, Span),
    /// Capture: `$name:frag`
    Capture {
        /// The capture name (e.g., `x` in `$x:expr`)
        name: Spanned<Symbol>,
        /// The fragment kind
        fragment: FragmentKind,
        /// Full span including `$name:frag`
        span: Span,
    },
    /// Repetition: `$(...)*`, `$(...)+`, `$(...)?`
    Repetition {
        /// The pattern inside the repetition
        pattern: Vec<MacroPatternPart>,
        /// Optional separator token (e.g., `,` in `$($x:expr),*`)
        separator: Option<crate::lexer::TokenKind>,
        /// Repetition kind
        kind: RepetitionKind,
        /// Full span
        span: Span,
    },
    /// A delimited group in the pattern
    Group {
        delimiter: MacroDelimiter,
        pattern: Vec<MacroPatternPart>,
        span: Span,
    },
}

/// Fragment specifier for macro captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentKind {
    /// Expression: `$x:expr`
    Expr,
    /// Type: `$t:ty`
    Ty,
    /// Pattern: `$p:pat`
    Pat,
    /// Identifier: `$i:ident`
    Ident,
    /// Literal: `$l:literal`
    Literal,
    /// Block: `$b:block`
    Block,
    /// Statement: `$s:stmt`
    Stmt,
    /// Item/Declaration: `$d:item`
    Item,
    /// Single token tree: `$t:tt`
    TokenTree,
}

impl FragmentKind {
    /// Parse a fragment kind from its string representation.
    #[allow(clippy::should_implement_trait)] // Returns Option, not Result<_, Err> as FromStr requires
    pub fn from_str(s: &str) -> Option<FragmentKind> {
        match s {
            "expr" => Some(FragmentKind::Expr),
            "ty" => Some(FragmentKind::Ty),
            "pat" => Some(FragmentKind::Pat),
            "ident" => Some(FragmentKind::Ident),
            "literal" => Some(FragmentKind::Literal),
            "block" => Some(FragmentKind::Block),
            "stmt" => Some(FragmentKind::Stmt),
            "item" => Some(FragmentKind::Item),
            "tt" => Some(FragmentKind::TokenTree),
            _ => None,
        }
    }
}

/// Repetition kind in macro patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepetitionKind {
    /// Zero or more: `*`
    ZeroOrMore,
    /// One or more: `+`
    OneOrMore,
    /// Zero or one: `?`
    ZeroOrOne,
}

/// A macro expansion template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroExpansion {
    pub parts: Vec<MacroExpansionPart>,
    pub span: Span,
}

/// A part of a macro expansion template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroExpansionPart {
    /// Literal tokens to emit
    Tokens(Vec<MacroToken>),
    /// Substitution: `$name`
    Substitution {
        name: Spanned<Symbol>,
        span: Span,
    },
    /// Repetition: `$(...)*`
    Repetition {
        parts: Vec<MacroExpansionPart>,
        separator: Option<MacroToken>,
        span: Span,
    },
    /// A delimited group
    Group {
        delimiter: MacroDelimiter,
        parts: Vec<MacroExpansionPart>,
        span: Span,
    },
}
