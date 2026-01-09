//! # Handler Compilation
//!
//! Implements effect handler compilation and continuation capture.
//!
//! ## Handler Kinds
//!
//! Blood supports two kinds of handlers as specified in SPECIFICATION.md §2.9:
//!
//! - **Deep handlers**: Persist across resumes, can be multi-shot
//! - **Shallow handlers**: Consumed on resume, always single-shot
//!
//! ## Continuation Capture
//!
//! Blood uses **segmented stacks** for continuation capture, following the
//! strategy outlined in ROADMAP.md §13.4:
//!
//! - Fibers use segmented/cactus stacks
//! - Capture = save current segment
//! - Resume = restore segment
//!
//! ## Tail-Resumptive Optimization
//!
//! When a handler operation immediately resumes (tail-resumptive), no
//! continuation capture is needed. This is the common case for State,
//! Reader, and Writer effects.

use super::row::EffectRow;
use crate::hir::{DefId, Expr, Type};

/// The kind of effect handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandlerKind {
    /// Deep handler - persists across resumes, can be multi-shot.
    #[default]
    Deep,
    /// Shallow handler - consumed on resume, single-shot only.
    Shallow,
}

/// A compiled handler definition.
#[derive(Debug, Clone)]
pub struct Handler {
    /// The handler definition ID.
    pub def_id: DefId,
    /// The effect being handled.
    pub effect: DefId,
    /// The kind of handler.
    pub kind: HandlerKind,
    /// State variables for the handler.
    pub state: Vec<HandlerState>,
    /// Operation implementations.
    pub operations: Vec<OperationImpl>,
    /// Return clause (transforms the final result).
    pub return_clause: Option<ReturnClause>,
}

/// Handler state variable.
#[derive(Debug, Clone)]
pub struct HandlerState {
    /// The state variable name.
    pub name: String,
    /// The state type.
    pub ty: Type,
    /// Initial value expression.
    pub init: Option<Expr>,
}

/// An operation implementation in a handler.
#[derive(Debug, Clone)]
pub struct OperationImpl {
    /// The operation being implemented.
    pub operation: DefId,
    /// Parameter bindings.
    pub params: Vec<String>,
    /// The implementation body.
    pub body: Expr,
    /// Whether this operation is tail-resumptive.
    pub is_tail_resumptive: bool,
}

/// Return clause for transforming the final result.
#[derive(Debug, Clone)]
pub struct ReturnClause {
    /// The result parameter name.
    pub param: String,
    /// The transformation body.
    pub body: Expr,
}

/// A captured continuation.
///
/// Represents the suspended computation that can be resumed.
#[derive(Debug, Clone)]
pub struct Continuation {
    /// Unique continuation ID.
    pub id: u64,
    /// The effect row at capture time.
    pub effect_row: EffectRow,
    /// Whether this continuation has been consumed (for linearity checking).
    pub consumed: bool,
    /// The handler depth when captured.
    pub handler_depth: usize,
}

impl Continuation {
    /// Create a new continuation.
    pub fn new(id: u64, effect_row: EffectRow, handler_depth: usize) -> Self {
        Self {
            id,
            effect_row,
            consumed: false,
            handler_depth,
        }
    }

    /// Mark this continuation as consumed.
    pub fn consume(&mut self) {
        self.consumed = true;
    }

    /// Check if this continuation can be resumed.
    pub fn can_resume(&self) -> bool {
        !self.consumed
    }
}

/// Resumption mode for continuations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeMode {
    /// Tail resumption - no capture needed.
    Tail,
    /// Direct resumption - returns to handler.
    Direct,
    /// Multi-shot resumption - continuation may be used multiple times.
    MultiShot,
}

/// Analyze an operation to determine if it's tail-resumptive.
///
/// An operation is tail-resumptive if `resume` is called in tail position
/// with no further computation. This enables significant optimization.
pub fn analyze_tail_resumptive(body: &Expr) -> bool {
    // TODO: Implement proper tail-position analysis
    // For now, conservatively return false
    matches!(body, Expr { .. } if false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handler_kind_default() {
        assert_eq!(HandlerKind::default(), HandlerKind::Deep);
    }

    #[test]
    fn test_continuation_consume() {
        let mut cont = Continuation::new(1, EffectRow::pure(), 0);
        assert!(cont.can_resume());

        cont.consume();
        assert!(!cont.can_resume());
    }

    #[test]
    fn test_continuation_creation() {
        let cont = Continuation::new(42, EffectRow::pure(), 2);
        assert_eq!(cont.id, 42);
        assert_eq!(cont.handler_depth, 2);
        assert!(!cont.consumed);
    }
}
