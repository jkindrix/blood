//! FFI type validation for Blood.
//!
//! This module validates that types used in FFI declarations are compatible
//! with the C ABI (or other foreign ABIs). Types must be "FFI-safe" to be
//! used in bridge blocks.
//!
//! # FFI-Safe Types
//!
//! The following types are FFI-safe:
//! - Primitive integers: `i8`, `i16`, `i32`, `i64`, `isize`, `u8`, `u16`, `u32`, `u64`, `usize`
//! - Floating-point: `f32`, `f64`
//! - `bool` (represented as C `_Bool` or `int` depending on ABI)
//! - `()` (void in C context)
//! - Raw pointers: `*const T`, `*mut T`
//! - Arrays: `[T; N]` where `T` is FFI-safe
//! - Function pointers: `fn(...) -> ...` (with FFI-safe parameter and return types)
//! - Structs/enums declared in bridge blocks or marked with `#[repr(C)]`
//! - Opaque types declared in bridge blocks
//!
//! # Non-FFI-Safe Types
//!
//! The following types are NOT FFI-safe:
//! - `str`, `String` (Rust-specific layout)
//! - Slices `[T]` (fat pointer)
//! - References `&T`, `&mut T` (use pointers instead)
//! - Closures (capture environment)
//! - Trait objects `dyn Trait` (vtable layout)
//! - Generic type parameters (unknown layout)
//! - `i128`, `u128` (not in C standard)
//! - `char` (4 bytes in Rust, varies in C)
//! - Tuples (non-standard layout)
//! - Ranges

use crate::diagnostics::Diagnostic;
use crate::hir::ty::{PrimitiveTy, Type, TypeKind};
use crate::hir::def::{IntTy, UintTy, FloatTy};
use crate::hir::DefId;
use crate::span::Span;

use std::collections::HashSet;

/// The result of validating a type for FFI compatibility.
#[derive(Debug, Clone)]
pub enum FfiSafety {
    /// The type is FFI-safe.
    Safe,
    /// The type is FFI-safe but with a warning (e.g., `bool` representation).
    Warning(String),
    /// The type is not FFI-safe.
    Unsafe(String),
}

impl FfiSafety {
    /// Check if the type is safe (possibly with warnings).
    pub fn is_safe(&self) -> bool {
        matches!(self, FfiSafety::Safe | FfiSafety::Warning(_))
    }
}

/// Context for FFI type validation.
#[derive(Debug)]
pub struct FfiValidator {
    /// Set of DefIds for types declared in bridge blocks (known FFI-safe).
    bridge_types: HashSet<DefId>,
    /// Set of DefIds for opaque types (always FFI-safe as pointers).
    opaque_types: HashSet<DefId>,
    /// Set of DefIds for types marked with #[repr(C)].
    repr_c_types: HashSet<DefId>,
    /// Collected diagnostics.
    diagnostics: Vec<Diagnostic>,
}

