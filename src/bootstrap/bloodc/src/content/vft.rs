//! # Virtual Function Table (VFT)
//!
//! Maps content hashes to runtime entry points.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    VIRTUAL FUNCTION TABLE                    │
//! ├─────────────────────────────────────────────────────────────┤
//! │ Hash                    │ Entry Point    │ Metadata          │
//! ├─────────────────────────┼────────────────┼───────────────────┤
//! │ #a7f2k9m3xp...          │ 0x00401000     │ { arity: 2 }      │
//! │ #b3c1xp5jht...          │ 0x00401050     │ { arity: 2 }      │
//! │ #c4d2yr6kiu...          │ 0x004010A0     │ { arity: 2 }      │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use super::hash::ContentHash;

/// Calling convention for VFT entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CallingConvention {
    /// Standard Blood calling convention.
    #[default]
    Blood,
    /// Tail-call optimized.
    Tail,
    /// Effect-aware (captures continuation).
    Effect,
    /// FFI callback.
    Foreign,
}

/// Bit flags for effect categories.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectMask(u32);

impl EffectMask {
    pub const NONE: Self = Self(0);
    pub const IO: Self = Self(1 << 0);
    pub const STATE: Self = Self(1 << 1);
    pub const EXCEPTION: Self = Self(1 << 2);
    pub const FIBER: Self = Self(1 << 3);

    pub fn contains(&self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn is_pure(&self) -> bool {
        self.0 == 0
    }
}

/// An entry in the VFT.
#[derive(Debug, Clone)]
pub struct VFTEntry {
    /// The content hash this entry is for.
    pub hash: ContentHash,
    /// Pointer to native code (as usize for cross-platform).
    pub entry_point: usize,
    /// Number of arguments.
    pub arity: u8,
    /// Calling convention.
    pub calling_convention: CallingConvention,
    /// Effect categories used by this function.
    pub effects: EffectMask,
    /// Size of compiled code in bytes.
    pub compiled_size: u32,
    /// Generation number for hot-swap.
    pub generation: u64,
}

impl VFTEntry {
    /// Create a new VFT entry.
    pub fn new(hash: ContentHash, entry_point: usize, arity: u8) -> Self {
        Self {
            hash,
            entry_point,
            arity,
            calling_convention: CallingConvention::Blood,
            effects: EffectMask::NONE,
            compiled_size: 0,
            generation: 0,
        }
    }

    /// Set the calling convention.
    pub fn with_convention(mut self, convention: CallingConvention) -> Self {
        self.calling_convention = convention;
        self
    }

    /// Set the effect mask.
    pub fn with_effects(mut self, effects: EffectMask) -> Self {
        self.effects = effects;
        self
    }

    /// Set the compiled size.
    pub fn with_size(mut self, size: u32) -> Self {
        self.compiled_size = size;
        self
    }

    /// Set the generation number.
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    /// Check if this function is pure (no effects).
    pub fn is_pure(&self) -> bool {
        self.effects.is_pure()
    }

    /// Check if this function uses tail-call optimization.
    pub fn is_tail_call(&self) -> bool {
        self.calling_convention == CallingConvention::Tail
    }
}

/// Swap mode for hot-swapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapMode {
    /// Immediate: new calls use new code, in-flight continue with old.
    Immediate,
    /// Barrier: wait for in-flight calls to complete, then swap.
    Barrier,
    /// Epoch: swap at next epoch boundary.
    Epoch,
}

/// Result of a hot-swap operation.
#[derive(Debug, Clone)]
pub enum HotSwapResult {
    /// Swap completed successfully.
    Success {
        /// The old hash that was replaced.
        old_hash: ContentHash,
        /// The new hash now active.
        new_hash: ContentHash,
        /// New generation number.
        generation: u64,
    },
    /// Swap was scheduled for later (Epoch mode).
    Scheduled {
        /// Target epoch for the swap.
        target_epoch: u64,
    },
    /// Swap failed with an error.
    Error(HotSwapError),
}

