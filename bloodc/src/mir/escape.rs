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

use std::collections::{HashMap, HashSet};

use crate::hir::{DefId, LocalId};
use super::body::MirBody;
use super::ptr::MemoryTier;
use super::types::{
    Operand, Place, PlaceElem, Rvalue, StatementKind, TerminatorKind,
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
}

impl EscapeResults {
    /// Create new empty results.
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            effect_captured: HashSet::new(),
            stack_promotable: HashSet::new(),
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

    /// Get recommended memory tier for a local.
    pub fn recommended_tier(&self, local: LocalId) -> MemoryTier {
        // Effect-captured values need region allocation for snapshot
        if self.is_effect_captured(local) {
            return MemoryTier::Region;
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
}

impl EscapeAnalyzer {
    /// Create a new escape analyzer.
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            effect_captured: HashSet::new(),
            globals: HashSet::new(),
        }
    }

    /// Add a known global definition.
    pub fn add_global(&mut self, def_id: DefId) {
        self.globals.insert(def_id);
    }

    /// Analyze a MIR body and return escape results.
    pub fn analyze(&mut self, body: &MirBody) -> EscapeResults {
        self.states.clear();
        self.effect_captured.clear();

        // Initialize all locals to NoEscape
        for local in &body.locals {
            self.states.insert(local.id, EscapeState::NoEscape);
        }

        // Return place always escapes (returned to caller)
        self.states.insert(body.return_place(), EscapeState::ArgEscape);

        // Parameters may escape depending on their usage
        // For now, mark them as potentially escaping
        for param_id in body.param_ids() {
            self.states.insert(param_id, EscapeState::ArgEscape);
        }

        // Iterate to fixed point
        loop {
            let mut changed = false;

            for (_bb_id, block) in body.blocks() {
                for stmt in &block.statements {
                    changed |= self.analyze_statement(&stmt.kind);
                }

                if let Some(term) = &block.terminator {
                    changed |= self.analyze_terminator(&term.kind);
                }
            }

            if !changed {
                break;
            }
        }

        // Determine stack-promotable allocations
        let mut stack_promotable = HashSet::new();
        for (local, state) in &self.states {
            if state.can_stack_allocate() && !self.effect_captured.contains(local) {
                stack_promotable.insert(*local);
            }
        }

        EscapeResults {
            states: self.states.clone(),
            effect_captured: self.effect_captured.clone(),
            stack_promotable,
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
        Statement, Terminator, TerminatorKind, Constant, ConstantKind,
    };

    fn dummy_def_id() -> DefId {
        DefId::new(0)
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
    fn test_escape_analyzer_with_assignment() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());

        // _0: return place
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        // _1: temporary
        let temp = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());

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

        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        let temp = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());

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
            },
            Span::dummy(),
        ));

        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(&body);

        // temp is effect-captured
        assert!(results.is_effect_captured(temp));
        // Cannot stack allocate effect-captured values
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
}
