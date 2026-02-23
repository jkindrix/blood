//! # Generation Snapshots
//!
//! This module implements generation snapshots for safe effect handler continuations.
//!
//! ## Purpose
//!
//! When an effect operation captures a continuation, any generational references
//! in the captured environment must be validated when the continuation resumes.
//! Generation snapshots record the expected generations at capture time.
//!
//! ## Design References
//!
//! - [MEMORY_MODEL.md §6](../../MEMORY_MODEL.md): Generation Snapshots
//! - [ROADMAP.md §7.4](../../ROADMAP.md): Generation Snapshot Algorithm
//!
//! ## Algorithm
//!
//! From MEMORY_MODEL.md:
//! ```text
//! fn capture_snapshot(env: &Environment) -> GenerationSnapshot {
//!     let mut entries = Vec::new();
//!     for binding in env.bindings() {
//!         if binding.ty.contains_genref() {
//!             for genref in extract_genrefs(&binding.value) {
//!                 entries.push((genref.address, genref.generation));
//!             }
//!         }
//!     }
//!     GenerationSnapshot { entries }
//! }
//! ```
//!
//! ## Validation
//!
//! On resume, each entry in the snapshot is validated:
//! - If generation matches: reference is still valid
//! - If generation mismatches: StaleReference effect is raised

use std::collections::HashSet;

use crate::hir::{LocalId, Type, TypeKind};
use super::body::MirBody;
use super::ptr::{BloodPtr, PERSISTENT_MARKER};
use super::types::{Place, Operand, Rvalue, StatementKind, TerminatorKind};

// ============================================================================
// Snapshot Identification
// ============================================================================

/// Unique identifier for a snapshot in a snapshot chain.
///
/// Used to track parent-child relationships in nested handler contexts.
/// Snapshot IDs are assigned sequentially and are unique within a compilation unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotId(pub u32);

impl SnapshotId {
    /// Create a new snapshot ID.
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw ID value.
    pub fn index(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "snapshot_{}", self.0)
    }
}

// ============================================================================
// Generation Snapshot
// ============================================================================

/// A snapshot of generation values at continuation capture time.
///
/// From MEMORY_MODEL.md §6:
/// "Generation snapshots record the expected generations at capture time.
/// On resume, each entry is validated."
///
/// ## Snapshot Sharing
///
/// For nested effect handlers, snapshots can form a chain via parent references.
/// This reduces memory from O(n²) to O(n) for deeply nested handlers:
///
/// ```text
/// Snapshot_C → Snapshot_B → Snapshot_A → None
///    │              │             │
///    └── delta_C    └── delta_B   └── full_snapshot
/// ```
///
/// Each child snapshot only stores entries that differ from its parent,
/// avoiding duplication of common data.
#[derive(Debug, Clone, Default)]
pub struct GenerationSnapshot {
    /// Optional unique identifier for this snapshot.
    pub id: Option<SnapshotId>,
    /// Reference to parent snapshot for nested handlers.
    /// When set, this snapshot only contains entries added since the parent.
    pub parent: Option<SnapshotId>,
    /// Entries mapping addresses to expected generations.
    /// For child snapshots, this only includes new/modified entries.
    pub entries: Vec<SnapshotEntry>,
}

impl GenerationSnapshot {
    /// Create an empty snapshot.
    pub fn new() -> Self {
        Self {
            id: None,
            parent: None,
            entries: Vec::new(),
        }
    }

    /// Create a snapshot with a specific ID.
    pub fn with_id(id: SnapshotId) -> Self {
        Self {
            id: Some(id),
            parent: None,
            entries: Vec::new(),
        }
    }

    /// Create a child snapshot that references a parent.
    ///
    /// The child snapshot only needs to store entries that are new or modified
    /// compared to the parent, reducing memory for nested handlers.
    pub fn with_parent(id: SnapshotId, parent_id: SnapshotId) -> Self {
        Self {
            id: Some(id),
            parent: Some(parent_id),
            entries: Vec::new(),
        }
    }

    /// Create with capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            id: None,
            parent: None,
            entries: Vec::with_capacity(capacity),
        }
    }

    /// Check if this is a root snapshot (no parent).
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// Check if this is a child snapshot (has parent).
    pub fn has_parent(&self) -> bool {
        self.parent.is_some()
    }

    /// Get the parent snapshot ID, if any.
    pub fn parent_id(&self) -> Option<SnapshotId> {
        self.parent
    }

    /// Get this snapshot's ID, if assigned.
    pub fn snapshot_id(&self) -> Option<SnapshotId> {
        self.id
    }

    /// Set the snapshot ID.
    pub fn set_id(&mut self, id: SnapshotId) {
        self.id = Some(id);
    }

    /// Set the parent snapshot ID.
    pub fn set_parent(&mut self, parent_id: SnapshotId) {
        self.parent = Some(parent_id);
    }

    /// Add an entry to the snapshot.
    pub fn add_entry(&mut self, entry: SnapshotEntry) {
        // Skip persistent pointers (they don't need validation)
        if entry.generation != PERSISTENT_MARKER {
            self.entries.push(entry);
        }
    }

    /// Add a pointer to the snapshot.
    pub fn add_ptr(&mut self, ptr: &BloodPtr, local: LocalId) {
        if ptr.generation != PERSISTENT_MARKER && !ptr.is_null() {
            self.entries.push(SnapshotEntry {
                address: ptr.address,
                generation: ptr.generation,
                local,
            });
        }
    }

    /// Check if the snapshot is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Validate all entries against current generations.
    ///
    /// Returns `Ok(())` if all generations match, or `Err` with the first
    /// stale entry found.
    pub fn validate<F>(&self, get_current_gen: F) -> Result<(), SnapshotValidationError>
    where
        F: Fn(u64) -> Option<u32>,
    {
        for entry in &self.entries {
            if let Some(current_gen) = get_current_gen(entry.address) {
                if entry.generation != current_gen {
                    return Err(SnapshotValidationError {
                        entry: entry.clone(),
                        actual_generation: current_gen,
                    });
                }
            } else {
                // Address not found - slot was freed
                return Err(SnapshotValidationError {
                    entry: entry.clone(),
                    actual_generation: 0, // freed
                });
            }
        }
        Ok(())
    }

    /// Merge another snapshot into this one.
    pub fn merge(&mut self, other: &GenerationSnapshot) {
        for entry in &other.entries {
            // Avoid duplicates
            if !self.entries.iter().any(|e| e.address == entry.address) {
                self.entries.push(entry.clone());
            }
        }
    }

    /// Get all unique addresses in this snapshot.
    pub fn addresses(&self) -> HashSet<u64> {
        self.entries.iter().map(|e| e.address).collect()
    }

    /// Get all locals referenced in this snapshot.
    pub fn locals(&self) -> HashSet<LocalId> {
        self.entries.iter().map(|e| e.local).collect()
    }

    /// Get the total entry count including parent chain.
    ///
    /// This requires access to a SnapshotStore to traverse the parent chain.
    pub fn total_entries(&self, store: &SnapshotStore) -> usize {
        let mut count = self.entries.len();
        if let Some(parent_id) = self.parent {
            if let Some(parent) = store.get(parent_id) {
                count += parent.total_entries(store);
            }
        }
        count
    }
}