/// Errors that can occur during hot-swap.
#[derive(Debug, Clone)]
pub enum HotSwapError {
    /// Old hash not found in VFT.
    OldHashNotFound,
    /// New hash not registered yet.
    NewHashNotRegistered,
    /// Arity mismatch between old and new.
    ArityMismatch {
        old_arity: u8,
        new_arity: u8,
    },
    /// Effect mismatch (new effects not subset of old).
    EffectMismatch {
        old_effects: EffectMask,
        new_effects: EffectMask,
    },
    /// Calling convention mismatch.
    ConventionMismatch {
        old: CallingConvention,
        new: CallingConvention,
    },
    /// In-flight calls blocking barrier swap.
    InFlightCallsBlocking {
        count: u64,
    },
}

/// A pending VFT update.
#[derive(Debug, Clone)]
pub struct VFTUpdate {
    /// Old hash being replaced.
    pub old_hash: ContentHash,
    /// New hash to use.
    pub new_hash: ContentHash,
    /// Swap mode.
    pub mode: SwapMode,
    /// Scheduled epoch (for Epoch mode).
    pub target_epoch: Option<u64>,
}

/// The Virtual Function Table.
#[derive(Debug)]
pub struct VFT {
    /// Hash-indexed lookup.
    entries: HashMap<ContentHash, VFTEntry>,
    /// All entries for iteration.
    all_entries: Vec<ContentHash>,
    /// Current version/generation.
    version: AtomicU64,
    /// Pending updates.
    pending_updates: Vec<VFTUpdate>,
    /// Redirects for hot-swap (old hash → new hash).
    redirects: HashMap<ContentHash, ContentHash>,
}

