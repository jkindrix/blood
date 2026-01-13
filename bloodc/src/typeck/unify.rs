//! Type unification for Blood.
//!
//! This module implements unification for type inference. The algorithm
//! is based on Hindley-Milner with extensions for Blood's type system.
//!
//! # Specification Alignment
//!
//! This implementation follows the UNIFY algorithm specified in DISPATCH.md §4.4.
//!
//! ## Implemented Cases
//!
//! | Spec Case | Implementation |
//! |-----------|----------------|
//! | Case 1: Identical types | `Primitive(p1) == Primitive(p2)` |
//! | Case 2-3: Type variables | `Infer(id)` binding |
//! | Case 4: Type constructors | `Primitive` comparison |
//! | Case 5: Type applications | `Adt { def_id, args }` |
//! | Case 6: Function types | `Fn { params, ret }` |
//! | Case 7: Record types (row polymorphism) | `Record { fields, row_var }` |
//! | Case 8: Forall types (instantiation) | `Forall { params, body }` |
//!
//! ## Related Modules
//!
//! - Effect row unification is implemented in [`effect.rs`](super::effect)
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

use crate::hir::{PrimitiveTy, Type, TypeKind, TyVarId, RecordRowVarId, RecordField};

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
    /// Row variable substitutions for record types.
    /// Maps row variable ID to a record type (fields + optional row var).
    row_substitutions: HashMap<RecordRowVarId, (Vec<RecordField>, Option<RecordRowVarId>)>,
    /// The next row variable ID to assign.
    next_row_var: u32,
}