// ============================================================================
// Snapshot Store
// ============================================================================

/// A store for managing snapshot chains in nested handlers.
///
/// The SnapshotStore enables efficient sharing of snapshot data between
/// nested effect handlers. Each nested handler creates a child snapshot
/// that references its parent, avoiding duplication of common entries.
///
/// ## Memory Model
///
/// Without sharing (n nested handlers with m entries each):
/// - Memory: O(n * m) per handler = O(n² * m) total
///
/// With sharing:
/// - Memory: O(m) for root + O(delta) per child = O(n * delta) total
///
/// For deeply nested handlers where delta << m, this provides significant
/// memory savings.
///
/// ## Example
///
/// ```ignore
/// let mut store = SnapshotStore::new();
///
/// // Create root snapshot
/// let root_id = store.create_root();
/// store.get_mut(root_id).unwrap().add_entry(...);
///
/// // Create child snapshot for nested handler
/// let child_id = store.create_child(root_id);
/// store.get_mut(child_id).unwrap().add_entry(...); // Only delta entries
///
/// // Validate walks the entire chain
/// store.validate_chain(child_id, |addr| ...)?;
/// ```
#[derive(Debug, Default)]
pub struct SnapshotStore {
    /// All snapshots indexed by ID.
    snapshots: Vec<GenerationSnapshot>,
    /// Counter for generating unique IDs.
    next_id: u32,
}

impl SnapshotStore {
    /// Create a new empty snapshot store.
    pub fn new() -> Self {
        Self {
            snapshots: Vec::new(),
            next_id: 0,
        }
    }

    /// Create a root snapshot (no parent) and return its ID.
    pub fn create_root(&mut self) -> SnapshotId {
        let id = SnapshotId::new(self.next_id);
        self.next_id += 1;
        self.snapshots.push(GenerationSnapshot::with_id(id));
        id
    }

    /// Create a child snapshot with the given parent and return its ID.
    pub fn create_child(&mut self, parent_id: SnapshotId) -> SnapshotId {
        let id = SnapshotId::new(self.next_id);
        self.next_id += 1;
        self.snapshots.push(GenerationSnapshot::with_parent(id, parent_id));
        id
    }

    /// Get a snapshot by ID.
    pub fn get(&self, id: SnapshotId) -> Option<&GenerationSnapshot> {
        self.snapshots.get(id.0 as usize)
    }

    /// Get a mutable reference to a snapshot by ID.
    pub fn get_mut(&mut self, id: SnapshotId) -> Option<&mut GenerationSnapshot> {
        self.snapshots.get_mut(id.0 as usize)
    }

    /// Validate a snapshot and its entire parent chain.
    ///
    /// Validation walks from the given snapshot up through all parent
    /// snapshots, checking each entry against the current generation.
    pub fn validate_chain<F>(&self, id: SnapshotId, get_current_gen: F) -> Result<(), SnapshotValidationError>
    where
        F: Fn(u64) -> Option<u32>,
    {
        let mut current_id = Some(id);

        while let Some(snap_id) = current_id {
            let snapshot = self.get(snap_id).ok_or_else(|| SnapshotValidationError {
                entry: SnapshotEntry::new(0, 0, LocalId::new(0)),
                actual_generation: 0,
            })?;

            // Validate this snapshot's entries
            snapshot.validate(&get_current_gen)?;

            // Move to parent
            current_id = snapshot.parent;
        }

        Ok(())
    }

    /// Get the depth of a snapshot chain (number of snapshots from root to this one).
    pub fn chain_depth(&self, id: SnapshotId) -> usize {
        let mut depth = 0;
        let mut current_id = Some(id);

        while let Some(snap_id) = current_id {
            depth += 1;
            current_id = self.get(snap_id).and_then(|s| s.parent);
        }

        depth
    }

    /// Get all snapshot IDs in the chain from root to the given snapshot.
    pub fn chain_to_root(&self, id: SnapshotId) -> Vec<SnapshotId> {
        let mut chain = Vec::new();
        let mut current_id = Some(id);

        while let Some(snap_id) = current_id {
            chain.push(snap_id);
            current_id = self.get(snap_id).and_then(|s| s.parent);
        }

        chain.reverse(); // Return root-first order
        chain
    }

    /// Get all entries in a snapshot chain (flattened).
    ///
    /// Returns entries from root to leaf, with later entries overriding
    /// earlier ones for the same address.
    pub fn flatten_chain(&self, id: SnapshotId) -> Vec<SnapshotEntry> {
        let chain = self.chain_to_root(id);
        let mut seen_addresses = HashSet::new();
        let mut result = Vec::new();

        // Process from root to leaf so that child entries override parent
        for snap_id in chain {
            if let Some(snapshot) = self.get(snap_id) {
                for entry in &snapshot.entries {
                    if seen_addresses.insert(entry.address) {
                        result.push(entry.clone());
                    } else {
                        // Update existing entry with newer generation
                        if let Some(existing) = result.iter_mut().find(|e| e.address == entry.address) {
                            *existing = entry.clone();
                        }
                    }
                }
            }
        }

        result
    }

    /// Get total entry count across all snapshots.
    pub fn total_entries(&self) -> usize {
        self.snapshots.iter().map(|s| s.entries.len()).sum()
    }

    /// Get the number of snapshots in the store.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Remove a snapshot and all its descendants.
    ///
    /// Note: This invalidates IDs. Use with caution.
    pub fn remove_chain(&mut self, id: SnapshotId) {
        // Find all descendants
        let descendants: Vec<_> = self.snapshots
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                // Check if this snapshot is in the chain rooted at id
                let mut current = s.parent;
                while let Some(pid) = current {
                    if pid == id {
                        return true;
                    }
                    current = self.get(pid).and_then(|p| p.parent);
                }
                s.id == Some(id)
            })
            .map(|(i, _)| i)
            .collect();

        // Remove in reverse order to maintain indices
        for idx in descendants.into_iter().rev() {
            self.snapshots.remove(idx);
        }
    }
}

/// An entry in a generation snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotEntry {
    /// The address of the generational reference.
    pub address: u64,
    /// The expected generation at capture time.
    pub generation: u32,
    /// The local variable this came from (for error reporting).
    pub local: LocalId,
}