impl FfiValidator {
    /// Create a new FFI validator.
    pub fn new() -> Self {
        Self {
            bridge_types: HashSet::new(),
            opaque_types: HashSet::new(),
            repr_c_types: HashSet::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Register a type declared in a bridge block.
    pub fn register_bridge_type(&mut self, def_id: DefId) {
        self.bridge_types.insert(def_id);
    }

    /// Register an opaque type from a bridge block.
    pub fn register_opaque_type(&mut self, def_id: DefId) {
        self.opaque_types.insert(def_id);
    }

    /// Register a type with #[repr(C)].
    pub fn register_repr_c_type(&mut self, def_id: DefId) {
        self.repr_c_types.insert(def_id);
    }

    /// Check if a DefId refers to an FFI-safe type.
    pub fn is_ffi_safe_def(&self, def_id: DefId) -> bool {
        self.bridge_types.contains(&def_id)
            || self.opaque_types.contains(&def_id)
            || self.repr_c_types.contains(&def_id)
    }

    /// Validate a type for FFI compatibility.
    pub fn validate_type(&self, ty: &Type) -> FfiSafety {
        self.validate_type_inner(ty, &mut HashSet::new())
    }

    fn validate_type_inner(&self, ty: &Type, visited: &mut HashSet<DefId>) -> FfiSafety {
        match ty.kind() {
            // Primitive types
            TypeKind::Primitive(prim) => self.validate_primitive(*prim),

            // Unit type is FFI-safe (void)
            TypeKind::Tuple(tys) if tys.is_empty() => FfiSafety::Safe,

            // Non-empty tuples are not FFI-safe
            TypeKind::Tuple(_) => FfiSafety::Unsafe(
                "tuples have non-standard C layout; use a struct instead".to_string(),
            ),

            // Arrays are FFI-safe if element is FFI-safe
            TypeKind::Array { element, .. } => {
                match self.validate_type_inner(element, visited) {
                    FfiSafety::Safe => FfiSafety::Safe,
                    FfiSafety::Warning(msg) => FfiSafety::Warning(format!(
                        "array element has FFI warning: {}", msg
                    )),
                    FfiSafety::Unsafe(msg) => FfiSafety::Unsafe(format!(
                        "array element is not FFI-safe: {}", msg
                    )),
                }
            }

            // Slices are not FFI-safe (fat pointer)
            TypeKind::Slice { .. } => FfiSafety::Unsafe(
                "slices are fat pointers; use a pointer and length instead".to_string(),
            ),

            // References should use pointers instead
            TypeKind::Ref { .. } => FfiSafety::Unsafe(
                "references are not FFI-safe; use raw pointers `*const T` or `*mut T`".to_string(),
            ),

            // Raw pointers are FFI-safe (the pointee is opaque to C)
            TypeKind::Ptr { inner, .. } => {
                // Check if inner type is FFI-safe for better diagnostics,
                // but pointers themselves are always safe (can point to opaque data)
                match self.validate_type_inner(inner, visited) {
                    FfiSafety::Safe => FfiSafety::Safe,
                    FfiSafety::Warning(_) => FfiSafety::Safe, // Pointer is still safe
                    FfiSafety::Unsafe(_) => {
                        // Pointer to non-FFI type is still safe (opaque pointer)
                        // but emit a warning
                        FfiSafety::Warning(
                            "pointer to non-FFI-safe type; ensure foreign code treats it as opaque".to_string()
                        )
                    }
                }
            }

            // Function pointers are FFI-safe if all types are FFI-safe
            TypeKind::Fn { params, ret } => {
                for (i, param) in params.iter().enumerate() {
                    if let FfiSafety::Unsafe(msg) = self.validate_type_inner(param, visited) {
                        return FfiSafety::Unsafe(format!(
                            "parameter {} is not FFI-safe: {}", i + 1, msg
                        ));
                    }
                }
                if let FfiSafety::Unsafe(msg) = self.validate_type_inner(ret, visited) {
                    return FfiSafety::Unsafe(format!(
                        "return type is not FFI-safe: {}", msg
                    ));
                }
                FfiSafety::Safe
            }

            // Closures are not FFI-safe (capture environment)
            TypeKind::Closure { .. } => FfiSafety::Unsafe(
                "closures are not FFI-safe; use function pointers with explicit context".to_string(),
            ),

            // ADT types (struct/enum)
            TypeKind::Adt { def_id, args } => {
                // Check for cycles
                if visited.contains(def_id) {
                    return FfiSafety::Safe; // Assume safe for recursive types
                }
                visited.insert(*def_id);

                // Check if this type is known FFI-safe
                if self.is_ffi_safe_def(*def_id) {
                    // For generic types, also check the type arguments
                    for arg in args {
                        if let FfiSafety::Unsafe(msg) = self.validate_type_inner(arg, visited) {
                            return FfiSafety::Unsafe(format!(
                                "type argument is not FFI-safe: {}", msg
                            ));
                        }
                    }
                    FfiSafety::Safe
                } else {
                    FfiSafety::Unsafe(
                        "type is not declared in bridge block or marked with #[repr(C)]".to_string(),
                    )
                }
            }

            // Type variables are not FFI-safe (unknown layout)
            TypeKind::Infer(_) => FfiSafety::Unsafe(
                "type could not be inferred; FFI requires concrete types".to_string(),
            ),

            TypeKind::Param(_) => FfiSafety::Unsafe(
                "generic type parameters are not FFI-safe; use concrete types".to_string(),
            ),

            // Never type is FFI-safe (function that doesn't return)
            TypeKind::Never => FfiSafety::Safe,

            // Ranges are not FFI-safe
            TypeKind::Range { .. } => FfiSafety::Unsafe(
                "range types are not FFI-safe".to_string(),
            ),

            // Error types - don't add more errors
            TypeKind::Error => FfiSafety::Safe,

            // Trait objects are not FFI-safe
            TypeKind::DynTrait { .. } => FfiSafety::Unsafe(
                "trait objects are not FFI-safe; use concrete types or opaque pointers".to_string(),
            ),

            // Anonymous records are not FFI-safe
            TypeKind::Record { .. } => FfiSafety::Unsafe(
                "anonymous record types are not FFI-safe; use named structs with #[repr(C)]".to_string(),
            ),

            // Forall types are not FFI-safe (polymorphic)
            TypeKind::Forall { .. } => FfiSafety::Unsafe(
                "polymorphic (forall) types are not FFI-safe; use concrete types".to_string(),
            ),

            // Ownership-qualified types - validate the inner type
            TypeKind::Ownership { inner, .. } => self.validate_type_inner(inner, visited),
        }
    }

    fn validate_primitive(&self, prim: PrimitiveTy) -> FfiSafety {
        match prim {
            // Standard C-compatible integers
            PrimitiveTy::Int(IntTy::I8)
            | PrimitiveTy::Int(IntTy::I16)
            | PrimitiveTy::Int(IntTy::I32)
            | PrimitiveTy::Int(IntTy::I64) => FfiSafety::Safe,

            // isize depends on platform, matches C's intptr_t/ssize_t
            PrimitiveTy::Int(IntTy::Isize) => FfiSafety::Warning(
                "isize size varies by platform; consider using fixed-width type".to_string(),
            ),

            // i128 not in C standard
            PrimitiveTy::Int(IntTy::I128) => FfiSafety::Unsafe(
                "i128 is not a standard C type; use i64 or a struct instead".to_string(),
            ),

            // Standard C-compatible unsigned integers
            PrimitiveTy::Uint(UintTy::U8)
            | PrimitiveTy::Uint(UintTy::U16)
            | PrimitiveTy::Uint(UintTy::U32)
            | PrimitiveTy::Uint(UintTy::U64) => FfiSafety::Safe,

            // usize depends on platform, matches C's size_t
            PrimitiveTy::Uint(UintTy::Usize) => FfiSafety::Warning(
                "usize size varies by platform; consider using fixed-width type".to_string(),
            ),

            // u128 not in C standard
            PrimitiveTy::Uint(UintTy::U128) => FfiSafety::Unsafe(
                "u128 is not a standard C type; use u64 or a struct instead".to_string(),
            ),

            // Standard C-compatible floats
            PrimitiveTy::Float(FloatTy::F32) | PrimitiveTy::Float(FloatTy::F64) => FfiSafety::Safe,

            // Bool has different representations across platforms
            PrimitiveTy::Bool => FfiSafety::Warning(
                "bool representation may vary; C99 uses _Bool, older code may use int".to_string(),
            ),

            // Char is 4 bytes in Blood, varies in C
            PrimitiveTy::Char => FfiSafety::Unsafe(
                "char is 4 bytes in Blood but 1 byte in C; use u8 or u32 instead".to_string(),
            ),

            // Str is unsized
            PrimitiveTy::Str => FfiSafety::Unsafe(
                "str is unsized; use *const u8 with a length parameter".to_string(),
            ),

            // String is heap-allocated with specific layout
            PrimitiveTy::String => FfiSafety::Unsafe(
                "String has Rust-specific layout; use *const u8 with a length parameter".to_string(),
            ),

            // Unit is void
            PrimitiveTy::Unit => FfiSafety::Safe,
        }
    }

    /// Validate a type and emit diagnostics if invalid.
    pub fn check_ffi_type(&mut self, ty: &Type, span: Span, context: &str) {
        match self.validate_type(ty) {
            FfiSafety::Safe => {}
            FfiSafety::Warning(msg) => {
                self.diagnostics.push(Diagnostic::warning(format!(
                    "FFI type in {} may have portability issues: {}", context, msg
                ), span));
            }
            FfiSafety::Unsafe(msg) => {
                self.diagnostics.push(Diagnostic::error(format!(
                    "type in {} is not FFI-safe: {}", context, msg
                ), span));
            }
        }
    }

    /// Take the collected diagnostics.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }
}

impl Default for FfiValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::ty::Type;

    #[test]
    fn test_primitive_ffi_safety() {
        let validator = FfiValidator::new();

        // Safe primitives
        assert!(validator.validate_type(&Type::i32()).is_safe());
        assert!(validator.validate_type(&Type::i64()).is_safe());
        assert!(validator.validate_type(&Type::u32()).is_safe());
        assert!(validator.validate_type(&Type::f32()).is_safe());
        assert!(validator.validate_type(&Type::f64()).is_safe());
        assert!(validator.validate_type(&Type::unit()).is_safe());

        // Primitives with warnings
        assert!(matches!(
            validator.validate_type(&Type::bool()),
            FfiSafety::Warning(_)
        ));
        assert!(matches!(
            validator.validate_type(&Type::usize()),
            FfiSafety::Warning(_)
        ));

        // Unsafe primitives
        assert!(matches!(
            validator.validate_type(&Type::str()),
            FfiSafety::Unsafe(_)
        ));
        assert!(matches!(
            validator.validate_type(&Type::string()),
            FfiSafety::Unsafe(_)
        ));
        assert!(matches!(
            validator.validate_type(&Type::char()),
            FfiSafety::Unsafe(_)
        ));
    }

    #[test]
    fn test_pointer_ffi_safety() {
        let validator = FfiValidator::new();

        // Raw pointers are always safe
        let ptr_i32 = Type::new(TypeKind::Ptr {
            inner: Type::i32(),
            mutable: false,
        });
        assert!(validator.validate_type(&ptr_i32).is_safe());

        // Even pointers to non-FFI types are safe (treated as opaque)
        let ptr_string = Type::new(TypeKind::Ptr {
            inner: Type::string(),
            mutable: true,
        });
        assert!(validator.validate_type(&ptr_string).is_safe());
    }

    #[test]
    fn test_array_ffi_safety() {
        let validator = FfiValidator::new();

        // Array of FFI-safe type
        let arr_i32 = Type::array(Type::i32(), 10);
        assert!(validator.validate_type(&arr_i32).is_safe());

        // Array of non-FFI-safe type
        let arr_string = Type::array(Type::string(), 10);
        assert!(matches!(
            validator.validate_type(&arr_string),
            FfiSafety::Unsafe(_)
        ));
    }

    #[test]
    fn test_reference_not_ffi_safe() {
        let validator = FfiValidator::new();

        let ref_i32 = Type::reference(Type::i32(), false);
        assert!(matches!(
            validator.validate_type(&ref_i32),
            FfiSafety::Unsafe(_)
        ));
    }

    #[test]
    fn test_slice_not_ffi_safe() {
        let validator = FfiValidator::new();

        let slice_i32 = Type::slice(Type::i32());
        assert!(matches!(
            validator.validate_type(&slice_i32),
            FfiSafety::Unsafe(_)
        ));
    }

    #[test]
    fn test_function_pointer_ffi_safety() {
        let validator = FfiValidator::new();

        // Function with FFI-safe types
        let fn_safe = Type::function(vec![Type::i32(), Type::f64()], Type::i32());
        assert!(validator.validate_type(&fn_safe).is_safe());

        // Function with non-FFI-safe parameter
        let fn_unsafe_param = Type::function(vec![Type::string()], Type::i32());
        assert!(matches!(
            validator.validate_type(&fn_unsafe_param),
            FfiSafety::Unsafe(_)
        ));

        // Function with non-FFI-safe return
        let fn_unsafe_ret = Type::function(vec![Type::i32()], Type::string());
        assert!(matches!(
            validator.validate_type(&fn_unsafe_ret),
            FfiSafety::Unsafe(_)
        ));
    }

    #[test]
    fn test_tuple_not_ffi_safe() {
        let validator = FfiValidator::new();

        // Non-empty tuple is not FFI-safe
        let tuple = Type::tuple(vec![Type::i32(), Type::i32()]);
        assert!(matches!(
            validator.validate_type(&tuple),
            FfiSafety::Unsafe(_)
        ));

        // Empty tuple (unit) is safe
        let unit = Type::tuple(vec![]);
        assert!(validator.validate_type(&unit).is_safe());
    }
}
