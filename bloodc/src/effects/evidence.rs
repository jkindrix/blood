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
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

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

// ============================================================================
// Evidence Caching
// ============================================================================

/// A pattern of handlers that uniquely identifies an evidence vector configuration.
///
/// Two handler patterns are equal if they map the same effects to the same handlers
/// in the same order. This enables caching and reuse of evidence vectors.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HandlerPattern {
    /// Ordered pairs of (effect_id, handler_id) defining the pattern.
    bindings: Vec<(DefId, DefId)>,
}

impl HandlerPattern {
    /// Create a new handler pattern from an evidence vector.
    pub fn from_evidence(ev: &EvidenceVector) -> Self {
        let bindings: Vec<_> = ev.iter()
            .map(|entry| (entry.effect.def_id, entry.handler_id))
            .collect();
        Self { bindings }
    }

    /// Create a new empty handler pattern.
    pub fn empty() -> Self {
        Self { bindings: Vec::new() }
    }

    /// Add a handler binding to the pattern.
    pub fn add(&mut self, effect_id: DefId, handler_id: DefId) {
        self.bindings.push((effect_id, handler_id));
    }

    /// Get the number of bindings in the pattern.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Check if the pattern is empty.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Compute a hash of this pattern for cache lookup.
    pub fn hash_value(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

/// Cache for evidence vectors, enabling reuse of identical handler patterns.
///
/// This optimization detects when the same set of handlers is used multiple times
/// and reuses the cached evidence vector instead of creating a new one.
///
/// ## Benefits
///
/// - Reduced memory allocation in loops with repeated handler installation
/// - Faster evidence lookup when patterns are recognized
/// - Better cache locality from evidence vector reuse
///
/// ## Usage
///
/// ```ignore
/// let mut cache = EvidenceCache::new();
///
/// // First time: creates and caches evidence
/// let ev1 = cache.get_or_create(&pattern, || create_evidence());
///
/// // Second time: returns cached evidence
/// let ev2 = cache.get_or_create(&pattern, || unreachable!());
///
/// assert!(Arc::ptr_eq(&ev1, &ev2));  // Same instance
/// ```
#[derive(Debug, Clone, Default)]
pub struct EvidenceCache {
    /// Map from pattern hash to cached evidence vectors.
    cache: HashMap<u64, EvidenceVector>,
    /// Statistics for cache performance monitoring.
    stats: CacheStats,
}

/// Statistics about evidence cache usage.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of cache hits (reused evidence).
    pub hits: u64,
    /// Number of cache misses (new evidence created).
    pub misses: u64,
    /// Number of patterns currently cached.
    pub cached_patterns: usize,
}

impl CacheStats {
    /// Calculate the cache hit rate as a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }

    /// Reset all statistics.
    pub fn reset(&mut self) {
        self.hits = 0;
        self.misses = 0;
        // cached_patterns is updated separately
    }
}

impl EvidenceCache {
    /// Create a new empty evidence cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a cache with a specified initial capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: HashMap::with_capacity(capacity),
            stats: CacheStats::default(),
        }
    }

    /// Look up or create an evidence vector for a handler pattern.
    ///
    /// If an evidence vector for this pattern exists in the cache, returns it.
    /// Otherwise, calls the factory function to create a new one, caches it,
    /// and returns it.
    pub fn get_or_create<F>(&mut self, pattern: &HandlerPattern, factory: F) -> EvidenceVector
    where
        F: FnOnce() -> EvidenceVector,
    {
        let hash = pattern.hash_value();

        if let Some(cached) = self.cache.get(&hash) {
            self.stats.hits += 1;
            return cached.clone();
        }

        // Cache miss - create new evidence
        self.stats.misses += 1;
        let evidence = factory();
        self.cache.insert(hash, evidence.clone());
        self.stats.cached_patterns = self.cache.len();
        evidence
    }

    /// Check if a pattern is already cached.
    pub fn contains(&self, pattern: &HandlerPattern) -> bool {
        let hash = pattern.hash_value();
        self.cache.contains_key(&hash)
    }

    /// Get the cached evidence for a pattern, if it exists.
    pub fn get(&self, pattern: &HandlerPattern) -> Option<&EvidenceVector> {
        let hash = pattern.hash_value();
        self.cache.get(&hash)
    }

    /// Insert an evidence vector for a pattern.
    ///
    /// Returns the previous evidence vector if one was cached for this pattern.
    pub fn insert(&mut self, pattern: &HandlerPattern, evidence: EvidenceVector) -> Option<EvidenceVector> {
        let hash = pattern.hash_value();
        let prev = self.cache.insert(hash, evidence);
        self.stats.cached_patterns = self.cache.len();
        prev
    }

    /// Clear all cached evidence vectors.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.stats.cached_patterns = 0;
    }

    /// Get the number of cached patterns.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get mutable cache statistics (for resetting).
    pub fn stats_mut(&mut self) -> &mut CacheStats {
        &mut self.stats
    }
}

