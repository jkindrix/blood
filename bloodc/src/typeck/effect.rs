//! Effect row unification for Blood.
//!
//! This module implements effect row unification following the approach from:
//! - [Koka: Programming with Row-Polymorphic Effect Types](https://arxiv.org/abs/1406.2061)
//! - [Adding row polymorphism to Damas-Hindley-Milner](https://bernsteinbear.com/blog/row-poly/)
//!
//! ## Row Polymorphism
//!
//! Effect rows are ordered sets with an optional row variable for polymorphism:
//!
//! ```text
//! EffectRow ::= pure                     -- No effects
//!             | {E₁, E₂, ...}            -- Closed row
//!             | {E₁, E₂, ... | ρ}        -- Open row with row variable ρ
//! ```
//!
//! ## Unification Algorithm
//!
//! Row unification follows these rules:
//!
//! 1. **Empty rows**: `pure` unifies only with `pure` or row variables
//! 2. **Common effects**: Extract matching effects, unify their type arguments
//! 3. **Row extension**: Remaining effects extend via row variable substitution
//!
//! ## Example
//!
//! ```text
//! unify({State<i32>, IO | ρ₁}, {State<α>, Error | ρ₂})
//!   => State<i32> ~ State<α>     // α = i32
//!   => ρ₁ = {Error | ρ₂}         // Row extension
//!   => ρ₂ = {IO | ρ₃}            // Fresh row variable
//! ```

use std::collections::{HashMap, BTreeSet};
use crate::hir::Type;
use crate::effects::row::{EffectRow, EffectRef, RowVar};

#[cfg(test)]
use crate::hir::DefId;
use crate::span::Span;
use super::error::{TypeError, TypeErrorKind};
use super::unify::Unifier;

/// Row variable identifier for effect polymorphism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowVarId(pub u32);

impl RowVarId {
    /// Create a new row variable ID.
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Effect row unifier extends the type unifier with row polymorphism.
#[derive(Debug)]
pub struct EffectUnifier {
    /// Type unifier for effect type arguments.
    type_unifier: Unifier,
    /// Row variable substitutions.
    row_substitutions: HashMap<RowVarId, EffectRow>,
    /// Next row variable ID.
    next_row_var: u32,
}

impl EffectUnifier {
    /// Create a new effect unifier.
    pub fn new() -> Self {
        Self {
            type_unifier: Unifier::new(),
            row_substitutions: HashMap::new(),
            next_row_var: 0,
        }
    }

    /// Create a new effect unifier with an existing type unifier.
    pub fn with_type_unifier(type_unifier: Unifier) -> Self {
        Self {
            type_unifier,
            row_substitutions: HashMap::new(),
            next_row_var: 0,
        }
    }

    /// Create a fresh row variable.
    pub fn fresh_row_var(&mut self) -> RowVar {
        let id = self.next_row_var;
        self.next_row_var += 1;
        RowVar::new(id)
    }

    /// Get the type unifier.
    pub fn type_unifier(&self) -> &Unifier {
        &self.type_unifier
    }

    /// Get the mutable type unifier.
    pub fn type_unifier_mut(&mut self) -> &mut Unifier {
        &mut self.type_unifier
    }

