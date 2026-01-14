//! # MIR Visitor Infrastructure
//!
//! This module provides visitor traits for traversing MIR structures.
//!
//! ## Design
//!
//! Following [rustc_middle::mir::visit](https://doc.rust-lang.org/nightly/nightly-rustc/rustc_middle/mir/visit/index.html),
//! we provide both:
//! - `Visitor` for immutable traversal (analysis passes)
//! - `MutVisitor` for mutable traversal (transformation passes)
//!
//! ## Usage Example
//!
//! ```ignore
//! use bloodc::mir::visitor::Visitor;
//!
//! struct MyAnalysis {
//!     count: usize,
//! }
//!
//! impl Visitor for MyAnalysis {
//!     fn visit_statement(&mut self, stmt: &Statement, location: Location) {
//!         self.count += 1;
//!         self.super_statement(stmt, location);
//!     }
//! }
//! ```
//!
//! ## Visitor Methods
//!
//! Each `visit_X` method has a corresponding `super_X` method that performs
//! the default recursive traversal. Override `visit_X` for custom behavior,
//! call `super_X` to continue traversal into children.

use crate::hir::LocalId;
use super::body::MirBody;
use super::types::{
    BasicBlockId, BasicBlockData, Statement, StatementKind,
    Terminator, TerminatorKind, Place, PlaceElem, Operand, Rvalue,
    Constant, BinOp, UnOp, AggregateKind,
};

// ============================================================================
// Location
// ============================================================================

/// A location in the MIR - identifies where a statement or terminator is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Location {
    /// The basic block.
    pub block: BasicBlockId,
    /// Index within the block (statement index, or `TERMINATOR` for terminator).
    pub statement_index: usize,
}

impl Location {
    /// Sentinel value for terminator position.
    pub const TERMINATOR: usize = usize::MAX;

    /// Create a location for a statement.
    pub fn statement(block: BasicBlockId, index: usize) -> Self {
        Self { block, statement_index: index }
    }

    /// Create a location for a terminator.
    pub fn terminator(block: BasicBlockId) -> Self {
        Self { block, statement_index: Self::TERMINATOR }
    }

    /// Check if this location points to a terminator.
    pub fn is_terminator(&self) -> bool {
        self.statement_index == Self::TERMINATOR
    }
}

// ============================================================================
// Visitor Trait (Immutable)
// ============================================================================

/// Visitor trait for immutable MIR traversal.
///
/// Override `visit_*` methods to customize traversal behavior.
/// Call `super_*` methods to continue default recursive traversal.
pub trait Visitor: Sized {
    /// Visit the entire MIR body.
    fn visit_body(&mut self, body: &MirBody) {
        self.super_body(body);
    }

    /// Default implementation: visit all blocks.
    fn super_body(&mut self, body: &MirBody) {
        for (bb_id, block) in body.blocks() {
            self.visit_basic_block(bb_id, block);
        }
    }

    /// Visit a basic block.
    fn visit_basic_block(&mut self, bb_id: BasicBlockId, block: &BasicBlockData) {
        self.super_basic_block(bb_id, block);
    }

    /// Default implementation: visit statements then terminator.
    fn super_basic_block(&mut self, bb_id: BasicBlockId, block: &BasicBlockData) {
        for (idx, stmt) in block.statements.iter().enumerate() {
            let location = Location::statement(bb_id, idx);
            self.visit_statement(stmt, location);
        }
        if let Some(ref term) = block.terminator {
            let location = Location::terminator(bb_id);
            self.visit_terminator(term, location);
        }
    }

    /// Visit a statement.
    fn visit_statement(&mut self, stmt: &Statement, location: Location) {
        self.super_statement(stmt, location);
    }

    /// Default implementation: visit statement kind.
    fn super_statement(&mut self, stmt: &Statement, location: Location) {
        self.visit_statement_kind(&stmt.kind, location);
    }

    /// Visit a statement kind.
    fn visit_statement_kind(&mut self, kind: &StatementKind, location: Location) {
        self.super_statement_kind(kind, location);
    }