// ============================================================================
// Static Evidence Optimization (EFF-OPT-001)
// ============================================================================

/// A unique identifier for a static evidence pattern.
///
/// Static evidence patterns are identified at compile time and emitted as
/// global constants, avoiding runtime allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StaticEvidenceId(pub u32);

impl StaticEvidenceId {
    /// Create a new static evidence ID.
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the numeric ID value.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// Classification of handler state for static evidence optimization.
///
/// Determines whether a handler's state can be pre-allocated at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerStateKind {
    /// Handler has no state (unit type `()`).
    /// Optimal: no state allocation or initialization needed.
    Stateless,

    /// Handler state is a constant value known at compile time.
    /// State can be embedded in static data section.
    Constant,

    /// Handler state is zero-initialized.
    /// State can be BSS-allocated (zero-initialized by loader).
    ZeroInit,

    /// Handler state is computed at runtime.
    /// Cannot use static evidence; requires dynamic allocation.
    Dynamic,
}

impl HandlerStateKind {
    /// Check if this state kind allows static evidence optimization.
    pub fn is_static(self) -> bool {
        matches!(self, Self::Stateless | Self::Constant | Self::ZeroInit)
    }
}

/// A single handler entry in static evidence.
///
/// Contains compile-time known information about a handler installation.
#[derive(Debug, Clone)]
pub struct StaticEvidenceEntry {
    /// The effect this handler handles.
    pub effect_id: DefId,
    /// The handler definition ID.
    pub handler_id: DefId,
    /// The kind of handler state.
    pub state_kind: HandlerStateKind,
    /// For constant state, the serialized constant value (if small enough).
    /// This is the LLVM constant representation.
    pub constant_state: Option<Vec<u8>>,
}

impl StaticEvidenceEntry {
    /// Create a new static evidence entry for a stateless handler.
    pub fn stateless(effect_id: DefId, handler_id: DefId) -> Self {
        Self {
            effect_id,
            handler_id,
            state_kind: HandlerStateKind::Stateless,
            constant_state: None,
        }
    }

    /// Create a new static evidence entry for a handler with constant state.
    pub fn with_constant(effect_id: DefId, handler_id: DefId, state: Vec<u8>) -> Self {
        Self {
            effect_id,
            handler_id,
            state_kind: HandlerStateKind::Constant,
            constant_state: Some(state),
        }
    }

    /// Create a new static evidence entry for a zero-initialized handler.
    pub fn zero_init(effect_id: DefId, handler_id: DefId) -> Self {
        Self {
            effect_id,
            handler_id,
            state_kind: HandlerStateKind::ZeroInit,
            constant_state: None,
        }
    }

    /// Create a new static evidence entry for a dynamic handler.
    pub fn dynamic(effect_id: DefId, handler_id: DefId) -> Self {
        Self {
            effect_id,
            handler_id,
            state_kind: HandlerStateKind::Dynamic,
            constant_state: None,
        }
    }
}