impl VFT {
    /// Create a new empty VFT.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            all_entries: Vec::new(),
            version: AtomicU64::new(0),
            pending_updates: Vec::new(),
            redirects: HashMap::new(),
        }
    }

    /// Register a new entry in the VFT.
    pub fn register(&mut self, entry: VFTEntry) {
        let hash = entry.hash;
        if !self.entries.contains_key(&hash) {
            self.all_entries.push(hash);
        }
        self.entries.insert(hash, entry);
    }

    /// Look up an entry by hash.
    pub fn lookup(&self, hash: ContentHash) -> Option<&VFTEntry> {
        // Follow redirects
        let actual_hash = self.resolve_redirect(hash);
        self.entries.get(&actual_hash)
    }

    /// Look up an entry, following redirects.
    fn resolve_redirect(&self, hash: ContentHash) -> ContentHash {
        let mut current = hash;
        let mut depth = 0;
        const MAX_REDIRECT_DEPTH: usize = 10;

        while let Some(&next) = self.redirects.get(&current) {
            current = next;
            depth += 1;
            if depth > MAX_REDIRECT_DEPTH {
                break;
            }
        }

        current
    }

    /// Check if an entry exists.
    pub fn contains(&self, hash: ContentHash) -> bool {
        self.entries.contains_key(&hash) || self.redirects.contains_key(&hash)
    }

    /// Get the current version.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// Get the entry point address for a hash.
    pub fn get_entry_point(&self, hash: ContentHash) -> Option<usize> {
        self.lookup(hash).map(|e| e.entry_point)
    }

    /// Get the arity for a hash.
    pub fn get_arity(&self, hash: ContentHash) -> Option<u8> {
        self.lookup(hash).map(|e| e.arity)
    }

    /// Prepare a hot-swap update.
    pub fn prepare_swap(&mut self, old_hash: ContentHash, new_hash: ContentHash, mode: SwapMode) {
        self.pending_updates.push(VFTUpdate {
            old_hash,
            new_hash,
            mode,
            target_epoch: None,
        });
    }

    /// Execute an immediate swap.
    pub fn immediate_swap(&mut self, old_hash: ContentHash, new_hash: ContentHash) {
        // Create redirect from old to new
        self.redirects.insert(old_hash, new_hash);
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    /// Execute pending swaps.
    pub fn commit_pending_swaps(&mut self) {
        let updates = std::mem::take(&mut self.pending_updates);
        for update in updates {
            match update.mode {
                SwapMode::Immediate => {
                    self.immediate_swap(update.old_hash, update.new_hash);
                }
                SwapMode::Barrier | SwapMode::Epoch => {
                    // In a real implementation, these would have different semantics
                    self.immediate_swap(update.old_hash, update.new_hash);
                }
            }
        }
    }

    /// Validate compatibility between old and new entries for hot-swap.
    ///
    /// Returns `Ok(())` if the swap is compatible, or an error describing the mismatch.
    pub fn validate_swap_compatibility(
        &self,
        old_hash: ContentHash,
        new_hash: ContentHash,
    ) -> Result<(), HotSwapError> {
        // Get both entries
        let old_entry = self.entries.get(&old_hash)
            .ok_or(HotSwapError::OldHashNotFound)?;
        let new_entry = self.entries.get(&new_hash)
            .ok_or(HotSwapError::NewHashNotRegistered)?;

        // Check arity (must match exactly for compatibility)
        if old_entry.arity != new_entry.arity {
            return Err(HotSwapError::ArityMismatch {
                old_arity: old_entry.arity,
                new_arity: new_entry.arity,
            });
        }

        // Check effects: new effects must be subset of old (can remove effects, not add)
        // This ensures callers that expected certain effects won't be surprised
        let old_effects = old_entry.effects.0;
        let new_effects = new_entry.effects.0;
        if new_effects & !old_effects != 0 {
            // New effects has bits not in old effects
            return Err(HotSwapError::EffectMismatch {
                old_effects: old_entry.effects,
                new_effects: new_entry.effects,
            });
        }

        // Check calling convention (must match for safe interop)
        if old_entry.calling_convention != new_entry.calling_convention {
            return Err(HotSwapError::ConventionMismatch {
                old: old_entry.calling_convention,
                new: new_entry.calling_convention,
            });
        }

        Ok(())
    }

    /// Perform a validated hot-swap with compatibility checking.
    ///
    /// This is the primary API for safe hot-swapping. It validates compatibility
    /// before performing the swap.
    pub fn hot_swap(
        &mut self,
        old_hash: ContentHash,
        new_hash: ContentHash,
        mode: SwapMode,
    ) -> HotSwapResult {
        // Validate compatibility first
        if let Err(e) = self.validate_swap_compatibility(old_hash, new_hash) {
            return HotSwapResult::Error(e);
        }

        match mode {
            SwapMode::Immediate => {
                self.immediate_swap(old_hash, new_hash);
                let generation = self.version.load(Ordering::SeqCst);
                HotSwapResult::Success {
                    old_hash,
                    new_hash,
                    generation,
                }
            }
            SwapMode::Barrier => {
                // In a full implementation, this would:
                // 1. Mark the entry as "draining"
                // 2. Wait for in-flight calls to complete
                // 3. Then swap
                // For now, we implement it as immediate
                self.immediate_swap(old_hash, new_hash);
                let generation = self.version.load(Ordering::SeqCst);
                HotSwapResult::Success {
                    old_hash,
                    new_hash,
                    generation,
                }
            }
            SwapMode::Epoch => {
                // Schedule for next epoch
                let target_epoch = self.version.load(Ordering::SeqCst) + 1;
                self.pending_updates.push(VFTUpdate {
                    old_hash,
                    new_hash,
                    mode: SwapMode::Epoch,
                    target_epoch: Some(target_epoch),
                });
                HotSwapResult::Scheduled { target_epoch }
            }
        }
    }

    /// Advance to the next epoch, executing any scheduled updates.
    pub fn advance_epoch(&mut self) {
        let current_epoch = self.version.load(Ordering::SeqCst);
        let next_epoch = current_epoch + 1;

        // Execute updates scheduled for this epoch
        let (to_execute, remaining): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_updates)
            .into_iter()
            .partition(|u| u.target_epoch == Some(next_epoch));

        self.pending_updates = remaining;

        for update in to_execute {
            self.immediate_swap(update.old_hash, update.new_hash);
        }

        // Bump the epoch even if no updates (for epoch tracking)
        self.version.store(next_epoch, Ordering::SeqCst);
    }

    /// Get the current epoch.
    pub fn current_epoch(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// Unregister an entry.
    pub fn unregister(&mut self, hash: ContentHash) {
        self.entries.remove(&hash);
        self.all_entries.retain(|h| *h != hash);
        self.redirects.remove(&hash);
    }

    /// Get all entry hashes.
    pub fn all_hashes(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.all_entries.iter().copied()
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the number of pending updates.
    pub fn pending_count(&self) -> usize {
        self.pending_updates.len()
    }

    /// Get the number of active redirects.
    pub fn redirect_count(&self) -> usize {
        self.redirects.len()
    }

    /// Clear stale redirects (clean up after GC).
    pub fn cleanup_redirects(&mut self) {
        // Remove redirects to non-existent entries
        self.redirects.retain(|_, target| self.entries.contains_key(target));
    }

    /// Iterate over all entries.
    pub fn iter(&self) -> impl Iterator<Item = &VFTEntry> {
        self.entries.values()
    }
}

impl Default for VFT {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for VFT {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            all_entries: self.all_entries.clone(),
            version: AtomicU64::new(self.version.load(Ordering::SeqCst)),
            pending_updates: self.pending_updates.clone(),
            redirects: self.redirects.clone(),
        }
    }
}

