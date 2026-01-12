//! Type unification for Blood.
//!
//! This module implements unification for type inference. The algorithm
//! is based on Hindley-Milner with some extensions for Blood's type system.
//!
//! # Specification Alignment
//!
//! This implementation follows the UNIFY algorithm specified in DISPATCH.md §4.4.
//!
//! ## Implemented Cases (Phase 1)
//!
//! | Spec Case | Implementation |
//! |-----------|----------------|
//! | Case 1: Identical types | `Primitive(p1) == Primitive(p2)` |
//! | Case 2-3: Type variables | `Infer(id)` binding |
//! | Case 4: Type constructors | `Primitive` comparison |
//! | Case 5: Type applications | `Adt { def_id, args }` |
//! | Case 6: Function types | `Fn { params, ret }` |
//!
//! ## Deferred Cases (Phase 2+)
//!
//! | Spec Case | Status |
//! |-----------|--------|
//! | Case 7: Record types (row polymorphism) | Phase 2+ |
//! | Case 8: Forall types (instantiation) | Phase 2+ |
//! | Effect row unification (§4.4.3) | Phase 2+ |
//!
//! # Algorithm
//!
//! Unification finds a substitution that makes two types equal:
//!
//! ```text
//! unify(?T, i32)     => ?T = i32
//! unify(?T, ?U)      => ?T = ?U (or vice versa)
//! unify(i32, String) => ERROR
//! ```
//!
//! The algorithm uses a union-find structure for efficient variable resolution.

use std::collections::HashMap;

use crate::hir::{PrimitiveTy, Type, TypeKind, TyVarId};

use super::error::{TypeError, TypeErrorKind};
use crate::span::Span;

/// The unifier maintains type variable substitutions.
#[derive(Debug, Clone)]
pub struct Unifier {
    /// Type variable substitutions.
    /// Maps type variable ID to its resolved type (or another variable).
    substitutions: HashMap<TyVarId, Type>,
    /// The next type variable ID to assign.
    next_var: u32,
}

impl Unifier {
    /// Create a new unifier.
    pub fn new() -> Self {
        Self {
            substitutions: HashMap::new(),
            next_var: 0,
        }
    }

    /// Create a fresh type variable.
    pub fn fresh_var(&mut self) -> Type {
        let id = TyVarId::new(self.next_var);
        self.next_var += 1;
        Type::infer(id)
    }

    /// Create multiple fresh type variables.
    pub fn fresh_vars(&mut self, count: usize) -> Vec<Type> {
        (0..count).map(|_| self.fresh_var()).collect()
    }

    /// Unify two types, recording substitutions.
    ///
    /// Returns Ok(()) if unification succeeds, Err if types are incompatible.
    pub fn unify(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<(), TypeError> {
        // Resolve any existing substitutions
        let t1 = self.resolve(t1);
        let t2 = self.resolve(t2);

        match (t1.kind(), t2.kind()) {
            // Same primitive types
            (TypeKind::Primitive(p1), TypeKind::Primitive(p2)) if p1 == p2 => Ok(()),

            // Same ADT with unifiable arguments
            (TypeKind::Adt { def_id: d1, args: a1 }, TypeKind::Adt { def_id: d2, args: a2 })
                if d1 == d2 =>
            {
                if a1.len() != a2.len() {
                    return Err(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: t1.clone(),
                            found: t2.clone(),
                        },
                        span,
                    ));
                }
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    self.unify(arg1, arg2, span)?;
                }
                Ok(())
            }

            // Tuples with same length
            (TypeKind::Tuple(ts1), TypeKind::Tuple(ts2)) if ts1.len() == ts2.len() => {
                for (t1, t2) in ts1.iter().zip(ts2.iter()) {
                    self.unify(t1, t2, span)?;
                }
                Ok(())
            }

            // Arrays with same size
            (
                TypeKind::Array { element: e1, size: s1 },
                TypeKind::Array { element: e2, size: s2 },
            ) if s1 == s2 => self.unify(e1, e2, span),

            // Slices
            (TypeKind::Slice { element: e1 }, TypeKind::Slice { element: e2 }) => {
                self.unify(e1, e2, span)
            }

            // References with same mutability
            (
                TypeKind::Ref { inner: i1, mutable: m1 },
                TypeKind::Ref { inner: i2, mutable: m2 },
            ) if m1 == m2 => self.unify(i1, i2, span),

            // Pointers with same mutability
            (
                TypeKind::Ptr { inner: i1, mutable: m1 },
                TypeKind::Ptr { inner: i2, mutable: m2 },
            ) if m1 == m2 => self.unify(i1, i2, span),