impl SnapshotEntry {
    /// Create a new snapshot entry.
    pub fn new(address: u64, generation: u32, local: LocalId) -> Self {
        Self {
            address,
            generation,
            local,
        }
    }
}

/// Error when snapshot validation fails.
#[derive(Debug, Clone)]
pub struct SnapshotValidationError {
    /// The entry that failed validation.
    pub entry: SnapshotEntry,
    /// The actual generation found.
    pub actual_generation: u32,
}

impl std::fmt::Display for SnapshotValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stale reference in continuation: local _{} at {:#x} expected generation {}, found {}",
            self.entry.local.index, self.entry.address, self.entry.generation, self.actual_generation
        )
    }
}

impl std::error::Error for SnapshotValidationError {}

// ============================================================================
// Lazy Validation
// ============================================================================

/// A lazily-validated generation snapshot.
///
/// This optimization defers validation until a reference is actually used,
/// avoiding unnecessary validation of entries that are never accessed.
///
/// # Use Case
///
/// In many cases, only a subset of captured references are accessed after
/// resumption. Lazy validation skips validation for unused references,
/// reducing overhead in common cases.
///
/// # Thread Safety
///
/// This type uses interior mutability for validation state tracking.
/// It is NOT thread-safe and should only be used from a single thread.
///
/// # Example
///
/// ```ignore
/// let lazy = LazySnapshot::new(snapshot, get_gen);
///
/// // Only validates entry 0
/// lazy.validate_entry(0)?;
///
/// // Validates all remaining entries
/// lazy.validate_all()?;
/// ```
#[derive(Debug)]
pub struct LazySnapshot<F>
where
    F: Fn(u64) -> Option<u32>,
{
    /// The underlying snapshot.
    snapshot: GenerationSnapshot,
    /// Validation state for each entry (true = validated).
    validated: std::cell::RefCell<Vec<bool>>,
    /// Function to get current generation for an address.
    get_gen: F,
    /// Cached first validation error (if any).
    cached_error: std::cell::RefCell<Option<SnapshotValidationError>>,
}

impl<F> LazySnapshot<F>
where
    F: Fn(u64) -> Option<u32>,
{
    /// Create a new lazy snapshot wrapper.
    pub fn new(snapshot: GenerationSnapshot, get_gen: F) -> Self {
        let len = snapshot.entries.len();
        Self {
            snapshot,
            validated: std::cell::RefCell::new(vec![false; len]),
            get_gen,
            cached_error: std::cell::RefCell::new(None),
        }
    }

    /// Validate a single entry by index.
    ///
    /// Returns `Ok(())` if the entry is valid or was already validated.
    /// Returns `Err` if validation fails.
    pub fn validate_entry(&self, index: usize) -> Result<(), SnapshotValidationError> {
        // Check if already validated
        {
            let validated = self.validated.borrow();
            if index >= validated.len() {
                return Ok(()); // Index out of bounds - nothing to validate
            }
            if validated[index] {
                return Ok(()); // Already validated
            }
        }

        // Check for cached error
        if let Some(err) = self.cached_error.borrow().as_ref() {
            return Err(err.clone());
        }

        // Perform validation
        let entry = &self.snapshot.entries[index];
        if let Some(current_gen) = (self.get_gen)(entry.address) {
            if entry.generation != current_gen {
                let err = SnapshotValidationError {
                    entry: entry.clone(),
                    actual_generation: current_gen,
                };
                *self.cached_error.borrow_mut() = Some(err.clone());
                return Err(err);
            }
        } else {
            // Address not found - slot was freed
            let err = SnapshotValidationError {
                entry: entry.clone(),
                actual_generation: 0,
            };
            *self.cached_error.borrow_mut() = Some(err.clone());
            return Err(err);
        }

        // Mark as validated
        self.validated.borrow_mut()[index] = true;
        Ok(())
    }

    /// Validate entry for a specific local.
    ///
    /// Returns `Ok(())` if no entries exist for this local or all are valid.
    pub fn validate_local(&self, local: LocalId) -> Result<(), SnapshotValidationError> {
        for (idx, entry) in self.snapshot.entries.iter().enumerate() {
            if entry.local == local {
                self.validate_entry(idx)?;
            }
        }
        Ok(())
    }

    /// Validate all entries that haven't been validated yet.
    ///
    /// This is equivalent to calling `validate_entry` for each entry.
    pub fn validate_all(&self) -> Result<(), SnapshotValidationError> {
        // Check for cached error first
        if let Some(err) = self.cached_error.borrow().as_ref() {
            return Err(err.clone());
        }

        for idx in 0..self.snapshot.entries.len() {
            self.validate_entry(idx)?;
        }
        Ok(())
    }

    /// Check if all entries have been validated.
    pub fn is_fully_validated(&self) -> bool {
        self.validated.borrow().iter().all(|&v| v)
    }

    /// Get the number of validated entries.
    pub fn validated_count(&self) -> usize {
        self.validated.borrow().iter().filter(|&&v| v).count()
    }

    /// Get the total number of entries.
    pub fn total_count(&self) -> usize {
        self.snapshot.entries.len()
    }

    /// Get validation statistics.
    pub fn stats(&self) -> LazyValidationStats {
        let validated = self.validated.borrow();
        LazyValidationStats {
            total: validated.len(),
            validated: validated.iter().filter(|&&v| v).count(),
            skipped: validated.iter().filter(|&&v| !v).count(),
        }
    }

    /// Get the underlying snapshot (consumes the lazy wrapper).
    pub fn into_snapshot(self) -> GenerationSnapshot {
        self.snapshot
    }

    /// Get a reference to the underlying snapshot.
    pub fn snapshot(&self) -> &GenerationSnapshot {
        &self.snapshot
    }
}

/// Statistics about lazy validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LazyValidationStats {
    /// Total number of entries.
    pub total: usize,
    /// Number of entries that were validated.
    pub validated: usize,
    /// Number of entries that were skipped (never accessed).
    pub skipped: usize,
}

impl LazyValidationStats {
    /// Calculate the percentage of entries that were skipped.
    pub fn skip_percentage(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.skipped as f64 / self.total as f64) * 100.0
        }
    }
}

// ============================================================================
// Snapshot Capture Analysis
// ============================================================================

/// Analyzes which locals need to be included in generation snapshots.
#[derive(Debug)]
pub struct SnapshotAnalyzer {
    /// Locals that contain generational references.
    genref_locals: HashSet<LocalId>,
}

impl SnapshotAnalyzer {
    /// Create a new snapshot analyzer.
    pub fn new() -> Self {
        Self {
            genref_locals: HashSet::new(),
        }
    }

