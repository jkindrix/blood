//! Effect row operations for effect-aware dispatch.

use std::cmp::Ordering;

/// An effect row for effect-aware dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRow {
    /// The effects in this row.
    pub effects: Vec<String>,
    /// Whether this is an open row (has a row variable).
    pub is_open: bool,
}

impl EffectRow {
    /// Create a pure (empty, closed) effect row.
    pub fn pure() -> Self {
        Self {
            effects: Vec::new(),
            is_open: false,
        }
    }

    /// Create an effect row with the given effects.
    pub fn with_effects(effects: Vec<String>) -> Self {
        Self {
            effects,
            is_open: false,
        }
    }

    /// Create an open effect row with a row variable.
    pub fn open(effects: Vec<String>) -> Self {
        Self {
            effects,
            is_open: true,
        }
    }

    /// Check if this effect row is pure (no effects and closed).
    pub fn is_pure(&self) -> bool {
        self.effects.is_empty() && !self.is_open
    }

    /// Count the number of concrete effects.
    pub fn effect_count(&self) -> usize {
        self.effects.len()
    }

    /// Check if this row is a subset of another row.
    ///
    /// For effect dispatch compatibility:
    /// - A method with effects {A, B} can be called in a context that handles {A, B, C}
    /// - An open row {A | rho} is compatible with any superset of {A}
    /// - A closed row {A, B} requires exactly those effects to be handled
    pub fn is_subset_of(&self, other: &EffectRow) -> bool {
        // Check that all effects in self are present in other
        for effect in &self.effects {
            if !other.effects.contains(effect) {
                return false;
            }
        }

        // If self is open, other must also be open (to accommodate unknown effects)
        // OR self's concrete effects must all be in other
        if self.is_open && !other.is_open {
            // Open row can only be subset of another open row
            // because the row variable could expand to anything
            return false;
        }

        true
    }

    /// Compare effect specificity between two rows.
    ///
    /// Returns:
    /// - `Ordering::Less` if self is more specific (more restrictive)
    /// - `Ordering::Greater` if other is more specific
    /// - `Ordering::Equal` if they are equally specific
    ///
    /// Specificity rules:
    /// 1. Pure (no effects) is most specific
    /// 2. Closed rows are more specific than open rows
    /// 3. Fewer effects = more specific
    /// 4. For same effect count, compare lexicographically for determinism
    pub fn compare_specificity(&self, other: &EffectRow) -> Ordering {
        // Rule 1: Pure is most specific
        let self_pure = self.is_pure();
        let other_pure = other.is_pure();
        match (self_pure, other_pure) {
            (true, false) => return Ordering::Less,    // self is more specific
            (false, true) => return Ordering::Greater, // other is more specific
            (true, true) => return Ordering::Equal,    // both pure
            (false, false) => {}                        // continue comparison
        }

        // Rule 2: Closed rows are more specific than open rows
        match (self.is_open, other.is_open) {
            (false, true) => return Ordering::Less,    // closed is more specific
            (true, false) => return Ordering::Greater, // open is less specific
            _ => {}                                     // continue comparison
        }

        // Rule 3: Fewer effects = more specific
        match self.effects.len().cmp(&other.effects.len()) {
            Ordering::Less => return Ordering::Less,    // fewer effects = more specific
            Ordering::Greater => return Ordering::Greater,
            Ordering::Equal => {}
        }

        // Rule 4: Same count, compare lexicographically for determinism
        let mut self_sorted: Vec<_> = self.effects.iter().collect();
        let mut other_sorted: Vec<_> = other.effects.iter().collect();
        self_sorted.sort();
        other_sorted.sort();

        for (s, o) in self_sorted.iter().zip(other_sorted.iter()) {
            match s.cmp(o) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }

        Ordering::Equal
    }
}
