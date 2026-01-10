//! Type representation for HIR.
//!
//! This module defines the semantic type system for Blood. Unlike the AST
//! `Type` which represents the syntactic form, these types are fully resolved
//! and normalized.
//!
//! # Type Structure
//!
//! Blood's type system includes:
//! - **Primitive types**: `i32`, `f64`, `bool`, `char`, `str`
//! - **Composite types**: tuples, arrays, slices
//! - **Nominal types**: structs, enums (referenced by DefId)
//! - **Function types**: `fn(A, B) -> C`
//! - **Reference types**: `&T`, `&mut T`
//! - **Type variables**: for inference and generics
//!
//! # Phase 1 Scope
//!
//! Phase 1 implements a subset without:
//! - Effect rows (added in Phase 2)
//! - Generational pointers (added in Phase 3)
//! - Linear/affine types (added in Phase 3)

use std::fmt;
use std::sync::Arc;

use super::DefId;
use super::def::{IntTy, UintTy, FloatTy};

/// The unique identifier for a type variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyVarId(pub u32);

impl TyVarId {
    /// Create a new type variable ID.
    pub const fn new(id: u32) -> Self {
        TyVarId(id)
    }
}

/// A semantic type in Blood.
///
/// Types are interned and compared by identity. The `Arc` wrapper allows
/// efficient cloning and sharing.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Type {
    pub kind: Arc<TypeKind>,
}

impl Type {
    /// Create a new type from a kind.
    pub fn new(kind: TypeKind) -> Self {
        Self {
            kind: Arc::new(kind),
        }
    }

    /// Get the kind of this type.
    pub fn kind(&self) -> &TypeKind {
        &self.kind
    }

    /// Check if this type is a primitive.
    pub fn is_primitive(&self) -> bool {
        matches!(self.kind(), TypeKind::Primitive(_))
    }

    /// Check if this type is the unit type.
    pub fn is_unit(&self) -> bool {
        matches!(self.kind(), TypeKind::Tuple(tys) if tys.is_empty())
    }

    /// Check if this type is the never type.
    pub fn is_never(&self) -> bool {
        matches!(self.kind(), TypeKind::Never)
    }

    /// Check if this type is a reference.
    pub fn is_ref(&self) -> bool {
        matches!(self.kind(), TypeKind::Ref { .. })
    }

    /// Check if this type is a function.
    pub fn is_fn(&self) -> bool {
        matches!(self.kind(), TypeKind::Fn { .. })
    }

    /// Check if this type is an error type.
    pub fn is_error(&self) -> bool {
        matches!(self.kind(), TypeKind::Error)
    }

    /// Check if this type contains any type variables.
    pub fn has_type_vars(&self) -> bool {
        match self.kind() {
            TypeKind::Infer(_) | TypeKind::Param(_) => true,
            TypeKind::Primitive(_) | TypeKind::Never | TypeKind::Error => false,
            TypeKind::Tuple(tys) => tys.iter().any(|t| t.has_type_vars()),
            TypeKind::Array { element, .. } => element.has_type_vars(),
            TypeKind::Slice { element } => element.has_type_vars(),
            TypeKind::Ref { inner, .. } => inner.has_type_vars(),
            TypeKind::Ptr { inner, .. } => inner.has_type_vars(),
            TypeKind::Fn { params, ret, .. } => {
                params.iter().any(|t| t.has_type_vars()) || ret.has_type_vars()
            }
            TypeKind::Closure { params, ret, .. } => {
                params.iter().any(|t| t.has_type_vars()) || ret.has_type_vars()
            }
            TypeKind::Adt { args, .. } => args.iter().any(|t| t.has_type_vars()),
        }
    }

    // Convenience constructors for common types

    /// Create the unit type `()`.
    pub fn unit() -> Self {
        Self::new(TypeKind::Tuple(Vec::new()))
    }

    /// Create the never type `!`.
    pub fn never() -> Self {
        Self::new(TypeKind::Never)
    }

    /// Create an error type (used for error recovery).
    pub fn error() -> Self {
        Self::new(TypeKind::Error)
    }