    /// Default implementation: visit components of statement kind.
    fn super_statement_kind(&mut self, kind: &StatementKind, location: Location) {
        match kind {
            StatementKind::Assign(place, rvalue) => {
                self.visit_place(place, PlaceContext::Store, location);
                self.visit_rvalue(rvalue, location);
            }
            StatementKind::StorageLive(local) => {
                self.visit_local(*local, PlaceContext::StorageLive, location);
            }
            StatementKind::StorageDead(local) => {
                self.visit_local(*local, PlaceContext::StorageDead, location);
            }
            StatementKind::Drop(place) => {
                self.visit_place(place, PlaceContext::Drop, location);
            }
            StatementKind::IncrementGeneration(place) => {
                self.visit_place(place, PlaceContext::MutatingUse, location);
            }
            StatementKind::CaptureSnapshot(local) => {
                self.visit_local(*local, PlaceContext::Read, location);
            }
            StatementKind::ValidateGeneration { ptr, expected_gen } => {
                self.visit_place(ptr, PlaceContext::Read, location);
                self.visit_operand(expected_gen, location);
            }
            StatementKind::PushHandler { handler_id: _, state_place, state_kind: _, allocation_tier: _, inline_mode: _ } => {
                self.visit_place(state_place, PlaceContext::Read, location);
            }
            StatementKind::PopHandler => {
                // No places or operands to visit
            }
            StatementKind::CallReturnClause { handler_id: _, handler_name: _, body_result, state_place, destination } => {
                self.visit_operand(body_result, location);
                self.visit_place(state_place, PlaceContext::Read, location);
                self.visit_place(destination, PlaceContext::Store, location);
            }
            StatementKind::Nop => {}
        }
    }

    /// Visit a terminator.
    fn visit_terminator(&mut self, term: &Terminator, location: Location) {
        self.super_terminator(term, location);
    }

    /// Default implementation: visit terminator kind.
    fn super_terminator(&mut self, term: &Terminator, location: Location) {
        self.visit_terminator_kind(&term.kind, location);
    }

    /// Visit a terminator kind.
    fn visit_terminator_kind(&mut self, kind: &TerminatorKind, location: Location) {
        self.super_terminator_kind(kind, location);
    }

    /// Default implementation: visit components of terminator kind.
    fn super_terminator_kind(&mut self, kind: &TerminatorKind, location: Location) {
        match kind {
            TerminatorKind::Goto { target: _ } => {}
            TerminatorKind::SwitchInt { discr, targets: _ } => {
                self.visit_operand(discr, location);
            }
            TerminatorKind::Return => {}
            TerminatorKind::Unreachable => {}
            TerminatorKind::Call { func, args, destination, target: _, unwind: _ } => {
                self.visit_operand(func, location);
                for arg in args {
                    self.visit_operand(arg, location);
                }
                self.visit_place(destination, PlaceContext::Store, location);
            }
            TerminatorKind::Assert { cond, target: _, msg: _, expected: _, unwind: _ } => {
                self.visit_operand(cond, location);
            }
            TerminatorKind::DropAndReplace { place, value, target: _, unwind: _ } => {
                self.visit_place(place, PlaceContext::Store, location);
                self.visit_operand(value, location);
            }
            TerminatorKind::Perform { effect_id: _, op_index: _, args, destination, target: _, is_tail_resumptive: _ } => {
                for arg in args {
                    self.visit_operand(arg, location);
                }
                self.visit_place(destination, PlaceContext::Store, location);
            }
            TerminatorKind::Resume { value } => {
                if let Some(v) = value {
                    self.visit_operand(v, location);
                }
            }
            TerminatorKind::StaleReference { ptr, expected: _, actual: _ } => {
                self.visit_place(ptr, PlaceContext::Read, location);
            }
        }
    }

    /// Visit an rvalue.
    fn visit_rvalue(&mut self, rvalue: &Rvalue, location: Location) {
        self.super_rvalue(rvalue, location);
    }

