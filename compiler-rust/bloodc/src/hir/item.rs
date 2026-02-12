//! HIR items (top-level declarations).
//!
//! This module defines the HIR representation of top-level items:
//! functions, structs, enums, type aliases, etc.

use crate::ast::Visibility;
use crate::span::Span;
use super::{DefId, DefKind, Type, BodyId, TyVarId};

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
            ItemKind::ExternFn(_) => DefKind::Fn, // External functions are still functions
            ItemKind::Bridge(_) => DefKind::Struct, // Bridge as a module-like namespace
            ItemKind::Module(_) => DefKind::Mod,
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
        /// Handler kind: deep (reify continuation) or shallow (single shot).
        kind: HandlerKind,
        effect: Type,
        state: Vec<HandlerState>,
        operations: Vec<HandlerOp>,
        return_clause: Option<ReturnClause>,
    },
    /// An external (FFI) function.
    ExternFn(ExternFnDef),
    /// An FFI bridge block.
    Bridge(BridgeDef),
    /// A module.
    Module(ModuleDef),
}

/// A module definition.
#[derive(Debug, Clone)]
pub struct ModuleDef {
    /// Items contained within this module.
    pub items: Vec<DefId>,
    /// Whether this is an external module (loaded from file) or inline.
    pub is_external: bool,
    /// Re-exported items via `pub use`.
    /// Each entry is (local_name, original_def_id, visibility).
    pub reexports: Vec<(String, DefId, Visibility)>,
}

/// Handler kind: determines continuation semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandlerKind {
    /// Deep handler: reifies the continuation, allowing multiple resumes.
    #[default]
    Deep,
    /// Shallow handler: single-shot continuation, more efficient but restricted.
    Shallow,
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
    /// Generic type parameters (TyVarIds for each type parameter).
    pub generics: Vec<TyVarId>,
    /// Const generic parameters (ConstParamIds for each const parameter).
    pub const_generics: Vec<super::ty::ConstParamId>,
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
            generics: Vec::new(),
            const_generics: Vec::new(),
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

/// Variance annotation for a type parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VarianceAnnotation {
    /// No explicit annotation - variance will be inferred.
    #[default]
    Inferred,
    /// Explicitly covariant (`+T`).
    Covariant,
    /// Explicitly contravariant (`-T`).
    Contravariant,
    /// Explicitly invariant (`=T`).
    Invariant,
}

impl VarianceAnnotation {
    /// Returns true if variance should be inferred.
    pub fn is_inferred(&self) -> bool {
        matches!(self, VarianceAnnotation::Inferred)
    }

    /// Parse from annotation string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "+" => Some(VarianceAnnotation::Covariant),
            "-" => Some(VarianceAnnotation::Contravariant),
            "=" => Some(VarianceAnnotation::Invariant),
            _ => None,
        }
    }

    /// Get the annotation character.
    pub fn as_str(&self) -> &'static str {
        match self {
            VarianceAnnotation::Inferred => "",
            VarianceAnnotation::Covariant => "+",
            VarianceAnnotation::Contravariant => "-",
            VarianceAnnotation::Invariant => "=",
        }
    }
}

