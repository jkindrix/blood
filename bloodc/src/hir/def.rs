//! Definition identifiers for HIR.
//!
//! Blood uses two kinds of identifiers:
//! - [`DefId`] - Global identifier for items (functions, structs, etc.)
//! - [`LocalId`] - Local identifier for variables within a body

use std::fmt;

/// A globally unique identifier for a definition (item).
///
/// DefIds are assigned during name resolution and remain stable
/// throughout compilation. They serve as keys in the HIR item map.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId {
    /// Crate-local index for this definition.
    pub index: u32,
}

impl DefId {
    /// Create a new DefId with the given index.
    pub const fn new(index: u32) -> Self {
        Self { index }
    }

    /// The index of this definition.
    pub const fn index(self) -> u32 {
        self.index
    }
}

impl fmt::Debug for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DefId({})", self.index)
    }
}

impl fmt::Display for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "def{}", self.index)
    }
}

/// A local variable identifier within a function body.
///
/// LocalIds are unique within a single Body and are assigned
/// sequentially during HIR lowering.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId {
    /// Body-local index for this variable.
    pub index: u32,
}

impl LocalId {
    /// Create a new LocalId with the given index.
    pub const fn new(index: u32) -> Self {
        Self { index }
    }

    /// The index of this local.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The special "return place" local (always index 0).
    pub const RETURN_PLACE: LocalId = LocalId { index: 0 };
}

impl fmt::Debug for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalId({})", self.index)
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.index == 0 {
            write!(f, "_ret")
        } else {
            write!(f, "_{}", self.index)
        }
    }
}

/// The kind of definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefKind {
    /// A function or method.
    Fn,
    /// A struct type.
    Struct,
    /// An enum type.
    Enum,
    /// A type alias.
    TypeAlias,
    /// A trait.
    Trait,
    /// An effect.
    Effect,
    /// A handler.
    Handler,
    /// A constant.
    Const,
    /// A static.
    Static,
    /// An associated item in an impl block.
    AssocFn,
    /// An associated type.
    AssocType,
    /// An associated constant.
    AssocConst,
    /// A closure.
    Closure,
    /// A type parameter.
    TyParam,
    /// A lifetime parameter.
    LifetimeParam,
    /// A const parameter.
    ConstParam,
    /// A local variable (for lookups).
    Local,
    /// An effect operation.
    EffectOp,
    /// An enum variant.
    Variant,
    /// A struct field.
    Field,
    /// A module.
    Mod,
}

impl DefKind {
    /// Returns the article to use before this kind (e.g., "a" or "an").
    pub fn article(&self) -> &'static str {
        match self {
            DefKind::AssocFn
            | DefKind::AssocType
            | DefKind::AssocConst
            | DefKind::Enum
            | DefKind::Effect
            | DefKind::EffectOp => "an",
            _ => "a",
        }
    }

    /// Returns the name of this kind for diagnostics.
    pub fn descr(&self) -> &'static str {
        match self {
            DefKind::Fn => "function",
            DefKind::Struct => "struct",
            DefKind::Enum => "enum",
            DefKind::TypeAlias => "type alias",
            DefKind::Trait => "trait",
            DefKind::Effect => "effect",
            DefKind::Handler => "handler",
            DefKind::Const => "constant",
            DefKind::Static => "static",
            DefKind::AssocFn => "associated function",
            DefKind::AssocType => "associated type",
            DefKind::AssocConst => "associated constant",
            DefKind::Closure => "closure",
            DefKind::TyParam => "type parameter",
            DefKind::LifetimeParam => "lifetime parameter",
            DefKind::ConstParam => "const parameter",
            DefKind::Local => "local variable",
            DefKind::EffectOp => "effect operation",
            DefKind::Variant => "variant",
            DefKind::Field => "field",
            DefKind::Mod => "module",
        }
    }
}

/// A resolution result for a name lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Res {
    /// Resolved to a definition.
    Def(DefKind, DefId),
    /// Resolved to a local variable.
    Local(LocalId),
    /// Resolved to a primitive type.
    PrimTy(PrimTyRes),
    /// Unresolved (error).
    Err,
}

/// Primitive type resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimTyRes {
    Int(IntTy),
    Uint(UintTy),
    Float(FloatTy),
    Bool,
    Char,
    Str,
    Never,
}

/// Signed integer types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
}

/// Unsigned integer types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UintTy {
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}

/// Floating-point types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatTy {
    F32,
    F64,
}
