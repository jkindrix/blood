//! High-level Intermediate Representation (HIR) for Blood.
//!
//! The HIR is a simplified, typed representation of the AST. Key differences from AST:
//!
//! 1. **Types are resolved** - All type annotations are resolved to concrete `Type` values
//! 2. **Names are resolved** - All identifiers are resolved to `DefId` or `LocalId`
//! 3. **Desugaring** - Some syntactic sugar is expanded (e.g., `for` loops to `while`)
//! 4. **No source info** - Spans are preserved for error reporting but not for formatting
//!
//! # HIR Structure
//!
//! - [`Crate`] - Root node containing all items in a compilation unit
//! - [`Item`] - Top-level items (functions, structs, enums, etc.)
//! - [`Body`] - Function/closure body with local variables and expressions
//! - [`Expr`] - Typed expressions
//!
//! # Lowering Pipeline
//!
//! ```text
//! AST -> Name Resolution -> Type Checking -> HIR
//! ```

pub mod def;
pub mod expr;
pub mod item;
pub mod ty;

pub use def::{DefId, DefKind, FloatTy, IntTy, LocalId, PrimTyRes, Res, UintTy};
pub use expr::{
    Body, BodyId, Capture, Expr, ExprKind, FieldExpr, FieldPattern, InlineOpHandler, LiteralValue,
    Local, LoopId, MatchArm, Pattern, PatternKind, RecordFieldExpr, Stmt,
};
pub use item::{
    BridgeDef,
    BridgeTypeAlias,
    EffectOp,
    EnumDef,
    // FFI types
    ExternFnDef,
    ExternFnItem,
    FfiCallback,
    FfiConst,
    FfiEnum,
    FfiEnumVariant,
    FfiField,
    FfiStruct,
    FfiUnion,
    FieldDef,
    FinallyClause,
    FnDef,
    FnSig,
    GenericParam,
    GenericParamKind,
    Generics,
    HandlerKind,
    HandlerOp,
    HandlerState,
    ImplItem,
    ImplItemKind,
    Item,
    ItemKind,
    LinkKind,
    LinkSpec,
    // Module
    ModuleDef,
    OpaqueType,
    ReturnClause,
    StructDef,
    StructKind,
    TraitItem,
    TraitItemKind,
    TraitRef,
    VarianceAnnotation,
    Variant,
    WherePredicate,
};
use std::collections::HashMap;
pub use ty::{
    ConstParamId, ConstValue, FnEffect, GenericArg, LifetimeId, PrimitiveTy, RecordField,
    RecordRowVarId, TyVarId, Type, TypeKind,
};

/// Information about a trait implementation, used for vtable generation.
#[derive(Debug, Clone)]
pub struct TraitImplInfo {
    /// The trait being implemented.
    pub trait_id: DefId,
    /// The concrete type implementing the trait.
    pub self_ty: Type,
    /// Method implementations: (method_name, impl_method_def_id).
    pub methods: Vec<(String, DefId)>,
}

/// A compilation unit (crate) in HIR form.
#[derive(Debug, Clone)]
pub struct Crate {
    /// All items in the crate, indexed by DefId.
    pub items: HashMap<DefId, Item>,
    /// All function/closure bodies, indexed by BodyId.
    pub bodies: HashMap<BodyId, Body>,
    /// The entry point (main function), if present.
    pub entry: Option<DefId>,
    /// Builtin functions: DefId -> function name.
    /// These are runtime functions with no source code.
    pub builtin_fns: HashMap<DefId, String>,
    /// Trait implementations for vtable generation.
    pub trait_impls: Vec<TraitImplInfo>,
}

impl Crate {
    /// Create an empty crate.
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
            bodies: HashMap::new(),
            entry: None,
            builtin_fns: HashMap::new(),
            trait_impls: Vec::new(),
        }
    }

    /// Get an item by its DefId.
    pub fn get_item(&self, id: DefId) -> Option<&Item> {
        self.items.get(&id)
    }

    /// Get a body by its BodyId.
    pub fn get_body(&self, id: BodyId) -> Option<&Body> {
        self.bodies.get(&id)
    }
}

impl Default for Crate {
    fn default() -> Self {
        Self::new()
    }
}

/// A definition index for items and types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(pub u32);

impl From<u32> for ItemId {
    fn from(id: u32) -> Self {
        ItemId(id)
    }
}