/// A complete static evidence pattern.
///
/// Represents a compile-time known handler configuration that can be
/// emitted as a static global constant.
#[derive(Debug, Clone)]
pub struct StaticEvidence {
    /// Unique identifier for this static evidence.
    pub id: StaticEvidenceId,
    /// The handler entries in this evidence.
    pub entries: Vec<StaticEvidenceEntry>,
    /// Whether all entries are fully static (no runtime state needed).
    pub fully_static: bool,
}

impl StaticEvidence {
    /// Create new static evidence with the given ID.
    pub fn new(id: StaticEvidenceId) -> Self {
        Self {
            id,
            entries: Vec::new(),
            fully_static: true,
        }
    }

    /// Add a handler entry to this static evidence.
    pub fn add_entry(&mut self, entry: StaticEvidenceEntry) {
        if !entry.state_kind.is_static() {
            self.fully_static = false;
        }
        self.entries.push(entry);
    }

    /// Get the number of handlers in this evidence.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if this evidence is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Check if this evidence can be fully emitted as static data.
    pub fn can_emit_static(&self) -> bool {
        self.fully_static && !self.entries.is_empty()
    }

    /// Iterate over entries.
    pub fn iter(&self) -> impl Iterator<Item = &StaticEvidenceEntry> {
        self.entries.iter()
    }
}

/// Registry of static evidence patterns discovered during MIR analysis.
///
/// Collects all handler patterns that can be optimized with static evidence.
/// Used during codegen to emit static global constants.
#[derive(Debug, Clone, Default)]
pub struct StaticEvidenceRegistry {
    /// All registered static evidence patterns.
    patterns: Vec<StaticEvidence>,
    /// Lookup from handler pattern hash to static evidence ID.
    pattern_lookup: HashMap<u64, StaticEvidenceId>,
    /// Next available ID.
    next_id: u32,
    /// Statistics about static evidence optimization.
    stats: StaticEvidenceStats,
}

/// Statistics about static evidence optimization.
#[derive(Debug, Clone, Default)]
pub struct StaticEvidenceStats {
    /// Total number of PushHandler statements analyzed.
    pub handlers_analyzed: u64,
    /// Number of handlers eligible for static evidence.
    pub static_eligible: u64,
    /// Number of unique static patterns registered.
    pub unique_patterns: u64,
    /// Number of pattern reuses (same pattern at multiple sites).
    pub pattern_reuses: u64,
}

impl StaticEvidenceStats {
    /// Calculate the percentage of handlers eligible for static evidence.
    pub fn eligibility_rate(&self) -> f64 {
        if self.handlers_analyzed == 0 {
            0.0
        } else {
            (self.static_eligible as f64 / self.handlers_analyzed as f64) * 100.0
        }
    }
}

