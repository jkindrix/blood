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

use std::collections::HashMap;
pub use def::{DefId, LocalId, DefKind, Res, PrimTyRes, IntTy, UintTy, FloatTy};
pub use expr::{Expr, ExprKind, Body, Local, BodyId, Stmt, LiteralValue, MatchArm, Pattern, PatternKind, FieldPattern, LoopId, Capture, FieldExpr};
pub use item::{Item, ItemKind, FnSig, FnDef, StructDef, StructKind, FieldDef, EnumDef, Variant, Generics, GenericParam, GenericParamKind, TraitRef, WherePredicate, TraitItem, TraitItemKind, ImplItem, ImplItemKind, EffectOp, HandlerState, HandlerOp, ReturnClause};
pub use ty::{Type, TypeKind, PrimitiveTy, TyVarId};

/// A compilation unit (crate) in HIR form.
#[derive(Debug, Clone)]
pub struct Crate {
    /// All items in the crate, indexed by DefId.
    pub items: HashMap<DefId, Item>,
    /// All function/closure bodies, indexed by BodyId.
    pub bodies: HashMap<BodyId, Body>,
    /// The entry point (main function), if present.
    pub entry: Option<DefId>,
}

impl Crate {
    /// Create an empty crate.
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
            bodies: HashMap::new(),
            entry: None,
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
