//! # MIR Function Bodies
//!
//! This module defines the structure of MIR function bodies.
//!
//! ## Body Structure
//!
//! A MIR body contains:
//! - **Locals**: All local variables (return place, parameters, temporaries)
//! - **Basic Blocks**: The control-flow graph
//! - **Metadata**: Span information, source scopes
//!
//! ## Local Convention
//!
//! Following [Rust MIR](https://rustc-dev-guide.rust-lang.org/mir/index.html):
//! - `_0`: Return place
//! - `_1..=_n`: Parameters (where n = param_count)
//! - `_(n+1)..`: Temporaries and user variables

use std::collections::HashMap;

use crate::hir::{DefId, LocalId, Type};
use crate::span::Span;
use super::types::{BasicBlockData, BasicBlockId, Terminator, Statement};

#[cfg(test)]
use super::types::TerminatorKind;

/// A MIR function body.
///
/// This is the main data structure representing a function in MIR form.
#[derive(Debug, Clone)]
pub struct MirBody {
    /// The definition ID of this function.
    pub def_id: DefId,
    /// All local variables (return place at index 0, then params, then temps).
    pub locals: Vec<MirLocal>,
    /// Number of parameters (not counting return place).
    pub param_count: usize,
    /// The basic blocks of the CFG.
    pub basic_blocks: Vec<BasicBlockData>,
    /// Source span for the function.
    pub span: Span,
    /// Effect row for this function (from HIR).
    pub effect_row: Option<crate::effects::EffectRow>,
    /// Spread of the function body for error reporting.
    pub source_scopes: Vec<SourceScope>,
}

impl MirBody {
    /// Create a new MIR body.
    pub fn new(def_id: DefId, span: Span) -> Self {
        Self {
            def_id,
            locals: Vec::new(),
            param_count: 0,
            basic_blocks: Vec::new(),
            span,
            effect_row: None,
            source_scopes: Vec::new(),
        }
    }

    /// Get the return place (always local 0).
    pub fn return_place(&self) -> LocalId {
        LocalId::new(0)
    }

    /// Get the return type.
    pub fn return_type(&self) -> &Type {
        &self.locals[0].ty
    }

    /// Iterate over parameter locals.
    pub fn params(&self) -> impl Iterator<Item = &MirLocal> {
        self.locals.iter().skip(1).take(self.param_count)
    }

    /// Iterate over parameter local IDs.
    pub fn param_ids(&self) -> impl Iterator<Item = LocalId> {
        (1..=self.param_count).map(|i| LocalId::new(i as u32))
    }

    /// Get a local by ID.
    pub fn get_local(&self, id: LocalId) -> Option<&MirLocal> {
        self.locals.get(id.index as usize)
    }

    /// Allocate a new local and return its ID.
    pub fn new_local(&mut self, ty: Type, kind: LocalKind, span: Span) -> LocalId {
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(MirLocal {
            id,
            ty,
            kind,
            mutable: false,
            name: None,
            span,
        });
        id
    }

    /// Allocate a new temporary local.
    pub fn new_temp(&mut self, ty: Type, span: Span) -> LocalId {
        self.new_local(ty, LocalKind::Temp, span)
    }

    /// Get a basic block by ID.
    pub fn get_block(&self, id: BasicBlockId) -> Option<&BasicBlockData> {
        self.basic_blocks.get(id.index())
    }

    /// Get a mutable reference to a basic block.
    pub fn get_block_mut(&mut self, id: BasicBlockId) -> Option<&mut BasicBlockData> {
        self.basic_blocks.get_mut(id.index())
    }

    /// Allocate a new basic block and return its ID.
    pub fn new_block(&mut self) -> BasicBlockId {
        let id = BasicBlockId::new(self.basic_blocks.len() as u32);
        self.basic_blocks.push(BasicBlockData::new());
        id
    }

    /// Get the entry block.
    pub fn entry_block(&self) -> &BasicBlockData {
        &self.basic_blocks[0]
    }

    /// Get all block IDs.
    pub fn block_ids(&self) -> impl Iterator<Item = BasicBlockId> {
        (0..self.basic_blocks.len()).map(|i| BasicBlockId::new(i as u32))
    }

