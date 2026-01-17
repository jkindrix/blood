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

/// An effect annotation on a function type.
///
/// Used to preserve effect type arguments in function types for monomorphization.
/// For example, `fn() / {Emit<T>}` stores `FnEffect { def_id: Emit, type_args: [T] }`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnEffect {
    /// The effect definition ID.
    pub def_id: DefId,
    /// Type arguments for parameterized effects (e.g., Emit<T>).
    pub type_args: Vec<Type>,
}

impl FnEffect {
    /// Create a new effect annotation with type arguments.
    pub fn new(def_id: DefId, type_args: Vec<Type>) -> Self {
        Self { def_id, type_args }
    }
}

/// The unique identifier for a type variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyVarId(pub u32);

impl TyVarId {
    /// Create a new type variable ID.
    pub const fn new(id: u32) -> Self {
        TyVarId(id)
    }
}

/// The unique identifier for a const parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConstParamId(pub u32);

impl ConstParamId {
    /// Create a new const parameter ID.
    pub const fn new(id: u32) -> Self {
        ConstParamId(id)
    }
}

/// The unique identifier for a lifetime parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LifetimeId(pub u32);

impl LifetimeId {
    /// Create a new lifetime ID.
    pub const fn new(id: u32) -> Self {
        LifetimeId(id)
    }

    /// The static lifetime.
    pub const STATIC: Self = LifetimeId(0);
}

/// The unique identifier for a row variable in record types.
///
/// Row variables enable row polymorphism, allowing functions to operate
/// on records with extra fields: `fn get_x(r: {x: i32 | R}) -> i32`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordRowVarId(pub u32);

impl RecordRowVarId {
    /// Create a new record row variable ID.
    pub const fn new(id: u32) -> Self {
        RecordRowVarId(id)
    }
}

/// A field in a record type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordField {
    /// The field name.
    pub name: crate::ast::Symbol,
    /// The field type.
    pub ty: Type,
}

/// Ownership qualifier for linear/affine types.
///
/// Linear types must be used exactly once, while affine types must be
/// used at most once. These qualifiers enable resource management
/// without garbage collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnershipQualifier {
    /// Linear: must be used exactly once.
    /// Violation: error if not used or used more than once.
    Linear,
    /// Affine: must be used at most once.
    /// Violation: error if used more than once (not using is OK).
    Affine,
}

/// A const value that can appear in type positions (e.g., array sizes).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstValue {
    /// A concrete integer value.
    Int(i128),
    /// A concrete unsigned integer value.
    Uint(u128),
    /// A concrete boolean value.
    Bool(bool),
    /// A const parameter (not yet evaluated).
    Param(ConstParamId),
    /// An error value (for error recovery).
    Error,
}

impl ConstValue {
    /// Try to get a concrete u64 value (for array sizes).
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            ConstValue::Int(v) if *v >= 0 => Some(*v as u64),
            ConstValue::Uint(v) if *v <= u64::MAX as u128 => Some(*v as u64),
            _ => None,
        }
    }

    /// Check if this is an unevaluated const parameter.
    pub fn is_param(&self) -> bool {
        matches!(self, ConstValue::Param(_))
    }
}

impl fmt::Display for ConstValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstValue::Int(v) => write!(f, "{}", v),
            ConstValue::Uint(v) => write!(f, "{}", v),
            ConstValue::Bool(v) => write!(f, "{}", v),
            ConstValue::Param(id) => write!(f, "const_{}", id.0),
            ConstValue::Error => write!(f, "{{const error}}"),
        }
    }
}