    /// Analyze a MIR body to find locals that need snapshot capture.
    pub fn analyze(&mut self, body: &MirBody) -> SnapshotRequirements {
        self.genref_locals.clear();

        // Find all locals that may contain generational references
        for local in &body.locals {
            if self.type_contains_genref(&local.ty) {
                self.genref_locals.insert(local.id);
            }
        }

        // Find effect suspension points and their live locals
        let mut requirements = SnapshotRequirements::new();

        for (bb_id, block) in body.blocks() {
            if let Some(term) = &block.terminator {
                if let TerminatorKind::Perform { .. } = &term.kind {
                    // This is an effect suspension point
                    let live_genrefs = self.compute_live_genrefs(body, bb_id);
                    requirements.suspension_points.push(SuspensionPoint {
                        block: bb_id,
                        locals_to_capture: live_genrefs,
                    });
                }
            }
        }

        requirements
    }

    /// Check if a type may contain generational references.
    fn type_contains_genref(&self, ty: &Type) -> bool {
        match &*ty.kind {
            TypeKind::Ptr { .. } | TypeKind::Ref { .. } => true,
            TypeKind::Array { ref element, .. } => self.type_contains_genref(element),
            TypeKind::Slice { ref element } => self.type_contains_genref(element),
            TypeKind::Tuple(elems) => elems.iter().any(|e| self.type_contains_genref(e)),
            TypeKind::Adt { .. } => true, // Conservative: ADTs might contain refs
            TypeKind::Fn { .. } => true, // Closures might capture refs
            TypeKind::Closure { .. } => true, // Closures capture environment which might contain refs
            TypeKind::Range { ref element, .. } => self.type_contains_genref(element),
            TypeKind::DynTrait { .. } => true, // Trait objects are fat pointers
            // Primitives, inference vars, type params, never, and error types don't contain refs
            TypeKind::Primitive(_) => false,
            TypeKind::Infer(_) => false,
            TypeKind::Param(_) => false,
            TypeKind::Never => false,
            TypeKind::Error => false,
            // Records may contain refs if any field does
            TypeKind::Record { fields, .. } => fields.iter().any(|f| self.type_contains_genref(&f.ty)),
            // Forall types may contain refs if body does
            TypeKind::Forall { body, .. } => self.type_contains_genref(body),
            // Ownership-qualified types delegate to the inner type
            TypeKind::Ownership { inner, .. } => self.type_contains_genref(inner),
        }
    }

    /// Compute live generational reference locals at a block.
    ///
    /// Uses proper dataflow liveness analysis to determine which locals
    /// containing generational references are actually live at the suspension
    /// point. This optimizes snapshot size by only capturing references that
    /// will actually be used after resumption.
    ///
    /// Based on [rustc_mir_dataflow](https://nnethercote.github.io/2024/12/19/streamlined-dataflow-analysis-code-in-rustc.html)
    /// liveness analysis approach.
    fn compute_live_genrefs(&self, body: &MirBody, target_bb: super::types::BasicBlockId) -> Vec<LocalId> {
        // Perform liveness analysis on the MIR body
        let liveness = LivenessAnalysis::analyze(body);

        // Get the live locals at the entry of the target block
        // (for effect operations, we want locals live after the effect returns)
        let live_at_target = liveness.live_out.get(&target_bb)
            .cloned()
            .unwrap_or_default();

        // Filter to only include locals that:
        // 1. Are marked as containing genrefs
        // 2. Are actually live at the suspension point
        let mut live_genrefs: Vec<_> = self.genref_locals.iter()
            .filter(|local| live_at_target.contains(local))
            .cloned()
            .collect();

        // Sort for deterministic output
        live_genrefs.sort_by_key(|l| l.index);

        live_genrefs
    }
}

impl Default for SnapshotAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Requirements for generation snapshots in a function.
#[derive(Debug, Clone, Default)]
pub struct SnapshotRequirements {
    /// Effect suspension points that need snapshots.
    pub suspension_points: Vec<SuspensionPoint>,
}

impl SnapshotRequirements {
    /// Create empty requirements.
    pub fn new() -> Self {
        Self {
            suspension_points: Vec::new(),
        }
    }

    /// Check if any snapshots are needed.
    pub fn needs_snapshots(&self) -> bool {
        !self.suspension_points.is_empty()
    }

    /// Get all locals that need to be captured across all suspension points.
    pub fn all_captured_locals(&self) -> HashSet<LocalId> {
        let mut all = HashSet::new();
        for sp in &self.suspension_points {
            for &local in &sp.locals_to_capture {
                all.insert(local);
            }
        }
        all
    }
}

/// A suspension point where a continuation may be captured.
#[derive(Debug, Clone)]
pub struct SuspensionPoint {
    /// The basic block containing the suspension.
    pub block: super::types::BasicBlockId,
    /// Locals that need to be captured in the snapshot.
    pub locals_to_capture: Vec<LocalId>,
}

// ============================================================================
// Snapshot Lowering
// ============================================================================

/// Generates MIR statements for snapshot capture and validation.
#[derive(Debug)]
pub struct SnapshotLowering;

impl SnapshotLowering {
    /// Generate statements to capture a snapshot.
    pub fn generate_capture(
        locals: &[LocalId],
        snapshot_local: LocalId,
    ) -> Vec<StatementKind> {
        let mut stmts = Vec::new();

        // For each local, emit a CaptureSnapshot statement
        // The actual capture happens at runtime
        for &local in locals {
            stmts.push(StatementKind::CaptureSnapshot(local));
        }

        // Store the snapshot in the designated local
        let _ = snapshot_local; // The snapshot is built incrementally

        stmts
    }

    /// Generate MIR-level validation statements for captured locals.
    ///
    /// # Important: MIR vs Runtime Validation
    ///
    /// This function generates `ValidateGeneration` statements at the MIR level.
    /// These are **separate from** the runtime snapshot validation that happens
    /// via `blood_snapshot_validate()` at Perform/Resume terminators.
    ///
    /// - **MIR-level validation**: Static checks embedded in the IR, validated
    ///   during code execution. Uses expected generation from local's allocation.
    /// - **Runtime snapshot validation**: Dynamic checks at effect boundaries,
    ///   captures actual generations into a snapshot struct and validates them
    ///   when resuming from an effect.
    ///
    /// # Stack-Only Validation
    ///
    /// This function generates validation for stack-allocated locals where the
    /// expected generation is always 1 (STACK_GENERATION constant). For region-
    /// allocated values, the runtime snapshot validation handles the actual
    /// generation tracking and validation.
    ///
    /// Stack locals don't need runtime snapshot validation because:
    /// - They are never deallocated while in scope
    /// - Their generation is always 1 (immutable)
    /// - Escape analysis ensures they aren't captured across effect boundaries
    pub fn generate_validation(
        locals: &[LocalId],
    ) -> Vec<StatementKind> {
        let mut stmts = Vec::new();

        for &local in locals {
            // Generate a validation check for stack-allocated locals.
            // Generation 1 is the STACK_GENERATION constant - stack allocations
            // always have this generation and it never changes.
            stmts.push(StatementKind::ValidateGeneration {
                ptr: Place::local(local),
                expected_gen: Operand::Constant(super::types::Constant::int(1, Type::u32())),
            });
        }

        stmts
    }
}