    /// Iterate over all blocks.
    pub fn blocks(&self) -> impl Iterator<Item = (BasicBlockId, &BasicBlockData)> {
        self.basic_blocks
            .iter()
            .enumerate()
            .map(|(i, bb)| (BasicBlockId::new(i as u32), bb))
    }

    /// Check if all blocks are terminated.
    pub fn is_complete(&self) -> bool {
        self.basic_blocks.iter().all(|bb| bb.is_terminated())
    }

    /// Add a statement to a block.
    pub fn push_statement(&mut self, block: BasicBlockId, stmt: Statement) {
        if let Some(bb) = self.basic_blocks.get_mut(block.index()) {
            bb.statements.push(stmt);
        }
    }

    /// Set the terminator for a block.
    pub fn set_terminator(&mut self, block: BasicBlockId, term: Terminator) {
        if let Some(bb) = self.basic_blocks.get_mut(block.index()) {
            bb.terminator = Some(term);
        }
    }

    /// Compute predecessors for all blocks.
    pub fn predecessors(&self) -> HashMap<BasicBlockId, Vec<BasicBlockId>> {
        let mut preds: HashMap<_, Vec<_>> = HashMap::new();

        // Initialize all blocks with empty predecessor lists
        for id in self.block_ids() {
            preds.insert(id, Vec::new());
        }

        // Populate predecessors from successors
        for (id, block) in self.blocks() {
            for succ in block.successors() {
                preds.entry(succ).or_default().push(id);
            }
        }

        preds
    }

    /// Check if a block is reachable from entry.
    pub fn is_reachable(&self, target: BasicBlockId) -> bool {
        let mut visited = vec![false; self.basic_blocks.len()];
        let mut worklist = vec![BasicBlockId::ENTRY];

        while let Some(bb) = worklist.pop() {
            if bb == target {
                return true;
            }
            if visited[bb.index()] {
                continue;
            }
            visited[bb.index()] = true;

            if let Some(block) = self.get_block(bb) {
                for succ in block.successors() {
                    if !visited[succ.index()] {
                        worklist.push(succ);
                    }
                }
            }
        }

        false
    }

    /// Get blocks in reverse postorder (for dataflow analysis).
    pub fn reverse_postorder(&self) -> Vec<BasicBlockId> {
        let mut visited = vec![false; self.basic_blocks.len()];
        let mut postorder = Vec::new();

        fn visit(
            body: &MirBody,
            bb: BasicBlockId,
            visited: &mut [bool],
            postorder: &mut Vec<BasicBlockId>,
        ) {
            if visited[bb.index()] {
                return;
            }
            visited[bb.index()] = true;

            if let Some(block) = body.get_block(bb) {
                for succ in block.successors() {
                    visit(body, succ, visited, postorder);
                }
            }

            postorder.push(bb);
        }

        visit(self, BasicBlockId::ENTRY, &mut visited, &mut postorder);
        postorder.reverse();
        postorder
    }
}

/// A local variable in MIR.
#[derive(Debug, Clone)]
pub struct MirLocal {
    /// The local ID.
    pub id: LocalId,
    /// The type of this local.
    pub ty: Type,
    /// The kind of local (return, param, temp, user).
    pub kind: LocalKind,
    /// Whether this local is mutable.
    pub mutable: bool,
    /// The name (for debugging/error messages).
    pub name: Option<String>,
    /// Source span where this local was declared.
    pub span: Span,
}

impl MirLocal {
    /// Create a new MIR local.
    pub fn new(id: LocalId, ty: Type, kind: LocalKind, span: Span) -> Self {
        Self {
            id,
            ty,
            kind,
            mutable: false,
            name: None,
            span,
        }
    }

    /// Check if this is the return place.
    pub fn is_return_place(&self) -> bool {
        self.id.index == 0
    }

    /// Check if this is a parameter.
    pub fn is_param(&self) -> bool {
        matches!(self.kind, LocalKind::Arg)
    }

    /// Check if this is a temporary.
    pub fn is_temp(&self) -> bool {
        matches!(self.kind, LocalKind::Temp)
    }
}

