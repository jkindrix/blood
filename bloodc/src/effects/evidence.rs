//! # Evidence Passing
//!
//! Implements evidence vectors for effect handler compilation.
//!
//! ## Overview
//!
//! Evidence passing is the compilation strategy for algebraic effects, based on:
//! - [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
//!
//! Instead of searching for handlers at runtime, evidence vectors are passed
//! explicitly to effectful functions, providing O(1) handler lookup.
//!
//! ## Example
//!
//! ```text
//! // Source code
//! fn foo() / {State<i32>, Error} {
//!     let x = get()
//!     if x < 0 { throw("negative") }
//!     put(x + 1)
//! }
//!
//! // After evidence translation
//! fn foo(ev: Evidence) {
//!     let x = ev.state.get()
//!     if x < 0 { ev.error.throw("negative") }
//!     ev.state.put(x + 1)
//! }
//! ```

use super::row::EffectRef;
use crate::hir::DefId;
use std::collections::HashMap;

/// An evidence entry for a single effect.
///
/// Contains the handler implementation for one effect in the evidence vector.
#[derive(Debug, Clone)]
pub struct EvidenceEntry {
    /// The effect this evidence handles.
    pub effect: EffectRef,
    /// The handler definition ID.
    pub handler_id: DefId,
    /// Index into the evidence vector.
    pub index: usize,
}

impl EvidenceEntry {
    /// Create a new evidence entry.
    pub fn new(effect: EffectRef, handler_id: DefId, index: usize) -> Self {
        Self {
            effect,
            handler_id,
            index,
        }
    }
}

/// An evidence vector mapping effects to their handlers.
///
/// The evidence vector is passed to effectful functions at runtime,
/// enabling O(1) lookup of handler implementations.
#[derive(Debug, Clone, Default)]
pub struct EvidenceVector {
    /// Mapping from effect DefId to evidence entry.
    entries: HashMap<DefId, EvidenceEntry>,
    /// Ordered list of entries for vector representation.
    ordered: Vec<DefId>,
}

impl EvidenceVector {
    /// Create an empty evidence vector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an evidence entry for an effect.
    pub fn add(&mut self, effect: EffectRef, handler_id: DefId) {
        let index = self.ordered.len();
        let entry = EvidenceEntry::new(effect.clone(), handler_id, index);
        self.entries.insert(effect.def_id, entry);
        self.ordered.push(effect.def_id);
    }

    /// Look up evidence for an effect.
    pub fn lookup(&self, effect_id: DefId) -> Option<&EvidenceEntry> {
        self.entries.get(&effect_id)
    }

    /// Get the number of entries in the vector.
    pub fn len(&self) -> usize {
        self.ordered.len()
    }

    /// Check if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

    /// Iterate over evidence entries in order.
    pub fn iter(&self) -> impl Iterator<Item = &EvidenceEntry> {
        self.ordered.iter().filter_map(|id| self.entries.get(id))
    }
}

/// Evidence structure passed to effectful functions at runtime.
///
/// This is the runtime representation of handler implementations.
#[derive(Debug, Clone)]
pub struct Evidence {
    /// The evidence vector.
    pub vector: EvidenceVector,
    /// The current handler depth (for nested handlers).
    pub depth: usize,
}

impl Evidence {
    /// Create new evidence from a vector.
    pub fn new(vector: EvidenceVector) -> Self {
        Self { vector, depth: 0 }
    }

    /// Create evidence with a specific depth.
    pub fn with_depth(vector: EvidenceVector, depth: usize) -> Self {
        Self { vector, depth }
    }

    /// Push a new handler scope, incrementing depth.
    pub fn push_scope(&self, additional: EvidenceVector) -> Self {
        let mut combined = self.vector.clone();
        for entry in additional.iter() {
            combined.add(entry.effect.clone(), entry.handler_id);
        }
        Self {
            vector: combined,
            depth: self.depth + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evidence_vector_new() {
        let ev = EvidenceVector::new();
        assert!(ev.is_empty());
    }

    #[test]
    fn test_evidence_vector_add() {
        let mut ev = EvidenceVector::new();
        let effect = EffectRef::new(DefId::new(1));
        let handler = DefId::new(2);

        ev.add(effect.clone(), handler);

        assert_eq!(ev.len(), 1);
        assert!(ev.lookup(effect.def_id).is_some());
    }

    #[test]
    fn test_evidence_push_scope() {
        let mut ev1 = EvidenceVector::new();
        ev1.add(EffectRef::new(DefId::new(1)), DefId::new(2));

        let evidence = Evidence::new(ev1);

        let mut ev2 = EvidenceVector::new();
        ev2.add(EffectRef::new(DefId::new(3)), DefId::new(4));

        let scoped = evidence.push_scope(ev2);

        assert_eq!(scoped.depth, 1);
        assert_eq!(scoped.vector.len(), 2);
    }
}
