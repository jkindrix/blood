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
//! ## Translation Process
//!
//! 1. **Function translation**: Add evidence parameter to effectful functions
//! 2. **Operation translation**: Replace `perform op(args)` with `ev[idx].op(args)`
//! 3. **Handler translation**: Create evidence vector from handler block
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
//!
//! ## Tail-Resumptive Optimization
//!
//! When a handler operation immediately resumes (tail-resumptive), the
//! continuation doesn't need to be captured. This is common for State,
//! Reader, and Writer effects.
//!
//! ```text
//! // Tail-resumptive (no capture needed):
//! get => resume(state)
//!
//! // Non-tail-resumptive (needs capture):
//! fork => { resume(true); resume(false) }
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

    /// Get the evidence entry for an effect.
    pub fn get(&self, effect_id: DefId) -> Option<&EvidenceEntry> {
        self.vector.lookup(effect_id)
    }

    /// Get the evidence index for an effect.
    pub fn index_of(&self, effect_id: DefId) -> Option<usize> {
        self.vector.lookup(effect_id).map(|e| e.index)
    }
}

// ============================================================================
// Evidence Translation
// ============================================================================

/// Represents an evidence-translated operation call.
///
/// After translation, `perform Effect.op(args)` becomes `ev[idx].op(args)`.
#[derive(Debug, Clone)]
pub struct TranslatedOp {
    /// The evidence index for the effect.
    pub evidence_index: usize,
    /// The operation index within the effect.
    pub operation_index: usize,
    /// The handler DefId.
    pub handler_id: DefId,
}

impl TranslatedOp {
    /// Create a new translated operation.
    pub fn new(evidence_index: usize, operation_index: usize, handler_id: DefId) -> Self {
        Self {
            evidence_index,
            operation_index,
            handler_id,
        }
    }
}

/// Evidence translation context for a function.
///
/// Tracks evidence requirements and provides lookup during translation.
#[derive(Debug, Clone)]
pub struct EvidenceContext {
    /// The evidence parameter name (e.g., "ev").
    pub param_name: String,
    /// Mapping from effect DefId to evidence index.
    effect_indices: HashMap<DefId, usize>,
    /// Whether this context has any evidence requirements.
    has_evidence: bool,
}

impl EvidenceContext {
    /// Create a new empty evidence context.
    pub fn new() -> Self {
        Self {
            param_name: "ev".to_string(),
            effect_indices: HashMap::new(),
            has_evidence: false,
        }
    }

    /// Create an evidence context from an evidence vector.
    pub fn from_evidence(ev: &EvidenceVector) -> Self {
        let mut effect_indices = HashMap::new();
        for entry in ev.iter() {
            effect_indices.insert(entry.effect.def_id, entry.index);
        }
        Self {
            param_name: "ev".to_string(),
            effect_indices,
            has_evidence: !ev.is_empty(),
        }
    }

    /// Register an effect in the context.
    pub fn register_effect(&mut self, effect_id: DefId, index: usize) {
        self.effect_indices.insert(effect_id, index);
        self.has_evidence = true;
    }

    /// Look up the evidence index for an effect.
    pub fn lookup(&self, effect_id: DefId) -> Option<usize> {
        self.effect_indices.get(&effect_id).copied()
    }

    /// Check if this context has any evidence requirements.
    pub fn has_evidence(&self) -> bool {
        self.has_evidence
    }

    /// Get the number of effects in the context.
    pub fn effect_count(&self) -> usize {
        self.effect_indices.len()
    }
}

impl Default for EvidenceContext {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn test_evidence_index_of() {
        let mut ev = EvidenceVector::new();
        let effect1 = EffectRef::new(DefId::new(1));
        let effect2 = EffectRef::new(DefId::new(2));

        ev.add(effect1.clone(), DefId::new(10));
        ev.add(effect2.clone(), DefId::new(20));

        let evidence = Evidence::new(ev);

        assert_eq!(evidence.index_of(DefId::new(1)), Some(0));
        assert_eq!(evidence.index_of(DefId::new(2)), Some(1));
        assert_eq!(evidence.index_of(DefId::new(99)), None);
    }

    #[test]
    fn test_evidence_context_new() {
        let ctx = EvidenceContext::new();
        assert!(!ctx.has_evidence());
        assert_eq!(ctx.effect_count(), 0);
    }

    #[test]
    fn test_evidence_context_register() {
        let mut ctx = EvidenceContext::new();

        ctx.register_effect(DefId::new(1), 0);
        ctx.register_effect(DefId::new(2), 1);

        assert!(ctx.has_evidence());
        assert_eq!(ctx.effect_count(), 2);
        assert_eq!(ctx.lookup(DefId::new(1)), Some(0));
        assert_eq!(ctx.lookup(DefId::new(2)), Some(1));
    }

    #[test]
    fn test_evidence_context_from_vector() {
        let mut ev = EvidenceVector::new();
        ev.add(EffectRef::new(DefId::new(1)), DefId::new(10));
        ev.add(EffectRef::new(DefId::new(2)), DefId::new(20));

        let ctx = EvidenceContext::from_evidence(&ev);

        assert!(ctx.has_evidence());
        assert_eq!(ctx.effect_count(), 2);
        assert_eq!(ctx.lookup(DefId::new(1)), Some(0));
        assert_eq!(ctx.lookup(DefId::new(2)), Some(1));
    }

    #[test]
    fn test_translated_op() {
        let op = TranslatedOp::new(0, 1, DefId::new(42));

        assert_eq!(op.evidence_index, 0);
        assert_eq!(op.operation_index, 1);
        assert_eq!(op.handler_id, DefId::new(42));
    }
}
