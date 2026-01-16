//! # Escape Analysis
//!
//! This module implements escape analysis for Blood's memory model.
//!
//! ## Purpose
//!
//! Escape analysis determines whether values can be allocated on the stack
//! (Tier 0) or must be heap-allocated (Tier 1/2). Values that "escape" their
//! defining scope require heap allocation with generational references.
//!
//! ## Design References
//!
//! - [MEMORY_MODEL.md §5](../../MEMORY_MODEL.md): Escape Analysis
//! - [Java Escape Analysis](https://dl.acm.org/doi/10.1145/320384.320386) (OOPSLA 1999)
//! - [ROADMAP.md §7.3](../../ROADMAP.md): Escape Analysis Algorithm
//!
//! ## Escape States and Memory Tiers
//!
//! | State | Description | Memory Tier | Allocation | Generation Checks |
//! |-------|-------------|-------------|------------|-------------------|
//! | NoEscape | Value doesn't escape function | Stack (Tier 0) | `alloca` | NO |
//! | ArgEscape | Escapes via argument/return | Region (Tier 1) | `blood_alloc_or_abort` | YES |
//! | GlobalEscape | Escapes to global/heap | Region (Tier 1) | `blood_alloc_or_abort` | YES |
//! | Effect-captured | Captured by effect operation | Region (Tier 1) | `blood_alloc_or_abort` | YES |
//!
//! ## Generation Check Details
//!
//! **Stack (Tier 0) - No Generation Checks:**
//! - Values that don't escape their defining function are stack-allocated
//! - Use thin pointers (no metadata overhead)
//! - Fastest path with zero runtime overhead
//! - Safe because stack lifetime is lexically scoped
//!
//! **Region/Persistent (Tier 1/2) - With Generation Checks:**
//! - Values that escape are allocated via `blood_alloc_or_abort`
//! - Each allocation is registered in the global slot registry with a generation
//! - On dereference, `blood_validate_generation(address, expected_gen)` is called
//! - Returns 0 (valid) if generation matches, 1 (stale) if use-after-free detected
//! - Stale references trigger `blood_stale_reference_panic` and abort
//!
//! **Effect-Captured Values:**
//! - Even if escape analysis says NoEscape, if captured by an effect operation
//!   (e.g., passed to a handler), the value is promoted to Region tier
//! - This ensures safety across effect boundaries where control flow is non-local
//!
//! ## Algorithm
//!
//! The analysis uses a lattice-based dataflow algorithm:
//!
//! ```text
//! NoEscape < ArgEscape < GlobalEscape
//! ```
//!
//! Iteration continues until a fixed point is reached.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::hir::{DefId, LocalId};
use super::body::MirBody;
use super::ptr::MemoryTier;
use super::types::{
    AggregateKind, Operand, Place, PlaceElem, Rvalue, StatementKind, TerminatorKind,
};

// ============================================================================
// Escape State
// ============================================================================

/// The escape state of a value.
///
/// Forms a lattice: NoEscape < ArgEscape < GlobalEscape
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum EscapeState {
    /// Value does not escape its defining function.
    /// Can be stack-allocated (Tier 0).
    #[default]
    NoEscape,

    /// Value escapes via function argument but not globally.
    /// May be stack-allocated if callee is inlined, otherwise Tier 1.
    ArgEscape,

    /// Value escapes to a global or heap location.
    /// Must be heap-allocated (Tier 1 or Tier 2).
    GlobalEscape,
}

impl EscapeState {
    /// Join two escape states (least upper bound in lattice).
    pub fn join(self, other: EscapeState) -> EscapeState {
        std::cmp::max(self, other)
    }

    /// Check if this state allows stack allocation.
    pub fn can_stack_allocate(self) -> bool {
        matches!(self, EscapeState::NoEscape)
    }

    /// Get the recommended memory tier for this escape state.
    pub fn recommended_tier(self) -> MemoryTier {
        match self {
            EscapeState::NoEscape => MemoryTier::Stack,
            EscapeState::ArgEscape => MemoryTier::Region,
            EscapeState::GlobalEscape => MemoryTier::Region, // or Persistent if long-lived
        }
    }
}


// ============================================================================
// Escape Results
// ============================================================================

/// Results of escape analysis for a function.
#[derive(Debug, Clone)]
pub struct EscapeResults {
    /// Escape state for each local.
    pub states: HashMap<LocalId, EscapeState>,
    /// Which locals are involved in effect operations.
    pub effect_captured: HashSet<LocalId>,
    /// Allocations that can be promoted to stack.
    pub stack_promotable: HashSet<LocalId>,
    /// Closures and their captured locals.
    /// Maps closure local → list of captured locals.
    pub closure_captures: HashMap<LocalId, Vec<LocalId>>,
    /// Locals that are captured by closures.
    pub captured_by_closure: HashSet<LocalId>,
}

impl EscapeResults {
    /// Create new empty results.
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            effect_captured: HashSet::new(),
            stack_promotable: HashSet::new(),
            closure_captures: HashMap::new(),
            captured_by_closure: HashSet::new(),
        }
    }

    /// Get the escape state for a local.
    pub fn get(&self, local: LocalId) -> EscapeState {
        self.states.get(&local).copied().unwrap_or(EscapeState::NoEscape)
    }

    /// Check if a local can be stack-allocated.
    pub fn can_stack_allocate(&self, local: LocalId) -> bool {
        self.stack_promotable.contains(&local)
    }

    /// Check if a local is captured by an effect operation.
    pub fn is_effect_captured(&self, local: LocalId) -> bool {
        self.effect_captured.contains(&local)
    }

    /// Check if a local is captured by a closure.
    pub fn is_closure_captured(&self, local: LocalId) -> bool {
        self.captured_by_closure.contains(&local)
    }

    /// Get the closures that capture a specific local.
    pub fn capturing_closures(&self, local: LocalId) -> Vec<LocalId> {
        self.closure_captures.iter()
            .filter(|(_, captures)| captures.contains(&local))
            .map(|(closure, _)| *closure)
            .collect()
    }

    /// Get the captures for a specific closure.
    pub fn get_captures(&self, closure: LocalId) -> Option<&Vec<LocalId>> {
        self.closure_captures.get(&closure)
    }

    /// Get recommended memory tier for a local.
    pub fn recommended_tier(&self, local: LocalId) -> MemoryTier {
        // Stack-promotable locals can always use stack allocation.
        // This includes:
        // 1. Locals with NoEscape state that aren't effect/closure-captured
        // 2. Primitive types (values are copied, storage doesn't need to escape)
        if self.stack_promotable.contains(&local) {
            return MemoryTier::Stack;
        }

        // Effect-captured values need region allocation for snapshot
        if self.is_effect_captured(local) {
            return MemoryTier::Region;
        }

        // Closure-captured values need region allocation if the closure escapes
        if self.is_closure_captured(local) {
            // Check if any capturing closure escapes
            for closure in self.capturing_closures(local) {
                if self.get(closure) != EscapeState::NoEscape {
                    return MemoryTier::Region;
                }
            }
        }

        self.get(local).recommended_tier()
    }
}

