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
// Generation Snapshot
// ============================================================================

/// A snapshot of generation values at continuation capture time.
///
/// From MEMORY_MODEL.md §6:
/// "Generation snapshots record the expected generations at capture time.
/// On resume, each entry is validated."
#[derive(Debug, Clone, Default)]
pub struct GenerationSnapshot {
    /// Entries mapping addresses to expected generations.
    pub entries: Vec<SnapshotEntry>,
}

impl GenerationSnapshot {
    /// Create an empty snapshot.
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Create with capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
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
                    // Place is a def
                    defs.insert(place.local);
                }
                StatementKind::Drop(place) | StatementKind::IncrementGeneration(place) => {
                    uses.insert(place.local);
                }
                StatementKind::ValidateGeneration { ptr, expected_gen } => {
                    uses.insert(ptr.local);
                    if let Operand::Copy(p) | Operand::Move(p) = expected_gen {
                        uses.insert(p.local);
                    }
                }
                StatementKind::CaptureSnapshot(local) => {
                    uses.insert(*local);
                }
                StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                    let _ = local;
                }
                StatementKind::Nop => {}
                StatementKind::PushHandler { .. } | StatementKind::PopHandler => {
                    // Effect handler statements don't use or define locals
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
                uses.insert(place.local);
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
            Rvalue::Discriminant(place) | Rvalue::Len(place) | Rvalue::ReadGeneration(place) => {
                uses.insert(place.local);
            }
            Rvalue::NullCheck(op) => {
                Self::collect_operand_uses(op, uses);
            }
            Rvalue::MakeGenPtr { address, generation, metadata } => {
                Self::collect_operand_uses(address, uses);
                Self::collect_operand_uses(generation, uses);
                Self::collect_operand_uses(metadata, uses);
            }
        }
    }

    /// Collect uses from an operand.
    fn collect_operand_uses(op: &Operand, uses: &mut HashSet<LocalId>) {
        if let Operand::Copy(place) | Operand::Move(place) = op {
            uses.insert(place.local);
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
                uses.insert(place.local);
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
                uses.insert(ptr.local);
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
}
