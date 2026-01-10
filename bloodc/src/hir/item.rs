//! HIR items (top-level declarations).
//!
//! This module defines the HIR representation of top-level items:
//! functions, structs, enums, type aliases, etc.

use crate::ast::Visibility;
use crate::span::Span;
use super::{DefId, DefKind, Type, BodyId};

/// A top-level item in HIR.
#[derive(Debug, Clone)]
pub struct Item {
    /// The definition ID.
    pub def_id: DefId,
    /// The kind of item.
    pub kind: ItemKind,
    /// The visibility.
    pub vis: Visibility,
    /// The name of this item.
    pub name: String,
    /// The source span.
    pub span: Span,
}

impl Item {
    /// Get the DefKind for this item.
    pub fn def_kind(&self) -> DefKind {
        match &self.kind {
            ItemKind::Fn(_) => DefKind::Fn,
            ItemKind::Struct(_) => DefKind::Struct,
            ItemKind::Enum(_) => DefKind::Enum,
            ItemKind::TypeAlias { .. } => DefKind::TypeAlias,
            ItemKind::Const { .. } => DefKind::Const,
            ItemKind::Static { .. } => DefKind::Static,
            ItemKind::Trait { .. } => DefKind::Trait,
            ItemKind::Impl { .. } => DefKind::AssocFn, // Simplification
            ItemKind::Effect { .. } => DefKind::Effect,
            ItemKind::Handler { .. } => DefKind::Handler,
        }
    }
}

/// The kind of an item.
#[derive(Debug, Clone)]
pub enum ItemKind {
    /// A function.
    Fn(FnDef),
    /// A struct.
    Struct(StructDef),
    /// An enum.
    Enum(EnumDef),
    /// A type alias.
    TypeAlias {
        generics: Generics,
        ty: Type,
    },
    /// A constant.
    Const {
        ty: Type,
        body_id: BodyId,
    },
    /// A static.
    Static {
        ty: Type,
        mutable: bool,
        body_id: BodyId,
    },
    /// A trait definition.
    Trait {
        generics: Generics,
        items: Vec<TraitItem>,
    },
    /// An impl block.
    Impl {
        generics: Generics,
        /// The trait being implemented, if any.
        trait_ref: Option<TraitRef>,
        /// The type being implemented for.
        self_ty: Type,
        items: Vec<ImplItem>,
    },
    /// An effect definition.
    Effect {
        generics: Generics,
        operations: Vec<EffectOp>,
    },
    /// A handler definition.
    Handler {
        generics: Generics,
        effect: Type,
        state: Vec<HandlerState>,
        operations: Vec<HandlerOp>,
        return_clause: Option<ReturnClause>,
    },
}

/// A function definition.
#[derive(Debug, Clone)]
pub struct FnDef {
    /// The function signature.
    pub sig: FnSig,
    /// The function body, if present.
    pub body_id: Option<BodyId>,
    /// Generic parameters.
    pub generics: Generics,
}

/// A function signature.
#[derive(Debug, Clone)]
pub struct FnSig {
    /// Parameter types.
    pub inputs: Vec<Type>,
    /// Return type.
    pub output: Type,
    /// Whether this is a const function.
    pub is_const: bool,
    /// Whether this is an async function.
    pub is_async: bool,
    /// Whether this is an unsafe function.
    pub is_unsafe: bool,
}

impl FnSig {
    /// Create a simple function signature.
    pub fn new(inputs: Vec<Type>, output: Type) -> Self {
        Self {
            inputs,
            output,
            is_const: false,
            is_async: false,
            is_unsafe: false,
        }
    }
}

/// A struct definition.
#[derive(Debug, Clone)]
pub struct StructDef {
    /// Generic parameters.
    pub generics: Generics,
    /// The kind of struct.
    pub kind: StructKind,
}

/// The kind of struct.
#[derive(Debug, Clone)]
pub enum StructKind {
    /// A struct with named fields.
    Record(Vec<FieldDef>),
    /// A tuple struct.
    Tuple(Vec<FieldDef>),
    /// A unit struct.
    Unit,
}

/// A struct field definition.
#[derive(Debug, Clone)]
pub struct FieldDef {
    /// Field index.
    pub index: u32,
    /// Field name (None for tuple structs).
    pub name: Option<String>,
    /// Field type.
    pub ty: Type,
    /// Visibility.
    pub vis: Visibility,
    /// Source span.
    pub span: Span,
}