/// Dispatch table for multiple dispatch.
#[derive(Debug, Clone, Default)]
pub struct DispatchTable {
    /// Method hash → dispatch entries.
    methods: HashMap<ContentHash, Vec<DispatchEntry>>,
}

/// An entry in the dispatch table.
#[derive(Debug, Clone)]
pub struct DispatchEntry {
    /// Type pattern for matching.
    pub type_pattern: TypePattern,
    /// Specialization specificity (higher = more specific).
    pub specificity: u32,
    /// Implementation hash.
    pub impl_hash: ContentHash,
}

/// A type pattern for dispatch matching.
#[derive(Debug, Clone)]
pub enum TypePattern {
    /// Match any type.
    Any,
    /// Match a specific concrete type.
    Concrete(ContentHash),
    /// Match a type constructor with arguments.
    Applied {
        constructor: ContentHash,
        args: Vec<TypePattern>,
    },
}

impl TypePattern {
    /// Check if this pattern matches the given type hash.
    pub fn matches(&self, type_hash: ContentHash) -> bool {
        match self {
            Self::Any => true,
            Self::Concrete(h) => *h == type_hash,
            Self::Applied { constructor, .. } => {
                // Simplified: just check constructor
                *constructor == type_hash
            }
        }
    }

    /// Calculate the specificity of this pattern.
    pub fn specificity(&self) -> u32 {
        match self {
            Self::Any => 0,
            Self::Concrete(_) => 10,
            Self::Applied { args, .. } => {
                10 + args.iter().map(|a| a.specificity()).sum::<u32>()
            }
        }
    }
}

impl DispatchTable {
    /// Create a new dispatch table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an implementation.
    pub fn register(&mut self, method_hash: ContentHash, entry: DispatchEntry) {
        self.methods.entry(method_hash).or_default().push(entry);
    }