impl Default for EscapeResults {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Escape Statistics (PERF-V-002)
// ============================================================================

/// Comprehensive statistics for escape analysis.
///
/// Used to validate the claim: ">95% stack allocation" (PERF-V-002)
#[derive(Debug, Clone, Default)]
pub struct EscapeStatistics {
    /// Total number of locals analyzed.
    pub total_locals: usize,
    /// Number of locals that can be stack-allocated.
    pub stack_promotable: usize,
    /// Number of locals that require heap allocation (Region tier).
    pub heap_required: usize,
    /// Number of locals captured by effect operations.
    pub effect_captured: usize,
    /// Number of locals captured by closures.
    pub closure_captured: usize,
    /// Breakdown by escape state.
    pub by_state: EscapeStateBreakdown,
    /// Number of functions analyzed.
    pub functions_analyzed: usize,
}

/// Breakdown of locals by escape state.
#[derive(Debug, Clone, Default)]
pub struct EscapeStateBreakdown {
    /// Locals with NoEscape state.
    pub no_escape: usize,
    /// Locals with ArgEscape state.
    pub arg_escape: usize,
    /// Locals with GlobalEscape state.
    pub global_escape: usize,
}

impl EscapeStatistics {
    /// Create new empty statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute statistics from escape analysis results.
    pub fn from_results(results: &EscapeResults) -> Self {
        let mut stats = Self::new();
        stats.add_results(results);
        stats
    }

    /// Add results from one function to the aggregate statistics.
    pub fn add_results(&mut self, results: &EscapeResults) {
        self.functions_analyzed += 1;
        self.total_locals += results.states.len();
        self.stack_promotable += results.stack_promotable.len();
        self.effect_captured += results.effect_captured.len();
        self.closure_captured += results.captured_by_closure.len();

        // Count by escape state
        for state in results.states.values() {
            match state {
                EscapeState::NoEscape => self.by_state.no_escape += 1,
                EscapeState::ArgEscape => self.by_state.arg_escape += 1,
                EscapeState::GlobalEscape => self.by_state.global_escape += 1,
            }
        }

        // Heap required = total - stack_promotable
        self.heap_required = self.total_locals.saturating_sub(self.stack_promotable);
    }

    /// Merge another statistics object into this one.
    pub fn merge(&mut self, other: &EscapeStatistics) {
        self.total_locals += other.total_locals;
        self.stack_promotable += other.stack_promotable;
        self.heap_required += other.heap_required;
        self.effect_captured += other.effect_captured;
        self.closure_captured += other.closure_captured;
        self.by_state.no_escape += other.by_state.no_escape;
        self.by_state.arg_escape += other.by_state.arg_escape;
        self.by_state.global_escape += other.by_state.global_escape;
        self.functions_analyzed += other.functions_analyzed;
    }

    /// Calculate the stack allocation percentage.
    ///
    /// This is the key metric for PERF-V-002: ">95% stack allocation"
    pub fn stack_percentage(&self) -> f64 {
        if self.total_locals == 0 {
            return 100.0; // No locals = 100% stack (trivially)
        }
        (self.stack_promotable as f64 / self.total_locals as f64) * 100.0
    }

    /// Calculate the heap allocation percentage.
    pub fn heap_percentage(&self) -> f64 {
        100.0 - self.stack_percentage()
    }

    /// Check if the ">95% stack allocation" claim holds.
    pub fn meets_95_percent_target(&self) -> bool {
        self.stack_percentage() >= 95.0
    }

    /// Format a detailed statistics report.
    pub fn format_report(&self) -> String {
        let mut report = String::new();

        report.push_str("╔══════════════════════════════════════════════════════════════════╗\n");
        report.push_str("║           ESCAPE ANALYSIS STATISTICS (PERF-V-002)                ║\n");
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");
        report.push_str(&format!(
            "║  Functions analyzed:           {:>6}                            ║\n",
            self.functions_analyzed
        ));
        report.push_str(&format!(
            "║  Total locals:                 {:>6}                            ║\n",
            self.total_locals
        ));
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");
        report.push_str("║  ALLOCATION TIER BREAKDOWN                                       ║\n");
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");
        report.push_str(&format!(
            "║  Stack-promotable (Tier 0):    {:>6} ({:>5.1}%)                   ║\n",
            self.stack_promotable,
            self.stack_percentage()
        ));
        report.push_str(&format!(
            "║  Heap-required (Tier 1/2):     {:>6} ({:>5.1}%)                   ║\n",
            self.heap_required,
            self.heap_percentage()
        ));
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");
        report.push_str("║  ESCAPE STATE BREAKDOWN                                          ║\n");
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");

        let no_escape_pct = if self.total_locals > 0 {
            (self.by_state.no_escape as f64 / self.total_locals as f64) * 100.0
        } else {
            0.0
        };
        let arg_escape_pct = if self.total_locals > 0 {
            (self.by_state.arg_escape as f64 / self.total_locals as f64) * 100.0
        } else {
            0.0
        };
        let global_escape_pct = if self.total_locals > 0 {
            (self.by_state.global_escape as f64 / self.total_locals as f64) * 100.0
        } else {
            0.0
        };

        report.push_str(&format!(
            "║  NoEscape:                     {:>6} ({:>5.1}%)                   ║\n",
            self.by_state.no_escape, no_escape_pct
        ));
        report.push_str(&format!(
            "║  ArgEscape:                    {:>6} ({:>5.1}%)                   ║\n",
            self.by_state.arg_escape, arg_escape_pct
        ));
        report.push_str(&format!(
            "║  GlobalEscape:                 {:>6} ({:>5.1}%)                   ║\n",
            self.by_state.global_escape, global_escape_pct
        ));
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");
        report.push_str("║  SPECIAL CAPTURES                                                ║\n");
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");
        report.push_str(&format!(
            "║  Effect-captured:              {:>6}                            ║\n",
            self.effect_captured
        ));
        report.push_str(&format!(
            "║  Closure-captured:             {:>6}                            ║\n",
            self.closure_captured
        ));
        report.push_str("╠══════════════════════════════════════════════════════════════════╣\n");

        let target_met = self.meets_95_percent_target();
        let status = if target_met { "✓ PASS" } else { "✗ FAIL" };
        let status_line = format!(
            "║  TARGET (>95% stack):          {:>5.1}%  {}                      ║\n",
            self.stack_percentage(),
            status
        );
        report.push_str(&status_line);
        report.push_str("╚══════════════════════════════════════════════════════════════════╝\n");

        report
    }

