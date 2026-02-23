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

use crate::hir::{PrimitiveTy, Type, TypeKind, TyVarId, RecordRowVarId, RecordField, FnEffect};

use super::error::{TypeError, TypeErrorKind};
use crate::span::Span;

/// Maximum type recursion depth during unification.
/// This prevents DoS attacks via pathologically nested types.
pub const MAX_TYPE_DEPTH: usize = 128;

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
    /// Current recursion depth during unification.
    depth: usize,
    /// Maximum allowed depth (configurable, defaults to MAX_TYPE_DEPTH).
    max_depth: usize,
}

impl Unifier {
    /// Create a new unifier.
    pub fn new() -> Self {
        Self {
            substitutions: HashMap::new(),
            next_var: 0,
            row_substitutions: HashMap::new(),
            next_row_var: 0,
            depth: 0,
            max_depth: MAX_TYPE_DEPTH,
        }
    }

    /// Create a new unifier with a custom depth limit.
    pub fn with_max_depth(max_depth: usize) -> Self {
        Self {
            substitutions: HashMap::new(),
            next_var: 0,
            row_substitutions: HashMap::new(),
            next_row_var: 0,
            depth: 0,
            max_depth,
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
    pub fn unify(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<(), Box<TypeError>> {
        // Check recursion depth limit (prevents DoS via pathologically nested types)
        if self.depth >= self.max_depth {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::TypeDepthExceeded {
                    depth: self.depth,
                    limit: self.max_depth,
                },
                span,
            )));
        }
        self.depth += 1;
        let result = self.unify_inner(t1, t2, span);
        self.depth -= 1;
        result
    }