    /// Find the best matching implementation for given argument types.
    pub fn dispatch(
        &self,
        method_hash: ContentHash,
        arg_type: ContentHash,
    ) -> Option<ContentHash> {
        let entries = self.methods.get(&method_hash)?;

        entries
            .iter()
            .filter(|e| e.type_pattern.matches(arg_type))
            .max_by_key(|e| e.specificity)
            .map(|e| e.impl_hash)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hash(name: &str) -> ContentHash {
        ContentHash::compute(name.as_bytes())
    }

    #[test]
    fn test_vft_register_lookup() {
        let mut vft = VFT::new();
        let hash = test_hash("add");
        let entry = VFTEntry::new(hash, 0x1000, 2);

        vft.register(entry);

        let found = vft.lookup(hash);
        assert!(found.is_some());
        assert_eq!(found.unwrap().arity, 2);
        assert_eq!(found.unwrap().entry_point, 0x1000);
    }

    #[test]
    fn test_vft_entry_builder() {
        let hash = test_hash("effect_fn");
        let entry = VFTEntry::new(hash, 0x2000, 1)
            .with_convention(CallingConvention::Effect)
            .with_effects(EffectMask::IO)
            .with_size(64)
            .with_generation(5);

        assert_eq!(entry.calling_convention, CallingConvention::Effect);
        assert!(entry.effects.contains(EffectMask::IO));
        assert_eq!(entry.compiled_size, 64);
        assert_eq!(entry.generation, 5);
    }

    #[test]
    fn test_vft_immediate_swap() {
        let mut vft = VFT::new();

        let old_hash = test_hash("v1");
        let new_hash = test_hash("v2");

        vft.register(VFTEntry::new(old_hash, 0x1000, 1));
        vft.register(VFTEntry::new(new_hash, 0x2000, 1));

        vft.immediate_swap(old_hash, new_hash);

        // Looking up old hash should redirect to new
        let entry = vft.lookup(old_hash).unwrap();
        assert_eq!(entry.hash, new_hash);
    }

    #[test]
    fn test_vft_version_increment() {
        let mut vft = VFT::new();
        let initial = vft.version();

        let old_hash = test_hash("v1");
        let new_hash = test_hash("v2");
        vft.register(VFTEntry::new(old_hash, 0x1000, 1));
        vft.register(VFTEntry::new(new_hash, 0x2000, 1));

        vft.immediate_swap(old_hash, new_hash);

        assert_eq!(vft.version(), initial + 1);
    }

    #[test]
    fn test_vft_unregister() {
        let mut vft = VFT::new();
        let hash = test_hash("remove_me");

        vft.register(VFTEntry::new(hash, 0x1000, 0));
        assert!(vft.contains(hash));

        vft.unregister(hash);
        assert!(!vft.contains(hash));
    }

    #[test]
    fn test_effect_mask() {
        let io = EffectMask::IO;
        let state = EffectMask::STATE;
        let combined = io.union(state);

        assert!(combined.contains(EffectMask::IO));
        assert!(combined.contains(EffectMask::STATE));
        assert!(!combined.contains(EffectMask::EXCEPTION));
        assert!(!combined.is_pure());
    }

    #[test]
    fn test_dispatch_table() {
        let mut table = DispatchTable::new();

        let method = test_hash("show");
        let int_impl = test_hash("show_int");
        let string_impl = test_hash("show_string");
        let int_type = test_hash("Int");
        let string_type = test_hash("String");

        table.register(
            method,
            DispatchEntry {
                type_pattern: TypePattern::Concrete(int_type),
                specificity: 10,
                impl_hash: int_impl,
            },
        );
        table.register(
            method,
            DispatchEntry {
                type_pattern: TypePattern::Concrete(string_type),
                specificity: 10,
                impl_hash: string_impl,
            },
        );

        assert_eq!(table.dispatch(method, int_type), Some(int_impl));
        assert_eq!(table.dispatch(method, string_type), Some(string_impl));
    }

    #[test]
    fn test_type_pattern_specificity() {
        assert_eq!(TypePattern::Any.specificity(), 0);
        assert_eq!(TypePattern::Concrete(test_hash("Int")).specificity(), 10);

        let applied = TypePattern::Applied {
            constructor: test_hash("List"),
            args: vec![TypePattern::Concrete(test_hash("Int"))],
        };
        assert_eq!(applied.specificity(), 20);
    }

    #[test]
    fn test_vft_pending_swaps() {
        let mut vft = VFT::new();

        let old = test_hash("old");
        let new = test_hash("new");
        vft.register(VFTEntry::new(old, 0x1000, 1));
        vft.register(VFTEntry::new(new, 0x2000, 1));

        vft.prepare_swap(old, new, SwapMode::Immediate);
        assert_eq!(vft.pending_count(), 1);

        vft.commit_pending_swaps();
        assert_eq!(vft.pending_count(), 0);
        assert_eq!(vft.redirect_count(), 1);
    }

    #[test]
    fn test_vft_is_pure() {
        let hash = test_hash("pure_fn");
        let pure_entry = VFTEntry::new(hash, 0x1000, 1);
        assert!(pure_entry.is_pure());

        let impure_entry = pure_entry.with_effects(EffectMask::IO);
        assert!(!impure_entry.is_pure());
    }

    // ========================================================================
    // Hot-swap validation tests
    // ========================================================================

    #[test]
    fn test_validate_swap_compatibility_success() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        // Same arity, same effects, same convention
        vft.register(VFTEntry::new(old, 0x1000, 2).with_effects(EffectMask::IO));
        vft.register(VFTEntry::new(new, 0x2000, 2).with_effects(EffectMask::NONE));

        // New has fewer effects (subset), should be OK
        assert!(vft.validate_swap_compatibility(old, new).is_ok());
    }