            // Functions
            (
                TypeKind::Fn { params: p1, ret: r1 },
                TypeKind::Fn { params: p2, ret: r2 },
            ) if p1.len() == p2.len() => {
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    self.unify(param1, param2, span)?;
                }
                self.unify(r1, r2, span)
            }

            // Range types
            (
                TypeKind::Range { element: e1, inclusive: i1 },
                TypeKind::Range { element: e2, inclusive: i2 },
            ) if i1 == i2 => {
                self.unify(e1, e2, span)
            }

            // Never type unifies with anything
            (TypeKind::Never, _) | (_, TypeKind::Never) => Ok(()),

            // Error type unifies with anything (for error recovery)
            (TypeKind::Error, _) | (_, TypeKind::Error) => Ok(()),

            // Unit type equivalence: Primitive(Unit) == Tuple([])
            // The unit type can be represented as either:
            // - PrimitiveTy::Unit (from parsing `unit` keyword)
            // - Tuple([]) (from parsing `()` or Type::unit())
            // These should unify successfully.
            (TypeKind::Primitive(PrimitiveTy::Unit), TypeKind::Tuple(ts))
            | (TypeKind::Tuple(ts), TypeKind::Primitive(PrimitiveTy::Unit))
                if ts.is_empty() => Ok(()),

            // Same inference variable - trivially equal
            (TypeKind::Infer(id1), TypeKind::Infer(id2)) if id1 == id2 => Ok(()),

            // Inference variable - bind it
            (TypeKind::Infer(id), _) => {
                self.bind(*id, t2.clone(), span)
            }
            (_, TypeKind::Infer(id)) => {
                self.bind(*id, t1.clone(), span)
            }

            // Type parameter - for now, treat as error (needs constraint solving)
            (TypeKind::Param(id1), TypeKind::Param(id2)) if id1 == id2 => Ok(()),

            // No match
            _ => Err(TypeError::new(
                TypeErrorKind::Mismatch {
                    expected: t1.clone(),
                    found: t2.clone(),
                },
                span,
            )),
        }
    }

    /// Bind a type variable to a type.
    fn bind(&mut self, var: TyVarId, ty: Type, span: Span) -> Result<(), TypeError> {
        // Occurs check: prevent infinite types like ?T = List<?T>
        if self.occurs_in(var, &ty) {
            return Err(TypeError::new(TypeErrorKind::InfiniteType, span));
        }

        self.substitutions.insert(var, ty);
        Ok(())
    }

    /// Check if a type variable occurs in a type.
    fn occurs_in(&self, var: TyVarId, ty: &Type) -> bool {
        let ty = self.resolve(ty);
        match ty.kind() {
            TypeKind::Infer(id) => *id == var,
            TypeKind::Tuple(tys) => tys.iter().any(|t| self.occurs_in(var, t)),
            TypeKind::Array { element, .. } => self.occurs_in(var, element),
            TypeKind::Slice { element } => self.occurs_in(var, element),
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                self.occurs_in(var, inner)
            }
            TypeKind::Fn { params, ret } => {
                params.iter().any(|t| self.occurs_in(var, t)) || self.occurs_in(var, ret)
            }
            TypeKind::Adt { args, .. } => args.iter().any(|t| self.occurs_in(var, t)),
            TypeKind::Range { element, .. } => self.occurs_in(var, element),
            _ => false,
        }
    }

    /// Resolve a type by following substitutions.
    pub fn resolve(&self, ty: &Type) -> Type {
        match ty.kind() {
            TypeKind::Infer(id) => {
                if let Some(substituted) = self.substitutions.get(id) {
                    self.resolve(substituted)
                } else {
                    ty.clone()
                }
            }
            TypeKind::Tuple(tys) => {
                Type::tuple(tys.iter().map(|t| self.resolve(t)).collect())
            }
            TypeKind::Array { element, size } => {
                Type::array(self.resolve(element), *size)
            }
            TypeKind::Slice { element } => Type::slice(self.resolve(element)),
            TypeKind::Ref { inner, mutable } => {
                Type::reference(self.resolve(inner), *mutable)
            }
            TypeKind::Ptr { inner, mutable } => {
                Type::new(TypeKind::Ptr {
                    inner: self.resolve(inner),
                    mutable: *mutable,
                })
            }
            TypeKind::Fn { params, ret } => Type::function(
                params.iter().map(|t| self.resolve(t)).collect(),
                self.resolve(ret),
            ),
            TypeKind::Adt { def_id, args } => Type::adt(
                *def_id,
                args.iter().map(|t| self.resolve(t)).collect(),
            ),
            TypeKind::Range { element, inclusive } => Type::new(TypeKind::Range {
                element: self.resolve(element),
                inclusive: *inclusive,
            }),
            _ => ty.clone(),
        }
    }

    /// Check if a type is fully resolved (no inference variables).
    pub fn is_resolved(&self, ty: &Type) -> bool {
        let ty = self.resolve(ty);
        !ty.has_type_vars()
    }

    /// Get all substitutions.
    pub fn substitutions(&self) -> &HashMap<TyVarId, Type> {
        &self.substitutions
    }
}