    /// Default implementation: visit components of rvalue.
    fn super_rvalue(&mut self, rvalue: &Rvalue, location: Location) {
        match rvalue {
            Rvalue::Use(op) => {
                self.visit_operand(op, location);
            }
            Rvalue::Ref { place, mutable: _ } => {
                let ctx = PlaceContext::Borrow;
                self.visit_place(place, ctx, location);
            }
            Rvalue::AddressOf { place, mutable: _ } => {
                self.visit_place(place, PlaceContext::AddressOf, location);
            }
            Rvalue::BinaryOp { op, left, right } => {
                self.visit_binop(*op, location);
                self.visit_operand(left, location);
                self.visit_operand(right, location);
            }
            Rvalue::CheckedBinaryOp { op, left, right } => {
                self.visit_binop(*op, location);
                self.visit_operand(left, location);
                self.visit_operand(right, location);
            }
            Rvalue::UnaryOp { op, operand } => {
                self.visit_unop(*op, location);
                self.visit_operand(operand, location);
            }
            Rvalue::Cast { operand, target_ty: _ } => {
                self.visit_operand(operand, location);
            }
            Rvalue::Aggregate { kind, operands } => {
                self.visit_aggregate_kind(kind, location);
                for op in operands {
                    self.visit_operand(op, location);
                }
            }
            Rvalue::Discriminant(place) => {
                self.visit_place(place, PlaceContext::Read, location);
            }
            Rvalue::Len(place) => {
                self.visit_place(place, PlaceContext::Read, location);
            }
            Rvalue::ReadGeneration(place) => {
                self.visit_place(place, PlaceContext::Read, location);
            }
            Rvalue::NullCheck(op) => {
                self.visit_operand(op, location);
            }
            Rvalue::MakeGenPtr { address, generation, metadata: _ } => {
                self.visit_operand(address, location);
                self.visit_operand(generation, location);
            }
            Rvalue::ZeroInit(_) => {
                // Zero-initialization has no operands to visit
            }
        }
    }

    /// Visit an operand.
    fn visit_operand(&mut self, operand: &Operand, location: Location) {
        self.super_operand(operand, location);
    }

    /// Default implementation: visit operand components.
    fn super_operand(&mut self, operand: &Operand, location: Location) {
        match operand {
            Operand::Copy(place) => {
                self.visit_place(place, PlaceContext::Copy, location);
            }
            Operand::Move(place) => {
                self.visit_place(place, PlaceContext::Move, location);
            }
            Operand::Constant(constant) => {
                self.visit_constant(constant, location);
            }
        }
    }

    /// Visit a place.
    fn visit_place(&mut self, place: &Place, context: PlaceContext, location: Location) {
        self.super_place(place, context, location);
    }

    /// Default implementation: visit place components.
    fn super_place(&mut self, place: &Place, context: PlaceContext, location: Location) {
        self.visit_local(place.local, context, location);
        for elem in &place.projection {
            self.visit_projection_elem(elem, location);
        }
    }

    /// Visit a local variable reference.
    fn visit_local(&mut self, _local: LocalId, _context: PlaceContext, _location: Location) {
        // Leaf node - no default traversal
    }

    /// Visit a projection element.
    fn visit_projection_elem(&mut self, elem: &PlaceElem, location: Location) {
        self.super_projection_elem(elem, location);
    }

    /// Default implementation: visit projection element components.
    fn super_projection_elem(&mut self, elem: &PlaceElem, location: Location) {
        match elem {
            PlaceElem::Deref => {}
            PlaceElem::Field(_) => {}
            PlaceElem::Index(local) => {
                // Index with a local variable
                self.visit_local(*local, PlaceContext::Read, location);
            }
            PlaceElem::ConstantIndex { .. } => {}
            PlaceElem::Subslice { .. } => {}
            PlaceElem::Downcast(_) => {}
        }
    }

    /// Visit a constant.
    fn visit_constant(&mut self, _constant: &Constant, _location: Location) {
        // Leaf node - no default traversal
    }

    /// Visit a binary operator.
    fn visit_binop(&mut self, _op: BinOp, _location: Location) {
        // Leaf node - no default traversal
    }

    /// Visit a unary operator.
    fn visit_unop(&mut self, _op: UnOp, _location: Location) {
        // Leaf node - no default traversal
    }