/// A generic argument that can be a type, const, or lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GenericArg {
    /// A type argument.
    Type(Type),
    /// A const argument.
    Const(ConstValue),
    /// A lifetime argument.
    Lifetime(LifetimeId),
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
    /// Unit can be represented as either empty tuple `()` or `Primitive(Unit)`.
    pub fn is_unit(&self) -> bool {
        match self.kind() {
            TypeKind::Tuple(tys) if tys.is_empty() => true,
            TypeKind::Primitive(PrimitiveTy::Unit) => true,
            _ => false,
        }
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

    /// Check if this type implements Copy semantics.
    ///
    /// A type is Copy if its values can be duplicated by simple bitwise copy
    /// without any special behavior. This includes:
    /// - Primitives (integers, floats, bool, char)
    /// - Unit and Never types
    /// - Tuples and arrays where all elements are Copy
    /// - Structs where all fields are Copy (requires ADT lookup)
    ///
    /// The `adt_fields` callback is used to look up field types for ADTs.
    /// It takes a DefId and returns the field types if the ADT is a struct.
    /// For enums, it should return None (enums are not Copy by default).
    pub fn is_copy<F>(&self, adt_fields: &F) -> bool
    where
        F: Fn(DefId) -> Option<Vec<Type>>,
    {
        match self.kind() {
            // Primitives are always Copy (except String which is heap-allocated)
            TypeKind::Primitive(prim) => !matches!(prim, PrimitiveTy::String),

            // Unit and Never are trivially Copy
            TypeKind::Tuple(tys) if tys.is_empty() => true,
            TypeKind::Never => true,

            // Tuples are Copy if all elements are Copy
            TypeKind::Tuple(tys) => tys.iter().all(|t| t.is_copy(adt_fields)),

            // Arrays are Copy if the element type is Copy
            TypeKind::Array { element, .. } => element.is_copy(adt_fields),

            // References are NOT Copy (they have identity and lifetime concerns)
            // Note: In Rust, &T is Copy but &mut T is not. Blood treats all refs as non-Copy
            // for simplicity in the memory model.
            TypeKind::Ref { .. } => false,

            // Raw pointers are Copy (like in C)
            TypeKind::Ptr { .. } => true,

            // Slices are NOT Copy (they are unsized)
            TypeKind::Slice { .. } => false,

            // Functions are Copy (function pointers)
            TypeKind::Fn { .. } => true,

            // Closures are NOT Copy (they capture environment)
            TypeKind::Closure { .. } => false,

            // ADTs: look up fields and check if all are Copy
            TypeKind::Adt { def_id, args: _ } => {
                // Try to get field types. If we can't look them up, assume not Copy.
                if let Some(field_types) = adt_fields(*def_id) {
                    field_types.iter().all(|t| t.is_copy(adt_fields))
                } else {
                    // Enums or unknown ADTs are not Copy
                    false
                }
            }

            // Ranges are Copy if the element is Copy
            TypeKind::Range { element, .. } => element.is_copy(adt_fields),

            // Trait objects are NOT Copy (they are unsized and have vtables)
            TypeKind::DynTrait { .. } => false,

            // Records are Copy if all fields are Copy
            TypeKind::Record { fields, row_var } => {
                // Open records (with row_var) are not Copy
                row_var.is_none() && fields.iter().all(|f| f.ty.is_copy(adt_fields))
            }

            // Forall types: check the body
            TypeKind::Forall { body, .. } => body.is_copy(adt_fields),

            // Ownership-qualified types delegate to inner
            TypeKind::Ownership { inner, .. } => inner.is_copy(adt_fields),

            // Type variables and inference vars: conservatively not Copy
            TypeKind::Infer(_) | TypeKind::Param(_) => false,

            // Error types: treat as Copy to avoid cascading errors
            TypeKind::Error => true,
        }
    }

    /// Check if this type is Copy without ADT lookup.
    ///
    /// This is a simpler version that only handles primitives and built-in types.
    /// For full Copy detection including structs, use `is_copy` with an ADT lookup.
    pub fn is_trivially_copy(&self) -> bool {
        self.is_copy(&|_| None)
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
            TypeKind::Range { element, .. } => element.has_type_vars(),
            // Trait objects don't contain type variables (only DefIds)
            TypeKind::DynTrait { .. } => false,
            // Records have type variables if any field has them or if there's a row variable
            TypeKind::Record { fields, row_var } => {
                row_var.is_some() || fields.iter().any(|f| f.ty.has_type_vars())
            }
            // Forall types bind their params, but may have free vars in the body
            TypeKind::Forall { body, .. } => body.has_type_vars(),
            // Ownership-qualified types delegate to the inner type
            TypeKind::Ownership { inner, .. } => inner.has_type_vars(),
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

    /// Create an i8 type.
    pub fn i8() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8)))
    }

    /// Create an i16 type.
    pub fn i16() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16)))
    }

    /// Create an i32 type.
    pub fn i32() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I32)))
    }

    /// Create an i64 type.
    pub fn i64() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I64)))
    }

    /// Create an i128 type.
    pub fn i128() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128)))
    }

    /// Create an isize type.
    pub fn isize() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::Isize)))
    }

    /// Create a u8 type.
    pub fn u8() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8)))
    }

    /// Create a u16 type.
    pub fn u16() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16)))
    }

    /// Create a u32 type.
    pub fn u32() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U32)))
    }

    /// Create a u64 type.
    pub fn u64() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U64)))
    }

    /// Create a u128 type.
    pub fn u128() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128)))
    }

    /// Create a usize type.
    pub fn usize() -> Self {
        Self::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::Usize)))
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

    /// Create a function type (without effects).
    pub fn function(params: Vec<Type>, ret: Type) -> Self {
        Self::new(TypeKind::Fn { params, ret, effects: Vec::new() })
    }

    /// Create a function type with effect annotations.
    pub fn function_with_effects(params: Vec<Type>, ret: Type, effects: Vec<FnEffect>) -> Self {
        Self::new(TypeKind::Fn { params, ret, effects })
    }

    /// Create an ADT (struct/enum) type.
    pub fn adt(def_id: DefId, args: Vec<Type>) -> Self {
        Self::new(TypeKind::Adt { def_id, args })
    }

    /// Create a trait object type: `dyn Trait` or `dyn Trait + Send + Sync`.
    pub fn dyn_trait(trait_id: DefId, auto_traits: Vec<DefId>) -> Self {
        Self::new(TypeKind::DynTrait { trait_id, auto_traits })
    }

    /// Create a record type with known fields and optional row variable.
    pub fn record(fields: Vec<RecordField>, row_var: Option<RecordRowVarId>) -> Self {
        Self::new(TypeKind::Record { fields, row_var })
    }

    /// Create a universally quantified (forall) type.
    pub fn forall(params: Vec<TyVarId>, body: Type) -> Self {
        Self::new(TypeKind::Forall { params, body: Box::new(body) })
    }

    /// Create a linear type (must be used exactly once).
    pub fn linear(inner: Type) -> Self {
        Self::new(TypeKind::Ownership {
            qualifier: OwnershipQualifier::Linear,
            inner,
        })
    }

    /// Create an affine type (must be used at most once).
    pub fn affine(inner: Type) -> Self {
        Self::new(TypeKind::Ownership {
            qualifier: OwnershipQualifier::Affine,
            inner,
        })
    }

    /// Create an ownership-qualified type with a specific qualifier.
    pub fn ownership(qualifier: OwnershipQualifier, inner: Type) -> Self {
        Self::new(TypeKind::Ownership { qualifier, inner })
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

    /// A function type: `fn(A, B) -> C` or `fn(A, B) -> C / {E1, E2}`
    Fn {
        params: Vec<Type>,
        ret: Type,
        /// Effect annotations on this function type.
        /// Empty for pure functions.
        effects: Vec<FnEffect>,
    },

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

    /// A range type: `Range<T>` or `RangeInclusive<T>`
    ///
    /// Represents the built-in range types used by `..` and `..=` syntax.
    /// Layout: { start: T, end: T } for Range, { start: T, end: T, exhausted: bool } for RangeInclusive.
    Range {
        /// The element type (must support Step trait for iteration).
        element: Type,
        /// Whether this is an inclusive range (`..=`).
        inclusive: bool,
    },

    /// An error type (for error recovery).
    Error,

    /// A trait object type: `dyn Trait` or `dyn Trait + Send + Sync`
    ///
    /// Represents a dynamically-dispatched trait object. Values of this type
    /// are fat pointers containing a data pointer and a vtable pointer.
    DynTrait {
        /// The primary trait this object implements.
        trait_id: DefId,
        /// Additional auto-trait bounds (Send, Sync, etc.).
        auto_traits: Vec<DefId>,
    },

    /// An anonymous record type: `{x: i32, y: bool}` or `{x: i32 | R}`
    ///
    /// Records support row polymorphism via row variables. A record with
    /// a row variable can be used where a record with additional fields
    /// is expected.
    Record {
        /// The known fields of this record.
        fields: Vec<RecordField>,
        /// Optional row variable for row polymorphism.
        /// If present, this record may have additional unknown fields.
        row_var: Option<RecordRowVarId>,
    },

    /// A universally quantified (polymorphic) type: `forall<T>. T -> T`
    ///
    /// Enables higher-rank polymorphism where types themselves can be polymorphic.
    /// When used as a parameter type, the function receives a polymorphic value
    /// that it can instantiate with any concrete type.
    Forall {
        /// The type parameters bound by this forall.
        params: Vec<TyVarId>,
        /// The body type where params are in scope.
        body: Box<Type>,
    },

    /// An ownership-qualified type: `linear T`, `affine T`
    ///
    /// Enables substructural type checking for resource management:
    /// - `linear T`: value must be used exactly once
    /// - `affine T`: value must be used at most once
    Ownership {
        /// The ownership qualifier (linear or affine).
        qualifier: OwnershipQualifier,
        /// The underlying type.
        inner: Type,
    },
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
            TypeKind::Fn { params, ret, effects } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")?;
                if !effects.is_empty() {
                    write!(f, " / {{")?;
                    for (i, eff) in effects.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "E{}", eff.def_id.index())?;
                        if !eff.type_args.is_empty() {
                            write!(f, "<")?;
                            for (j, arg) in eff.type_args.iter().enumerate() {
                                if j > 0 {
                                    write!(f, ", ")?;
                                }
                                write!(f, "{arg}")?;
                            }
                            write!(f, ">")?;
                        }
                    }
                    write!(f, "}}")?;
                }
                Ok(())
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
            TypeKind::Range { element, inclusive: false } => write!(f, "Range<{element}>"),
            TypeKind::Range { element, inclusive: true } => write!(f, "RangeInclusive<{element}>"),
            TypeKind::Error => write!(f, "{{error}}"),
            TypeKind::DynTrait { trait_id, auto_traits } => {
                write!(f, "dyn {trait_id}")?;
                for auto_trait in auto_traits {
                    write!(f, " + {auto_trait}")?;
                }
                Ok(())
            }
            TypeKind::Record { fields, row_var } => {
                write!(f, "{{ ")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    // Use Debug for Symbol since it doesn't implement Display
                    write!(f, "{:?}: {}", field.name, field.ty)?;
                }
                if let Some(rv) = row_var {
                    if !fields.is_empty() {
                        write!(f, " ")?;
                    }
                    write!(f, "| RowVar{}", rv.0)?;
                }
                write!(f, " }}")
            }
            TypeKind::Forall { params, body } => {
                write!(f, "forall<")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "T{}", param.0)?;
                }
                write!(f, ">. {body}")
            }
            TypeKind::Ownership { qualifier, inner } => {
                write!(f, "{qualifier} {inner}")
            }
        }
    }
}

impl fmt::Display for OwnershipQualifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnershipQualifier::Linear => write!(f, "linear"),
            OwnershipQualifier::Affine => write!(f, "affine"),
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
    /// Never type: `!` (bottom type, for expressions that never return)
    Never,
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
            PrimitiveTy::Never => write!(f, "!"),
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
            PrimitiveTy::Never => 0, // Uninhabited type
        })
    }
}