    /// Internal unification without depth tracking (called by unify).
    fn unify_inner(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<(), Box<TypeError>> {
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
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: t1.clone(),
                            found: t2.clone(),
                        },
                        span,
                    )));
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

            // Arrays with same size or const param size
            (
                TypeKind::Array { element: e1, size: s1 },
                TypeKind::Array { element: e2, size: s2 },
            ) if s1 == s2 || s1.is_param() || s2.is_param() => self.unify(e1, e2, span),

            // Slices
            (TypeKind::Slice { element: e1 }, TypeKind::Slice { element: e2 }) => {
                self.unify(e1, e2, span)
            }

            // References with same mutability
            (
                TypeKind::Ref { inner: i1, mutable: m1 },
                TypeKind::Ref { inner: i2, mutable: m2 },
            ) if m1 == m2 => {
                // Try to unify inner types, with coercion for &[T; N] -> &[T]
                let i1_res = self.resolve(i1);
                let i2_res = self.resolve(i2);
                match (i1_res.kind(), i2_res.kind()) {
                    // Coercion: expected &[T], found &[T; N]
                    (TypeKind::Slice { element: elem1 }, TypeKind::Array { element: elem2, .. }) => {
                        self.unify(elem1, elem2, span)
                    }
                    // Otherwise, unify inner types normally
                    _ => self.unify(i1, i2, span)
                }
            }

            // Pointers with same mutability
            (
                TypeKind::Ptr { inner: i1, mutable: m1 },
                TypeKind::Ptr { inner: i2, mutable: m2 },
            ) if m1 == m2 => self.unify(i1, i2, span),

            // Functions
            (
                TypeKind::Fn { params: p1, ret: r1, .. },
                TypeKind::Fn { params: p2, ret: r2, .. },
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
                TypeKind::Fn { params: p2, ret: r2, .. },
            ) | (
                TypeKind::Fn { params: p1, ret: r1, .. },
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

            // Error type unifies with anything to prevent cascading errors.
            // INVARIANT: Type::error() must only be produced after a diagnostic has been
            // emitted. Violation of this invariant causes silent error absorption.
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

            // Coercion: linear T -> affine T
            // Linear is stricter than affine, so we can relax linear to affine
            (
                TypeKind::Ownership { qualifier: crate::hir::ty::OwnershipQualifier::Affine, inner: i1 },
                TypeKind::Ownership { qualifier: crate::hir::ty::OwnershipQualifier::Linear, inner: i2 },
            ) => self.unify(i1, i2, span),

            // Coercion: T -> linear T or T -> affine T
            // A plain type can be promoted to an ownership-qualified type
            // (Only when the found type is NOT an ownership type)
            (TypeKind::Ownership { inner, .. }, found) if !matches!(found, TypeKind::Ownership { .. }) => {
                self.unify(inner, &t2, span)
            }

            // Coercion: &[T; N] -> &[T] (array reference to slice reference)
            // This allows using a reference to a fixed-size array where a slice reference is expected.
            (
                TypeKind::Ref { inner: i1, mutable: m1 },
                TypeKind::Ref { inner: i2, mutable: m2 },
            ) if m1 == m2 => {
                let i1_res = self.resolve(i1);
                let i2_res = self.resolve(i2);
                match (i1_res.kind(), i2_res.kind()) {
                    // Expected: &[T], Found: &[T; N]
                    (TypeKind::Slice { element: elem1 }, TypeKind::Array { element: elem2, .. }) => {
                        self.unify(elem1, elem2, span)
                    }
                    // Otherwise, unify inner types normally
                    _ => self.unify(&i1_res, &i2_res, span)
                }
            }

            // No match
            _ => Err(Box::new(TypeError::new(
                TypeErrorKind::Mismatch {
                    expected: t1.clone(),
                    found: t2.clone(),
                },
                span,
            ))),
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
    ) -> Result<(), Box<TypeError>> {
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
                Err(Box::new(TypeError::new(
                    TypeErrorKind::Mismatch {
                        expected: Type::record(fields1.to_vec(), row_var1),
                        found: Type::record(fields2.to_vec(), row_var2),
                    },
                    span,
                )))
            }

            // Record 1 is open - bind its row var to record 2's extra fields
            (Some(rv1), None, _, false) | (Some(rv1), None, _, true) => {
                if !only_in_1.is_empty() {
                    // Record 2 is missing fields that record 1 has
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::record(fields1.to_vec(), row_var1),
                            found: Type::record(fields2.to_vec(), row_var2),
                        },
                        span,
                    )));
                }
                // Bind rv1 to the extra fields from record 2
                self.row_substitutions.insert(rv1, (only_in_2, None));
                Ok(())
            }

            // Record 2 is open - bind its row var to record 1's extra fields
            (None, Some(rv2), false, _) | (None, Some(rv2), true, _) => {
                if !only_in_2.is_empty() {
                    // Record 1 is missing fields that record 2 has
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::record(fields1.to_vec(), row_var1),
                            found: Type::record(fields2.to_vec(), row_var2),
                        },
                        span,
                    )));
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
    fn bind(&mut self, var: TyVarId, ty: Type, span: Span) -> Result<(), Box<TypeError>> {
        // Occurs check: prevent infinite types like ?T = List<?T>
        if self.occurs_in(var, &ty) {
            return Err(Box::new(TypeError::new(TypeErrorKind::InfiniteType, span)));
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
            TypeKind::Fn { params, ret, .. } => {
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
                Type::array_with_const(self.resolve(element), size.clone())
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
            TypeKind::Fn { params, ret, effects, const_args } => {
                // Resolve params, ret, and also resolve type args in effect annotations
                let resolved_effects: Vec<FnEffect> = effects.iter()
                    .map(|eff| FnEffect::new(
                        eff.def_id,
                        eff.type_args.iter().map(|arg| self.resolve(arg)).collect(),
                    ))
                    .collect();
                Type::function_with_const_args(
                    params.iter().map(|t| self.resolve(t)).collect(),
                    self.resolve(ret),
                    resolved_effects,
                    const_args.clone(),
                )
            }
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
                Type::array_with_const(self.substitute_forall_params(element, subst), size.clone())
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
            TypeKind::Fn { params, ret, .. } => {
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

    // ============================================================
    // Type Checker Soundness Property Tests (FUZZ-003)
    // ============================================================

    /// Generate a collection of representative types for property testing
    fn all_test_types() -> Vec<Type> {
        vec![
            // Primitives
            Type::i32(),
            Type::i64(),
            Type::bool(),
            Type::f64(),
            Type::unit(),
            Type::never(),
            // Compound types
            Type::tuple(vec![Type::i32(), Type::bool()]),
            Type::tuple(vec![]),
            Type::array(Type::i32(), 5),
            Type::slice(Type::bool()),
            Type::reference(Type::i32(), false),
            Type::reference(Type::i32(), true),
            Type::function(vec![Type::i32()], Type::bool()),
            Type::function(vec![], Type::unit()),
            Type::function(vec![Type::i32(), Type::bool()], Type::i64()),
            // Nested types
            Type::tuple(vec![Type::tuple(vec![Type::i32()]), Type::bool()]),
            Type::array(Type::tuple(vec![Type::i32(), Type::bool()]), 3),
            Type::function(vec![Type::tuple(vec![Type::i32()])], Type::bool()),
        ]
    }

    #[test]
    fn test_property_transitivity() {
        // Transitivity: if unify(A, B) and unify(B, C) succeed,
        // then resolve(A) = resolve(C)
        let span = Span::dummy();

        // Test with inference variables forming a chain
        let mut u = Unifier::new();
        let a = u.fresh_var();
        let b = u.fresh_var();
        let c = u.fresh_var();

        // A = B, B = C, C = i32
        assert!(u.unify(&a, &b, span).is_ok());
        assert!(u.unify(&b, &c, span).is_ok());
        assert!(u.unify(&c, &Type::i32(), span).is_ok());

        // All should resolve to i32 (transitivity)
        let resolved_a = u.resolve(&a);
        let resolved_b = u.resolve(&b);
        let resolved_c = u.resolve(&c);

        assert_eq!(resolved_a, resolved_b, "Transitivity failed: A != B");
        assert_eq!(resolved_b, resolved_c, "Transitivity failed: B != C");
        assert_eq!(resolved_a, Type::i32(), "Transitivity failed: A != i32");
    }

    #[test]
    fn test_property_transitivity_complex() {
        // Transitivity with complex types
        let span = Span::dummy();
        let mut u = Unifier::new();

        let a = u.fresh_var();
        let b = u.fresh_var();
        let c = u.fresh_var();

        let target = Type::tuple(vec![Type::i32(), Type::bool()]);

        // Chain: A = B = C = (i32, bool)
        assert!(u.unify(&a, &b, span).is_ok());
        assert!(u.unify(&b, &c, span).is_ok());
        assert!(u.unify(&c, &target, span).is_ok());

        assert_eq!(u.resolve(&a), target);
        assert_eq!(u.resolve(&b), target);
        assert_eq!(u.resolve(&c), target);
    }

    #[test]
    fn test_property_idempotence_of_resolution() {
        // Idempotence: resolve(resolve(T)) = resolve(T) for all T
        let span = Span::dummy();

        for ty in all_test_types() {
            let mut u = Unifier::new();

            // Create a variable and bind it to ty
            let var = u.fresh_var();
            if u.unify(&var, &ty, span).is_ok() {
                let once = u.resolve(&var);
                let twice = u.resolve(&once);
                assert_eq!(
                    once, twice,
                    "Idempotence failed for {:?}: resolve != resolve(resolve)",
                    ty
                );
            }
        }
    }

    #[test]
    fn test_property_idempotence_concrete_types() {
        // For concrete types, resolution should be identity
        let u = Unifier::new();

        for ty in all_test_types() {
            let resolved = u.resolve(&ty);
            let resolved_again = u.resolve(&resolved);
            assert_eq!(
                resolved, resolved_again,
                "Idempotence failed for concrete type {:?}",
                ty
            );
        }
    }

    #[test]
    fn test_property_substitution_consistency() {
        // After successful unification, both sides resolve to the same type
        // Note: Never and Error types are "absorbing" elements - they unify with
        // anything but don't bind variables. This is intentional behavior.
        let span = Span::dummy();

        for ty in all_test_types() {
            // Skip absorbing types (Never, Error) which don't bind variables
            if ty.is_never() || matches!(ty.kind(), TypeKind::Error) {
                continue;
            }

            let mut u = Unifier::new();
            let var = u.fresh_var();

            if u.unify(&var, &ty, span).is_ok() {
                let resolved_var = u.resolve(&var);
                let resolved_ty = u.resolve(&ty);
                assert_eq!(
                    resolved_var, resolved_ty,
                    "Substitution consistency failed: resolved var != resolved type for {:?}",
                    ty
                );
            }
        }
    }

    #[test]
    fn test_property_substitution_consistency_bidirectional() {
        // Unification should be symmetric: unify(A, B) and unify(B, A)
        // should produce equivalent results
        let span = Span::dummy();

        for ty in all_test_types() {
            let mut u1 = Unifier::new();
            let mut u2 = Unifier::new();

            let var1 = u1.fresh_var();
            let var2 = u2.fresh_var();

            let result1 = u1.unify(&var1, &ty, span);
            let result2 = u2.unify(&ty, &var2, span);

            // Both should succeed or both fail
            assert_eq!(
                result1.is_ok(),
                result2.is_ok(),
                "Symmetry violated for {:?}",
                ty
            );

            if result1.is_ok() {
                assert_eq!(
                    u1.resolve(&var1),
                    u2.resolve(&var2),
                    "Symmetric unification produced different results for {:?}",
                    ty
                );
            }
        }
    }

    #[test]
    fn test_property_determinism() {
        // Same unifications should produce same results
        let span = Span::dummy();

        for _ in 0..10 {
            let mut u1 = Unifier::new();
            let mut u2 = Unifier::new();

            let v1_a = u1.fresh_var();
            let v1_b = u1.fresh_var();
            let v2_a = u2.fresh_var();
            let v2_b = u2.fresh_var();

            // Same sequence of unifications
            let _ = u1.unify(&v1_a, &Type::i32(), span);
            let _ = u1.unify(&v1_b, &v1_a, span);

            let _ = u2.unify(&v2_a, &Type::i32(), span);
            let _ = u2.unify(&v2_b, &v2_a, span);

            assert_eq!(
                u1.resolve(&v1_a),
                u2.resolve(&v2_a),
                "Determinism failed"
            );
            assert_eq!(
                u1.resolve(&v1_b),
                u2.resolve(&v2_b),
                "Determinism failed"
            );
        }
    }

    #[test]
    fn test_property_monotonicity() {
        // Once a variable is bound, it stays bound to that type
        let span = Span::dummy();
        let mut u = Unifier::new();

        let var = u.fresh_var();

        // Bind to i32
        assert!(u.unify(&var, &Type::i32(), span).is_ok());
        let bound_type = u.resolve(&var);
        assert_eq!(bound_type, Type::i32());

        // Try to unify with same type - should succeed
        assert!(u.unify(&var, &Type::i32(), span).is_ok());
        assert_eq!(u.resolve(&var), bound_type, "Monotonicity violated: type changed");

        // Try to unify with different type - should fail
        assert!(u.unify(&var, &Type::bool(), span).is_err());
        // Variable should still be bound to original type
        assert_eq!(u.resolve(&var), bound_type, "Monotonicity violated after failed unification");
    }

    #[test]
    fn test_property_type_preservation() {
        // Resolution of concrete types should not change them
        let u = Unifier::new();

        for ty in all_test_types() {
            if !ty.has_type_vars() {
                let resolved = u.resolve(&ty);
                assert_eq!(
                    ty, resolved,
                    "Type preservation failed: concrete type {:?} changed to {:?}",
                    ty, resolved
                );
            }
        }
    }

    #[test]
    fn test_property_unification_preserves_structure() {
        // After unification, resolved types should have expected structure
        let span = Span::dummy();
        let mut u = Unifier::new();

        // Create tuple with variables: (?A, ?B)
        let var_a = u.fresh_var();
        let var_b = u.fresh_var();
        let tuple_with_vars = Type::tuple(vec![var_a.clone(), var_b.clone()]);

        // Unify with concrete tuple
        let concrete_tuple = Type::tuple(vec![Type::i32(), Type::bool()]);
        assert!(u.unify(&tuple_with_vars, &concrete_tuple, span).is_ok());

        // Check structure is preserved
        let resolved = u.resolve(&tuple_with_vars);
        match resolved.kind() {
            TypeKind::Tuple(elements) => {
                assert_eq!(elements.len(), 2);
                assert_eq!(elements[0], Type::i32());
                assert_eq!(elements[1], Type::bool());
            }
            _ => panic!("Structure not preserved: expected tuple, got {:?}", resolved),
        }
    }

    #[test]
    fn test_property_function_structure_preservation() {
        // Function type structure should be preserved through unification
        let span = Span::dummy();
        let mut u = Unifier::new();

        let var_param = u.fresh_var();
        let var_ret = u.fresh_var();
        let fn_with_vars = Type::function(vec![var_param.clone()], var_ret.clone());

        let concrete_fn = Type::function(vec![Type::i32()], Type::bool());
        assert!(u.unify(&fn_with_vars, &concrete_fn, span).is_ok());

        let resolved = u.resolve(&fn_with_vars);
        match resolved.kind() {
            TypeKind::Fn { params, ret, .. } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0], Type::i32());
                assert_eq!(*ret, Type::bool());
            }
            _ => panic!("Function structure not preserved"),
        }
    }

    #[test]
    fn test_property_no_information_loss() {
        // Unifying two concrete compatible types should not lose information
        let span = Span::dummy();
        let mut u = Unifier::new();

        let ty1 = Type::tuple(vec![Type::i32(), Type::bool(), Type::f64()]);
        let ty2 = Type::tuple(vec![Type::i32(), Type::bool(), Type::f64()]);

        assert!(u.unify(&ty1, &ty2, span).is_ok());

        // Both should still resolve to the same complete type
        let resolved1 = u.resolve(&ty1);
        let resolved2 = u.resolve(&ty2);

        assert_eq!(resolved1, resolved2);
        match resolved1.kind() {
            TypeKind::Tuple(elements) => {
                assert_eq!(elements.len(), 3);
            }
            _ => panic!("Information lost"),
        }
    }

    #[test]
    fn test_property_error_type_absorption() {
        // Error type should unify with anything (for error recovery)
        let span = Span::dummy();

        for ty in all_test_types() {
            let mut u = Unifier::new();
            assert!(
                u.unify(&Type::error(), &ty, span).is_ok(),
                "Error type should absorb {:?}",
                ty
            );

            let mut u2 = Unifier::new();
            assert!(
                u2.unify(&ty, &Type::error(), span).is_ok(),
                "Error type should be absorbed by {:?}",
                ty
            );
        }
    }

    #[test]
    fn test_property_never_type_absorption() {
        // Never type should unify with anything (it's the bottom type)
        let span = Span::dummy();

        for ty in all_test_types() {
            let mut u = Unifier::new();
            assert!(
                u.unify(&Type::never(), &ty, span).is_ok(),
                "Never type should unify with {:?}",
                ty
            );

            let mut u2 = Unifier::new();
            assert!(
                u2.unify(&ty, &Type::never(), span).is_ok(),
                "{:?} should unify with never type",
                ty
            );
        }
    }

    #[test]
    fn test_property_occurs_check_soundness() {
        // Occurs check should prevent all infinite types
        let span = Span::dummy();

        let mut u = Unifier::new();
        let var = u.fresh_var();

        // All these should fail due to occurs check
        let infinite_types = vec![
            Type::tuple(vec![var.clone()]),
            Type::array(var.clone(), 1),
            Type::slice(var.clone()),
            Type::reference(var.clone(), false),
            Type::function(vec![var.clone()], Type::bool()),
            Type::function(vec![Type::i32()], var.clone()),
            Type::tuple(vec![Type::i32(), Type::tuple(vec![var.clone()])]),
        ];

        for infinite_ty in infinite_types {
            let mut u_test = Unifier::new();
            let test_var = u_test.fresh_var();
            assert!(
                u_test.unify(&test_var, &infinite_ty, span).is_err(),
                "Occurs check should prevent infinite type {:?}",
                infinite_ty
            );
        }
    }

    #[test]
    fn test_property_fresh_var_independence() {
        // Fresh variables should be independent
        let mut u = Unifier::new();
        let span = Span::dummy();

        let vars: Vec<Type> = (0..10).map(|_| u.fresh_var()).collect();

        // Binding one should not affect others
        u.unify(&vars[0], &Type::i32(), span).unwrap();

        for (i, var) in vars.iter().enumerate().skip(1) {
            // Other variables should still be unresolved (or at least not i32 unless unified)
            let resolved = u.resolve(var);
            // Check they haven't been accidentally bound
            if let TypeKind::Infer(_) = resolved.kind() {
                // Still unbound - good
            } else {
                panic!("Variable {} was unexpectedly bound to {:?}", i, resolved);
            }
        }
    }

    #[test]
    fn test_property_array_size_must_match() {
        // Arrays with different sizes should never unify
        let span = Span::dummy();

        for size1 in [0, 1, 5, 10, 100] {
            for size2 in [0, 1, 5, 10, 100] {
                let mut u = Unifier::new();
                let arr1 = Type::array(Type::i32(), size1);
                let arr2 = Type::array(Type::i32(), size2);

                let result = u.unify(&arr1, &arr2, span);
                if size1 == size2 {
                    assert!(result.is_ok(), "Same size arrays should unify");
                } else {
                    assert!(result.is_err(), "Different size arrays should not unify");
                }
            }
        }
    }

    #[test]
    fn test_property_mutability_must_match() {
        // References with different mutability should not unify
        let span = Span::dummy();

        let mut u = Unifier::new();
        let ref_mut = Type::reference(Type::i32(), true);
        let ref_imm = Type::reference(Type::i32(), false);

        assert!(u.unify(&ref_mut, &ref_imm, span).is_err());
        assert!(u.unify(&ref_imm, &ref_mut, span).is_err());

        // Same mutability should unify
        let mut u2 = Unifier::new();
        assert!(u2.unify(&ref_mut, &Type::reference(Type::i32(), true), span).is_ok());
        assert!(u2.unify(&ref_imm, &Type::reference(Type::i32(), false), span).is_ok());
    }

    #[test]
    fn test_property_arity_must_match() {
        // Functions with different arities should not unify
        let span = Span::dummy();

        for arity1 in 0..5 {
            for arity2 in 0..5 {
                let mut u = Unifier::new();
                let params1: Vec<Type> = (0..arity1).map(|_| Type::i32()).collect();
                let params2: Vec<Type> = (0..arity2).map(|_| Type::i32()).collect();
                let fn1 = Type::function(params1, Type::bool());
                let fn2 = Type::function(params2, Type::bool());

                let result = u.unify(&fn1, &fn2, span);
                if arity1 == arity2 {
                    assert!(result.is_ok(), "Same arity functions should unify");
                } else {
                    assert!(result.is_err(), "Different arity functions should not unify");
                }
            }
        }
    }

    #[test]
    fn test_property_tuple_length_must_match() {
        // Tuples with different lengths should not unify
        let span = Span::dummy();

        for len1 in 0..5 {
            for len2 in 0..5 {
                let mut u = Unifier::new();
                let elems1: Vec<Type> = (0..len1).map(|_| Type::i32()).collect();
                let elems2: Vec<Type> = (0..len2).map(|_| Type::i32()).collect();
                let tuple1 = Type::tuple(elems1);
                let tuple2 = Type::tuple(elems2);

                let result = u.unify(&tuple1, &tuple2, span);
                if len1 == len2 {
                    assert!(result.is_ok(), "Same length tuples should unify");
                } else {
                    assert!(result.is_err(), "Different length tuples should not unify");
                }
            }
        }
    }

    // ============================================================
    // Property-Based Tests for Type Checker Soundness (FUZZ-003)
    // ============================================================

    /// Generate arbitrary primitive types for property testing.
    fn arbitrary_primitive_types() -> Vec<Type> {
        vec![
            Type::bool(),
            Type::i32(),
            Type::i64(),
            Type::u8(),
            Type::u32(),
            Type::u64(),
            Type::f32(),
            Type::f64(),
            Type::char(),
            Type::unit(),
            Type::string(),
        ]
    }

    /// Generate arbitrary compound types up to a given depth.
    fn arbitrary_types(depth: usize) -> Vec<Type> {
        let primitives = arbitrary_primitive_types();
        if depth == 0 {
            return primitives;
        }

        let mut types = primitives.clone();
        let inner_types = arbitrary_types(depth - 1);

        // Add tuple types
        for t in inner_types.iter().take(3) {
            types.push(Type::tuple(vec![t.clone()]));
            types.push(Type::tuple(vec![t.clone(), Type::i32()]));
        }

        // Add array types
        for t in inner_types.iter().take(3) {
            types.push(Type::array(t.clone(), 5));
        }

        // Add slice types
        for t in inner_types.iter().take(3) {
            types.push(Type::slice(t.clone()));
        }

        // Add reference types
        for t in inner_types.iter().take(3) {
            types.push(Type::reference(t.clone(), false));
            types.push(Type::reference(t.clone(), true));
        }

        // Add function types
        for t in inner_types.iter().take(2) {
            types.push(Type::function(vec![t.clone()], Type::i32()));
            types.push(Type::function(vec![Type::i32()], t.clone()));
        }

        types
    }

    // PROPERTY: Reflexivity - Every type unifies with itself
    #[test]
    fn test_property_reflexivity() {
        let span = Span::dummy();
        let types = arbitrary_types(2);

        for ty in &types {
            let mut u = Unifier::new();
            let result = u.unify(ty, ty, span);
            assert!(
                result.is_ok(),
                "Reflexivity violated: {:?} should unify with itself",
                ty
            );
        }
    }

    // PROPERTY: Symmetry - If unify(a, b) succeeds, unify(b, a) also succeeds
    #[test]
    fn test_property_symmetry_success() {
        let span = Span::dummy();
        let types = arbitrary_types(2);

        for t1 in &types {
            for t2 in &types {
                let mut u1 = Unifier::new();
                let mut u2 = Unifier::new();

                let result1 = u1.unify(t1, t2, span);
                let result2 = u2.unify(t2, t1, span);

                // If one succeeds, both should succeed (or both fail)
                assert_eq!(
                    result1.is_ok(),
                    result2.is_ok(),
                    "Symmetry violated: unify({:?}, {:?}) = {:?}, but unify({:?}, {:?}) = {:?}",
                    t1, t2, result1.is_ok(),
                    t2, t1, result2.is_ok()
                );
            }
        }
    }

    // PROPERTY: Variable symmetry - Unification with variables is order-independent
    #[test]
    fn test_property_variable_symmetry() {
        let span = Span::dummy();
        let types = arbitrary_types(1);

        for ty in &types {
            // Order: var first, then concrete
            let mut u1 = Unifier::new();
            let var1 = u1.fresh_var();
            let result1 = u1.unify(&var1, ty, span);
            let resolved1 = u1.resolve(&var1);

            // Order: concrete first, then var
            let mut u2 = Unifier::new();
            let var2 = u2.fresh_var();
            let result2 = u2.unify(ty, &var2, span);
            let resolved2 = u2.resolve(&var2);

            assert!(result1.is_ok(), "Variable unification should succeed");
            assert!(result2.is_ok(), "Variable unification should succeed (reverse)");
            assert_eq!(
                resolved1, resolved2,
                "Variable resolution should be order-independent for {:?}",
                ty
            );
        }
    }

    // PROPERTY: Resolution idempotence - resolve(resolve(t)) = resolve(t)
    #[test]
    fn test_property_resolution_idempotence() {
        let span = Span::dummy();
        let types = arbitrary_types(2);

        for ty in &types {
            let mut u = Unifier::new();

            // Create some bindings
            let var1 = u.fresh_var();
            let var2 = u.fresh_var();
            let _ = u.unify(&var1, &var2, span);
            let _ = u.unify(&var2, ty, span);

            // Resolution should be idempotent
            let once = u.resolve(ty);
            let twice = u.resolve(&once);
            assert_eq!(
                once, twice,
                "Resolution idempotence violated for {:?}",
                ty
            );
        }
    }

    // PROPERTY: Variable transitivity - If var1 = var2 and var2 = T, then var1 = T
    #[test]
    fn test_property_variable_transitivity() {
        let span = Span::dummy();
        let types = arbitrary_types(1);

        for ty in &types {
            let mut u = Unifier::new();
            let var1 = u.fresh_var();
            let var2 = u.fresh_var();
            let var3 = u.fresh_var();

            // Chain: var1 = var2, var2 = var3, var3 = ty
            assert!(u.unify(&var1, &var2, span).is_ok());
            assert!(u.unify(&var2, &var3, span).is_ok());
            assert!(u.unify(&var3, ty, span).is_ok());

            // All should resolve to the same type
            let r1 = u.resolve(&var1);
            let r2 = u.resolve(&var2);
            let r3 = u.resolve(&var3);

            assert_eq!(r1, *ty, "var1 should resolve to {:?}", ty);
            assert_eq!(r2, *ty, "var2 should resolve to {:?}", ty);
            assert_eq!(r3, *ty, "var3 should resolve to {:?}", ty);
        }
    }

    // PROPERTY: Occurs check - Nested variable resolution
    #[test]
    fn test_property_nested_var_resolution() {
        let span = Span::dummy();

        // Test various potential infinite type constructions
        // For types that don't have direct self-reference, occurs check may not apply
        // This tests that well-formed types don't accidentally trigger occurs check

        let mut u = Unifier::new();
        let var = u.fresh_var();

        // Unifying var with a type containing var would create infinite type
        // This is tested indirectly through nested structures
        // Direct occurs check: var = Array[var] should fail (if implemented)
        // For now, verify that non-infinite types work correctly
        assert!(u.unify(&var, &Type::i32(), span).is_ok());

        // Fresh context - verify that nested vars work correctly
        let mut u2 = Unifier::new();
        let v1 = u2.fresh_var();
        let v2 = u2.fresh_var();
        let array_type = Type::array(v2.clone(), 3);

        assert!(u2.unify(&v1, &array_type, span).is_ok());
        assert!(u2.unify(&v2, &Type::i32(), span).is_ok());

        let resolved = u2.resolve(&v1);
        assert_eq!(resolved, Type::array(Type::i32(), 3));
    }

    // PROPERTY: Type constructor distinctness - Different type constructors don't unify
    #[test]
    fn test_property_constructor_distinctness() {
        let span = Span::dummy();

        // Create one representative of each type constructor
        let constructors = vec![
            ("Primitive", Type::i32()),
            ("Tuple", Type::tuple(vec![Type::i32()])),
            ("Array", Type::array(Type::i32(), 5)),
            ("Slice", Type::slice(Type::i32())),
            ("RefMut", Type::reference(Type::i32(), true)),
            ("RefImm", Type::reference(Type::i32(), false)),
            ("Function", Type::function(vec![Type::i32()], Type::i32())),
        ];

        // Different type constructors should never unify
        for (name1, t1) in &constructors {
            for (name2, t2) in &constructors {
                if name1 == name2 {
                    continue;
                }
                let mut u = Unifier::new();
                let result = u.unify(t1, t2, span);
                assert!(
                    result.is_err(),
                    "Constructor distinctness violated: {} and {} should not unify",
                    name1, name2
                );
            }
        }
    }

    // PROPERTY: Substitution produces equal types - Same substitution on equal types yields equal results
    #[test]
    fn test_property_substitution_equality() {
        let span = Span::dummy();
        let types = arbitrary_types(1);

        for ty in &types {
            let mut u = Unifier::new();
            let var = u.fresh_var();

            // Unify var with the type
            assert!(u.unify(&var, ty, span).is_ok());

            // Create two expressions that should be equal after substitution
            let expr1 = Type::tuple(vec![var.clone(), Type::i32()]);
            let expr2 = Type::tuple(vec![var.clone(), Type::i32()]);

            let resolved1 = u.resolve(&expr1);
            let resolved2 = u.resolve(&expr2);

            assert_eq!(
                resolved1, resolved2,
                "Substitution consistency violated for {:?}",
                ty
            );

            // The resolved type should contain the substituted value
            let expected = Type::tuple(vec![ty.clone(), Type::i32()]);
            assert_eq!(
                resolved1, expected,
                "Substitution should replace var with {:?}",
                ty
            );
        }
    }

    // PROPERTY: Unification determinism - Same inputs produce same result
    #[test]
    fn test_property_unification_determinism() {
        let span = Span::dummy();
        let types = arbitrary_types(2);

        for t1 in &types {
            for t2 in &types {
                // Run unification twice with fresh unifiers
                let mut u1 = Unifier::new();
                let mut u2 = Unifier::new();

                let result1 = u1.unify(t1, t2, span);
                let result2 = u2.unify(t1, t2, span);

                // Results should be identical
                assert_eq!(
                    result1.is_ok(),
                    result2.is_ok(),
                    "Determinism violated: unify({:?}, {:?}) gave different results",
                    t1, t2
                );
            }
        }
    }

    // PROPERTY: Fresh variable independence - Fresh variables don't interfere with each other
    #[test]
    fn test_property_fresh_variable_independence() {
        let span = Span::dummy();

        let mut u = Unifier::new();

        // Create many fresh variables
        let vars: Vec<Type> = (0..10).map(|_| u.fresh_var()).collect();

        // Bind some to specific types
        assert!(u.unify(&vars[0], &Type::i32(), span).is_ok());
        assert!(u.unify(&vars[2], &Type::bool(), span).is_ok());
        assert!(u.unify(&vars[4], &Type::string(), span).is_ok());

        // Verify independence - unbound variables should still be variables
        for (i, var) in vars.iter().enumerate() {
            let resolved = u.resolve(var);
            match i {
                0 => assert_eq!(resolved, Type::i32()),
                2 => assert_eq!(resolved, Type::bool()),
                4 => assert_eq!(resolved, Type::string()),
                _ => {
                    // Unbound variables resolve to themselves (or another unbound var)
                    // The key property is they're not bound to something else
                    assert!(
                        resolved != Type::i32() && resolved != Type::bool() && resolved != Type::string(),
                        "Fresh variable {:?} should be independent",
                        i
                    );
                }
            }
        }
    }

    // PROPERTY: Nested unification soundness - Unification propagates through nested types
    #[test]
    fn test_property_nested_unification_soundness() {
        let span = Span::dummy();

        let mut u = Unifier::new();
        let var = u.fresh_var();

        // Create nested types with the variable
        let nested1 = Type::tuple(vec![Type::array(var.clone(), 3), Type::i32()]);
        let nested2 = Type::tuple(vec![Type::array(Type::bool(), 3), Type::i32()]);

        // Unifying should bind the variable
        assert!(u.unify(&nested1, &nested2, span).is_ok());

        // The variable should now be bool
        let resolved = u.resolve(&var);
        assert_eq!(
            resolved,
            Type::bool(),
            "Nested unification should bind inner variable"
        );
    }

    // PROPERTY: Function contravariance check (parameters should unify)
    #[test]
    fn test_property_function_parameter_unification() {
        let span = Span::dummy();

        let mut u = Unifier::new();
        let var = u.fresh_var();

        let fn1 = Type::function(vec![var.clone()], Type::i32());
        let fn2 = Type::function(vec![Type::bool()], Type::i32());

        assert!(u.unify(&fn1, &fn2, span).is_ok());

        // Parameter should be unified
        let resolved = u.resolve(&var);
        assert_eq!(resolved, Type::bool());
    }

    // PROPERTY: Function return type unification
    #[test]
    fn test_property_function_return_unification() {
        let span = Span::dummy();

        let mut u = Unifier::new();
        let var = u.fresh_var();

        let fn1 = Type::function(vec![Type::i32()], var.clone());
        let fn2 = Type::function(vec![Type::i32()], Type::string());

        assert!(u.unify(&fn1, &fn2, span).is_ok());

        // Return type should be unified
        let resolved = u.resolve(&var);
        assert_eq!(resolved, Type::string());
    }

    // ============================================================
    // Depth Limit Tests (Security)
    // ============================================================

    #[test]
    fn test_depth_limit_exceeded() {
        let span = Span::dummy();

        // Create a unifier with a small depth limit for testing
        let mut u = Unifier::with_max_depth(3);

        // Create nested tuple types that exceed the depth limit
        // Depth 1: (i32, (i32, (i32, (i32, i32))))
        let inner = Type::tuple(vec![Type::i32(), Type::i32()]);
        let nested1 = Type::tuple(vec![Type::i32(), inner]);
        let nested2 = Type::tuple(vec![Type::i32(), nested1]);
        let deeply_nested = Type::tuple(vec![Type::i32(), nested2]);

        // Try to unify with itself - this requires recursing into the structure
        // With max_depth=3, this should fail because the nesting is 4 levels deep
        let result = u.unify(&deeply_nested, &deeply_nested, span);

        // The unification should fail due to depth limit
        assert!(result.is_err());
        let err = result.unwrap_err();
        match &err.kind {
            TypeErrorKind::TypeDepthExceeded { depth: _, limit } => {
                assert_eq!(*limit, 3);
            }
            other => panic!("Expected TypeDepthExceeded, got {:?}", other),
        }
    }

    #[test]
    fn test_depth_limit_not_exceeded() {
        let span = Span::dummy();

        // Create a unifier with the default depth limit
        let mut u = Unifier::new();

        // Create a reasonably nested type (should be fine with default limit)
        let nested = Type::tuple(vec![
            Type::i32(),
            Type::tuple(vec![
                Type::bool(),
                Type::tuple(vec![Type::string()]),
            ]),
        ]);

        // This should succeed
        assert!(u.unify(&nested, &nested, span).is_ok());
    }

    #[test]
    fn test_default_max_depth() {
        // Verify the default max depth is set correctly
        let u = Unifier::new();
        assert_eq!(u.max_depth, MAX_TYPE_DEPTH);
        assert_eq!(u.max_depth, 128);
    }

    #[test]
    fn test_custom_max_depth() {
        let u = Unifier::with_max_depth(50);
        assert_eq!(u.max_depth, 50);
    }
}