    /// Create a boolean type.
    pub fn bool() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Bool))
    }

    /// Create an i32 type.
    pub fn i32() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I32)))
    }

    /// Create an i64 type.
    pub fn i64() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I64)))
    }

    /// Create a u32 type.
    pub fn u32() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U32)))
    }

    /// Create a u64 type.
    pub fn u64() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U64)))
    }

    /// Create an f32 type.
    pub fn f32() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Float(FloatTy::F32)))
    }

    /// Create an f64 type.
    pub fn f64() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Float(FloatTy::F64)))
    }

    /// Create a char type.
    pub fn char() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Char))
    }

    /// Create a str type (borrowed string slice).
    pub fn str() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Str))
    }

    /// Create a String type (owned string).
    pub fn string() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::String))
    }

    /// Create an inference variable.
    pub fn infer(id: TyVarId) -> Self {
        Self::new(TypeKind::Infer(id))
    }

    /// Create a type parameter.
    pub fn param(id: TyVarId) -> Self {
        Self::new(TypeKind::Param(id))
    }

    /// Create a reference type.
    pub fn reference(inner: Type, mutable: bool) -> Self {
        Self::new(TypeKind::Ref { inner, mutable })
    }

    /// Create an array type.
    pub fn array(element: Type, size: u64) -> Self {
        Self::new(TypeKind::Array { element, size })
    }

    /// Create a slice type.
    pub fn slice(element: Type) -> Self {
        Self::new(TypeKind::Slice { element })
    }

    /// Create a tuple type.
    pub fn tuple(elements: Vec<Type>) -> Self {
        Self::new(TypeKind::Tuple(elements))
    }

    /// Create a function type.
    pub fn function(params: Vec<Type>, ret: Type) -> Self {
        Self::new(TypeKind::Fn { params, ret })
    }

    /// Create an ADT (struct/enum) type.
    pub fn adt(def_id: DefId, args: Vec<Type>) -> Self {
        Self::new(TypeKind::Adt { def_id, args })
    }
}

impl fmt::Debug for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.kind)
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

/// The kind of a type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    /// A primitive type: `i32`, `f64`, `bool`, etc.
    Primitive(PrimitiveTy),

    /// A tuple type: `()`, `(T,)`, `(T, U)`
    Tuple(Vec<Type>),

    /// An array type: `[T; N]`
    Array { element: Type, size: u64 },

    /// A slice type: `[T]`
    Slice { element: Type },

    /// A reference type: `&T`, `&mut T`
    Ref { inner: Type, mutable: bool },

    /// A raw pointer type: `*const T`, `*mut T`
    Ptr { inner: Type, mutable: bool },

    /// A function type: `fn(A, B) -> C`
    Fn { params: Vec<Type>, ret: Type },

    /// A closure type.
    ///
    /// Unlike regular function types, closures capture their environment
    /// and have a specific DefId identifying the closure function.
    Closure {
        /// The DefId of the closure function.
        def_id: DefId,
        /// The parameter types.
        params: Vec<Type>,
        /// The return type.
        ret: Type,
    },

    /// An algebraic data type (struct or enum).
    Adt {
        /// The definition ID of the type.
        def_id: DefId,
        /// Type arguments (for generic types).
        args: Vec<Type>,
    },

    /// A type variable for inference.
    Infer(TyVarId),

    /// A type parameter (generic).
    Param(TyVarId),

    /// The never type: `!`
    Never,

    /// An error type (for error recovery).
    Error,
}

impl fmt::Display for TypeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeKind::Primitive(p) => write!(f, "{p}"),
            TypeKind::Tuple(tys) if tys.is_empty() => write!(f, "()"),
            TypeKind::Tuple(tys) if tys.len() == 1 => write!(f, "({},)", tys[0]),
            TypeKind::Tuple(tys) => {
                write!(f, "(")?;
                for (i, ty) in tys.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{ty}")?;
                }
                write!(f, ")")
            }
            TypeKind::Array { element, size } => write!(f, "[{element}; {size}]"),
            TypeKind::Slice { element } => write!(f, "[{element}]"),
            TypeKind::Ref { inner, mutable: false } => write!(f, "&{inner}"),
            TypeKind::Ref { inner, mutable: true } => write!(f, "&mut {inner}"),
            TypeKind::Ptr { inner, mutable: false } => write!(f, "*const {inner}"),
            TypeKind::Ptr { inner, mutable: true } => write!(f, "*mut {inner}"),
            TypeKind::Fn { params, ret } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            TypeKind::Closure { def_id: _, params, ret } => {
                write!(f, "|")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, "| -> {ret}")
            }
            TypeKind::Adt { def_id, args } if args.is_empty() => {
                write!(f, "{def_id}")
            }
            TypeKind::Adt { def_id, args } => {
                write!(f, "{def_id}<")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ">")
            }
            TypeKind::Infer(id) => write!(f, "?{}", id.0),
            TypeKind::Param(id) => write!(f, "T{}", id.0),
            TypeKind::Never => write!(f, "!"),
            TypeKind::Error => write!(f, "{{error}}"),
        }
    }
}

/// A primitive type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTy {
    /// Signed integers: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`
    Int(IntTy),
    /// Unsigned integers: `u8`, `u16`, `u32`, `u64`, `u128`, `usize`
    Uint(UintTy),
    /// Floating-point: `f32`, `f64`
    Float(FloatTy),
    /// Boolean: `bool`
    Bool,
    /// Character: `char`
    Char,
    /// String slice: `str`
    Str,
    /// Owned String: `String`
    String,
    /// Unit type: `()`
    Unit,
}