    /// Unify two effect rows.
    ///
    /// Returns Ok(()) if unification succeeds, Err if rows are incompatible.
    pub fn unify_rows(
        &mut self,
        row1: &EffectRow,
        row2: &EffectRow,
        span: Span,
    ) -> Result<(), TypeError> {
        // Resolve any existing substitutions
        let row1 = self.resolve_row(row1);
        let row2 = self.resolve_row(row2);

        // Handle pure rows
        if row1.is_pure() && row2.is_pure() {
            return Ok(());
        }

        // If one is pure and the other has effects, they can only unify
        // if the effectful row has a row variable
        if row1.is_pure() {
            return self.unify_pure_with_row(&row2, span);
        }
        if row2.is_pure() {
            return self.unify_pure_with_row(&row1, span);
        }

        // Both rows have effects - find common and unique effects
        let effects1: BTreeSet<_> = row1.effects().cloned().collect();
        let effects2: BTreeSet<_> = row2.effects().cloned().collect();

        // Unify type arguments for effects with the same DefId
        for e1 in &effects1 {
            if let Some(e2) = effects2.iter().find(|e| e.def_id == e1.def_id) {
                self.unify_effect_args(e1, e2, span)?;
            }
        }

        // Find effects unique to each row
        let only_in_1: Vec<_> = effects1
            .iter()
            .filter(|e| !effects2.iter().any(|e2| e2.def_id == e.def_id))
            .cloned()
            .collect();
        let only_in_2: Vec<_> = effects2
            .iter()
            .filter(|e| !effects1.iter().any(|e1| e1.def_id == e.def_id))
            .cloned()
            .collect();

        // Handle row extensions via row variables
        match (row1.row_var(), row2.row_var()) {
            // Both have row variables - unify via extension
            (Some(rv1), Some(rv2)) => {
                if rv1.0 == rv2.0 {
                    // Same row variable - effects must be identical
                    if !only_in_1.is_empty() || !only_in_2.is_empty() {
                        return Err(TypeError::new(
                            TypeErrorKind::EffectMismatch {
                                expected: format!("{:?}", row1),
                                found: format!("{:?}", row2),
                            },
                            span,
                        ));
                    }
                    Ok(())
                } else {
                    // Different row variables - create extensions
                    let fresh = self.fresh_row_var();

                    // rv1 = {only_in_2... | fresh}
                    let mut ext1 = EffectRow::polymorphic(fresh);
                    for e in &only_in_2 {
                        ext1.add_effect(e.clone());
                    }
                    self.bind_row_var(RowVarId(rv1.0), ext1);

                    // rv2 = {only_in_1... | fresh}
                    let mut ext2 = EffectRow::polymorphic(fresh);
                    for e in &only_in_1 {
                        ext2.add_effect(e.clone());
                    }
                    self.bind_row_var(RowVarId(rv2.0), ext2);

                    Ok(())
                }
            }

            // Only row1 has a row variable
            (Some(rv1), None) => {
                if only_in_2.is_empty() {
                    // row1's extra effects must come from rv1
                    // rv1 must be empty (pure)
                    if only_in_1.is_empty() {
                        self.bind_row_var(RowVarId(rv1.0), EffectRow::pure());
                        Ok(())
                    } else {
                        // row2 is closed but doesn't have all of row1's effects
                        Err(TypeError::new(
                            TypeErrorKind::EffectMismatch {
                                expected: format!("{:?}", row1),
                                found: format!("{:?}", row2),
                            },
                            span,
                        ))
                    }
                } else {
                    // row2 has effects not in row1's concrete part
                    // rv1 = {only_in_2...}
                    let mut ext = EffectRow::pure();
                    for e in &only_in_2 {
                        ext.add_effect(e.clone());
                    }

                    if only_in_1.is_empty() {
                        self.bind_row_var(RowVarId(rv1.0), ext);
                        Ok(())
                    } else {
                        // Mismatch - row1 has effects not in closed row2
                        Err(TypeError::new(
                            TypeErrorKind::EffectMismatch {
                                expected: format!("{:?}", row1),
                                found: format!("{:?}", row2),
                            },
                            span,
                        ))
                    }
                }
            }

            // Only row2 has a row variable
            (None, Some(rv2)) => {
                // Symmetric case
                if only_in_1.is_empty() {
                    if only_in_2.is_empty() {
                        self.bind_row_var(RowVarId(rv2.0), EffectRow::pure());
                        Ok(())
                    } else {
                        Err(TypeError::new(
                            TypeErrorKind::EffectMismatch {
                                expected: format!("{:?}", row1),
                                found: format!("{:?}", row2),
                            },
                            span,
                        ))
                    }
                } else {
                    let mut ext = EffectRow::pure();
                    for e in &only_in_1 {
                        ext.add_effect(e.clone());
                    }

                    if only_in_2.is_empty() {
                        self.bind_row_var(RowVarId(rv2.0), ext);
                        Ok(())
                    } else {
                        Err(TypeError::new(
                            TypeErrorKind::EffectMismatch {
                                expected: format!("{:?}", row1),
                                found: format!("{:?}", row2),
                            },
                            span,
                        ))
                    }
                }
            }

            // Neither has a row variable - closed rows must be equal
            (None, None) => {
                if only_in_1.is_empty() && only_in_2.is_empty() {
                    Ok(())
                } else {
                    Err(TypeError::new(
                        TypeErrorKind::EffectMismatch {
                            expected: format!("{:?}", row1),
                            found: format!("{:?}", row2),
                        },
                        span,
                    ))
                }
            }
        }
    }

    /// Unify a pure row with a potentially effectful row.
    fn unify_pure_with_row(&mut self, row: &EffectRow, span: Span) -> Result<(), TypeError> {
        if row.is_pure() {
            return Ok(());
        }

        // Pure can only unify with a row if the row is polymorphic
        // and its effects all come from the row variable
        if let Some(rv) = row.row_var() {
            if row.effects().next().is_none() {
                // Row is just a row variable - bind to pure
                self.bind_row_var(RowVarId(rv.0), EffectRow::pure());
                return Ok(());
            }
        }

        // Cannot unify pure with a row that has concrete effects
        Err(TypeError::new(
            TypeErrorKind::EffectMismatch {
                expected: "pure".to_string(),
                found: format!("{:?}", row),
            },
            span,
        ))
    }

    /// Unify type arguments for two effect references.
    fn unify_effect_args(
        &mut self,
        e1: &EffectRef,
        e2: &EffectRef,
        span: Span,
    ) -> Result<(), TypeError> {
        if e1.type_args.len() != e2.type_args.len() {
            return Err(TypeError::new(
                TypeErrorKind::EffectMismatch {
                    expected: format!("{:?}", e1),
                    found: format!("{:?}", e2),
                },
                span,
            ));
        }

        for (t1, t2) in e1.type_args.iter().zip(e2.type_args.iter()) {
            self.type_unifier.unify(t1, t2, span)?;
        }

        Ok(())
    }