    /// Visit an aggregate kind.
    fn visit_aggregate_kind(&mut self, _kind: &AggregateKind, _location: Location) {
        // Leaf node - no default traversal
    }
}

// ============================================================================
// Place Context
// ============================================================================

/// The context in which a place is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaceContext {
    /// Place is being read (non-mutating).
    Read,
    /// Place is being copied.
    Copy,
    /// Place is being moved from.
    Move,
    /// Place is being stored to.
    Store,
    /// Place is being borrowed.
    Borrow,
    /// Address of place is being taken.
    AddressOf,
    /// Place is being dropped.
    Drop,
    /// Place is subject to a mutating use.
    MutatingUse,
    /// Storage for local is being started.
    StorageLive,
    /// Storage for local is ending.
    StorageDead,
}

impl PlaceContext {
    /// Returns true if this is a mutating use.
    pub fn is_mutating(&self) -> bool {
        matches!(self, PlaceContext::Store | PlaceContext::MutatingUse | PlaceContext::Drop)
    }

    /// Returns true if this is a use (read, copy, move, borrow).
    pub fn is_use(&self) -> bool {
        matches!(self, PlaceContext::Read | PlaceContext::Copy | PlaceContext::Move | PlaceContext::Borrow)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Walk a MIR body with a visitor.
pub fn walk_body<V: Visitor>(visitor: &mut V, body: &MirBody) {
    visitor.visit_body(body);
}

/// Collect all locals used in an rvalue.
pub fn collect_rvalue_locals(rvalue: &Rvalue) -> Vec<LocalId> {
    struct LocalCollector {
        locals: Vec<LocalId>,
    }

    impl Visitor for LocalCollector {
        fn visit_local(&mut self, local: LocalId, _context: PlaceContext, _location: Location) {
            self.locals.push(local);
        }
    }

    let mut collector = LocalCollector { locals: Vec::new() };
    collector.visit_rvalue(rvalue, Location::statement(BasicBlockId::new(0), 0));
    collector.locals
}

/// Collect all locals used in an operand.
pub fn collect_operand_locals(operand: &Operand) -> Vec<LocalId> {
    struct LocalCollector {
        locals: Vec<LocalId>,
    }

    impl Visitor for LocalCollector {
        fn visit_local(&mut self, local: LocalId, _context: PlaceContext, _location: Location) {
            self.locals.push(local);
        }
    }

    let mut collector = LocalCollector { locals: Vec::new() };
    collector.visit_operand(operand, Location::statement(BasicBlockId::new(0), 0));
    collector.locals
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{DefId, Type};
    use crate::span::Span;
    use super::super::body::LocalKind;

    fn dummy_def_id() -> DefId {
        DefId::new(0)
    }

    #[test]
    fn test_location_creation() {
        let stmt_loc = Location::statement(BasicBlockId::new(5), 3);
        assert_eq!(stmt_loc.block.index(), 5);
        assert_eq!(stmt_loc.statement_index, 3);
        assert!(!stmt_loc.is_terminator());

        let term_loc = Location::terminator(BasicBlockId::new(2));
        assert_eq!(term_loc.block.index(), 2);
        assert!(term_loc.is_terminator());
    }

    #[test]
    fn test_place_context() {
        assert!(PlaceContext::Store.is_mutating());
        assert!(PlaceContext::Drop.is_mutating());
        assert!(!PlaceContext::Read.is_mutating());

        assert!(PlaceContext::Copy.is_use());
        assert!(PlaceContext::Move.is_use());
        assert!(!PlaceContext::Store.is_use());
    }

    #[test]
    fn test_walk_empty_body() {
        struct BlockCounter {
            count: usize,
        }

        impl Visitor for BlockCounter {
            fn visit_basic_block(&mut self, _bb_id: BasicBlockId, _block: &BasicBlockData) {
                self.count += 1;
            }
        }

        let body = MirBody::new(dummy_def_id(), Span::dummy());
        let mut counter = BlockCounter { count: 0 };
        walk_body(&mut counter, &body);
        assert_eq!(counter.count, 0);
    }

    #[test]
    fn test_walk_body_with_blocks() {
        struct BlockCounter {
            count: usize,
        }

        impl Visitor for BlockCounter {
            fn visit_basic_block(&mut self, _bb_id: BasicBlockId, block: &BasicBlockData) {
                self.count += 1;
                self.super_basic_block(_bb_id, block);
            }
        }

        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        body.new_block();
        body.new_block();
        body.new_block();

        let mut counter = BlockCounter { count: 0 };
        walk_body(&mut counter, &body);
        assert_eq!(counter.count, 3);
    }

    #[test]
    fn test_statement_visitor() {
        struct StatementCounter {
            count: usize,
        }

        impl Visitor for StatementCounter {
            fn visit_statement(&mut self, stmt: &Statement, location: Location) {
                self.count += 1;
                self.super_statement(stmt, location);
            }
        }

        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        let temp = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        let bb = body.new_block();

        // Add some statements
        use super::super::types::{ConstantKind};
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(temp),
                Rvalue::Use(Operand::Constant(Constant::new(Type::i32(), ConstantKind::Int(42)))),
            ),
            Span::dummy(),
        ));
        body.push_statement(bb, Statement::new(
            StatementKind::StorageLive(temp),
            Span::dummy(),
        ));