/// The kind of a local variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// Return place (always index 0).
    ReturnPlace,
    /// Function argument.
    Arg,
    /// User-declared variable.
    Var,
    /// Compiler-generated temporary.
    Temp,
}

/// Source scope information for debugging.
#[derive(Debug, Clone)]
pub struct SourceScope {
    /// Parent scope (if any).
    pub parent: Option<u32>,
    /// Span of this scope.
    pub span: Span,
}

// ============================================================================
// Builder helpers
// ============================================================================

/// Builder for constructing MIR bodies.
pub struct MirBodyBuilder {
    body: MirBody,
    current_block: BasicBlockId,
}

impl MirBodyBuilder {
    /// Create a new builder.
    pub fn new(def_id: DefId, span: Span) -> Self {
        let mut body = MirBody::new(def_id, span);
        let entry = body.new_block();
        Self {
            body,
            current_block: entry,
        }
    }

    /// Set the return type (creates return place).
    pub fn set_return_type(&mut self, ty: Type) {
        let local = MirLocal::new(LocalId::new(0), ty, LocalKind::ReturnPlace, self.body.span);
        if self.body.locals.is_empty() {
            self.body.locals.push(local);
        } else {
            self.body.locals[0] = local;
        }
    }

    /// Add a parameter.
    pub fn add_param(&mut self, name: Option<String>, ty: Type, span: Span) -> LocalId {
        let id = LocalId::new(self.body.locals.len() as u32);
        let mut local = MirLocal::new(id, ty, LocalKind::Arg, span);
        local.name = name;
        self.body.locals.push(local);
        self.body.param_count += 1;
        id
    }

    /// Create a new temporary.
    pub fn new_temp(&mut self, ty: Type, span: Span) -> LocalId {
        self.body.new_temp(ty, span)
    }

    /// Update a local's type.
    ///
    /// This is used when a local is assigned a value of a more specific type,
    /// such as when a closure is assigned to a local (Closure type vs Fn type).
    pub fn set_local_type(&mut self, local: LocalId, ty: Type) {
        if let Some(mir_local) = self.body.locals.get_mut(local.index() as usize) {
            mir_local.ty = ty;
        }
    }

    /// Get a local's type.
    pub fn get_local_type(&self, local: LocalId) -> Option<&Type> {
        self.body.locals.get(local.index() as usize).map(|l| &l.ty)
    }

    /// Create a new basic block.
    pub fn new_block(&mut self) -> BasicBlockId {
        self.body.new_block()
    }

    /// Get the current block.
    pub fn current_block(&self) -> BasicBlockId {
        self.current_block
    }

    /// Switch to a different block.
    pub fn switch_to(&mut self, block: BasicBlockId) {
        self.current_block = block;
    }

    /// Push a statement to the current block.
    pub fn push_stmt(&mut self, stmt: Statement) {
        self.body.push_statement(self.current_block, stmt);
    }

    /// Terminate the current block.
    pub fn terminate(&mut self, term: Terminator) {
        self.body.set_terminator(self.current_block, term);
    }

    /// Check if the current block is terminated.
    pub fn is_current_terminated(&self) -> bool {
        self.body.get_block(self.current_block)
            .map(|b| b.is_terminated())
            .unwrap_or(false)
    }

    /// Finish building and return the body.
    pub fn finish(self) -> MirBody {
        self.body
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn dummy_def_id() -> DefId {
        DefId::new(0)
    }

    #[test]
    fn test_mir_body_new() {
        let body = MirBody::new(dummy_def_id(), Span::dummy());
        assert!(body.locals.is_empty());
        assert!(body.basic_blocks.is_empty());
        assert_eq!(body.param_count, 0);
    }

    #[test]
    fn test_mir_body_new_local() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        let id = body.new_local(Type::i32(), LocalKind::Temp, Span::dummy());
        assert_eq!(id.index, 0);
        assert_eq!(body.locals.len(), 1);
    }

