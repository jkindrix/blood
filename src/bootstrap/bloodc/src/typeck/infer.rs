//! Type inference for Blood.
//!
//! This module provides the type inference engine that works alongside
//! the type checker. It handles:
//!
//! - Creating fresh type variables
//! - Collecting constraints
//! - Solving constraints via unification
//!
//! The main inference logic is integrated into TypeContext; this module
//! provides supporting infrastructure.

use crate::hir::{Type, TypeKind};
use super::unify::Unifier;

/// Type inference state.
#[derive(Debug)]
pub struct TypeInference {
    /// The unifier for type variable substitutions.
    unifier: Unifier,
}

impl TypeInference {
    /// Create a new type inference engine.
    pub fn new() -> Self {
        Self {
            unifier: Unifier::new(),
        }
    }

    /// Get the unifier.
    pub fn unifier(&self) -> &Unifier {
        &self.unifier
    }

    /// Get the unifier mutably.
    pub fn unifier_mut(&mut self) -> &mut Unifier {
        &mut self.unifier
    }

    /// Create a fresh type variable.
    pub fn fresh_var(&mut self) -> Type {
        self.unifier.fresh_var()
    }

    /// Resolve a type to its most concrete form.
    pub fn resolve(&self, ty: &Type) -> Type {
        self.unifier.resolve(ty)
    }

    /// Check if inference is complete (no remaining type variables).
    pub fn is_complete(&self, ty: &Type) -> bool {
        let resolved = self.resolve(ty);
        !resolved.has_type_vars()
    }

    /// Get the default type for a numeric literal.
    pub fn default_int_type() -> Type {
        Type::i32()
    }

    /// Get the default type for a float literal.
    pub fn default_float_type() -> Type {
        Type::f64()
    }
}

impl Default for TypeInference {
    fn default() -> Self {
        Self::new()
    }
}

/// Numeric type inference helper.
///
/// When the type of a numeric literal is ambiguous (no suffix),
/// we create a type variable and later default it.
pub fn numeric_type_variable(unifier: &mut Unifier) -> Type {
    // In a full implementation, we'd track this as a "numeric" constraint
    // For Phase 1, we just use i32 as default
    unifier.fresh_var()
}

/// Check if a type can be used in numeric operations.
pub fn is_numeric(ty: &Type) -> bool {
    match ty.kind() {
        TypeKind::Primitive(p) => p.is_numeric(),
        TypeKind::Infer(_) => true, // Could be numeric
        _ => false,
    }
}

/// Check if a type can be compared for equality.
pub fn is_eq(ty: &Type) -> bool {
    match ty.kind() {
        TypeKind::Primitive(_) => true,
        TypeKind::Tuple(tys) => tys.iter().all(is_eq),
        TypeKind::Array { element, .. } => is_eq(element),
        TypeKind::Ref { inner, .. } => is_eq(inner),
        TypeKind::Adt { .. } => true, // Assume yes for now
        TypeKind::Infer(_) => true,
        _ => false,
    }
}

/// Check if a type can be ordered.
pub fn is_ord(ty: &Type) -> bool {
    match ty.kind() {
        TypeKind::Primitive(p) => p.is_numeric() || matches!(p, crate::hir::PrimitiveTy::Char),
        TypeKind::Infer(_) => true,
        _ => false,
    }
}