impl Unifier {
    /// Create a new unifier.
    pub fn new() -> Self {
        Self {
            substitutions: HashMap::new(),
            next_var: 0,
            row_substitutions: HashMap::new(),
            next_row_var: 0,
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

    /// Create a fresh row variable for record types.
    pub fn fresh_row_var(&mut self) -> RecordRowVarId {
        let id = RecordRowVarId::new(self.next_row_var);
        self.next_row_var += 1;
        id
    }

    /// Create a fresh type variable ID for forall-bound parameters.
    /// These are distinct from inference variables and represent universally quantified types.
    pub fn fresh_forall_var(&mut self) -> TyVarId {
        let id = TyVarId::new(self.next_var);
        self.next_var += 1;
        id
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

            // Closure types - unify based on param and return types
            // Two closures unify if their signatures match (def_id is irrelevant for unification)
            (
                TypeKind::Closure { params: p1, ret: r1, .. },
                TypeKind::Closure { params: p2, ret: r2, .. },
            ) if p1.len() == p2.len() => {
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    self.unify(param1, param2, span)?;
                }
                self.unify(r1, r2, span)
            }

            // Closure can unify with compatible function type
            (
                TypeKind::Closure { params: p1, ret: r1, .. },
                TypeKind::Fn { params: p2, ret: r2 },
            ) | (
                TypeKind::Fn { params: p1, ret: r1 },
                TypeKind::Closure { params: p2, ret: r2, .. },
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

            // Record types - row polymorphism
            (
                TypeKind::Record { fields: f1, row_var: rv1 },
                TypeKind::Record { fields: f2, row_var: rv2 },
            ) => self.unify_records(f1, *rv1, f2, *rv2, span),

            // Forall types - higher-rank polymorphism
            // Two forall types unify if they have the same number of params
            // and their bodies unify under alpha-renaming
            (
                TypeKind::Forall { params: p1, body: b1 },
                TypeKind::Forall { params: p2, body: b2 },
            ) if p1.len() == p2.len() => {
                // For alpha-equivalence, we instantiate both with the same fresh variables
                // and check if the bodies unify
                let fresh_vars: Vec<Type> = (0..p1.len())
                    .map(|_| self.fresh_var())
                    .collect();

                let subst1: std::collections::HashMap<TyVarId, Type> = p1.iter()
                    .cloned()
                    .zip(fresh_vars.iter().cloned())
                    .collect();
                let subst2: std::collections::HashMap<TyVarId, Type> = p2.iter()
                    .cloned()
                    .zip(fresh_vars.iter().cloned())
                    .collect();

                let b1_inst = self.substitute_forall_params(b1, &subst1);
                let b2_inst = self.substitute_forall_params(b2, &subst2);

                self.unify(&b1_inst, &b2_inst, span)
            }

            // When unifying a forall with a non-forall on the right, instantiate the forall
            // This handles cases like: forall<T>. T -> T  vs  i32 -> i32
            (TypeKind::Forall { params, body }, _) => {
                // Instantiate with fresh inference variables
                let fresh_vars: Vec<Type> = (0..params.len())
                    .map(|_| self.fresh_var())
                    .collect();

                let subst: std::collections::HashMap<TyVarId, Type> = params.iter()
                    .cloned()
                    .zip(fresh_vars.iter().cloned())
                    .collect();

                let body_inst = self.substitute_forall_params(body, &subst);
                self.unify(&body_inst, &t2, span)
            }

            // When unifying a non-forall with a forall on the right
            (_, TypeKind::Forall { params, body }) => {
                // Instantiate with fresh inference variables
                let fresh_vars: Vec<Type> = (0..params.len())
                    .map(|_| self.fresh_var())
                    .collect();

                let subst: std::collections::HashMap<TyVarId, Type> = params.iter()
                    .cloned()
                    .zip(fresh_vars.iter().cloned())
                    .collect();

                let body_inst = self.substitute_forall_params(body, &subst);
                self.unify(&t1, &body_inst, span)
            }

            // Ownership types - must have same qualifier
            (
                TypeKind::Ownership { qualifier: q1, inner: i1 },
                TypeKind::Ownership { qualifier: q2, inner: i2 },
            ) if q1 == q2 => self.unify(i1, i2, span),

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

    /// Unify two record types with row polymorphism.
    ///
    /// Row polymorphism allows records with extra fields to match:
    /// - `{x: i32, y: bool}` matches `{x: i32, y: bool}`
    /// - `{x: i32 | R}` matches `{x: i32, y: bool}` (R binds to `{y: bool}`)
    fn unify_records(
        &mut self,
        fields1: &[RecordField],
        row_var1: Option<RecordRowVarId>,
        fields2: &[RecordField],
        row_var2: Option<RecordRowVarId>,
        span: Span,
    ) -> Result<(), TypeError> {
        use std::collections::HashMap;

        // Build maps of field name -> type
        let map1: HashMap<_, _> = fields1.iter().map(|f| (f.name, &f.ty)).collect();
        let map2: HashMap<_, _> = fields2.iter().map(|f| (f.name, &f.ty)).collect();

        // Find common fields and unify their types
        for (name, ty1) in &map1 {
            if let Some(ty2) = map2.get(name) {
                self.unify(ty1, ty2, span)?;
            }
        }

        // Find fields only in record 1
        let only_in_1: Vec<_> = fields1.iter()
            .filter(|f| !map2.contains_key(&f.name))
            .cloned()
            .collect();

        // Find fields only in record 2
        let only_in_2: Vec<_> = fields2.iter()
            .filter(|f| !map1.contains_key(&f.name))
            .cloned()
            .collect();

        // Handle row polymorphism
        match (row_var1, row_var2, only_in_1.is_empty(), only_in_2.is_empty()) {
            // Both closed, no extra fields - OK
            (None, None, true, true) => Ok(()),

            // Both closed but have extra fields - mismatch
            (None, None, false, _) | (None, None, _, false) => {
                Err(TypeError::new(
                    TypeErrorKind::Mismatch {
                        expected: Type::record(fields1.to_vec(), row_var1),
                        found: Type::record(fields2.to_vec(), row_var2),
                    },
                    span,
                ))
            }

            // Record 1 is open - bind its row var to record 2's extra fields
            (Some(rv1), None, _, false) | (Some(rv1), None, _, true) => {
                if !only_in_1.is_empty() {
                    // Record 2 is missing fields that record 1 has
                    return Err(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::record(fields1.to_vec(), row_var1),
                            found: Type::record(fields2.to_vec(), row_var2),
                        },
                        span,
                    ));
                }
                // Bind rv1 to the extra fields from record 2
                self.row_substitutions.insert(rv1, (only_in_2, None));
                Ok(())
            }

            // Record 2 is open - bind its row var to record 1's extra fields
            (None, Some(rv2), false, _) | (None, Some(rv2), true, _) => {
                if !only_in_2.is_empty() {
                    // Record 1 is missing fields that record 2 has
                    return Err(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::record(fields1.to_vec(), row_var1),
                            found: Type::record(fields2.to_vec(), row_var2),
                        },
                        span,
                    ));
                }
                // Bind rv2 to the extra fields from record 1
                self.row_substitutions.insert(rv2, (only_in_1, None));
                Ok(())
            }

            // Both open - create a fresh row variable for the union
            (Some(rv1), Some(rv2), _, _) => {
                // Both records are open, combine extra fields
                let mut combined: Vec<RecordField> = only_in_1;
                combined.extend(only_in_2);

                if combined.is_empty() {
                    // Same row variables can unify
                    if rv1 == rv2 {
                        return Ok(());
                    }
                    // Bind rv1 to rv2
                    self.row_substitutions.insert(rv1, (Vec::new(), Some(rv2)));
                } else {
                    // Create a fresh row variable for the remainder
                    let fresh_rv = self.fresh_row_var();
                    self.row_substitutions.insert(rv1, (combined.clone(), Some(fresh_rv)));
                    self.row_substitutions.insert(rv2, (combined, Some(fresh_rv)));
                }
                Ok(())
            }
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
            TypeKind::Record { fields, .. } => {
                fields.iter().any(|f| self.occurs_in(var, &f.ty))
            }
            TypeKind::Forall { params, body } => {
                // Don't check if var is in params (they're bound)
                // Only check body if var is not one of the bound params
                if params.contains(&var) {
                    false
                } else {
                    self.occurs_in(var, body)
                }
            }
            TypeKind::Closure { params, ret, .. } => {
                params.iter().any(|t| self.occurs_in(var, t)) || self.occurs_in(var, ret)
            }
            TypeKind::DynTrait { .. } => false, // DynTrait contains only DefId, no type vars
            TypeKind::Ownership { inner, .. } => self.occurs_in(var, inner),
            // Leaf types with no nested types
            TypeKind::Primitive(_) | TypeKind::Never | TypeKind::Error | TypeKind::Param(_) => false,
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
            TypeKind::Record { fields, row_var } => {
                // Resolve field types
                let mut resolved_fields: Vec<RecordField> = fields.iter()
                    .map(|f| RecordField {
                        name: f.name,
                        ty: self.resolve(&f.ty),
                    })
                    .collect();

                // Follow row variable substitutions
                let mut current_rv = *row_var;
                while let Some(rv) = current_rv {
                    if let Some((extra_fields, next_rv)) = self.row_substitutions.get(&rv) {
                        // Add extra fields from substitution
                        for ef in extra_fields {
                            // Only add if not already present
                            if !resolved_fields.iter().any(|f| f.name == ef.name) {
                                resolved_fields.push(RecordField {
                                    name: ef.name,
                                    ty: self.resolve(&ef.ty),
                                });
                            }
                        }
                        current_rv = *next_rv;
                    } else {
                        // No substitution - keep the row variable
                        break;
                    }
                }

                Type::record(resolved_fields, current_rv)
            }
            TypeKind::Forall { params, body } => {
                // Resolve body but keep params intact (they're bound)
                Type::forall(params.clone(), self.resolve(body))
            }
            TypeKind::Closure { def_id, params, ret } => {
                Type::new(TypeKind::Closure {
                    def_id: *def_id,
                    params: params.iter().map(|t| self.resolve(t)).collect(),
                    ret: self.resolve(ret),
                })
            }
            TypeKind::Ownership { qualifier, inner } => {
                Type::ownership(*qualifier, self.resolve(inner))
            }
            // Types with no nested type variables
            TypeKind::Primitive(_) | TypeKind::Never | TypeKind::Error |
            TypeKind::Param(_) | TypeKind::DynTrait { .. } => ty.clone(),
        }
    }

    /// Substitute forall-bound type parameters with given types.
    /// Used during instantiation of polymorphic types.
    fn substitute_forall_params(
        &self,
        ty: &Type,
        subst: &std::collections::HashMap<TyVarId, Type>,
    ) -> Type {
        match ty.kind() {
            TypeKind::Param(id) => {
                if let Some(replacement) = subst.get(id) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            TypeKind::Infer(id) => {
                // Also resolve inference variables through our substitution chain
                if let Some(substituted) = self.substitutions.get(id) {
                    self.substitute_forall_params(substituted, subst)
                } else {
                    ty.clone()
                }
            }
            TypeKind::Tuple(tys) => {
                Type::tuple(
                    tys.iter()
                        .map(|t| self.substitute_forall_params(t, subst))
                        .collect(),
                )
            }
            TypeKind::Array { element, size } => {
                Type::array(self.substitute_forall_params(element, subst), *size)
            }
            TypeKind::Slice { element } => {
                Type::slice(self.substitute_forall_params(element, subst))
            }
            TypeKind::Ref { inner, mutable } => {
                Type::reference(self.substitute_forall_params(inner, subst), *mutable)
            }
            TypeKind::Ptr { inner, mutable } => {
                Type::new(TypeKind::Ptr {
                    inner: self.substitute_forall_params(inner, subst),
                    mutable: *mutable,
                })
            }
            TypeKind::Fn { params, ret } => {
                Type::function(
                    params
                        .iter()
                        .map(|t| self.substitute_forall_params(t, subst))
                        .collect(),
                    self.substitute_forall_params(ret, subst),
                )
            }
            TypeKind::Adt { def_id, args } => {
                Type::adt(
                    *def_id,
                    args.iter()
                        .map(|t| self.substitute_forall_params(t, subst))
                        .collect(),
                )
            }
            TypeKind::Range { element, inclusive } => {
                Type::new(TypeKind::Range {
                    element: self.substitute_forall_params(element, subst),
                    inclusive: *inclusive,
                })
            }
            TypeKind::Record { fields, row_var } => {
                let new_fields: Vec<RecordField> = fields
                    .iter()
                    .map(|f| RecordField {
                        name: f.name,
                        ty: self.substitute_forall_params(&f.ty, subst),
                    })
                    .collect();
                Type::record(new_fields, *row_var)
            }
            TypeKind::Forall { params: inner_params, body } => {
                // Avoid capturing: skip substitution for inner-bound params
                let filtered_subst: std::collections::HashMap<TyVarId, Type> = subst
                    .iter()
                    .filter(|(k, _)| !inner_params.contains(k))
                    .map(|(k, v)| (*k, v.clone()))
                    .collect();
                Type::forall(
                    inner_params.clone(),
                    self.substitute_forall_params(body, &filtered_subst),
                )
            }
            TypeKind::Closure { def_id, params, ret } => {
                Type::new(TypeKind::Closure {
                    def_id: *def_id,
                    params: params
                        .iter()
                        .map(|t| self.substitute_forall_params(t, subst))
                        .collect(),
                    ret: self.substitute_forall_params(ret, subst),
                })
            }
            TypeKind::Ownership { qualifier, inner } => {
                Type::ownership(*qualifier, self.substitute_forall_params(inner, subst))
            }
            // Types without nested type parameters
            TypeKind::Primitive(_) | TypeKind::Never | TypeKind::Error |
            TypeKind::DynTrait { .. } => ty.clone(),
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

    // ============================================================
    // Record Type Unification Tests
    // ============================================================

    fn make_symbol(n: u32) -> crate::ast::Symbol {
        // Create a symbol from a u32 for testing
        // SAFETY: We use small values that are always valid symbol indices
        use string_interner::Symbol;
        <crate::ast::Symbol as Symbol>::try_from_usize(n as usize).unwrap()
    }

    #[test]
    fn test_unify_records_same_fields() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // { x: i32, y: bool } should unify with { x: i32, y: bool }
        let record1 = Type::record(
            vec![
                RecordField { name: make_symbol(1), ty: Type::i32() },
                RecordField { name: make_symbol(2), ty: Type::bool() },
            ],
            None,
        );
        let record2 = Type::record(
            vec![
                RecordField { name: make_symbol(1), ty: Type::i32() },
                RecordField { name: make_symbol(2), ty: Type::bool() },
            ],
            None,
        );

        assert!(u.unify(&record1, &record2, span).is_ok());
    }

    #[test]
    fn test_unify_records_different_field_types() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // { x: i32 } should NOT unify with { x: bool }
        let record1 = Type::record(
            vec![RecordField { name: make_symbol(1), ty: Type::i32() }],
            None,
        );
        let record2 = Type::record(
            vec![RecordField { name: make_symbol(1), ty: Type::bool() }],
            None,
        );

        assert!(u.unify(&record1, &record2, span).is_err());
    }

    #[test]
    fn test_unify_records_different_field_names() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // { x: i32 } should NOT unify with { y: i32 } (different field names)
        let record1 = Type::record(
            vec![RecordField { name: make_symbol(1), ty: Type::i32() }],
            None,
        );
        let record2 = Type::record(
            vec![RecordField { name: make_symbol(2), ty: Type::i32() }],
            None,
        );

        assert!(u.unify(&record1, &record2, span).is_err());
    }

    #[test]
    fn test_unify_records_different_number_of_fields() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // { x: i32 } should NOT unify with { x: i32, y: bool }
        let record1 = Type::record(
            vec![RecordField { name: make_symbol(1), ty: Type::i32() }],
            None,
        );
        let record2 = Type::record(
            vec![
                RecordField { name: make_symbol(1), ty: Type::i32() },
                RecordField { name: make_symbol(2), ty: Type::bool() },
            ],
            None,
        );

        assert!(u.unify(&record1, &record2, span).is_err());
    }

    #[test]
    fn test_unify_records_with_type_variables() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // { x: ?T } should unify with { x: i32 }, binding ?T = i32
        let var = u.fresh_var();
        let record1 = Type::record(
            vec![RecordField { name: make_symbol(1), ty: var.clone() }],
            None,
        );
        let record2 = Type::record(
            vec![RecordField { name: make_symbol(1), ty: Type::i32() }],
            None,
        );

        assert!(u.unify(&record1, &record2, span).is_ok());
        assert_eq!(u.resolve(&var), Type::i32());
    }

    #[test]
    fn test_unify_records_empty() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // {} should unify with {}
        let record1 = Type::record(vec![], None);
        let record2 = Type::record(vec![], None);

        assert!(u.unify(&record1, &record2, span).is_ok());
    }

    // ============================================================
    // Forall Type Unification Tests
    // ============================================================

    #[test]
    fn test_unify_forall_identical() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> T should unify with itself
        let param_id = TyVarId::new(100);
        let param_ty = Type::param(param_id);
        let forall1 = Type::forall(
            vec![param_id],
            Type::function(vec![param_ty.clone()], param_ty.clone()),
        );
        let forall2 = Type::forall(
            vec![param_id],
            Type::function(vec![param_ty.clone()], param_ty),
        );

        assert!(u.unify(&forall1, &forall2, span).is_ok());
    }

    #[test]
    fn test_unify_forall_alpha_equivalence() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> T should unify with forall<U>. U -> U
        // (alpha-equivalence: same structure, different bound variable names)
        let param_t = TyVarId::new(100);
        let param_u = TyVarId::new(200);

        let forall_t = Type::forall(
            vec![param_t],
            Type::function(vec![Type::param(param_t)], Type::param(param_t)),
        );
        let forall_u = Type::forall(
            vec![param_u],
            Type::function(vec![Type::param(param_u)], Type::param(param_u)),
        );

        assert!(u.unify(&forall_t, &forall_u, span).is_ok());
    }

    #[test]
    fn test_unify_forall_instantiation_left() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> T should unify with i32 -> i32
        // (instantiate T with i32)
        let param_id = TyVarId::new(100);
        let forall_ty = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::param(param_id)),
        );
        let concrete_ty = Type::function(vec![Type::i32()], Type::i32());

        assert!(u.unify(&forall_ty, &concrete_ty, span).is_ok());
    }

    #[test]
    fn test_unify_forall_instantiation_right() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // i32 -> i32 should unify with forall<T>. T -> T
        // (instantiate T with i32)
        let param_id = TyVarId::new(100);
        let forall_ty = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::param(param_id)),
        );
        let concrete_ty = Type::function(vec![Type::i32()], Type::i32());

        assert!(u.unify(&concrete_ty, &forall_ty, span).is_ok());
    }

    #[test]
    fn test_unify_forall_different_param_count() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> T vs forall<T, U>. T -> U
        // The implementation instantiates forall types asymmetrically,
        // so these CAN unify: T->T becomes ?0->?0, T->U becomes ?1->?2,
        // then ?0->?0 vs ?1->?2 unifies with ?0=?1=?2
        let param_t = TyVarId::new(100);
        let param_u = TyVarId::new(101);

        let forall1 = Type::forall(
            vec![param_t],
            Type::function(vec![Type::param(param_t)], Type::param(param_t)),
        );
        let forall2 = Type::forall(
            vec![param_t, param_u],
            Type::function(vec![Type::param(param_t)], Type::param(param_u)),
        );

        // This succeeds via instantiation (both instantiate and unify)
        assert!(u.unify(&forall1, &forall2, span).is_ok());
    }

    #[test]
    fn test_unify_forall_same_param_count_compatible_bodies() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> T vs forall<T>. T -> bool
        // The implementation substitutes SAME fresh var for T in both:
        //   ?0 -> ?0 vs ?0 -> bool
        // Params unify (?0 = ?0), return unifies (?0 = bool)
        // So both become bool -> bool - this SUCCEEDS
        let param_id = TyVarId::new(100);

        let forall1 = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::param(param_id)),
        );
        let forall2 = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::bool()),
        );

        // This unifies because both T's get the same fresh var
        assert!(u.unify(&forall1, &forall2, span).is_ok());
    }

    #[test]
    fn test_unify_forall_incompatible_bodies() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> i32 vs forall<T>. T -> bool
        // Both substitute ?0 for T:
        //   ?0 -> i32 vs ?0 -> bool
        // Params unify (?0 = ?0), but returns fail (i32 != bool)
        let param_id = TyVarId::new(100);

        let forall1 = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::i32()),
        );
        let forall2 = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::bool()),
        );

        // This fails because i32 != bool
        assert!(u.unify(&forall1, &forall2, span).is_err());
    }

    #[test]
    fn test_unify_forall_instantiation_mismatch() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> T should NOT unify with i32 -> bool
        // (T can't be both i32 and bool)
        let param_id = TyVarId::new(100);
        let forall_ty = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::param(param_id)),
        );
        let mismatched_ty = Type::function(vec![Type::i32()], Type::bool());

        assert!(u.unify(&forall_ty, &mismatched_ty, span).is_err());
    }

    #[test]
    fn test_unify_forall_multiple_params() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T, U>. T -> U should unify with forall<A, B>. A -> B
        let param_t = TyVarId::new(100);
        let param_u = TyVarId::new(101);
        let param_a = TyVarId::new(200);
        let param_b = TyVarId::new(201);

        let forall1 = Type::forall(
            vec![param_t, param_u],
            Type::function(vec![Type::param(param_t)], Type::param(param_u)),
        );
        let forall2 = Type::forall(
            vec![param_a, param_b],
            Type::function(vec![Type::param(param_a)], Type::param(param_b)),
        );

        assert!(u.unify(&forall1, &forall2, span).is_ok());
    }

    #[test]
    fn test_unify_forall_with_concrete_inner() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. i32 -> T should unify with forall<U>. i32 -> U
        let param_t = TyVarId::new(100);
        let param_u = TyVarId::new(200);

        let forall1 = Type::forall(
            vec![param_t],
            Type::function(vec![Type::i32()], Type::param(param_t)),
        );
        let forall2 = Type::forall(
            vec![param_u],
            Type::function(vec![Type::i32()], Type::param(param_u)),
        );

        assert!(u.unify(&forall1, &forall2, span).is_ok());
    }

    #[test]
    fn test_unify_forall_nested_in_tuple() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // (forall<T>. T -> T, i32) should unify with (forall<U>. U -> U, i32)
        let param_t = TyVarId::new(100);
        let param_u = TyVarId::new(200);

        let forall_t = Type::forall(
            vec![param_t],
            Type::function(vec![Type::param(param_t)], Type::param(param_t)),
        );
        let forall_u = Type::forall(
            vec![param_u],
            Type::function(vec![Type::param(param_u)], Type::param(param_u)),
        );

        let tuple1 = Type::tuple(vec![forall_t, Type::i32()]);
        let tuple2 = Type::tuple(vec![forall_u, Type::i32()]);

        assert!(u.unify(&tuple1, &tuple2, span).is_ok());
    }

    #[test]
    fn test_unify_forall_with_inference_variable() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<T>. T -> T should be assignable to an inference variable
        let param_id = TyVarId::new(100);
        let forall_ty = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::param(param_id)),
        );
        let var = u.fresh_var();

        assert!(u.unify(&var, &forall_ty, span).is_ok());

        // The variable should now resolve to the forall type
        let resolved = u.resolve(&var);
        match resolved.kind() {
            TypeKind::Forall { params, body } => {
                assert_eq!(params.len(), 1);
                // Body should be a function type T -> T
                match body.kind() {
                    TypeKind::Fn { params: fn_params, .. } => {
                        assert_eq!(fn_params.len(), 1);
                        // Both param and return should be the same Param type (or Infer after instantiation)
                    }
                    _ => panic!("Expected function body in forall"),
                }
            }
            _ => panic!("Expected forall type, got {:?}", resolved),
        }
    }

    #[test]
    fn test_unify_forall_empty_params() {
        let mut u = Unifier::new();
        let span = Span::dummy();

        // forall<>. i32 -> i32 should unify with forall<>. i32 -> i32
        // (degenerate case with no type parameters)
        let forall1 = Type::forall(vec![], Type::function(vec![Type::i32()], Type::i32()));
        let forall2 = Type::forall(vec![], Type::function(vec![Type::i32()], Type::i32()));

        assert!(u.unify(&forall1, &forall2, span).is_ok());
    }

    #[test]
    fn test_forall_occurs_check_free_var() {
        let mut u = Unifier::new();
        let _span = Span::dummy();

        // Test that occurs check finds free variables in forall body
        let outer_var = u.fresh_var(); // This creates TyVarId(0)
        let param_id = TyVarId::new(100);

        // forall<T>. T -> outer_var contains outer_var (TyVarId(0)) in the body
        let forall_ty = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], outer_var.clone()),
        );

        // TyVarId(0) DOES occur in the forall body (as a free variable)
        assert!(u.occurs_in(TyVarId::new(0), &forall_ty));
    }

    #[test]
    fn test_forall_occurs_check_bound_var() {
        let u = Unifier::new();
        let _span = Span::dummy();

        // Test that occurs check does NOT find bound variables
        let param_id = TyVarId::new(100);

        // forall<T>. T -> T - the T's (param_id=100) are bound
        let forall_ty = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::param(param_id)),
        );

        // TyVarId(100) is the bound param - occurs_in returns false because
        // the occurs check skips bound params (they're not free variables)
        assert!(!u.occurs_in(param_id, &forall_ty));
    }

    #[test]
    fn test_forall_occurs_check_unrelated_var() {
        let u = Unifier::new();
        let _span = Span::dummy();

        let param_id = TyVarId::new(100);

        // forall<T>. T -> i32
        let forall_ty = Type::forall(
            vec![param_id],
            Type::function(vec![Type::param(param_id)], Type::i32()),
        );

        // TyVarId(50) doesn't occur anywhere
        assert!(!u.occurs_in(TyVarId::new(50), &forall_ty));
    }
}