/// An enum definition.
#[derive(Debug, Clone)]
pub struct EnumDef {
    /// Generic parameters.
    pub generics: Generics,
    /// The variants.
    pub variants: Vec<Variant>,
}

/// An enum variant.
#[derive(Debug, Clone)]
pub struct Variant {
    /// Variant index.
    pub index: u32,
    /// Variant name.
    pub name: String,
    /// Variant fields.
    pub fields: StructKind,
    /// The DefId for this variant.
    pub def_id: DefId,
    /// Source span.
    pub span: Span,
}

/// Generic parameters.
#[derive(Debug, Clone, Default)]
pub struct Generics {
    /// Type parameters.
    pub params: Vec<GenericParam>,
    /// Where predicates.
    pub predicates: Vec<WherePredicate>,
}

impl Generics {
    /// Create empty generics.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Check if there are no generic parameters.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }
}

/// A generic parameter.
#[derive(Debug, Clone)]
pub struct GenericParam {
    /// The DefId for this parameter.
    pub def_id: DefId,
    /// The parameter name.
    pub name: String,
    /// The kind of parameter.
    pub kind: GenericParamKind,
    /// Source span.
    pub span: Span,
}

/// The kind of generic parameter.
#[derive(Debug, Clone)]
pub enum GenericParamKind {
    /// A type parameter.
    Type {
        default: Option<Type>,
    },
    /// A lifetime parameter.
    Lifetime,
    /// A const parameter.
    Const {
        ty: Type,
    },
}

/// A where predicate.
#[derive(Debug, Clone)]
pub struct WherePredicate {
    /// The type being constrained.
    pub ty: Type,
    /// The bounds.
    pub bounds: Vec<TraitRef>,
    /// Source span.
    pub span: Span,
}

/// A reference to a trait.
#[derive(Debug, Clone)]
pub struct TraitRef {
    /// The trait being referenced.
    pub def_id: DefId,
    /// Type arguments.
    pub args: Vec<Type>,
}

/// A trait item.
#[derive(Debug, Clone)]
pub struct TraitItem {
    /// The DefId of this item.
    pub def_id: DefId,
    /// The name.
    pub name: String,
    /// The kind of trait item.
    pub kind: TraitItemKind,
    /// Source span.
    pub span: Span,
}

/// The kind of trait item.
#[derive(Debug, Clone)]
pub enum TraitItemKind {
    /// A method.
    Fn(FnSig, Option<BodyId>),
    /// An associated type.
    Type(Option<Type>),
    /// An associated constant.
    Const(Type, Option<BodyId>),
}

/// An impl item.
#[derive(Debug, Clone)]
pub struct ImplItem {
    /// The DefId of this item.
    pub def_id: DefId,
    /// The name.
    pub name: String,
    /// The kind of impl item.
    pub kind: ImplItemKind,
    /// Source span.
    pub span: Span,
}

/// The kind of impl item.
#[derive(Debug, Clone)]
pub enum ImplItemKind {
    /// A method.
    Fn(FnSig, BodyId),
    /// An associated type.
    Type(Type),
    /// An associated constant.
    Const(Type, BodyId),
}

/// An effect operation.
#[derive(Debug, Clone)]
pub struct EffectOp {
    /// The DefId of this operation.
    pub def_id: DefId,
    /// The name.
    pub name: String,
    /// Parameter types.
    pub inputs: Vec<Type>,
    /// Return type.
    pub output: Type,
    /// Source span.
    pub span: Span,
}

/// Handler state variable.
#[derive(Debug, Clone)]
pub struct HandlerState {
    /// The name.
    pub name: String,
    /// The type.
    pub ty: Type,
    /// Whether mutable.
    pub mutable: bool,
    /// Default value body, if any.
    pub default: Option<BodyId>,
}

/// Handler operation implementation.
#[derive(Debug, Clone)]
pub struct HandlerOp {
    /// The name (must match an effect operation).
    pub name: String,
    /// The body.
    pub body_id: BodyId,
    /// Source span.
    pub span: Span,
}

/// Return clause for transforming the final result of a handled computation.
///
/// The return clause is called with the result of the handled computation
/// and can transform it before returning to the caller.
#[derive(Debug, Clone)]
pub struct ReturnClause {
    /// The parameter name for the result value.
    pub param: String,
    /// The transformation body.
    pub body_id: BodyId,
    /// Source span.
    pub span: Span,
}