impl fmt::Display for PrimitiveTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrimitiveTy::Int(int_ty) => write!(f, "{int_ty}"),
            PrimitiveTy::Uint(uint_ty) => write!(f, "{uint_ty}"),
            PrimitiveTy::Float(float_ty) => write!(f, "{float_ty}"),
            PrimitiveTy::Bool => write!(f, "bool"),
            PrimitiveTy::Char => write!(f, "char"),
            PrimitiveTy::Str => write!(f, "str"),
            PrimitiveTy::String => write!(f, "String"),
            PrimitiveTy::Unit => write!(f, "()"),
        }
    }
}

impl fmt::Display for IntTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            IntTy::I8 => "i8",
            IntTy::I16 => "i16",
            IntTy::I32 => "i32",
            IntTy::I64 => "i64",
            IntTy::I128 => "i128",
            IntTy::Isize => "isize",
        };
        write!(f, "{s}")
    }
}

impl fmt::Display for UintTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            UintTy::U8 => "u8",
            UintTy::U16 => "u16",
            UintTy::U32 => "u32",
            UintTy::U64 => "u64",
            UintTy::U128 => "u128",
            UintTy::Usize => "usize",
        };
        write!(f, "{s}")
    }
}

impl fmt::Display for FloatTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            FloatTy::F32 => "f32",
            FloatTy::F64 => "f64",
        };
        write!(f, "{s}")
    }
}

impl PrimitiveTy {
    /// Get the primitive type from a name, if valid.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "i8" => PrimitiveTy::Int(IntTy::I8),
            "i16" => PrimitiveTy::Int(IntTy::I16),
            "i32" => PrimitiveTy::Int(IntTy::I32),
            "i64" => PrimitiveTy::Int(IntTy::I64),
            "i128" => PrimitiveTy::Int(IntTy::I128),
            "isize" => PrimitiveTy::Int(IntTy::Isize),
            "u8" => PrimitiveTy::Uint(UintTy::U8),
            "u16" => PrimitiveTy::Uint(UintTy::U16),
            "u32" => PrimitiveTy::Uint(UintTy::U32),
            "u64" => PrimitiveTy::Uint(UintTy::U64),
            "u128" => PrimitiveTy::Uint(UintTy::U128),
            "usize" => PrimitiveTy::Uint(UintTy::Usize),
            "f32" => PrimitiveTy::Float(FloatTy::F32),
            "f64" => PrimitiveTy::Float(FloatTy::F64),
            "bool" => PrimitiveTy::Bool,
            "char" => PrimitiveTy::Char,
            "str" => PrimitiveTy::Str,
            "String" => PrimitiveTy::String,
            "unit" => PrimitiveTy::Unit,
            _ => return None,
        })
    }

    /// Check if this is a numeric type (int, uint, or float).
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            PrimitiveTy::Int(_) | PrimitiveTy::Uint(_) | PrimitiveTy::Float(_)
        )
    }

    /// Check if this is an integer type (signed or unsigned).
    pub fn is_integer(&self) -> bool {
        matches!(self, PrimitiveTy::Int(_) | PrimitiveTy::Uint(_))
    }

    /// Check if this is a floating-point type.
    pub fn is_float(&self) -> bool {
        matches!(self, PrimitiveTy::Float(_))
    }

    /// Get the size in bits for integer/float types.
    pub fn bit_width(&self) -> Option<u32> {
        Some(match self {
            PrimitiveTy::Int(IntTy::I8) | PrimitiveTy::Uint(UintTy::U8) => 8,
            PrimitiveTy::Int(IntTy::I16) | PrimitiveTy::Uint(UintTy::U16) => 16,
            PrimitiveTy::Int(IntTy::I32) | PrimitiveTy::Uint(UintTy::U32) => 32,
            PrimitiveTy::Int(IntTy::I64) | PrimitiveTy::Uint(UintTy::U64) => 64,
            PrimitiveTy::Int(IntTy::I128) | PrimitiveTy::Uint(UintTy::U128) => 128,
            PrimitiveTy::Int(IntTy::Isize) | PrimitiveTy::Uint(UintTy::Usize) => 64, // Assume 64-bit
            PrimitiveTy::Float(FloatTy::F32) => 32,
            PrimitiveTy::Float(FloatTy::F64) => 64,
            PrimitiveTy::Bool => 1,
            PrimitiveTy::Char => 32,
            PrimitiveTy::Str | PrimitiveTy::String => return None, // Unsized/heap-allocated
            PrimitiveTy::Unit => 0, // Zero-sized type
        })
    }
}