    /// Format a compact single-line summary.
    pub fn format_summary(&self) -> String {
        format!(
            "{} functions, {} locals: {:.1}% stack-promotable ({} stack, {} heap)",
            self.functions_analyzed,
            self.total_locals,
            self.stack_percentage(),
            self.stack_promotable,
            self.heap_required
        )
    }
}

// ============================================================================
// Escape Analyzer
// ============================================================================

/// Escape analysis pass for MIR.
///
/// Based on the algorithm in ROADMAP.md §7.3:
/// ```text
/// fn analyze(&mut self, func: &MirFunction) -> EscapeResults {
///     let mut states = HashMap::new();
///     for alloc in func.allocations() {
///         states.insert(alloc.id, EscapeState::NoEscape);
///     }
///     // Iterate to fixed point
///     loop {
///         let mut changed = false;
///         for block in func.blocks() {
///             for stmt in block.statements() {
///                 changed |= self.analyze_statement(stmt, &mut states);
///             }
///         }
///         if !changed { break; }
///     }
///     states
/// }
/// ```
#[derive(Debug)]
pub struct EscapeAnalyzer {
    /// Current escape states.
    states: HashMap<LocalId, EscapeState>,
    /// Locals captured by effect operations.
    effect_captured: HashSet<LocalId>,
    /// Global definitions (statics, etc.).
    globals: HashSet<DefId>,
    /// Maps closure local → captured locals.
    closure_captures: HashMap<LocalId, Vec<LocalId>>,
    /// All locals captured by any closure.
    captured_by_closure: HashSet<LocalId>,
}

impl EscapeAnalyzer {
    /// Create a new escape analyzer.
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            effect_captured: HashSet::new(),
            globals: HashSet::new(),
            closure_captures: HashMap::new(),
            captured_by_closure: HashSet::new(),
        }
    }

    /// Add a known global definition.
    pub fn add_global(&mut self, def_id: DefId) {
        self.globals.insert(def_id);
    }

    /// Analyze a MIR body and return escape results.
    ///
    /// This is a convenience method that uses primitive-only Copy detection.
    /// For full Copy detection including structs, use `analyze_with_adt_lookup`.
    pub fn analyze(&mut self, body: &MirBody) -> EscapeResults {
        self.analyze_with_adt_lookup(body, &|_| None)
    }

    /// Analyze a MIR body with ADT field lookup for Copy detection.
    ///
    /// The `adt_fields` callback is used to look up field types for ADTs.
    /// It takes a DefId and returns the field types if the ADT is a struct.
    /// This enables full Copy detection for struct types.
    pub fn analyze_with_adt_lookup<F>(&mut self, body: &MirBody, adt_fields: &F) -> EscapeResults
    where
        F: Fn(DefId) -> Option<Vec<crate::hir::Type>>,
    {
        self.states.clear();
        self.effect_captured.clear();
        self.closure_captures.clear();
        self.captured_by_closure.clear();

        // Initialize all locals to NoEscape
        for local in &body.locals {
            self.states.insert(local.id, EscapeState::NoEscape);
        }

        // Return place always escapes (returned to caller)
        self.states.insert(body.return_place(), EscapeState::ArgEscape);

        // Parameters start as NoEscape and only escape if they're:
        // - Stored into a global or field of something that escapes
        // - Returned from the function
        // - Passed to a function that might escape them
        //
        // This allows reference parameters that are only read locally to remain
        // stack-allocated with no generation checks, which is critical for performance.
        // The dataflow analysis below will promote their escape state if needed.

        // First pass: collect closure captures
        for (_bb_id, block) in body.blocks() {
            for stmt in &block.statements {
                if let StatementKind::Assign(place, rvalue) = &stmt.kind {
                    self.collect_closure_captures(place, rvalue);
                }
            }
        }

        // Iterate to fixed point using a worklist algorithm for closure propagation.
        //
        // The naive approach of iterating over ALL closures on every iteration is O(n²)
        // for closure chains. The worklist approach is O(n) amortized.
        //
        // Algorithm:
        // 1. Run statement/terminator analysis to propagate escape states
        // 2. Use a worklist to propagate escapes through closure capture chains
        // 3. Repeat until stable
        loop {
            let mut changed = false;

            // Phase 1: Statement and terminator analysis
            for (_bb_id, block) in body.blocks() {
                for stmt in &block.statements {
                    changed |= self.analyze_statement(&stmt.kind);
                }

                if let Some(term) = &block.terminator {
                    changed |= self.analyze_terminator(&term.kind);
                }
            }

            // Phase 2: Worklist-based closure propagation
            //
            // Instead of iterating over ALL closures every time, we:
            // 1. Initialize worklist with closures that currently escape
            // 2. Process each closure, propagating escape to its captures
            // 3. If a captured local is itself a closure and its state changed, add to worklist
            //
            // This is O(n) because each closure is processed at most once per outer iteration,
            // and the outer loop runs at most 3 times (bounded by escape state levels).
            let mut worklist: VecDeque<LocalId> = self.closure_captures.keys()
                .filter(|c| {
                    self.states.get(c).copied().unwrap_or(EscapeState::NoEscape)
                        != EscapeState::NoEscape
                })
                .copied()
                .collect();
            let mut processed: HashSet<LocalId> = HashSet::new();

            while let Some(closure) = worklist.pop_front() {
                // Skip if already processed in this propagation phase
                if processed.contains(&closure) {
                    continue;
                }
                processed.insert(closure);

                let closure_state = self.states.get(&closure).copied()
                    .unwrap_or(EscapeState::NoEscape);

                // Only propagate if closure escapes
                if closure_state == EscapeState::NoEscape {
                    continue;
                }

                // Get captures (clone to avoid borrow conflict)
                let captures = match self.closure_captures.get(&closure) {
                    Some(caps) => caps.clone(),
                    None => continue,
                };

                // Propagate escape state to all captured locals
                for captured in captures {
                    if self.update_state(captured, closure_state) {
                        changed = true;

                        // If the captured local is itself a closure and hasn't been
                        // processed yet, add it to the worklist
                        if self.closure_captures.contains_key(&captured)
                            && !processed.contains(&captured)
                        {
                            worklist.push_back(captured);
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }

        // Determine stack-promotable allocations
        // Build a map from LocalId to type for efficient lookup
        let local_types: HashMap<LocalId, &crate::hir::Type> = body.locals.iter()
            .map(|l| (l.id, &l.ty))
            .collect();

        let mut stack_promotable = HashSet::new();
        for (local, state) in &self.states {
            let ty = local_types.get(local);

            // Can stack-allocate if:
            // 1. Type is Copy (values are copied, not referenced)
            //    - For Copy types, escape state only tracks value flow, not storage
            //    - Storage can always be on the stack since values are copied on use
            //    - This includes primitives, unit, and structs with only Copy fields
            // 2. OR NoEscape state AND not captured by effect operations or closures
            let is_copy_type = ty.map(|t| t.is_copy(adt_fields)).unwrap_or(false);
            let escape_allows = state.can_stack_allocate()
                && !self.effect_captured.contains(local)
                && !self.is_captured_by_escaping_closure(*local);

            let can_stack = is_copy_type || escape_allows;
            if can_stack {
                stack_promotable.insert(*local);
            }
        }

        EscapeResults {
            states: self.states.clone(),
            effect_captured: self.effect_captured.clone(),
            stack_promotable,
            closure_captures: self.closure_captures.clone(),
            captured_by_closure: self.captured_by_closure.clone(),
        }
    }

    /// Check if a local is captured by a closure that escapes.
    fn is_captured_by_escaping_closure(&self, local: LocalId) -> bool {
        if !self.captured_by_closure.contains(&local) {
            return false;
        }

        for (closure_local, captures) in &self.closure_captures {
            if captures.contains(&local) {
                let closure_state = self.states.get(closure_local).copied()
                    .unwrap_or(EscapeState::NoEscape);
                if closure_state != EscapeState::NoEscape {
                    return true;
                }
            }
        }

        false
    }

    /// Collect closure capture information from an assignment.
    fn collect_closure_captures(&mut self, place: &Place, rvalue: &Rvalue) {
        if let Rvalue::Aggregate { kind: AggregateKind::Closure { .. }, operands } = rvalue {
            let closure_local = place.local;
            let mut captures = Vec::with_capacity(operands.len());

            for operand in operands {
                if let Operand::Copy(p) | Operand::Move(p) = operand {
                    captures.push(p.local);
                    self.captured_by_closure.insert(p.local);
                }
            }

            self.closure_captures.insert(closure_local, captures);
        }
    }

    /// Analyze a statement, returning true if state changed.
    fn analyze_statement(&mut self, kind: &StatementKind) -> bool {
        match kind {
            StatementKind::Assign(place, rvalue) => {
                self.analyze_assignment(place, rvalue)
            }
            StatementKind::CaptureSnapshot(local) => {
                // Effect snapshots capture references
                self.effect_captured.insert(*local);
                false
            }
            StatementKind::Drop(place) | StatementKind::IncrementGeneration(place) => {
                // These don't affect escape state
                let _ = place;
                false
            }
            StatementKind::ValidateGeneration { ptr, .. } => {
                let _ = ptr;
                false
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop => {
                false
            }
            StatementKind::PushHandler { .. } | StatementKind::PushInlineHandler { .. } | StatementKind::PopHandler | StatementKind::CallReturnClause { .. } => {
                // Effect handler statements don't affect escape state
                false
            }
        }
    }

    /// Analyze an assignment.
    fn analyze_assignment(&mut self, place: &Place, rvalue: &Rvalue) -> bool {
        let mut changed = false;

        // If we're storing to a place that escapes, the value escapes too
        let target_state = self.place_escape_state(place);

        match rvalue {
            Rvalue::Use(operand) => {
                changed |= self.propagate_to_operand(operand, target_state);
            }
            Rvalue::Ref { place: ref_place, .. } | Rvalue::AddressOf { place: ref_place, .. } => {
                // Creating a reference: if the reference escapes, the referent escapes
                changed |= self.update_state(ref_place.local, target_state);
            }
            Rvalue::BinaryOp { left, right, .. } | Rvalue::CheckedBinaryOp { left, right, .. } => {
                changed |= self.propagate_to_operand(left, target_state);
                changed |= self.propagate_to_operand(right, target_state);
            }
            Rvalue::UnaryOp { operand, .. } | Rvalue::Cast { operand, .. } => {
                changed |= self.propagate_to_operand(operand, target_state);
            }
            Rvalue::Aggregate { operands, .. } => {
                for op in operands {
                    changed |= self.propagate_to_operand(op, target_state);
                }
            }
            Rvalue::Discriminant(p) | Rvalue::Len(p) | Rvalue::ReadGeneration(p) => {
                // Reading properties doesn't cause escape
                let _ = p;
            }
            Rvalue::NullCheck(op) => {
                let _ = op;
            }
            Rvalue::MakeGenPtr { address, generation, metadata } => {
                // Creating a pointer might cause escape
                changed |= self.propagate_to_operand(address, target_state);
                let _ = generation;
                let _ = metadata;
            }
            Rvalue::ZeroInit(_) => {
                // Zero-initialization doesn't reference any locals
            }
        }

        changed
    }

    /// Analyze a terminator.
    fn analyze_terminator(&mut self, kind: &TerminatorKind) -> bool {
        let mut changed = false;

        match kind {
            TerminatorKind::Call { args, .. } => {
                // Arguments passed to functions may escape
                for arg in args {
                    changed |= self.propagate_to_operand(arg, EscapeState::ArgEscape);
                }
            }
            TerminatorKind::Return => {
                // Return value escapes to caller (already handled in initialization)
            }
            TerminatorKind::Perform { args, .. } => {
                // Effect operations may capture values
                for arg in args {
                    if let Some(place) = arg.place() {
                        self.effect_captured.insert(place.local);
                        changed |= self.update_state(place.local, EscapeState::ArgEscape);
                    }
                }
            }
            TerminatorKind::DropAndReplace { value, .. } => {
                changed |= self.propagate_to_operand(value, EscapeState::NoEscape);
            }
            TerminatorKind::Assert { cond, .. } => {
                let _ = cond;
            }
            TerminatorKind::SwitchInt { discr, .. } => {
                let _ = discr;
            }
            TerminatorKind::Goto { .. }
            | TerminatorKind::Unreachable
            | TerminatorKind::Resume { .. }
            | TerminatorKind::StaleReference { .. } => {}
        }

        changed
    }

    /// Get the escape state of a place.
    fn place_escape_state(&self, place: &Place) -> EscapeState {
        let base_state = self.states.get(&place.local).copied().unwrap_or(EscapeState::NoEscape);

        // If we're dereferencing, the target might have different escape properties
        for elem in &place.projection {
            if matches!(elem, PlaceElem::Deref) {
                // Dereferencing a pointer: the pointee's escape is determined by the pointer
                return EscapeState::GlobalEscape;
            }
        }

        base_state
    }

    /// Propagate escape state to an operand.
    fn propagate_to_operand(&mut self, operand: &Operand, state: EscapeState) -> bool {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                self.update_state(place.local, state)
            }
            Operand::Constant(_) => false,
        }
    }

    /// Update the escape state of a local, returning true if changed.
    fn update_state(&mut self, local: LocalId, new_state: EscapeState) -> bool {
        let current = self.states.get(&local).copied().unwrap_or(EscapeState::NoEscape);
        let joined = current.join(new_state);

        if joined != current {
            self.states.insert(local, joined);
            true
        } else {
            false
        }
    }
}

impl Default for EscapeAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Type;
    use crate::span::Span;
    use super::super::body::{MirBody, LocalKind};
    use super::super::types::{
        AggregateKind, Statement, Terminator, TerminatorKind, Constant, ConstantKind,
    };

    fn dummy_def_id() -> DefId {
        DefId::new(0)
    }

    /// Returns a non-primitive type (tuple) for testing escape analysis.
    ///
    /// Primitive/Copy types are always stack-promotable regardless of escape state
    /// because they are copied by value, not referenced. To test that escape analysis
    /// correctly prevents stack allocation, we need non-primitive types.
    fn non_primitive_ty() -> Type {
        // Use a reference type which is NOT Copy in Blood's model
        // (unlike Rust where &T is Copy, Blood treats references as non-Copy
        // to ensure proper generation checking)
        Type::reference(Type::i32(), false)
    }

    #[test]
    fn test_escape_state_ordering() {
        assert!(EscapeState::NoEscape < EscapeState::ArgEscape);
        assert!(EscapeState::ArgEscape < EscapeState::GlobalEscape);
    }

    #[test]
    fn test_escape_state_join() {
        assert_eq!(
            EscapeState::NoEscape.join(EscapeState::NoEscape),
            EscapeState::NoEscape
        );
        assert_eq!(
            EscapeState::NoEscape.join(EscapeState::ArgEscape),
            EscapeState::ArgEscape
        );
        assert_eq!(
            EscapeState::ArgEscape.join(EscapeState::GlobalEscape),
            EscapeState::GlobalEscape
        );
    }

    #[test]
    fn test_escape_state_can_stack_allocate() {
        assert!(EscapeState::NoEscape.can_stack_allocate());
        assert!(!EscapeState::ArgEscape.can_stack_allocate());
        assert!(!EscapeState::GlobalEscape.can_stack_allocate());
    }

    #[test]
    fn test_escape_state_recommended_tier() {
        assert_eq!(EscapeState::NoEscape.recommended_tier(), MemoryTier::Stack);
        assert_eq!(EscapeState::ArgEscape.recommended_tier(), MemoryTier::Region);
        assert_eq!(EscapeState::GlobalEscape.recommended_tier(), MemoryTier::Region);
    }

    #[test]
    fn test_escape_results_default() {
        let results = EscapeResults::new();
        assert!(results.states.is_empty());
        assert!(results.stack_promotable.is_empty());
    }

    #[test]
    fn test_escape_results_get() {
        let mut results = EscapeResults::new();
        results.states.insert(LocalId::new(0), EscapeState::ArgEscape);

        assert_eq!(results.get(LocalId::new(0)), EscapeState::ArgEscape);
        assert_eq!(results.get(LocalId::new(1)), EscapeState::NoEscape); // default
    }

    #[test]
    fn test_escape_analyzer_simple() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // Add return place
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

        // Add a temporary
        let temp = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());

        // Add a block with just a return
        let bb = body.new_block();
        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // Return place should escape
        assert_eq!(results.get(LocalId::new(0)), EscapeState::ArgEscape);

        // Unused temp should not escape
        assert_eq!(results.get(temp), EscapeState::NoEscape);
        assert!(results.can_stack_allocate(temp));
    }

    #[test]
    fn test_nonescape_struct_is_stack_promotable() {
        // Verify that a non-primitive (struct/tuple) local that doesn't escape
        // IS stack-promotable. This is critical for performance.
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // Use a non-primitive type (tuple)
        let struct_ty = non_primitive_ty();

        // Return place with primitive type (so it doesn't affect struct's escape state)
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

        // Add a struct temporary that doesn't escape
        let struct_local = body.new_local(struct_ty.clone(), LocalKind::Temp, Span::dummy());

        // Create block with just an assignment to the struct (no escape)
        let bb = body.new_block();
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(struct_local),
                Rvalue::Aggregate { kind: AggregateKind::Tuple, operands: vec![] },
            ),
            Span::dummy(),
        ));
        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // The struct local should be NoEscape
        assert_eq!(results.get(struct_local), EscapeState::NoEscape,
            "Non-escaping struct should have NoEscape state");

        // CRITICAL: Non-escaping struct MUST be stack-promotable
        assert!(results.can_stack_allocate(struct_local),
            "Non-escaping struct should be stack-promotable! \
             This is critical for performance - structs that don't escape \
             should use stack allocation, not blood_alloc_or_abort.");

        // Verify the recommended tier is Stack
        assert_eq!(results.recommended_tier(struct_local), MemoryTier::Stack,
            "Non-escaping struct should recommend Stack tier");
    }

    #[test]
    fn test_escape_analyzer_with_assignment() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // Use non-primitive type - primitives are always stack-promotable
        let ty = non_primitive_ty();

        // _0: return place
        body.new_local(ty.clone(), LocalKind::ReturnPlace, Span::dummy());
        // _1: temporary
        let temp = body.new_local(ty.clone(), LocalKind::Temp, Span::dummy());

        let bb = body.new_block();

        // _0 = _1 (temp escapes via return)
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(LocalId::new(0)),
                Rvalue::Use(Operand::Copy(Place::local(temp))),
            ),
            Span::dummy(),
        ));

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // temp should now escape because it's assigned to return place
        assert_eq!(results.get(temp), EscapeState::ArgEscape);
        // For non-primitive types, escaping values cannot be stack-allocated
        assert!(!results.can_stack_allocate(temp));
    }

    #[test]
    fn test_escape_analyzer_call() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // _0: return place
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        // _1: temporary passed to call
        let temp = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());

        let bb = body.new_block();

        // call foo(_1)
        body.set_terminator(bb, Terminator::new(
            TerminatorKind::Call {
                func: Operand::Constant(Constant::new(Type::unit(), ConstantKind::FnDef(DefId::new(1)))),
                args: vec![Operand::Copy(Place::local(temp))],
                destination: Place::local(LocalId::new(0)),
                target: None,
                unwind: None,
            },
            Span::dummy(),
        ));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // temp escapes via function call
        assert_eq!(results.get(temp), EscapeState::ArgEscape);
    }

    #[test]
    fn test_escape_analyzer_effect_capture() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // Use non-primitive type - primitives are always stack-promotable
        let ty = non_primitive_ty();

        body.new_local(ty.clone(), LocalKind::ReturnPlace, Span::dummy());
        let temp = body.new_local(ty.clone(), LocalKind::Temp, Span::dummy());

        let bb = body.new_block();
        let target_bb = body.new_block();

        // perform Effect.op(_1)
        body.set_terminator(bb, Terminator::new(
            TerminatorKind::Perform {
                effect_id: DefId::new(1),
                op_index: 0,
                args: vec![Operand::Copy(Place::local(temp))],
                destination: Place::local(LocalId::new(0)),
                target: target_bb,
                is_tail_resumptive: true,
            },
            Span::dummy(),
        ));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // temp is effect-captured
        assert!(results.is_effect_captured(temp));
        // For non-primitive types, effect-captured values cannot be stack-allocated
        assert!(!results.can_stack_allocate(temp));
    }

    #[test]
    fn test_escape_results_recommended_tier() {
        let mut results = EscapeResults::new();

        // No escape -> Stack
        results.states.insert(LocalId::new(0), EscapeState::NoEscape);
        results.stack_promotable.insert(LocalId::new(0));
        assert_eq!(results.recommended_tier(LocalId::new(0)), MemoryTier::Stack);

        // Arg escape -> Region
        results.states.insert(LocalId::new(1), EscapeState::ArgEscape);
        assert_eq!(results.recommended_tier(LocalId::new(1)), MemoryTier::Region);

        // Effect captured -> Region (even if NoEscape)
        results.states.insert(LocalId::new(2), EscapeState::NoEscape);
        results.effect_captured.insert(LocalId::new(2));
        assert_eq!(results.recommended_tier(LocalId::new(2)), MemoryTier::Region);
    }

    #[test]
    fn test_closure_capture_tracking() {
        let mut results = EscapeResults::new();

        // Set up closure captures: closure at local 2 captures locals 0 and 1
        results.closure_captures.insert(LocalId::new(2), vec![LocalId::new(0), LocalId::new(1)]);
        results.captured_by_closure.insert(LocalId::new(0));
        results.captured_by_closure.insert(LocalId::new(1));

        // Check capture tracking
        assert!(results.is_closure_captured(LocalId::new(0)));
        assert!(results.is_closure_captured(LocalId::new(1)));
        assert!(!results.is_closure_captured(LocalId::new(2)));

        // Check captures for closure
        let captures = results.get_captures(LocalId::new(2)).unwrap();
        assert_eq!(captures.len(), 2);
        assert!(captures.contains(&LocalId::new(0)));
        assert!(captures.contains(&LocalId::new(1)));

        // Check capturing closures
        let closures = results.capturing_closures(LocalId::new(0));
        assert_eq!(closures.len(), 1);
        assert!(closures.contains(&LocalId::new(2)));
    }

    #[test]
    fn test_closure_capture_escape_propagation() {
        let mut results = EscapeResults::new();

        // Local 0 captured by closure at local 2
        // Local 1 captured by closure at local 3
        results.closure_captures.insert(LocalId::new(2), vec![LocalId::new(0)]);
        results.closure_captures.insert(LocalId::new(3), vec![LocalId::new(1)]);
        results.captured_by_closure.insert(LocalId::new(0));
        results.captured_by_closure.insert(LocalId::new(1));

        // Closure 2 doesn't escape, closure 3 does
        results.states.insert(LocalId::new(0), EscapeState::NoEscape);
        results.states.insert(LocalId::new(1), EscapeState::NoEscape);
        results.states.insert(LocalId::new(2), EscapeState::NoEscape);
        results.states.insert(LocalId::new(3), EscapeState::ArgEscape);

        // Local 0: captured by non-escaping closure -> can use stack
        assert_eq!(results.recommended_tier(LocalId::new(0)), MemoryTier::Stack);

        // Local 1: captured by escaping closure -> must use Region
        assert_eq!(results.recommended_tier(LocalId::new(1)), MemoryTier::Region);
    }

    #[test]
    fn test_analyzer_closure_capture_collection() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // _0: return place
        body.new_local(Type::unit(), LocalKind::ReturnPlace, Span::dummy());
        // _1: captured value
        let captured = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        // _2: closure
        let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());

        let bb = body.new_block();

        // _2 = Closure { _1 } (create closure capturing _1)
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(closure),
                Rvalue::Aggregate {
                    kind: AggregateKind::Closure { def_id: DefId::new(100) },
                    operands: vec![Operand::Copy(Place::local(captured))],
                },
            ),
            Span::dummy(),
        ));

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // Verify closure captures were collected
        assert!(results.is_closure_captured(captured));
        let captures = results.get_captures(closure).unwrap();
        assert_eq!(captures.len(), 1);
        assert!(captures.contains(&captured));
    }

    #[test]
    fn test_analyzer_escaping_closure_propagates() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // _0: return place (where closure will be returned)
        let ret = body.new_local(Type::unit(), LocalKind::ReturnPlace, Span::dummy());
        let _ = ret;
        // _1: captured value - use non-primitive type for stack allocation test
        let captured = body.new_local(non_primitive_ty(), LocalKind::Temp, Span::dummy());
        // _2: closure
        let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());

        let bb = body.new_block();

        // _2 = Closure { _1 }
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(closure),
                Rvalue::Aggregate {
                    kind: AggregateKind::Closure { def_id: DefId::new(100) },
                    operands: vec![Operand::Copy(Place::local(captured))],
                },
            ),
            Span::dummy(),
        ));

        // _0 = _2 (return the closure - makes it escape)
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(LocalId::new(0)),
                Rvalue::Use(Operand::Copy(Place::local(closure))),
            ),
            Span::dummy(),
        ));

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // The closure escapes via return
        assert_eq!(results.get(closure), EscapeState::ArgEscape);

        // The captured value should also escape (propagated from closure)
        assert_eq!(results.get(captured), EscapeState::ArgEscape);

        // For non-primitive types, captured values that escape cannot be stack-allocated
        assert!(!results.can_stack_allocate(captured));
    }

    #[test]
    fn test_analyzer_non_escaping_closure() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // _0: return place
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        // _1: captured value
        let captured = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        // _2: closure (not returned or passed anywhere)
        let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());

        let bb = body.new_block();

        // _2 = Closure { _1 }
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(closure),
                Rvalue::Aggregate {
                    kind: AggregateKind::Closure { def_id: DefId::new(100) },
                    operands: vec![Operand::Copy(Place::local(captured))],
                },
            ),
            Span::dummy(),
        ));

        // Don't return or use the closure, just return a constant
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(LocalId::new(0)),
                Rvalue::Use(Operand::Constant(Constant::new(Type::i32(), ConstantKind::Int(42)))),
            ),
            Span::dummy(),
        ));

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // The closure doesn't escape
        assert_eq!(results.get(closure), EscapeState::NoEscape);

        // The captured value shouldn't escape (closure doesn't escape)
        assert_eq!(results.get(captured), EscapeState::NoEscape);

        // Can stack allocate captured value since closure doesn't escape
        assert!(results.can_stack_allocate(captured));
    }

    // ========================================================================
    // Property Tests (FUZZ-005)
    // ========================================================================

    /// All possible escape states for exhaustive enumeration.
    const ALL_STATES: [EscapeState; 3] = [
        EscapeState::NoEscape,
        EscapeState::ArgEscape,
        EscapeState::GlobalEscape,
    ];

    // Lattice property: join is commutative (a ⊔ b = b ⊔ a)
    #[test]
    fn test_property_join_commutative() {
        for &a in &ALL_STATES {
            for &b in &ALL_STATES {
                assert_eq!(
                    a.join(b),
                    b.join(a),
                    "join is not commutative: {:?}.join({:?}) != {:?}.join({:?})",
                    a, b, b, a
                );
            }
        }
    }

    // Lattice property: join is associative ((a ⊔ b) ⊔ c = a ⊔ (b ⊔ c))
    #[test]
    fn test_property_join_associative() {
        for &a in &ALL_STATES {
            for &b in &ALL_STATES {
                for &c in &ALL_STATES {
                    assert_eq!(
                        a.join(b).join(c),
                        a.join(b.join(c)),
                        "join is not associative: ({:?} ⊔ {:?}) ⊔ {:?} != {:?} ⊔ ({:?} ⊔ {:?})",
                        a, b, c, a, b, c
                    );
                }
            }
        }
    }

    // Lattice property: join is idempotent (a ⊔ a = a)
    #[test]
    fn test_property_join_idempotent() {
        for &a in &ALL_STATES {
            assert_eq!(
                a.join(a),
                a,
                "join is not idempotent: {:?}.join({:?}) != {:?}",
                a, a, a
            );
        }
    }

    // Lattice property: NoEscape is identity (a ⊔ NoEscape = a)
    #[test]
    fn test_property_join_identity() {
        for &a in &ALL_STATES {
            assert_eq!(
                a.join(EscapeState::NoEscape),
                a,
                "NoEscape is not identity: {:?}.join(NoEscape) != {:?}",
                a, a
            );
        }
    }

    // Lattice property: join is monotonic with respect to ordering (a ≤ a ⊔ b)
    #[test]
    fn test_property_join_monotonic() {
        for &a in &ALL_STATES {
            for &b in &ALL_STATES {
                let joined = a.join(b);
                assert!(
                    a <= joined,
                    "join is not monotonic: {:?} > {:?}.join({:?}) = {:?}",
                    a, a, b, joined
                );
                assert!(
                    b <= joined,
                    "join is not monotonic: {:?} > {:?}.join({:?}) = {:?}",
                    b, a, b, joined
                );
            }
        }
    }

    // Lattice property: ordering forms a total order (either a ≤ b or b ≤ a)
    #[test]
    fn test_property_total_order() {
        for &a in &ALL_STATES {
            for &b in &ALL_STATES {
                assert!(
                    a <= b || b <= a,
                    "escape states don't form total order: {:?} and {:?} are incomparable",
                    a, b
                );
            }
        }
    }

    // Soundness property: stack_promotable implies NoEscape state
    #[test]
    fn test_property_stack_promotable_implies_no_escape() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

        // Create several temporaries
        let temps: Vec<_> = (0..10)
            .map(|_| body.new_local(Type::i32(), LocalKind::Temp, Span::dummy()))
            .collect();

        let bb = body.new_block();
        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // Invariant: if stack_promotable, state must be NoEscape
        for &temp in &temps {
            if results.can_stack_allocate(temp) {
                assert_eq!(
                    results.get(temp),
                    EscapeState::NoEscape,
                    "stack_promotable local {:?} has escape state {:?}, expected NoEscape",
                    temp,
                    results.get(temp)
                );
            }
        }
    }

    // Soundness property: for non-primitive types, stack_promotable implies not effect_captured
    // Note: This invariant only applies to non-primitive types. Primitives can be both
    // effect_captured AND stack_promotable because they are copied by value.
    #[test]
    fn test_property_stack_promotable_implies_not_effect_captured() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        // Use non-primitive types - the invariant doesn't apply to primitives
        let ty = non_primitive_ty();
        body.new_local(ty.clone(), LocalKind::ReturnPlace, Span::dummy());

        let temps: Vec<_> = (0..5)
            .map(|_| body.new_local(ty.clone(), LocalKind::Temp, Span::dummy()))
            .collect();

        let bb = body.new_block();
        let target_bb = body.new_block();

        // Perform effect with first temp as argument
        body.set_terminator(bb, Terminator::new(
            TerminatorKind::Perform {
                effect_id: DefId::new(1),
                op_index: 0,
                args: vec![Operand::Copy(Place::local(temps[0]))],
                destination: Place::local(LocalId::new(0)),
                target: target_bb,
                is_tail_resumptive: true,
            },
            Span::dummy(),
        ));

        body.set_terminator(target_bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // Invariant: if stack_promotable, must not be effect_captured
        for local in body.locals.iter().map(|l| l.id) {
            if results.can_stack_allocate(local) {
                assert!(
                    !results.is_effect_captured(local),
                    "stack_promotable local {:?} is effect_captured (contradiction)",
                    local
                );
            }
        }
    }

    // Soundness property: can_stack_allocate implies recommended_tier is Stack
    #[test]
    fn test_property_stack_promotable_implies_stack_tier() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

        let temps: Vec<_> = (0..5)
            .map(|_| body.new_local(Type::i32(), LocalKind::Temp, Span::dummy()))
            .collect();

        let bb = body.new_block();
        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // Invariant: if stack_promotable, recommended_tier must be Stack
        for &temp in &temps {
            if results.can_stack_allocate(temp) {
                assert_eq!(
                    results.recommended_tier(temp),
                    MemoryTier::Stack,
                    "stack_promotable local {:?} has tier {:?}, expected Stack",
                    temp,
                    results.recommended_tier(temp)
                );
            }
        }
    }

    // Property: escape state consistency after analysis
    #[test]
    fn test_property_escape_state_consistency() {
        for &state in &ALL_STATES {
            // can_stack_allocate only for NoEscape
            assert_eq!(
                state.can_stack_allocate(),
                state == EscapeState::NoEscape,
                "can_stack_allocate inconsistent for {:?}",
                state
            );

            // recommended_tier matches the state
            let tier = state.recommended_tier();
            if state == EscapeState::NoEscape {
                assert_eq!(tier, MemoryTier::Stack);
            } else {
                assert_eq!(tier, MemoryTier::Region);
            }
        }
    }

    // Property: analysis is deterministic (same input → same output)
    #[test]
    fn test_property_analysis_deterministic() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

        let temps: Vec<_> = (0..10)
            .map(|_| body.new_local(Type::i32(), LocalKind::Temp, Span::dummy()))
            .collect();

        let bb = body.new_block();

        // Create some assignments
        for i in 0..temps.len() - 1 {
            body.push_statement(bb, Statement::new(
                StatementKind::Assign(
                    Place::local(temps[i + 1]),
                    Rvalue::Use(Operand::Copy(Place::local(temps[i]))),
                ),
                Span::dummy(),
            ));
        }

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        // Run analysis multiple times
        let mut analyzer1 = EscapeAnalyzer::new();
        let results1 = analyzer1.analyze(&body);

        let mut analyzer2 = EscapeAnalyzer::new();
        let results2 = analyzer2.analyze(&body);

        // Results must be identical
        for &temp in &temps {
            assert_eq!(
                results1.get(temp),
                results2.get(temp),
                "analysis not deterministic for {:?}: {:?} vs {:?}",
                temp,
                results1.get(temp),
                results2.get(temp)
            );
            assert_eq!(
                results1.can_stack_allocate(temp),
                results2.can_stack_allocate(temp),
                "stack_promotable not deterministic for {:?}",
                temp
            );
        }
    }

    // Property: return place always escapes (ArgEscape or higher)
    #[test]
    fn test_property_return_place_always_escapes() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        // Use non-primitive type - primitives are always stack-promotable
        body.new_local(non_primitive_ty(), LocalKind::ReturnPlace, Span::dummy());

        let bb = body.new_block();
        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        let return_state = results.get(body.return_place());
        assert!(
            return_state >= EscapeState::ArgEscape,
            "return place has state {:?}, expected ArgEscape or higher",
            return_state
        );
        // For non-primitive types, return place should not be stack-promotable
        assert!(
            !results.can_stack_allocate(body.return_place()),
            "return place should not be stack_promotable for non-primitive types"
        );
    }

    // Property: function arguments that are only used locally remain NoEscape
    // This is critical for performance - reference parameters that are only read
    // can be stack-allocated without generation checks.
    #[test]
    fn test_property_args_noescaping_unless_used() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        let arg1 = body.new_local(Type::i32(), LocalKind::Arg, Span::dummy());
        let arg2 = body.new_local(Type::i32(), LocalKind::Arg, Span::dummy());
        // Set param_count to match the number of args we added
        body.param_count = 2;

        let bb = body.new_block();
        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // Arguments that are not used should remain NoEscape
        for &arg in &[arg1, arg2] {
            let state = results.get(arg);
            assert_eq!(
                state, EscapeState::NoEscape,
                "unused arg {:?} has state {:?}, expected NoEscape",
                arg,
                state
            );
        }
    }

    // Property: GlobalEscape is top element (absorbs other states)
    #[test]
    fn test_property_global_escape_is_top() {
        for &a in &ALL_STATES {
            assert_eq!(
                a.join(EscapeState::GlobalEscape),
                EscapeState::GlobalEscape,
                "GlobalEscape should absorb {:?}",
                a
            );
        }
    }

    // Property: analysis reaches fixed point (no infinite loops)
    #[test]
    fn test_property_fixed_point_reached() {
        // Create a body with potential cycles in dataflow
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());

        let temps: Vec<_> = (0..5)
            .map(|_| body.new_local(Type::i32(), LocalKind::Temp, Span::dummy()))
            .collect();

        let bb = body.new_block();

        // Create a chain of assignments that could cause iteration
        // _1 = _2, _2 = _3, _3 = _4, _4 = _5, _5 = _1 (cycle)
        for i in 0..temps.len() {
            let next = (i + 1) % temps.len();
            body.push_statement(bb, Statement::new(
                StatementKind::Assign(
                    Place::local(temps[i]),
                    Rvalue::Use(Operand::Copy(Place::local(temps[next]))),
                ),
                Span::dummy(),
            ));
        }

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        // Analysis should terminate (this is a smoke test - if it doesn't terminate,
        // the test will timeout)
        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // Just verify we got results for all temps
        for &temp in &temps {
            let _state = results.get(temp);
        }
    }

    // Property: effect_captured values cannot be stack allocated
    #[test]
    fn test_property_effect_captured_not_stack_promotable() {
        let mut results = EscapeResults::new();
        let local = LocalId::new(5);

        // Mark as effect captured
        results.effect_captured.insert(local);
        results.states.insert(local, EscapeState::NoEscape);

        // Even though state is NoEscape, recommended tier should be Region
        // because effect_captured forces heap allocation
        assert_eq!(
            results.recommended_tier(local),
            MemoryTier::Region,
            "effect_captured local should have Region tier"
        );
    }

    // Property: escape states form a finite lattice (bounded iterations)
    #[test]
    fn test_property_lattice_bounded() {
        // The lattice has exactly 3 elements, so any ascending chain
        // has at most 3 elements, guaranteeing termination
        let mut state = EscapeState::NoEscape;
        let mut count = 0;

        // Simulate worst-case ascending chain
        while state < EscapeState::GlobalEscape {
            state = state.join(match state {
                EscapeState::NoEscape => EscapeState::ArgEscape,
                EscapeState::ArgEscape => EscapeState::GlobalEscape,
                EscapeState::GlobalEscape => EscapeState::GlobalEscape,
            });
            count += 1;
            assert!(count <= 3, "lattice chain exceeded expected bound");
        }
    }

    // ========================================================================
    // Closure Chain Tests (PERF-IMPL-001)
    // ========================================================================

    /// Test closure chain escape propagation correctness.
    ///
    /// This test verifies that when the last closure in a chain escapes,
    /// all closures captured transitively also escape correctly.
    ///
    /// PERF-IMPL-001: Closure chain escape analysis was optimized from O(n²)
    /// to O(n) using a worklist algorithm.
    #[test]
    fn test_closure_chain_propagation_correctness() {
        // Test with chains of various sizes
        for chain_length in [5, 10, 20, 50] {
            let mut body = MirBody::new(dummy_def_id(), Span::dummy());

            // _0: return place
            body.new_local(Type::unit(), LocalKind::ReturnPlace, Span::dummy());

            // Create locals that will be captured by closures
            let mut captured_locals: Vec<LocalId> = Vec::with_capacity(chain_length);
            for _ in 0..chain_length {
                let local = body.new_local(non_primitive_ty(), LocalKind::Temp, Span::dummy());
                captured_locals.push(local);
            }

            // Create chain of closures where each captures a local
            let mut closure_locals: Vec<LocalId> = Vec::with_capacity(chain_length);
            for _ in 0..chain_length {
                let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());
                closure_locals.push(closure);
            }

            let bb = body.new_block();

            // First closure captures captured_locals[0]
            body.push_statement(bb, Statement::new(
                StatementKind::Assign(
                    Place::local(closure_locals[0]),
                    Rvalue::Aggregate {
                        kind: AggregateKind::Closure { def_id: DefId::new(100) },
                        operands: vec![Operand::Copy(Place::local(captured_locals[0]))],
                    },
                ),
                Span::dummy(),
            ));

            // Each subsequent closure captures the previous closure
            for i in 1..chain_length {
                body.push_statement(bb, Statement::new(
                    StatementKind::Assign(
                        Place::local(closure_locals[i]),
                        Rvalue::Aggregate {
                            kind: AggregateKind::Closure { def_id: DefId::new(100 + i as u32) },
                            operands: vec![Operand::Copy(Place::local(closure_locals[i - 1]))],
                        },
                    ),
                    Span::dummy(),
                ));
            }

            // Make the LAST closure escape (return it)
            body.push_statement(bb, Statement::new(
                StatementKind::Assign(
                    Place::local(LocalId::new(0)),
                    Rvalue::Use(Operand::Copy(Place::local(closure_locals[chain_length - 1]))),
                ),
                Span::dummy(),
            ));

            body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

            // Run analysis
            let mut analyzer = EscapeAnalyzer::new();
            let results = analyzer.analyze(&body);

            // Verify: ALL closures in the chain should escape
            for (i, &closure) in closure_locals.iter().enumerate() {
                let state = results.get(closure);
                assert_eq!(
                    state, EscapeState::ArgEscape,
                    "Closure {} in chain of {} should escape (got {:?})",
                    i, chain_length, state
                );
            }

            // Verify: The first captured local should also escape
            // (it's captured by the first closure which escapes transitively)
            let first_captured_state = results.get(captured_locals[0]);
            assert_eq!(
                first_captured_state, EscapeState::ArgEscape,
                "First captured local should escape (got {:?})",
                first_captured_state
            );
        }
    }

    /// Test that worklist algorithm handles complex closure capture patterns.
    ///
    /// Creates a pattern where multiple closures capture the same local,
    /// and only some closures escape.
    #[test]
    fn test_closure_multiple_capturers() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        body.new_local(Type::unit(), LocalKind::ReturnPlace, Span::dummy());

        // Create a shared captured local
        let shared_local = body.new_local(non_primitive_ty(), LocalKind::Temp, Span::dummy());

        // Create three closures that all capture the shared local
        let closure1 = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());
        let closure2 = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());
        let closure3 = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());

        let bb = body.new_block();

        // All three closures capture the same local
        for (closure, def_id) in [(closure1, 100), (closure2, 101), (closure3, 102)] {
            body.push_statement(bb, Statement::new(
                StatementKind::Assign(
                    Place::local(closure),
                    Rvalue::Aggregate {
                        kind: AggregateKind::Closure { def_id: DefId::new(def_id) },
                        operands: vec![Operand::Copy(Place::local(shared_local))],
                    },
                ),
                Span::dummy(),
            ));
        }

        // Only closure2 escapes
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(LocalId::new(0)),
                Rvalue::Use(Operand::Copy(Place::local(closure2))),
            ),
            Span::dummy(),
        ));

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // closure1 and closure3 don't escape
        assert_eq!(results.get(closure1), EscapeState::NoEscape);
        assert_eq!(results.get(closure3), EscapeState::NoEscape);

        // closure2 escapes
        assert_eq!(results.get(closure2), EscapeState::ArgEscape);

        // shared_local escapes because closure2 escapes and captures it
        assert_eq!(results.get(shared_local), EscapeState::ArgEscape);

        // shared_local cannot be stack allocated
        assert!(!results.can_stack_allocate(shared_local));
    }

    /// Performance smoke test for closure chains.
    ///
    /// This test verifies that analysis completes in reasonable time
    /// for large closure chains, validating the O(n) worklist algorithm.
    #[test]
    fn test_closure_chain_performance_smoke() {
        use std::time::Instant;

        // Test with 500 closures - should complete quickly with O(n) algorithm
        // With O(n²) algorithm, this would take ~6.3ms according to original profiling
        // With O(n) algorithm, should take <1ms
        let chain_length = 500;

        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::unit(), LocalKind::ReturnPlace, Span::dummy());

        let mut closure_locals: Vec<LocalId> = Vec::with_capacity(chain_length);
        for _ in 0..chain_length {
            let closure = body.new_local(Type::unit(), LocalKind::Temp, Span::dummy());
            closure_locals.push(closure);
        }

        let bb = body.new_block();

        // First closure captures a dummy local
        let captured = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(closure_locals[0]),
                Rvalue::Aggregate {
                    kind: AggregateKind::Closure { def_id: DefId::new(100) },
                    operands: vec![Operand::Copy(Place::local(captured))],
                },
            ),
            Span::dummy(),
        ));

        // Chain: each closure captures the previous one
        for i in 1..chain_length {
            body.push_statement(bb, Statement::new(
                StatementKind::Assign(
                    Place::local(closure_locals[i]),
                    Rvalue::Aggregate {
                        kind: AggregateKind::Closure { def_id: DefId::new(100 + i as u32) },
                        operands: vec![Operand::Copy(Place::local(closure_locals[i - 1]))],
                    },
                ),
                Span::dummy(),
            ));
        }

        // Make last closure escape
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(LocalId::new(0)),
                Rvalue::Use(Operand::Copy(Place::local(closure_locals[chain_length - 1]))),
            ),
            Span::dummy(),
        ));

        body.set_terminator(bb, Terminator::new(TerminatorKind::Return, Span::dummy()));

        // Measure analysis time
        let start = Instant::now();
        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);
        let elapsed = start.elapsed();

        // Verify correctness
        assert_eq!(results.get(closure_locals[chain_length - 1]), EscapeState::ArgEscape);
        assert_eq!(results.get(closure_locals[0]), EscapeState::ArgEscape);

        // Performance assertion: should complete in <100ms even on slow machines
        // (With O(n²), 500 closures took 6.3ms; with O(n) should be much faster)
        assert!(
            elapsed.as_millis() < 100,
            "Closure chain analysis took {:?}, expected <100ms. \
             This suggests the worklist optimization may not be working.",
            elapsed
        );

        // For informational purposes (not a hard assertion)
        if elapsed.as_micros() > 1000 {
            eprintln!(
                "Note: Closure chain of {} took {:?} (expected <1ms with O(n) algorithm)",
                chain_length, elapsed
            );
        }
    }
}