// ============================================================================
// Liveness for Snapshots
// ============================================================================

/// From MEMORY_MODEL.md §6.4.1:
/// "A generational reference is live at point P if there exists a path
/// from P to a use of that reference."
#[derive(Debug, Default)]
pub struct LivenessAnalysis {
    /// Locals that are live at each block entry.
    pub live_in: std::collections::HashMap<super::types::BasicBlockId, HashSet<LocalId>>,
    /// Locals that are live at each block exit.
    pub live_out: std::collections::HashMap<super::types::BasicBlockId, HashSet<LocalId>>,
}

impl LivenessAnalysis {
    /// Perform liveness analysis on a MIR body.
    pub fn analyze(body: &MirBody) -> Self {
        let mut analysis = Self::default();

        // Initialize all blocks with empty sets
        for bb_id in body.block_ids() {
            analysis.live_in.insert(bb_id, HashSet::new());
            analysis.live_out.insert(bb_id, HashSet::new());
        }

        // Iterate to fixed point in reverse postorder
        let rpo = body.reverse_postorder();
        let mut changed = true;

        while changed {
            changed = false;

            // Process in reverse order for liveness (backward analysis)
            for &bb_id in rpo.iter().rev() {
                if let Some(block) = body.get_block(bb_id) {
                    // live_out = union of live_in of successors
                    let mut new_live_out = HashSet::new();
                    for succ in block.successors() {
                        if let Some(succ_live_in) = analysis.live_in.get(&succ) {
                            new_live_out.extend(succ_live_in.iter().cloned());
                        }
                    }

                    // live_in = use(block) ∪ (live_out - def(block))
                    let (uses, defs) = Self::compute_use_def(block);
                    let mut new_live_in: HashSet<_> = new_live_out.difference(&defs).cloned().collect();
                    new_live_in.extend(uses);

                    // Check for changes
                    if analysis.live_in.get(&bb_id) != Some(&new_live_in) {
                        analysis.live_in.insert(bb_id, new_live_in);
                        changed = true;
                    }
                    if analysis.live_out.get(&bb_id) != Some(&new_live_out) {
                        analysis.live_out.insert(bb_id, new_live_out);
                        changed = true;
                    }
                }
            }
        }

        analysis
    }