    #[test]
    fn test_validate_swap_arity_mismatch() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        vft.register(VFTEntry::new(old, 0x1000, 2));
        vft.register(VFTEntry::new(new, 0x2000, 3)); // Different arity

        let result = vft.validate_swap_compatibility(old, new);
        assert!(matches!(
            result,
            Err(HotSwapError::ArityMismatch { old_arity: 2, new_arity: 3 })
        ));
    }

    #[test]
    fn test_validate_swap_effect_mismatch() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        // Old has no effects, new has IO (adding effects is not allowed)
        vft.register(VFTEntry::new(old, 0x1000, 1).with_effects(EffectMask::NONE));
        vft.register(VFTEntry::new(new, 0x2000, 1).with_effects(EffectMask::IO));

        let result = vft.validate_swap_compatibility(old, new);
        assert!(matches!(result, Err(HotSwapError::EffectMismatch { .. })));
    }

    #[test]
    fn test_validate_swap_convention_mismatch() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        vft.register(VFTEntry::new(old, 0x1000, 1).with_convention(CallingConvention::Blood));
        vft.register(VFTEntry::new(new, 0x2000, 1).with_convention(CallingConvention::Tail));

        let result = vft.validate_swap_compatibility(old, new);
        assert!(matches!(
            result,
            Err(HotSwapError::ConventionMismatch {
                old: CallingConvention::Blood,
                new: CallingConvention::Tail
            })
        ));
    }

    #[test]
    fn test_validate_swap_old_not_found() {
        let mut vft = VFT::new();

        let old = test_hash("missing");
        let new = test_hash("fn_v2");

        vft.register(VFTEntry::new(new, 0x2000, 1));

        let result = vft.validate_swap_compatibility(old, new);
        assert!(matches!(result, Err(HotSwapError::OldHashNotFound)));
    }

    #[test]
    fn test_validate_swap_new_not_registered() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("missing");

        vft.register(VFTEntry::new(old, 0x1000, 1));

        let result = vft.validate_swap_compatibility(old, new);
        assert!(matches!(result, Err(HotSwapError::NewHashNotRegistered)));
    }

    // ========================================================================
    // Hot-swap execution tests
    // ========================================================================

    #[test]
    fn test_hot_swap_immediate_success() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        vft.register(VFTEntry::new(old, 0x1000, 1));
        vft.register(VFTEntry::new(new, 0x2000, 1));

        let result = vft.hot_swap(old, new, SwapMode::Immediate);

        match result {
            HotSwapResult::Success { old_hash, new_hash, generation } => {
                assert_eq!(old_hash, old);
                assert_eq!(new_hash, new);
                assert!(generation > 0);
            }
            _ => panic!("Expected HotSwapResult::Success"),
        }

        // Verify redirect works
        let entry = vft.lookup(old).unwrap();
        assert_eq!(entry.hash, new);
    }

    #[test]
    fn test_hot_swap_barrier_success() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        vft.register(VFTEntry::new(old, 0x1000, 1));
        vft.register(VFTEntry::new(new, 0x2000, 1));

        let result = vft.hot_swap(old, new, SwapMode::Barrier);

        assert!(matches!(result, HotSwapResult::Success { .. }));
    }

    #[test]
    fn test_hot_swap_epoch_scheduled() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        vft.register(VFTEntry::new(old, 0x1000, 1));
        vft.register(VFTEntry::new(new, 0x2000, 1));

        let current = vft.current_epoch();
        let result = vft.hot_swap(old, new, SwapMode::Epoch);

        match result {
            HotSwapResult::Scheduled { target_epoch } => {
                assert_eq!(target_epoch, current + 1);
            }
            _ => panic!("Expected HotSwapResult::Scheduled"),
        }

        // Swap not yet applied
        let entry = vft.lookup(old).unwrap();
        assert_eq!(entry.hash, old);

        // Advance epoch
        vft.advance_epoch();

        // Now swap should be applied
        let entry = vft.lookup(old).unwrap();
        assert_eq!(entry.hash, new);
    }

    #[test]
    fn test_hot_swap_validation_error() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        // Different arities - should fail validation
        vft.register(VFTEntry::new(old, 0x1000, 1));
        vft.register(VFTEntry::new(new, 0x2000, 2));

        let result = vft.hot_swap(old, new, SwapMode::Immediate);

        assert!(matches!(result, HotSwapResult::Error(HotSwapError::ArityMismatch { .. })));
    }

    // ========================================================================
    // Epoch management tests
    // ========================================================================

    #[test]
    fn test_advance_epoch_no_updates() {
        let mut vft = VFT::new();

        let initial = vft.current_epoch();
        vft.advance_epoch();
        assert_eq!(vft.current_epoch(), initial + 1);
    }

    #[test]
    fn test_advance_epoch_with_multiple_updates() {
        let mut vft = VFT::new();

        let old1 = test_hash("fn1_v1");
        let new1 = test_hash("fn1_v2");
        let old2 = test_hash("fn2_v1");
        let new2 = test_hash("fn2_v2");

        vft.register(VFTEntry::new(old1, 0x1000, 1));
        vft.register(VFTEntry::new(new1, 0x2000, 1));
        vft.register(VFTEntry::new(old2, 0x3000, 2));
        vft.register(VFTEntry::new(new2, 0x4000, 2));

        // Schedule both for epoch swap
        assert!(matches!(
            vft.hot_swap(old1, new1, SwapMode::Epoch),
            HotSwapResult::Scheduled { .. }
        ));
        assert!(matches!(
            vft.hot_swap(old2, new2, SwapMode::Epoch),
            HotSwapResult::Scheduled { .. }
        ));

        assert_eq!(vft.pending_count(), 2);

        // Advance epoch
        vft.advance_epoch();

        assert_eq!(vft.pending_count(), 0);

        // Both swaps applied
        assert_eq!(vft.lookup(old1).unwrap().hash, new1);
        assert_eq!(vft.lookup(old2).unwrap().hash, new2);
    }

    #[test]
    fn test_cleanup_redirects() {
        let mut vft = VFT::new();

        let old = test_hash("fn_v1");
        let new = test_hash("fn_v2");

        vft.register(VFTEntry::new(old, 0x1000, 1));
        vft.register(VFTEntry::new(new, 0x2000, 1));

        vft.immediate_swap(old, new);
        assert_eq!(vft.redirect_count(), 1);

        // Remove the target entry
        vft.unregister(new);

        // Cleanup stale redirects
        vft.cleanup_redirects();
        assert_eq!(vft.redirect_count(), 0);
    }
}