        let mut counter = StatementCounter { count: 0 };
        walk_body(&mut counter, &body);
        assert_eq!(counter.count, 2);
    }

    #[test]
    fn test_collect_rvalue_locals() {
        let local1 = LocalId::new(1);
        let local2 = LocalId::new(2);

        // Binary operation with two places
        let rvalue = Rvalue::BinaryOp {
            op: BinOp::Add,
            left: Operand::Copy(Place::local(local1)),
            right: Operand::Copy(Place::local(local2)),
        };

        let locals = collect_rvalue_locals(&rvalue);
        assert_eq!(locals.len(), 2);
        assert!(locals.contains(&local1));
        assert!(locals.contains(&local2));
    }

    #[test]
    fn test_local_context_tracking() {
        struct ContextTracker {
            stores: Vec<LocalId>,
            reads: Vec<LocalId>,
        }

        impl Visitor for ContextTracker {
            fn visit_local(&mut self, local: LocalId, context: PlaceContext, _location: Location) {
                if context.is_mutating() {
                    self.stores.push(local);
                } else if context.is_use() {
                    self.reads.push(local);
                }
            }
        }

        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        let dest = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        let src = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        let bb = body.new_block();

        // Assign src to dest
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(dest),
                Rvalue::Use(Operand::Copy(Place::local(src))),
            ),
            Span::dummy(),
        ));

        let mut tracker = ContextTracker { stores: Vec::new(), reads: Vec::new() };
        walk_body(&mut tracker, &body);

        assert_eq!(tracker.stores, vec![dest]);
        assert_eq!(tracker.reads, vec![src]);
    }

    #[test]
    fn test_visitor_override() {
        struct SelectiveVisitor {
            assignments_visited: usize,
        }

        impl Visitor for SelectiveVisitor {
            fn visit_statement_kind(&mut self, kind: &StatementKind, location: Location) {
                if matches!(kind, StatementKind::Assign(..)) {
                    self.assignments_visited += 1;
                }
                // Deliberately NOT calling super_statement_kind to skip deeper traversal
                let _ = location;
            }
        }

        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_local(Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        let temp = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        let bb = body.new_block();

        use super::super::types::ConstantKind;
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(temp),
                Rvalue::Use(Operand::Constant(Constant::new(Type::i32(), ConstantKind::Int(1)))),
            ),
            Span::dummy(),
        ));
        body.push_statement(bb, Statement::new(
            StatementKind::StorageLive(temp),
            Span::dummy(),
        ));
        body.push_statement(bb, Statement::new(
            StatementKind::Assign(
                Place::local(temp),
                Rvalue::Use(Operand::Constant(Constant::new(Type::i32(), ConstantKind::Int(2)))),
            ),
            Span::dummy(),
        ));

        let mut visitor = SelectiveVisitor { assignments_visited: 0 };
        walk_body(&mut visitor, &body);
        assert_eq!(visitor.assignments_visited, 2);
    }
}