impl Default for Unifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // Primitive Type Tests
    // ============================================================

    #[test]
    fn test_unify_primitives() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // Same primitives should unify
        assert!(u.unify(&Type::i32(), &Type::i32(), span).is_ok());
        assert!(u.unify(&Type::bool(), &Type::bool(), span).is_ok());
        assert!(u.unify(&Type::f64(), &Type::f64(), span).is_ok());
        assert!(u.unify(&Type::unit(), &Type::unit(), span).is_ok());

        // Different primitives should not unify
        assert!(u.unify(&Type::i32(), &Type::bool(), span).is_err());
        assert!(u.unify(&Type::i32(), &Type::f64(), span).is_err());
        assert!(u.unify(&Type::bool(), &Type::unit(), span).is_err());
    }

    #[test]
    fn test_unify_different_integer_types() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // i32 should not unify with i64 (different bit widths)
        assert!(u.unify(&Type::i32(), &Type::i64(), span).is_err());

        // Same types should always unify
        assert!(u.unify(&Type::i64(), &Type::i64(), span).is_ok());
    }

    // ============================================================
    // Type Variable Tests
    // ============================================================

    #[test]
    fn test_unify_variable() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        assert!(u.unify(&var, &Type::i32(), span).is_ok());

        // Variable should now resolve to i32
        let resolved = u.resolve(&var);
        assert_eq!(resolved, Type::i32());
    }

    #[test]
    fn test_unify_variable_reverse_order() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        // Unify in reverse order: concrete type first, then variable
        assert!(u.unify(&Type::bool(), &var, span).is_ok());

        let resolved = u.resolve(&var);
        assert_eq!(resolved, Type::bool());
    }

    #[test]
    fn test_unify_two_variables() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var1 = u.fresh_var();
        let var2 = u.fresh_var();

        // Two variables can unify
        assert!(u.unify(&var1, &var2, span).is_ok());

        // Binding one should bind the other
        assert!(u.unify(&var1, &Type::i32(), span).is_ok());

        assert_eq!(u.resolve(&var1), Type::i32());
        assert_eq!(u.resolve(&var2), Type::i32());
    }

    #[test]
    fn test_unify_variable_chaining() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // Create chain: ?a -> ?b -> ?c -> i32
        let a = u.fresh_var();
        let b = u.fresh_var();
        let c = u.fresh_var();

        assert!(u.unify(&a, &b, span).is_ok());
        assert!(u.unify(&b, &c, span).is_ok());
        assert!(u.unify(&c, &Type::i32(), span).is_ok());

        // All should resolve to i32
        assert_eq!(u.resolve(&a), Type::i32());
        assert_eq!(u.resolve(&b), Type::i32());
        assert_eq!(u.resolve(&c), Type::i32());
    }

    #[test]
    fn test_unify_already_bound_variable() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        assert!(u.unify(&var, &Type::i32(), span).is_ok());

        // Unifying with same type should succeed
        assert!(u.unify(&var, &Type::i32(), span).is_ok());

        // Unifying with different type should fail
        assert!(u.unify(&var, &Type::bool(), span).is_err());
    }

    // ============================================================
    // Tuple Type Tests
    // ============================================================

    #[test]
    fn test_unify_tuples() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let t1 = Type::tuple(vec![Type::i32(), Type::bool()]);
        let t2 = Type::tuple(vec![Type::i32(), Type::bool()]);
        assert!(u.unify(&t1, &t2, span).is_ok());

        // Different lengths should fail
        let t3 = Type::tuple(vec![Type::i32()]);
        assert!(u.unify(&t1, &t3, span).is_err());
    }

    #[test]
    fn test_unify_empty_tuples() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let empty1 = Type::tuple(vec![]);
        let empty2 = Type::tuple(vec![]);
        assert!(u.unify(&empty1, &empty2, span).is_ok());
    }

    #[test]
    fn test_unify_tuples_with_variables() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        let t1 = Type::tuple(vec![var.clone(), Type::bool()]);
        let t2 = Type::tuple(vec![Type::i32(), Type::bool()]);

        assert!(u.unify(&t1, &t2, span).is_ok());
        assert_eq!(u.resolve(&var), Type::i32());
    }

    #[test]
    fn test_unify_nested_tuples() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let inner1 = Type::tuple(vec![Type::i32(), Type::i32()]);
        let inner2 = Type::tuple(vec![Type::i32(), Type::i32()]);
        let t1 = Type::tuple(vec![inner1, Type::bool()]);
        let t2 = Type::tuple(vec![inner2, Type::bool()]);

        assert!(u.unify(&t1, &t2, span).is_ok());
    }

    // ============================================================
    // Array and Slice Tests
    // ============================================================

    #[test]
    fn test_unify_arrays_same_size() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let arr1 = Type::array(Type::i32(), 5);
        let arr2 = Type::array(Type::i32(), 5);
        assert!(u.unify(&arr1, &arr2, span).is_ok());
    }

    #[test]
    fn test_unify_arrays_different_size() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let arr1 = Type::array(Type::i32(), 5);
        let arr2 = Type::array(Type::i32(), 10);
        assert!(u.unify(&arr1, &arr2, span).is_err());
    }

    #[test]
    fn test_unify_arrays_different_element() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let arr1 = Type::array(Type::i32(), 5);
        let arr2 = Type::array(Type::bool(), 5);
        assert!(u.unify(&arr1, &arr2, span).is_err());
    }

    #[test]
    fn test_unify_slices() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let slice1 = Type::slice(Type::i32());
        let slice2 = Type::slice(Type::i32());
        assert!(u.unify(&slice1, &slice2, span).is_ok());

        let slice3 = Type::slice(Type::bool());
        assert!(u.unify(&slice1, &slice3, span).is_err());
    }

    #[test]
    fn test_unify_array_slice_different() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // Array and slice of same element type should NOT unify
        let arr = Type::array(Type::i32(), 5);
        let slice = Type::slice(Type::i32());
        assert!(u.unify(&arr, &slice, span).is_err());
    }

    // ============================================================
    // Reference and Pointer Tests
    // ============================================================

    #[test]
    fn test_unify_references() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let ref1 = Type::reference(Type::i32(), false);
        let ref2 = Type::reference(Type::i32(), false);
        assert!(u.unify(&ref1, &ref2, span).is_ok());
    }

    #[test]
    fn test_unify_references_mutability_mismatch() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let ref_imm = Type::reference(Type::i32(), false);
        let ref_mut = Type::reference(Type::i32(), true);
        // Different mutability should NOT unify
        assert!(u.unify(&ref_imm, &ref_mut, span).is_err());
    }

    #[test]
    fn test_unify_pointers() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let ptr1 = Type::new(TypeKind::Ptr {
            inner: Type::i32(),
            mutable: false,
        });
        let ptr2 = Type::new(TypeKind::Ptr {
            inner: Type::i32(),
            mutable: false,
        });
        assert!(u.unify(&ptr1, &ptr2, span).is_ok());
    }

    // ============================================================
    // Function Type Tests
    // ============================================================

    #[test]
    fn test_unify_functions_same() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let fn1 = Type::function(vec![Type::i32()], Type::bool());
        let fn2 = Type::function(vec![Type::i32()], Type::bool());
        assert!(u.unify(&fn1, &fn2, span).is_ok());
    }

    #[test]
    fn test_unify_functions_different_params() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let fn1 = Type::function(vec![Type::i32()], Type::bool());
        let fn2 = Type::function(vec![Type::bool()], Type::bool());
        assert!(u.unify(&fn1, &fn2, span).is_err());
    }

    #[test]
    fn test_unify_functions_different_return() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let fn1 = Type::function(vec![Type::i32()], Type::bool());
        let fn2 = Type::function(vec![Type::i32()], Type::i32());
        assert!(u.unify(&fn1, &fn2, span).is_err());
    }

    #[test]
    fn test_unify_functions_different_arity() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let fn1 = Type::function(vec![Type::i32()], Type::bool());
        let fn2 = Type::function(vec![Type::i32(), Type::i32()], Type::bool());
        assert!(u.unify(&fn1, &fn2, span).is_err());
    }

    #[test]
    fn test_unify_functions_with_variables() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        let fn1 = Type::function(vec![var.clone()], Type::bool());
        let fn2 = Type::function(vec![Type::i32()], Type::bool());

        assert!(u.unify(&fn1, &fn2, span).is_ok());
        assert_eq!(u.resolve(&var), Type::i32());
    }

    // ============================================================
    // Occurs Check Tests
    // ============================================================

    #[test]
    fn test_occurs_check() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        // Try to create infinite type: ?T = (?T,)
        let tuple = Type::tuple(vec![var.clone()]);
        assert!(u.unify(&var, &tuple, span).is_err());
    }

    #[test]
    fn test_occurs_check_in_array() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        // Try to create infinite type: ?T = [?T; 5]
        let array = Type::array(var.clone(), 5);
        assert!(u.unify(&var, &array, span).is_err());
    }

    #[test]
    fn test_occurs_check_in_function() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        // Try to create infinite type: ?T = fn(?T) -> bool
        let func = Type::function(vec![var.clone()], Type::bool());
        assert!(u.unify(&var, &func, span).is_err());
    }

    #[test]
    fn test_occurs_check_nested() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();
        // Deeply nested occurs: ?T = (i32, (?T, bool))
        let inner = Type::tuple(vec![var.clone(), Type::bool()]);
        let outer = Type::tuple(vec![Type::i32(), inner]);
        assert!(u.unify(&var, &outer, span).is_err());
    }

    // ============================================================
    // Special Type Tests
    // ============================================================

    #[test]
    fn test_unify_never_type() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // Never type should unify with anything
        assert!(u.unify(&Type::never(), &Type::i32(), span).is_ok());
        assert!(u.unify(&Type::bool(), &Type::never(), span).is_ok());
        assert!(u.unify(&Type::never(), &Type::never(), span).is_ok());
    }

    #[test]
    fn test_unify_error_type() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // Error type should unify with anything (for error recovery)
        assert!(u.unify(&Type::error(), &Type::i32(), span).is_ok());
        assert!(u.unify(&Type::bool(), &Type::error(), span).is_ok());
        assert!(u.unify(&Type::error(), &Type::error(), span).is_ok());
    }

    // ============================================================
    // Resolution Tests
    // ============================================================

    #[test]
    fn test_is_resolved() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var = u.fresh_var();

        // Unbound variable is not resolved
        assert!(!u.is_resolved(&var));

        // Primitive is always resolved
        assert!(u.is_resolved(&Type::i32()));

        // After binding, variable is resolved
        u.unify(&var, &Type::i32(), span).unwrap();
        assert!(u.is_resolved(&var));
    }

    #[test]
    fn test_resolve_nested_structure() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        let var1 = u.fresh_var();
        let var2 = u.fresh_var();

        let nested = Type::tuple(vec![var1.clone(), Type::tuple(vec![var2.clone()])]);

        u.unify(&var1, &Type::i32(), span).unwrap();
        u.unify(&var2, &Type::bool(), span).unwrap();

        let resolved = u.resolve(&nested);
        let expected = Type::tuple(vec![Type::i32(), Type::tuple(vec![Type::bool()])]);
        assert_eq!(resolved, expected);
    }

    // ============================================================
    // Property-Based Style Tests
    // ============================================================

    #[test]
    fn test_unification_reflexivity() {
        // For all types T: unify(T, T) succeeds
        let span = Span::dummy();

        let types = vec![
            Type::i32(),
            Type::bool(),
            Type::unit(),
            Type::tuple(vec![Type::i32(), Type::bool()]),
            Type::array(Type::i32(), 5),
            Type::function(vec![Type::i32()], Type::bool()),
        ];

        for ty in types {
            let mut u = Unifier::new();
            assert!(
                u.unify(&ty, &ty, span).is_ok(),
                "Reflexivity failed for {:?}",
                ty
            );
        }
    }

    #[test]
    fn test_unification_symmetry() {
        // For all types T, U: if unify(T, U) succeeds, unify(U, T) also succeeds
        let mut u1 = Unifier::new();
        let mut u2 = Unifier::new();
        let span = Span::dummy();

        let var1_a = u1.fresh_var();
        let var1_b = u2.fresh_var();

        // unify(var, i32) should behave same as unify(i32, var)
        assert!(u1.unify(&var1_a, &Type::i32(), span).is_ok());
        assert!(u2.unify(&Type::i32(), &var1_b, span).is_ok());

        assert_eq!(u1.resolve(&var1_a), u2.resolve(&var1_b));
    }
}