    /// Bind a row variable to a row.
    fn bind_row_var(&mut self, var: RowVarId, row: EffectRow) {
        self.row_substitutions.insert(var, row);
    }

    /// Resolve a row by following substitutions.
    pub fn resolve_row(&self, row: &EffectRow) -> EffectRow {
        if let Some(rv) = row.row_var() {
            if let Some(substituted) = self.row_substitutions.get(&RowVarId(rv.0)) {
                // Merge concrete effects with substituted row
                let mut result = self.resolve_row(substituted);
                for effect in row.effects() {
                    result.add_effect(effect.clone());
                }
                return result;
            }
        }
        row.clone()
    }

    /// Check if an effect row is fully resolved (no row variables).
    pub fn is_row_resolved(&self, row: &EffectRow) -> bool {
        let row = self.resolve_row(row);
        row.row_var().is_none()
    }

    /// Get all row substitutions.
    pub fn row_substitutions(&self) -> &HashMap<RowVarId, EffectRow> {
        &self.row_substitutions
    }
}

impl Default for EffectUnifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_effect(id: u32) -> EffectRef {
        EffectRef::new(DefId::new(id))
    }

    fn make_effect_with_arg(id: u32, ty: Type) -> EffectRef {
        EffectRef::with_args(DefId::new(id), vec![ty])
    }

    #[test]
    fn test_unify_pure_rows() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        let pure1 = EffectRow::pure();
        let pure2 = EffectRow::pure();

        assert!(u.unify_rows(&pure1, &pure2, span).is_ok());
    }

    #[test]
    fn test_unify_same_single_effect() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        let row1 = EffectRow::single(make_effect(1));
        let row2 = EffectRow::single(make_effect(1));

        assert!(u.unify_rows(&row1, &row2, span).is_ok());
    }

    #[test]
    fn test_unify_different_effects_fails() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        // Closed rows with different effects cannot unify
        let row1 = EffectRow::single(make_effect(1));
        let row2 = EffectRow::single(make_effect(2));

        assert!(u.unify_rows(&row1, &row2, span).is_err());
    }

    #[test]
    fn test_unify_with_row_variable() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        // {E1 | ρ} unifies with {E1, E2}
        let rv = u.fresh_row_var();
        let mut row1 = EffectRow::single(make_effect(1));
        row1.set_row_var(rv);

        let mut row2 = EffectRow::single(make_effect(1));
        row2.add_effect(make_effect(2));

        assert!(u.unify_rows(&row1, &row2, span).is_ok());

        // ρ should be bound to {E2}
        let resolved = u.resolve_row(&row1);
        assert!(resolved.contains(&make_effect(1)));
        assert!(resolved.contains(&make_effect(2)));
    }

    #[test]
    fn test_unify_polymorphic_rows() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        // {E1 | ρ1} unifies with {E2 | ρ2}
        let rv1 = u.fresh_row_var();
        let rv2 = u.fresh_row_var();

        let mut row1 = EffectRow::single(make_effect(1));
        row1.set_row_var(rv1);

        let mut row2 = EffectRow::single(make_effect(2));
        row2.set_row_var(rv2);

        assert!(u.unify_rows(&row1, &row2, span).is_ok());

        // Both should resolve to contain both effects
        let resolved1 = u.resolve_row(&row1);
        let resolved2 = u.resolve_row(&row2);

        assert!(resolved1.contains(&make_effect(1)));
        assert!(resolved1.contains(&make_effect(2)));
        assert!(resolved2.contains(&make_effect(1)));
        assert!(resolved2.contains(&make_effect(2)));
    }

    #[test]
    fn test_unify_effect_type_args() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        // {State<i32>} unifies with {State<?T>}
        let row1 = EffectRow::single(make_effect_with_arg(1, Type::i32()));

        let ty_var = u.type_unifier_mut().fresh_var();
        let row2 = EffectRow::single(make_effect_with_arg(1, ty_var.clone()));

        assert!(u.unify_rows(&row1, &row2, span).is_ok());

        // ?T should be bound to i32
        let resolved = u.type_unifier().resolve(&ty_var);
        assert_eq!(resolved, Type::i32());
    }

    #[test]
    fn test_unify_pure_with_polymorphic() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        let pure = EffectRow::pure();
        let rv = u.fresh_row_var();
        let poly = EffectRow::polymorphic(rv);

        assert!(u.unify_rows(&pure, &poly, span).is_ok());

        // ρ should be bound to pure
        let resolved = u.resolve_row(&poly);
        assert!(resolved.is_pure());
    }

    #[test]
    fn test_unify_pure_with_concrete_fails() {
        let mut u = EffectUnifier::new();
        let span = Span::dummy();

        let pure = EffectRow::pure();
        let concrete = EffectRow::single(make_effect(1));

        assert!(u.unify_rows(&pure, &concrete, span).is_err());
    }
}
