//! # Effect Row Types
//!
//! Implements row polymorphism for effect types as specified in SPECIFICATION.md §3.3.
//!
//! ## Row Types
//!
//! Effect rows are ordered sets of effects with an optional row variable for polymorphism:
//!
//! ```text
//! EffectRow ::= {} | {E, ...Es} | {E, ...Es | ρ}
//! ```
//!
//! Where `ρ` is a row variable enabling effect polymorphism.
//!
//! ## Unification
//!
//! Row unification follows the algorithm in DISPATCH.md §4.4.3:
//!
//! 1. Find common effects
//! 2. Unify effect arguments
//! 3. Handle row variables for remaining effects

use crate::hir::DefId;
use std::collections::BTreeSet;

/// A row variable for effect polymorphism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowVar(pub u32);

impl RowVar {
    /// Create a new row variable.
    pub fn new(id: u32) -> Self {
        Self(id)
    }
}

/// An effect reference with optional type arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectRef {
    /// The effect definition ID.
    pub def_id: DefId,
    /// Type arguments for parameterized effects (e.g., State<i32>).
    pub type_args: Vec<crate::hir::Type>,
}

impl EffectRef {
    /// Create a new effect reference.
    pub fn new(def_id: DefId) -> Self {
        Self {
            def_id,
            type_args: Vec::new(),
        }
    }

    /// Create an effect reference with type arguments.
    pub fn with_args(def_id: DefId, type_args: Vec<crate::hir::Type>) -> Self {
        Self { def_id, type_args }
    }
}

/// An effect row representing a set of effects.
///
/// Effect rows support row polymorphism through an optional row variable,
/// enabling functions to be polymorphic over unknown additional effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRow {
    /// The concrete effects in this row.
    effects: BTreeSet<EffectRef>,
    /// Optional row variable for polymorphism.
    row_var: Option<RowVar>,
}

impl EffectRow {
    /// Create an empty effect row (pure).
    pub fn pure() -> Self {
        Self {
            effects: BTreeSet::new(),
            row_var: None,
        }
    }

    /// Create an effect row with a single effect.
    pub fn single(effect: EffectRef) -> Self {
        let mut effects = BTreeSet::new();
        effects.insert(effect);
        Self {
            effects,
            row_var: None,
        }
    }

    /// Create a polymorphic effect row with just a row variable.
    pub fn polymorphic(row_var: RowVar) -> Self {
        Self {
            effects: BTreeSet::new(),
            row_var: Some(row_var),
        }
    }

    /// Add an effect to this row.
    pub fn add_effect(&mut self, effect: EffectRef) {
        self.effects.insert(effect);
    }

    /// Set the row variable for polymorphism.
    pub fn set_row_var(&mut self, row_var: RowVar) {
        self.row_var = Some(row_var);
    }

    /// Check if this row is pure (no effects).
    pub fn is_pure(&self) -> bool {
        self.effects.is_empty() && self.row_var.is_none()
    }

    /// Check if this row is polymorphic (has a row variable).
    pub fn is_polymorphic(&self) -> bool {
        self.row_var.is_some()
    }

    /// Get the concrete effects in this row.
    pub fn effects(&self) -> impl Iterator<Item = &EffectRef> {
        self.effects.iter()
    }

    /// Get the row variable if present.
    pub fn row_var(&self) -> Option<RowVar> {
        self.row_var
    }

    /// Check if this row contains a specific effect.
    pub fn contains(&self, effect: &EffectRef) -> bool {
        self.effects.contains(effect)
    }

    /// Extend this row with effects from another row.
    pub fn extend(&mut self, other: &EffectRow) {
        self.effects.extend(other.effects.iter().cloned());
        if other.row_var.is_some() && self.row_var.is_none() {
            self.row_var = other.row_var;
        }
    }
}

impl Default for EffectRow {
    fn default() -> Self {
        Self::pure()
    }
}

// Implement Ord for EffectRef to allow use in BTreeSet
impl PartialOrd for EffectRef {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EffectRef {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.def_id.index.cmp(&other.def_id.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pure_row() {
        let row = EffectRow::pure();
        assert!(row.is_pure());
        assert!(!row.is_polymorphic());
    }

    #[test]
    fn test_single_effect() {
        let effect = EffectRef::new(DefId::new(1));
        let row = EffectRow::single(effect.clone());
        assert!(!row.is_pure());
        assert!(row.contains(&effect));
    }

    #[test]
    fn test_polymorphic_row() {
        let row = EffectRow::polymorphic(RowVar::new(0));
        assert!(row.is_polymorphic());
        assert!(row.effects().next().is_none());
    }

    #[test]
    fn test_extend_row() {
        let effect1 = EffectRef::new(DefId::new(1));
        let effect2 = EffectRef::new(DefId::new(2));

        let mut row1 = EffectRow::single(effect1.clone());
        let row2 = EffectRow::single(effect2.clone());

        row1.extend(&row2);

        assert!(row1.contains(&effect1));
        assert!(row1.contains(&effect2));
    }
}