    #[test]
    fn test_mir_body_new_block() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        let bb0 = body.new_block();
        let bb1 = body.new_block();
        assert_eq!(bb0.0, 0);
        assert_eq!(bb1.0, 1);
        assert_eq!(body.basic_blocks.len(), 2);
    }

    #[test]
    fn test_mir_body_is_complete() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        body.new_block();

        assert!(!body.is_complete());

        body.set_terminator(
            BasicBlockId::new(0),
            Terminator::new(TerminatorKind::Return, Span::dummy()),
        );

        assert!(body.is_complete());
    }

    #[test]
    fn test_mir_body_predecessors() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        let bb0 = body.new_block();
        let bb1 = body.new_block();
        let bb2 = body.new_block();

        // bb0 -> bb1, bb0 -> bb2, bb1 -> bb2
        body.set_terminator(
            bb0,
            Terminator::new(
                TerminatorKind::SwitchInt {
                    discr: super::super::types::Operand::Constant(
                        super::super::types::Constant::bool(true),
                    ),
                    targets: super::super::types::SwitchTargets::new(
                        vec![(0, bb1)],
                        bb2,
                    ),
                },
                Span::dummy(),
            ),
        );
        body.set_terminator(
            bb1,
            Terminator::new(TerminatorKind::Goto { target: bb2 }, Span::dummy()),
        );
        body.set_terminator(
            bb2,
            Terminator::new(TerminatorKind::Return, Span::dummy()),
        );

        let preds = body.predecessors();
        assert!(preds[&bb0].is_empty());
        assert_eq!(preds[&bb1], vec![bb0]);
        assert!(preds[&bb2].contains(&bb0));
        assert!(preds[&bb2].contains(&bb1));
    }

    #[test]
    fn test_mir_body_is_reachable() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        let bb0 = body.new_block();
        let bb1 = body.new_block();
        let _bb2 = body.new_block(); // unreachable

        body.set_terminator(
            bb0,
            Terminator::new(TerminatorKind::Goto { target: bb1 }, Span::dummy()),
        );
        body.set_terminator(
            bb1,
            Terminator::new(TerminatorKind::Return, Span::dummy()),
        );

        assert!(body.is_reachable(bb0));
        assert!(body.is_reachable(bb1));
        // bb2 has no incoming edges and is not the entry, so it's unreachable
        // but our is_reachable starts from ENTRY which is bb0
    }

    #[test]
    fn test_mir_local_kind() {
        let local = MirLocal::new(LocalId::new(0), Type::i32(), LocalKind::ReturnPlace, Span::dummy());
        assert!(local.is_return_place());
        assert!(!local.is_param());
        assert!(!local.is_temp());

        let param = MirLocal::new(LocalId::new(1), Type::i32(), LocalKind::Arg, Span::dummy());
        assert!(!param.is_return_place());
        assert!(param.is_param());

        let temp = MirLocal::new(LocalId::new(2), Type::i32(), LocalKind::Temp, Span::dummy());
        assert!(temp.is_temp());
    }

    #[test]
    fn test_mir_body_builder() {
        let mut builder = MirBodyBuilder::new(dummy_def_id(), Span::dummy());

        builder.set_return_type(Type::i32());
        let p1 = builder.add_param(Some("x".into()), Type::i32(), Span::dummy());

        assert_eq!(p1.index, 1);
        assert_eq!(builder.body.param_count, 1);

        let body = builder.finish();
        assert_eq!(body.locals.len(), 2);
        assert_eq!(body.basic_blocks.len(), 1);
    }

    #[test]
    fn test_reverse_postorder() {
        let mut body = MirBody::new(dummy_def_id(), Span::dummy());
        let bb0 = body.new_block();
        let bb1 = body.new_block();
        let bb2 = body.new_block();

        // bb0 -> bb1 -> bb2
        body.set_terminator(
            bb0,
            Terminator::new(TerminatorKind::Goto { target: bb1 }, Span::dummy()),
        );
        body.set_terminator(
            bb1,
            Terminator::new(TerminatorKind::Goto { target: bb2 }, Span::dummy()),
        );
        body.set_terminator(
            bb2,
            Terminator::new(TerminatorKind::Return, Span::dummy()),
        );

        let rpo = body.reverse_postorder();
        assert_eq!(rpo, vec![bb0, bb1, bb2]);
    }
}