    /// Compute use and def sets for a basic block.
    fn compute_use_def(block: &super::types::BasicBlockData) -> (HashSet<LocalId>, HashSet<LocalId>) {
        let mut uses = HashSet::new();
        let mut defs = HashSet::new();

        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(place, rvalue) => {
                    // Collect uses from rvalue
                    Self::collect_rvalue_uses(rvalue, &mut uses);
                    // Place is a def (only track locals, not statics)
                    if let Some(local) = place.as_local() {
                        defs.insert(local);
                    }
                }
                StatementKind::Drop(place) | StatementKind::IncrementGeneration(place) => {
                    if let Some(local) = place.as_local() {
                        uses.insert(local);
                    }
                }
                StatementKind::ValidateGeneration { ptr, expected_gen } => {
                    if let Some(local) = ptr.as_local() {
                        uses.insert(local);
                    }
                    if let Operand::Copy(p) | Operand::Move(p) = expected_gen {
                        if let Some(local) = p.as_local() {
                            uses.insert(local);
                        }
                    }
                }
                StatementKind::CaptureSnapshot(local) => {
                    uses.insert(*local);
                }
                StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                    let _ = local;
                }
                StatementKind::Nop => {}
                StatementKind::PushHandler { .. } | StatementKind::PushInlineHandler { .. } | StatementKind::PopHandler => {
                    // Effect handler statements don't use or define locals
                }
                StatementKind::CallReturnClause { body_result, state_place, destination, .. } => {
                    // body_result is used, state_place is used, destination is defined
                    Self::collect_operand_uses(body_result, &mut uses);
                    // state_place is used (read)
                    if let Some(local) = state_place.as_local() {
                        uses.insert(local);
                    }
                    // destination is defined (written)
                    if let Some(local) = destination.as_local() {
                        defs.insert(local);
                    }
                }
            }
        }

        // Handle terminator
        if let Some(term) = &block.terminator {
            Self::collect_terminator_uses(&term.kind, &mut uses);
        }

        (uses, defs)
    }

    /// Collect uses from an rvalue.
    fn collect_rvalue_uses(rvalue: &Rvalue, uses: &mut HashSet<LocalId>) {
        match rvalue {
            Rvalue::Use(op) => Self::collect_operand_uses(op, uses),
            Rvalue::Ref { place, .. } | Rvalue::AddressOf { place, .. } => {
                if let Some(local) = place.as_local() {
                    uses.insert(local);
                }
            }
            Rvalue::BinaryOp { left, right, .. } | Rvalue::CheckedBinaryOp { left, right, .. } => {
                Self::collect_operand_uses(left, uses);
                Self::collect_operand_uses(right, uses);
            }
            Rvalue::UnaryOp { operand, .. } | Rvalue::Cast { operand, .. } => {
                Self::collect_operand_uses(operand, uses);
            }
            Rvalue::Aggregate { operands, .. } => {
                for op in operands {
                    Self::collect_operand_uses(op, uses);
                }
            }
            Rvalue::Discriminant(place) | Rvalue::Len(place) | Rvalue::VecLen(place) | Rvalue::ReadGeneration(place) => {
                if let Some(local) = place.as_local() {
                    uses.insert(local);
                }
            }
            Rvalue::NullCheck(op) => {
                Self::collect_operand_uses(op, uses);
            }
            Rvalue::MakeGenPtr { address, generation, metadata } => {
                Self::collect_operand_uses(address, uses);
                Self::collect_operand_uses(generation, uses);
                Self::collect_operand_uses(metadata, uses);
            }
            Rvalue::ZeroInit(_) => {
                // Zero-initialization doesn't use any locals
            }
            Rvalue::StringIndex { base, index } => {
                Self::collect_operand_uses(base, uses);
                Self::collect_operand_uses(index, uses);
            }
            Rvalue::ArrayToSlice { array_ref, .. } => {
                Self::collect_operand_uses(array_ref, uses);
            }
        }
    }

    /// Collect uses from an operand.
    fn collect_operand_uses(op: &Operand, uses: &mut HashSet<LocalId>) {
        if let Operand::Copy(place) | Operand::Move(place) = op {
            if let Some(local) = place.as_local() {
                uses.insert(local);
            }
        }
    }

    /// Collect uses from a terminator.
    fn collect_terminator_uses(kind: &TerminatorKind, uses: &mut HashSet<LocalId>) {
        match kind {
            TerminatorKind::SwitchInt { discr, .. } => {
                Self::collect_operand_uses(discr, uses);
            }
            TerminatorKind::Call { func, args, .. } => {
                Self::collect_operand_uses(func, uses);
                for arg in args {
                    Self::collect_operand_uses(arg, uses);
                }
            }
            TerminatorKind::Assert { cond, .. } => {
                Self::collect_operand_uses(cond, uses);
            }
            TerminatorKind::DropAndReplace { place, value, .. } => {
                if let Some(local) = place.as_local() {
                    uses.insert(local);
                }
                Self::collect_operand_uses(value, uses);
            }
            TerminatorKind::Perform { args, .. } => {
                for arg in args {
                    Self::collect_operand_uses(arg, uses);
                }
            }
            TerminatorKind::Resume { value: Some(op) } => {
                Self::collect_operand_uses(op, uses);
            }
            TerminatorKind::Resume { value: None } => {
                // Resume with no value - no uses to collect
            }
            TerminatorKind::StaleReference { ptr, .. } => {
                if let Some(local) = ptr.as_local() {
                    uses.insert(local);
                }
            }
            // Terminators with no operand uses
            TerminatorKind::Goto { .. } => {
                // Goto has no operands, only a target block
            }
            TerminatorKind::Return => {
                // Return uses the return place, but that's implicit
            }
            TerminatorKind::Unreachable => {
                // Unreachable has no uses
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generation_snapshot_new() {
        let snapshot = GenerationSnapshot::new();
        assert!(snapshot.is_empty());
        assert_eq!(snapshot.len(), 0);
    }

    #[test]
    fn test_generation_snapshot_add_entry() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        assert!(!snapshot.is_empty());
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.entries[0].address, 0x1000);
        assert_eq!(snapshot.entries[0].generation, 42);
    }

    #[test]
    fn test_generation_snapshot_skip_persistent() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, PERSISTENT_MARKER, LocalId::new(1)));

        // Persistent entries should be skipped
        assert!(snapshot.is_empty());
    }

    #[test]
    fn test_generation_snapshot_add_ptr() {
        let mut snapshot = GenerationSnapshot::new();

        let ptr = BloodPtr::new(0x2000, 99, super::super::ptr::PtrMetadata::region());
        snapshot.add_ptr(&ptr, LocalId::new(2));

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.entries[0].generation, 99);

        // Null pointer should be skipped
        let null_ptr = BloodPtr::null();
        snapshot.add_ptr(&null_ptr, LocalId::new(3));
        assert_eq!(snapshot.len(), 1); // Still 1
    }

    #[test]
    fn test_generation_snapshot_validate_success() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));

        let result = snapshot.validate(|addr| match addr {
            0x1000 => Some(42),
            0x2000 => Some(43),
            _ => None,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn test_generation_snapshot_validate_failure() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let result = snapshot.validate(|addr| match addr {
            0x1000 => Some(99), // Wrong generation
            _ => None,
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.entry.generation, 42);
        assert_eq!(err.actual_generation, 99);
    }

    #[test]
    fn test_generation_snapshot_validate_freed() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let result = snapshot.validate(|_| None); // Address not found (freed)

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.actual_generation, 0);
    }

    #[test]
    fn test_generation_snapshot_merge() {
        let mut snapshot1 = GenerationSnapshot::new();
        snapshot1.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let mut snapshot2 = GenerationSnapshot::new();
        snapshot2.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));
        snapshot2.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1))); // Duplicate

        snapshot1.merge(&snapshot2);

        assert_eq!(snapshot1.len(), 2); // Duplicate not added
        assert!(snapshot1.addresses().contains(&0x1000));
        assert!(snapshot1.addresses().contains(&0x2000));
    }

    #[test]
    fn test_snapshot_entry() {
        let entry = SnapshotEntry::new(0xDEAD_BEEF, 123, LocalId::new(5));
        assert_eq!(entry.address, 0xDEAD_BEEF);
        assert_eq!(entry.generation, 123);
        assert_eq!(entry.local.index, 5);
    }

    #[test]
    fn test_snapshot_validation_error_display() {
        let err = SnapshotValidationError {
            entry: SnapshotEntry::new(0x1000, 42, LocalId::new(1)),
            actual_generation: 99,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("42"));
        assert!(msg.contains("99"));
        assert!(msg.contains("0x1000"));
    }

    #[test]
    fn test_snapshot_requirements() {
        let mut reqs = SnapshotRequirements::new();
        assert!(!reqs.needs_snapshots());

        reqs.suspension_points.push(SuspensionPoint {
            block: super::super::types::BasicBlockId::new(1),
            locals_to_capture: vec![LocalId::new(1), LocalId::new(2)],
        });

        assert!(reqs.needs_snapshots());

        let all = reqs.all_captured_locals();
        assert!(all.contains(&LocalId::new(1)));
        assert!(all.contains(&LocalId::new(2)));
    }

    #[test]
    fn test_snapshot_lowering_capture() {
        let locals = vec![LocalId::new(1), LocalId::new(2)];
        let stmts = SnapshotLowering::generate_capture(&locals, LocalId::new(10));

        assert_eq!(stmts.len(), 2);
        assert!(matches!(stmts[0], StatementKind::CaptureSnapshot(l) if l.index == 1));
        assert!(matches!(stmts[1], StatementKind::CaptureSnapshot(l) if l.index == 2));
    }

    #[test]
    fn test_snapshot_lowering_validation() {
        let locals = vec![LocalId::new(1)];
        let stmts = SnapshotLowering::generate_validation(&locals);

        assert_eq!(stmts.len(), 1);
        assert!(matches!(stmts[0], StatementKind::ValidateGeneration { .. }));
    }

    // ========================================================================
    // Lazy Validation Tests
    // ========================================================================

    #[test]
    fn test_lazy_snapshot_new() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));

        let lazy = LazySnapshot::new(snapshot, |_| Some(42));

        assert_eq!(lazy.total_count(), 2);
        assert_eq!(lazy.validated_count(), 0);
        assert!(!lazy.is_fully_validated());
    }

    #[test]
    fn test_lazy_snapshot_validate_entry_success() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));

        let lazy = LazySnapshot::new(snapshot, |addr| match addr {
            0x1000 => Some(42),
            0x2000 => Some(43),
            _ => None,
        });

        // Validate only the first entry
        assert!(lazy.validate_entry(0).is_ok());
        assert_eq!(lazy.validated_count(), 1);

        // Second entry not yet validated
        assert!(!lazy.is_fully_validated());

        // Validate second entry
        assert!(lazy.validate_entry(1).is_ok());
        assert!(lazy.is_fully_validated());
    }

    #[test]
    fn test_lazy_snapshot_validate_entry_failure() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let lazy = LazySnapshot::new(snapshot, |_| Some(99)); // Wrong generation

        let result = lazy.validate_entry(0);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.entry.generation, 42);
        assert_eq!(err.actual_generation, 99);
    }

    #[test]
    fn test_lazy_snapshot_validate_entry_freed() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let lazy = LazySnapshot::new(snapshot, |_| None); // Address not found

        let result = lazy.validate_entry(0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().actual_generation, 0);
    }

    #[test]
    fn test_lazy_snapshot_validate_entry_already_validated() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let call_count = std::cell::Cell::new(0);
        let lazy = LazySnapshot::new(snapshot, |_| {
            call_count.set(call_count.get() + 1);
            Some(42)
        });

        // First validation
        assert!(lazy.validate_entry(0).is_ok());
        assert_eq!(call_count.get(), 1);

        // Second validation should not call get_gen again
        assert!(lazy.validate_entry(0).is_ok());
        assert_eq!(call_count.get(), 1); // Still 1
    }

    #[test]
    fn test_lazy_snapshot_validate_entry_out_of_bounds() {
        let snapshot = GenerationSnapshot::new();
        let lazy = LazySnapshot::new(snapshot, |_| Some(42));

        // Out of bounds should succeed (nothing to validate)
        assert!(lazy.validate_entry(100).is_ok());
    }

    #[test]
    fn test_lazy_snapshot_validate_local() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x1008, 43, LocalId::new(1))); // Same local
        snapshot.add_entry(SnapshotEntry::new(0x2000, 44, LocalId::new(2)));

        let lazy = LazySnapshot::new(snapshot, |addr| match addr {
            0x1000 => Some(42),
            0x1008 => Some(43),
            0x2000 => Some(44),
            _ => None,
        });

        // Validate only local 1 (has 2 entries)
        assert!(lazy.validate_local(LocalId::new(1)).is_ok());
        assert_eq!(lazy.validated_count(), 2);

        // Local 2 not yet validated
        assert!(!lazy.is_fully_validated());
    }

    #[test]
    fn test_lazy_snapshot_validate_all() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));

        let lazy = LazySnapshot::new(snapshot, |addr| match addr {
            0x1000 => Some(42),
            0x2000 => Some(43),
            _ => None,
        });

        assert!(lazy.validate_all().is_ok());
        assert!(lazy.is_fully_validated());
        assert_eq!(lazy.validated_count(), 2);
    }

    #[test]
    fn test_lazy_snapshot_validate_all_with_error() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));

        let lazy = LazySnapshot::new(snapshot, |addr| match addr {
            0x1000 => Some(42),
            0x2000 => Some(99), // Wrong generation
            _ => None,
        });

        let result = lazy.validate_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_lazy_snapshot_cached_error() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));

        let lazy = LazySnapshot::new(snapshot, |addr| match addr {
            0x1000 => Some(99), // Wrong generation
            0x2000 => Some(43),
            _ => None,
        });

        // First entry fails
        assert!(lazy.validate_entry(0).is_err());

        // Even valid entry 1 should return cached error
        let result = lazy.validate_entry(1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().entry.address, 0x1000); // Same error
    }

    #[test]
    fn test_lazy_snapshot_stats() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));
        snapshot.add_entry(SnapshotEntry::new(0x2000, 43, LocalId::new(2)));
        snapshot.add_entry(SnapshotEntry::new(0x3000, 44, LocalId::new(3)));

        let lazy = LazySnapshot::new(snapshot, |addr| match addr {
            0x1000 => Some(42),
            0x2000 => Some(43),
            0x3000 => Some(44),
            _ => None,
        });

        // Validate only one entry
        let _ = lazy.validate_entry(0);

        let stats = lazy.stats();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.validated, 1);
        assert_eq!(stats.skipped, 2);

        // ~66.7% skip rate
        let skip_pct = stats.skip_percentage();
        assert!(skip_pct > 66.0 && skip_pct < 67.0);
    }

    #[test]
    fn test_lazy_validation_stats_empty() {
        let stats = LazyValidationStats {
            total: 0,
            validated: 0,
            skipped: 0,
        };
        assert_eq!(stats.skip_percentage(), 0.0);
    }

    #[test]
    fn test_lazy_snapshot_into_snapshot() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let lazy = LazySnapshot::new(snapshot, |_| Some(42));
        let recovered = lazy.into_snapshot();

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered.entries[0].address, 0x1000);
    }

    #[test]
    fn test_lazy_snapshot_snapshot_ref() {
        let mut snapshot = GenerationSnapshot::new();
        snapshot.add_entry(SnapshotEntry::new(0x1000, 42, LocalId::new(1)));

        let lazy = LazySnapshot::new(snapshot, |_| Some(42));

        assert_eq!(lazy.snapshot().len(), 1);
    }

    // ========================================================================
    // Snapshot ID Tests
    // ========================================================================

    #[test]
    fn test_snapshot_id_new() {
        let id = SnapshotId::new(42);
        assert_eq!(id.index(), 42);
    }

    #[test]
    fn test_snapshot_id_display() {
        let id = SnapshotId::new(5);
        assert_eq!(format!("{}", id), "snapshot_5");
    }

    #[test]
    fn test_snapshot_id_equality() {
        let id1 = SnapshotId::new(10);
        let id2 = SnapshotId::new(10);
        let id3 = SnapshotId::new(20);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    // ========================================================================
    // Snapshot Store Tests
    // ========================================================================

    #[test]
    fn test_snapshot_store_new() {
        let store = SnapshotStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_snapshot_store_create_root() {
        let mut store = SnapshotStore::new();
        let root_id = store.create_root();

        assert_eq!(store.len(), 1);
        assert_eq!(root_id.index(), 0);

        let root = store.get(root_id).unwrap();
        assert!(root.is_root());
        assert!(!root.has_parent());
        assert_eq!(root.snapshot_id(), Some(root_id));
    }

    #[test]
    fn test_snapshot_store_create_child() {
        let mut store = SnapshotStore::new();
        let root_id = store.create_root();
        let child_id = store.create_child(root_id);

        assert_eq!(store.len(), 2);
        assert_eq!(child_id.index(), 1);

        let child = store.get(child_id).unwrap();
        assert!(!child.is_root());
        assert!(child.has_parent());
        assert_eq!(child.parent_id(), Some(root_id));
    }

    #[test]
    fn test_snapshot_store_chain_depth() {
        let mut store = SnapshotStore::new();
        let root_id = store.create_root();
        let child1_id = store.create_child(root_id);
        let child2_id = store.create_child(child1_id);

        assert_eq!(store.chain_depth(root_id), 1);
        assert_eq!(store.chain_depth(child1_id), 2);
        assert_eq!(store.chain_depth(child2_id), 3);
    }

    #[test]
    fn test_snapshot_store_chain_to_root() {
        let mut store = SnapshotStore::new();
        let root_id = store.create_root();
        let child1_id = store.create_child(root_id);
        let child2_id = store.create_child(child1_id);

        let chain = store.chain_to_root(child2_id);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0], root_id);
        assert_eq!(chain[1], child1_id);
        assert_eq!(chain[2], child2_id);
    }

    #[test]
    fn test_snapshot_store_validate_chain_success() {
        let mut store = SnapshotStore::new();

        // Create root with entry
        let root_id = store.create_root();
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x1000, 42, LocalId::new(1))
        );

        // Create child with additional entry
        let child_id = store.create_child(root_id);
        store.get_mut(child_id).unwrap().add_entry(
            SnapshotEntry::new(0x2000, 43, LocalId::new(2))
        );

        // Validate entire chain
        let result = store.validate_chain(child_id, |addr| match addr {
            0x1000 => Some(42),
            0x2000 => Some(43),
            _ => None,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn test_snapshot_store_validate_chain_failure_in_parent() {
        let mut store = SnapshotStore::new();

        // Create root with entry that will fail
        let root_id = store.create_root();
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x1000, 42, LocalId::new(1))
        );

        // Create child with valid entry
        let child_id = store.create_child(root_id);
        store.get_mut(child_id).unwrap().add_entry(
            SnapshotEntry::new(0x2000, 43, LocalId::new(2))
        );

        // Validation should fail on parent's entry
        let result = store.validate_chain(child_id, |addr| match addr {
            0x1000 => Some(99), // Wrong generation
            0x2000 => Some(43),
            _ => None,
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.entry.address, 0x1000);
    }

    #[test]
    fn test_snapshot_store_flatten_chain() {
        let mut store = SnapshotStore::new();

        // Create root with entries
        let root_id = store.create_root();
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x1000, 42, LocalId::new(1))
        );
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x2000, 43, LocalId::new(2))
        );

        // Create child that overrides one entry and adds new one
        let child_id = store.create_child(root_id);
        store.get_mut(child_id).unwrap().add_entry(
            SnapshotEntry::new(0x2000, 99, LocalId::new(2)) // Override
        );
        store.get_mut(child_id).unwrap().add_entry(
            SnapshotEntry::new(0x3000, 44, LocalId::new(3)) // New
        );

        let flattened = store.flatten_chain(child_id);
        assert_eq!(flattened.len(), 3);

        // Check that child's override took effect
        let entry_2000 = flattened.iter().find(|e| e.address == 0x2000).unwrap();
        assert_eq!(entry_2000.generation, 99); // Child's value
    }

    #[test]
    fn test_snapshot_store_total_entries() {
        let mut store = SnapshotStore::new();

        let root_id = store.create_root();
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x1000, 42, LocalId::new(1))
        );
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x2000, 43, LocalId::new(2))
        );

        let child_id = store.create_child(root_id);
        store.get_mut(child_id).unwrap().add_entry(
            SnapshotEntry::new(0x3000, 44, LocalId::new(3))
        );

        assert_eq!(store.total_entries(), 3);
    }

    #[test]
    fn test_snapshot_total_entries_with_store() {
        let mut store = SnapshotStore::new();

        let root_id = store.create_root();
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x1000, 42, LocalId::new(1))
        );
        store.get_mut(root_id).unwrap().add_entry(
            SnapshotEntry::new(0x2000, 43, LocalId::new(2))
        );

        let child_id = store.create_child(root_id);
        store.get_mut(child_id).unwrap().add_entry(
            SnapshotEntry::new(0x3000, 44, LocalId::new(3))
        );

        // Root has 2 entries
        let root = store.get(root_id).unwrap();
        assert_eq!(root.total_entries(&store), 2);

        // Child has 1 + 2 from parent = 3 total
        let child = store.get(child_id).unwrap();
        assert_eq!(child.total_entries(&store), 3);
    }

    #[test]
    fn test_snapshot_with_id() {
        let id = SnapshotId::new(42);
        let snapshot = GenerationSnapshot::with_id(id);

        assert_eq!(snapshot.snapshot_id(), Some(id));
        assert!(snapshot.is_root());
        assert!(snapshot.is_empty());
    }

    #[test]
    fn test_snapshot_with_parent() {
        let id = SnapshotId::new(2);
        let parent_id = SnapshotId::new(1);
        let snapshot = GenerationSnapshot::with_parent(id, parent_id);

        assert_eq!(snapshot.snapshot_id(), Some(id));
        assert_eq!(snapshot.parent_id(), Some(parent_id));
        assert!(snapshot.has_parent());
        assert!(!snapshot.is_root());
    }

    #[test]
    fn test_snapshot_set_id_and_parent() {
        let mut snapshot = GenerationSnapshot::new();

        assert!(snapshot.snapshot_id().is_none());
        assert!(snapshot.parent_id().is_none());

        snapshot.set_id(SnapshotId::new(5));
        snapshot.set_parent(SnapshotId::new(4));

        assert_eq!(snapshot.snapshot_id(), Some(SnapshotId::new(5)));
        assert_eq!(snapshot.parent_id(), Some(SnapshotId::new(4)));
    }

    #[test]
    fn test_deeply_nested_chain() {
        let mut store = SnapshotStore::new();

        // Create a chain of 10 nested snapshots
        let mut current_id = store.create_root();
        store.get_mut(current_id).unwrap().add_entry(
            SnapshotEntry::new(0x1000, 1, LocalId::new(1))
        );

        for i in 1..10 {
            let child_id = store.create_child(current_id);
            store.get_mut(child_id).unwrap().add_entry(
                SnapshotEntry::new(0x1000 + (i * 0x100) as u64, (i + 1) as u32, LocalId::new(1))
            );
            current_id = child_id;
        }

        // Verify depth
        assert_eq!(store.chain_depth(current_id), 10);

        // Verify we can validate the entire chain
        let result = store.validate_chain(current_id, |addr| {
            // All generations match based on address
            let gen = ((addr - 0x1000) / 0x100 + 1) as u32;
            Some(gen)
        });

        assert!(result.is_ok());

        // Verify flattening works
        let flattened = store.flatten_chain(current_id);
        assert_eq!(flattened.len(), 10);
    }
}