impl StaticEvidenceRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a static evidence pattern, returning its ID.
    ///
    /// If an identical pattern already exists, returns the existing ID.
    pub fn register(&mut self, pattern: &HandlerPattern, evidence: StaticEvidence) -> StaticEvidenceId {
        let hash = pattern.hash_value();

        if let Some(&id) = self.pattern_lookup.get(&hash) {
            self.stats.pattern_reuses += 1;
            return id;
        }

        let id = StaticEvidenceId::new(self.next_id);
        self.next_id += 1;

        self.pattern_lookup.insert(hash, id);
        self.patterns.push(evidence);
        self.stats.unique_patterns += 1;

        id
    }

    /// Look up static evidence by ID.
    pub fn get(&self, id: StaticEvidenceId) -> Option<&StaticEvidence> {
        self.patterns.get(id.index() as usize)
    }

    /// Check if a pattern is already registered.
    pub fn contains(&self, pattern: &HandlerPattern) -> bool {
        self.pattern_lookup.contains_key(&pattern.hash_value())
    }

    /// Get the ID for a registered pattern.
    pub fn lookup(&self, pattern: &HandlerPattern) -> Option<StaticEvidenceId> {
        self.pattern_lookup.get(&pattern.hash_value()).copied()
    }

    /// Get the number of registered patterns.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Iterate over all registered static evidence patterns.
    pub fn iter(&self) -> impl Iterator<Item = &StaticEvidence> {
        self.patterns.iter()
    }

    /// Record that a handler was analyzed.
    pub fn record_analyzed(&mut self) {
        self.stats.handlers_analyzed += 1;
    }

    /// Record that a handler is eligible for static evidence.
    pub fn record_static_eligible(&mut self) {
        self.stats.static_eligible += 1;
    }

    /// Get statistics.
    pub fn stats(&self) -> &StaticEvidenceStats {
        &self.stats
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

    // ========================================================================
    // Evidence Caching Tests
    // ========================================================================

    #[test]
    fn test_handler_pattern_empty() {
        let pattern = HandlerPattern::empty();
        assert!(pattern.is_empty());
        assert_eq!(pattern.len(), 0);
    }

    #[test]
    fn test_handler_pattern_add() {
        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));
        pattern.add(DefId::new(2), DefId::new(20));

        assert!(!pattern.is_empty());
        assert_eq!(pattern.len(), 2);
    }

    #[test]
    fn test_handler_pattern_from_evidence() {
        let mut ev = EvidenceVector::new();
        ev.add(EffectRef::new(DefId::new(1)), DefId::new(10));
        ev.add(EffectRef::new(DefId::new(2)), DefId::new(20));

        let pattern = HandlerPattern::from_evidence(&ev);
        assert_eq!(pattern.len(), 2);
    }

    #[test]
    fn test_handler_pattern_equality() {
        let mut pattern1 = HandlerPattern::empty();
        pattern1.add(DefId::new(1), DefId::new(10));
        pattern1.add(DefId::new(2), DefId::new(20));

        let mut pattern2 = HandlerPattern::empty();
        pattern2.add(DefId::new(1), DefId::new(10));
        pattern2.add(DefId::new(2), DefId::new(20));

        // Same bindings in same order should be equal
        assert_eq!(pattern1, pattern2);
        assert_eq!(pattern1.hash_value(), pattern2.hash_value());
    }

    #[test]
    fn test_handler_pattern_inequality() {
        let mut pattern1 = HandlerPattern::empty();
        pattern1.add(DefId::new(1), DefId::new(10));

        let mut pattern2 = HandlerPattern::empty();
        pattern2.add(DefId::new(1), DefId::new(20)); // Different handler

        assert_ne!(pattern1, pattern2);
        // Different patterns should have different hashes (usually)
        assert_ne!(pattern1.hash_value(), pattern2.hash_value());
    }

    #[test]
    fn test_cache_stats_default() {
        let stats = CacheStats::default();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.cached_patterns, 0);
        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_cache_stats_hit_rate() {
        let mut stats = CacheStats::default();
        stats.hits = 75;
        stats.misses = 25;
        assert!((stats.hit_rate() - 75.0).abs() < 0.001);
    }

    #[test]
    fn test_evidence_cache_new() {
        let cache = EvidenceCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_evidence_cache_get_or_create_miss() {
        let mut cache = EvidenceCache::new();
        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));

        let ev = cache.get_or_create(&pattern, || {
            let mut ev = EvidenceVector::new();
            ev.add(EffectRef::new(DefId::new(1)), DefId::new(10));
            ev
        });

        assert_eq!(ev.len(), 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 0);
    }

    #[test]
    fn test_evidence_cache_get_or_create_hit() {
        let mut cache = EvidenceCache::new();
        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));

        // First call - cache miss
        let _ev1 = cache.get_or_create(&pattern, || {
            let mut ev = EvidenceVector::new();
            ev.add(EffectRef::new(DefId::new(1)), DefId::new(10));
            ev
        });

        // Second call - cache hit
        let ev2 = cache.get_or_create(&pattern, || {
            panic!("factory should not be called on cache hit");
        });

        assert_eq!(ev2.len(), 1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().hit_rate(), 50.0);
    }

    #[test]
    fn test_evidence_cache_contains() {
        let mut cache = EvidenceCache::new();
        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));

        assert!(!cache.contains(&pattern));

        let _ = cache.get_or_create(&pattern, || EvidenceVector::new());

        assert!(cache.contains(&pattern));
    }

    #[test]
    fn test_evidence_cache_clear() {
        let mut cache = EvidenceCache::new();
        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));

        let _ = cache.get_or_create(&pattern, || EvidenceVector::new());
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.stats().cached_patterns, 0);
    }

    #[test]
    fn test_evidence_cache_multiple_patterns() {
        let mut cache = EvidenceCache::new();

        let mut pattern1 = HandlerPattern::empty();
        pattern1.add(DefId::new(1), DefId::new(10));

        let mut pattern2 = HandlerPattern::empty();
        pattern2.add(DefId::new(2), DefId::new(20));

        let _ = cache.get_or_create(&pattern1, || {
            let mut ev = EvidenceVector::new();
            ev.add(EffectRef::new(DefId::new(1)), DefId::new(10));
            ev
        });

        let _ = cache.get_or_create(&pattern2, || {
            let mut ev = EvidenceVector::new();
            ev.add(EffectRef::new(DefId::new(2)), DefId::new(20));
            ev
        });

        assert_eq!(cache.len(), 2);
        assert!(cache.contains(&pattern1));
        assert!(cache.contains(&pattern2));
    }

    // ========================================================================
    // Static Evidence Optimization Tests (EFF-OPT-001)
    // ========================================================================

    #[test]
    fn test_static_evidence_id() {
        let id = StaticEvidenceId::new(42);
        assert_eq!(id.index(), 42);
        assert_eq!(id, StaticEvidenceId(42));
    }

    #[test]
    fn test_handler_state_kind_is_static() {
        assert!(HandlerStateKind::Stateless.is_static());
        assert!(HandlerStateKind::Constant.is_static());
        assert!(HandlerStateKind::ZeroInit.is_static());
        assert!(!HandlerStateKind::Dynamic.is_static());
    }

    #[test]
    fn test_static_evidence_entry_stateless() {
        let entry = StaticEvidenceEntry::stateless(DefId::new(1), DefId::new(10));
        assert_eq!(entry.effect_id, DefId::new(1));
        assert_eq!(entry.handler_id, DefId::new(10));
        assert_eq!(entry.state_kind, HandlerStateKind::Stateless);
        assert!(entry.constant_state.is_none());
    }

    #[test]
    fn test_static_evidence_entry_with_constant() {
        let state = vec![1, 2, 3, 4];
        let entry = StaticEvidenceEntry::with_constant(DefId::new(1), DefId::new(10), state.clone());
        assert_eq!(entry.state_kind, HandlerStateKind::Constant);
        assert_eq!(entry.constant_state, Some(state));
    }

    #[test]
    fn test_static_evidence_entry_zero_init() {
        let entry = StaticEvidenceEntry::zero_init(DefId::new(1), DefId::new(10));
        assert_eq!(entry.state_kind, HandlerStateKind::ZeroInit);
        assert!(entry.constant_state.is_none());
    }

    #[test]
    fn test_static_evidence_entry_dynamic() {
        let entry = StaticEvidenceEntry::dynamic(DefId::new(1), DefId::new(10));
        assert_eq!(entry.state_kind, HandlerStateKind::Dynamic);
        assert!(!entry.state_kind.is_static());
    }

    #[test]
    fn test_static_evidence_new() {
        let ev = StaticEvidence::new(StaticEvidenceId::new(0));
        assert!(ev.is_empty());
        assert!(ev.fully_static);
        assert!(!ev.can_emit_static()); // Empty evidence cannot be emitted
    }

    #[test]
    fn test_static_evidence_add_entry() {
        let mut ev = StaticEvidence::new(StaticEvidenceId::new(0));

        ev.add_entry(StaticEvidenceEntry::stateless(DefId::new(1), DefId::new(10)));
        assert_eq!(ev.len(), 1);
        assert!(ev.fully_static);
        assert!(ev.can_emit_static());

        ev.add_entry(StaticEvidenceEntry::zero_init(DefId::new(2), DefId::new(20)));
        assert_eq!(ev.len(), 2);
        assert!(ev.fully_static);
        assert!(ev.can_emit_static());
    }

    #[test]
    fn test_static_evidence_becomes_dynamic() {
        let mut ev = StaticEvidence::new(StaticEvidenceId::new(0));

        ev.add_entry(StaticEvidenceEntry::stateless(DefId::new(1), DefId::new(10)));
        assert!(ev.fully_static);

        // Adding a dynamic entry makes the whole evidence non-static
        ev.add_entry(StaticEvidenceEntry::dynamic(DefId::new(2), DefId::new(20)));
        assert!(!ev.fully_static);
        assert!(!ev.can_emit_static());
    }

    #[test]
    fn test_static_evidence_registry_new() {
        let registry = StaticEvidenceRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_static_evidence_registry_register() {
        let mut registry = StaticEvidenceRegistry::new();

        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));

        let mut evidence = StaticEvidence::new(StaticEvidenceId::new(0));
        evidence.add_entry(StaticEvidenceEntry::stateless(DefId::new(1), DefId::new(10)));

        let id = registry.register(&pattern, evidence);
        assert_eq!(id.index(), 0);
        assert_eq!(registry.len(), 1);
        assert!(registry.contains(&pattern));
    }

    #[test]
    fn test_static_evidence_registry_dedup() {
        let mut registry = StaticEvidenceRegistry::new();

        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));

        let evidence1 = StaticEvidence::new(StaticEvidenceId::new(0));
        let evidence2 = StaticEvidence::new(StaticEvidenceId::new(0));

        let id1 = registry.register(&pattern, evidence1);
        let id2 = registry.register(&pattern, evidence2);

        // Same pattern should return same ID
        assert_eq!(id1, id2);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.stats().unique_patterns, 1);
        assert_eq!(registry.stats().pattern_reuses, 1);
    }

    #[test]
    fn test_static_evidence_registry_lookup() {
        let mut registry = StaticEvidenceRegistry::new();

        let mut pattern = HandlerPattern::empty();
        pattern.add(DefId::new(1), DefId::new(10));

        let mut evidence = StaticEvidence::new(StaticEvidenceId::new(0));
        evidence.add_entry(StaticEvidenceEntry::stateless(DefId::new(1), DefId::new(10)));

        let id = registry.register(&pattern, evidence);

        // Can look up by pattern
        assert_eq!(registry.lookup(&pattern), Some(id));

        // Can get evidence by ID
        let retrieved = registry.get(id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 1);
    }

    #[test]
    fn test_static_evidence_stats_eligibility_rate() {
        let mut stats = StaticEvidenceStats::default();
        assert_eq!(stats.eligibility_rate(), 0.0);

        stats.handlers_analyzed = 100;
        stats.static_eligible = 75;
        assert!((stats.eligibility_rate() - 75.0).abs() < 0.001);
    }

    #[test]
    fn test_static_evidence_registry_stats() {
        let mut registry = StaticEvidenceRegistry::new();

        registry.record_analyzed();
        registry.record_analyzed();
        registry.record_static_eligible();

        assert_eq!(registry.stats().handlers_analyzed, 2);
        assert_eq!(registry.stats().static_eligible, 1);
        assert!((registry.stats().eligibility_rate() - 50.0).abs() < 0.001);
    }
}