/// The kind of generic parameter.
#[derive(Debug, Clone)]
pub enum GenericParamKind {
    /// A type parameter.
    Type {
        /// Default type if any.
        default: Option<Type>,
        /// Explicit variance annotation (if provided).
        variance: VarianceAnnotation,
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

// ============================================================
// FFI Definitions
// ============================================================

/// An external (foreign) function definition.
#[derive(Debug, Clone)]
pub struct ExternFnDef {
    /// The function signature.
    pub sig: FnSig,
    /// The ABI/language (e.g., "C", "C++", "wasm").
    pub abi: String,
    /// The symbol name to link against (may differ from Rust name).
    pub link_name: Option<String>,
    /// Whether this function is variadic.
    pub is_variadic: bool,
}

/// A bridge block definition containing multiple FFI items.
#[derive(Debug, Clone)]
pub struct BridgeDef {
    /// The ABI/language for this bridge (e.g., "C", "C++", "wasm").
    pub abi: String,
    /// Link specifications for this bridge.
    pub link_specs: Vec<LinkSpec>,
    /// External functions in this bridge.
    pub extern_fns: Vec<ExternFnItem>,
    /// Opaque types (forward declarations).
    pub opaque_types: Vec<OpaqueType>,
    /// Type aliases.
    pub type_aliases: Vec<BridgeTypeAlias>,
    /// FFI structs.
    pub structs: Vec<FfiStruct>,
    /// FFI enums.
    pub enums: Vec<FfiEnum>,
    /// FFI unions.
    pub unions: Vec<FfiUnion>,
    /// FFI constants.
    pub consts: Vec<FfiConst>,
    /// Callback type definitions.
    pub callbacks: Vec<FfiCallback>,
}

/// Link specification for an FFI bridge.
#[derive(Debug, Clone)]
pub struct LinkSpec {
    /// The library name to link.
    pub name: String,
    /// The link kind (dylib, static, framework).
    pub kind: LinkKind,
    /// WASM import module name.
    pub wasm_import_module: Option<String>,
}

/// The kind of library linking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkKind {
    /// Dynamic library (default).
    #[default]
    Dylib,
    /// Static library.
    Static,
    /// macOS framework.
    Framework,
}

/// An external function item within a bridge.
#[derive(Debug, Clone)]
pub struct ExternFnItem {
    /// The DefId for this function.
    pub def_id: DefId,
    /// The function name.
    pub name: String,
    /// The function signature.
    pub sig: FnSig,
    /// Optional link name (if different from `name`).
    pub link_name: Option<String>,
    /// Whether this function is variadic.
    pub is_variadic: bool,
    /// Source span.
    pub span: Span,
}

/// An opaque type (forward declaration).
#[derive(Debug, Clone)]
pub struct OpaqueType {
    /// The DefId for this type.
    pub def_id: DefId,
    /// The type name.
    pub name: String,
    /// Source span.
    pub span: Span,
}

/// A type alias in a bridge.
#[derive(Debug, Clone)]
pub struct BridgeTypeAlias {
    /// The DefId for this type.
    pub def_id: DefId,
    /// The type name.
    pub name: String,
    /// The aliased type.
    pub ty: Type,
    /// Source span.
    pub span: Span,
}

/// An FFI struct (C-compatible layout).
#[derive(Debug, Clone)]
pub struct FfiStruct {
    /// The DefId for this struct.
    pub def_id: DefId,
    /// The struct name.
    pub name: String,
    /// The fields.
    pub fields: Vec<FfiField>,
    /// Whether this is packed.
    pub is_packed: bool,
    /// Explicit alignment (if specified).
    pub align: Option<u32>,
    /// Source span.
    pub span: Span,
}

/// A field in an FFI struct or union.
#[derive(Debug, Clone)]
pub struct FfiField {
    /// The field name.
    pub name: String,
    /// The field type.
    pub ty: Type,
    /// Source span.
    pub span: Span,
}

/// An FFI enum (C-compatible).
#[derive(Debug, Clone)]
pub struct FfiEnum {
    /// The DefId for this enum.
    pub def_id: DefId,
    /// The enum name.
    pub name: String,
    /// The representation type.
    pub repr: Type,
    /// The variants.
    pub variants: Vec<FfiEnumVariant>,
    /// Source span.
    pub span: Span,
}

/// A variant in an FFI enum.
#[derive(Debug, Clone)]
pub struct FfiEnumVariant {
    /// The variant name.
    pub name: String,
    /// The discriminant value.
    pub value: i64,
    /// Source span.
    pub span: Span,
}

/// An FFI union (C-compatible).
#[derive(Debug, Clone)]
pub struct FfiUnion {
    /// The DefId for this union.
    pub def_id: DefId,
    /// The union name.
    pub name: String,
    /// The fields (all at the same offset).
    pub fields: Vec<FfiField>,
    /// Source span.
    pub span: Span,
}

/// An FFI constant.
#[derive(Debug, Clone)]
pub struct FfiConst {
    /// The DefId for this constant.
    pub def_id: DefId,
    /// The constant name.
    pub name: String,
    /// The constant type.
    pub ty: Type,
    /// The constant value (as i64 for simplicity).
    pub value: i64,
    /// Source span.
    pub span: Span,
}

/// A callback type definition.
#[derive(Debug, Clone)]
pub struct FfiCallback {
    /// The DefId for this callback type.
    pub def_id: DefId,
    /// The callback name.
    pub name: String,
    /// Parameter types.
    pub params: Vec<Type>,
    /// Return type.
    pub return_type: Type,
    /// Source span.
    pub span: Span,
}
