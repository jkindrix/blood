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
///
/// Based on: [Effect Handlers, Evidently](https://dl.acm.org/doi/10.1145/3408981) (ICFP 2020)
/// "Tail-resumptive operations can execute _in-place_ (instead of yielding to the handler)"
///
/// Performance: Up to 150 million operations/second on Core i7@2.6GHz
/// See: [Implementing Algebraic Effects in C](https://www.microsoft.com/en-us/research/publication/implementing-algebraic-effects-in-c/)
pub fn analyze_tail_resumptive(body: &Expr) -> bool {
    is_tail_resume(body)
}

/// Check if an expression is a resume in tail position.
///
/// Tail position means the resume is the last thing executed with no
/// computation happening after it returns.
fn is_tail_resume(expr: &Expr) -> bool {
    use crate::hir::ExprKind;

    match &expr.kind {
        // Direct resume in tail position
        ExprKind::Resume { .. } => true,

        // Block: tail position is the trailing expression
        ExprKind::Block { expr: Some(tail), .. } => is_tail_resume(tail),
        ExprKind::Block { expr: None, .. } => false,

        // If: both branches must be tail-resumptive
        ExprKind::If { then_branch, else_branch: Some(else_br), .. } => {
            is_tail_resume(then_branch) && is_tail_resume(else_br)
        }
        ExprKind::If { else_branch: None, .. } => false,

        // Match: all arms must be tail-resumptive
        ExprKind::Match { arms, .. } => {
            arms.iter().all(|arm| is_tail_resume(&arm.body))
        }

        // Return with resume value
        ExprKind::Return(Some(inner)) => is_tail_resume(inner),

        // All other expressions are not tail-resumptive
        _ => false,
    }
}

/// Analyze a handler to determine if all operations are tail-resumptive.
///
/// A handler is fully tail-resumptive if every operation implementation
/// ends with a resume in tail position. Such handlers can be compiled
/// without any continuation capture.
pub fn analyze_handler_tail_resumptive(operations: &[OperationImpl]) -> bool {
    operations.iter().all(|op| op.is_tail_resumptive)
}

/// Analyze the resume mode for an operation.
///
/// This determines how the continuation should be captured and resumed.
pub fn analyze_resume_mode(body: &Expr, handler_kind: HandlerKind) -> ResumeMode {
    if is_tail_resume(body) {
        ResumeMode::Tail
    } else if handler_kind == HandlerKind::Deep {
        // Deep handlers may be multi-shot
        if contains_multiple_resumes(body) {
            ResumeMode::MultiShot
        } else {
            ResumeMode::Direct
        }
    } else {
        // Shallow handlers are always single-shot
        ResumeMode::Direct
    }
}

/// Check if an expression contains multiple resume calls.
///
/// This is used to detect multi-shot handlers.
fn contains_multiple_resumes(expr: &Expr) -> bool {
    count_resumes(expr) > 1
}

/// Count the number of resume expressions in a tree.
///
/// Public wrapper for use in effect lowering.
pub fn count_resumes_in_expr(expr: &Expr) -> usize {
    count_resumes(expr)
}

/// Count the number of resume expressions in a tree.
fn count_resumes(expr: &Expr) -> usize {
    use crate::hir::ExprKind;

    match &expr.kind {
        ExprKind::Resume { value } => {
            1 + value.as_ref().map(|v| count_resumes(v)).unwrap_or(0)
        }
        ExprKind::Block { stmts, expr: tail } => {
            let stmt_count: usize = stmts.iter().map(|s| match s {
                crate::hir::Stmt::Expr(e) => count_resumes(e),
                crate::hir::Stmt::Let { init: Some(e), .. } => count_resumes(e),
                _ => 0,
            }).sum();
            stmt_count + tail.as_ref().map(|e| count_resumes(e)).unwrap_or(0)
        }
        ExprKind::If { condition, then_branch, else_branch } => {
            count_resumes(condition)
                + count_resumes(then_branch)
                + else_branch.as_ref().map(|e| count_resumes(e)).unwrap_or(0)
        }
        ExprKind::Match { scrutinee, arms } => {
            count_resumes(scrutinee)
                + arms.iter().map(|a| count_resumes(&a.body)).sum::<usize>()
        }
        ExprKind::Binary { left, right, .. } => {
            count_resumes(left) + count_resumes(right)
        }
        ExprKind::Unary { operand, .. } => count_resumes(operand),
        ExprKind::Call { callee, args } => {
            count_resumes(callee) + args.iter().map(count_resumes).sum::<usize>()
        }
        ExprKind::Tuple(exprs) | ExprKind::Array(exprs) => {
            exprs.iter().map(count_resumes).sum()
        }
        ExprKind::Loop { body, .. } | ExprKind::While { body, .. } => {
            count_resumes(body)
        }
        ExprKind::Return(Some(e)) => count_resumes(e),
        ExprKind::Assign { value, .. } => count_resumes(value),
        ExprKind::Handle { body, .. } => count_resumes(body),
        ExprKind::Perform { args, .. } => args.iter().map(count_resumes).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{ExprKind, LiteralValue};
    use crate::span::Span;

    fn make_resume(value: Option<Expr>) -> Expr {
        Expr {
            kind: ExprKind::Resume { value: value.map(Box::new) },
            ty: Type::unit(),
            span: Span::dummy(),
        }
    }

    fn make_literal() -> Expr {
        Expr {
            kind: ExprKind::Literal(LiteralValue::Int(42)),
            ty: Type::i32(),
            span: Span::dummy(),
        }
    }

    fn make_block(stmts: Vec<crate::hir::Stmt>, tail: Option<Expr>) -> Expr {
        Expr {
            kind: ExprKind::Block {
                stmts,
                expr: tail.map(Box::new),
            },
            ty: Type::unit(),
            span: Span::dummy(),
        }
    }

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

    // ========================================================================
    // Tail-Resumptive Analysis Tests
    // ========================================================================

    #[test]
    fn test_direct_resume_is_tail_resumptive() {
        let resume = make_resume(Some(make_literal()));
        assert!(analyze_tail_resumptive(&resume));
    }

    #[test]
    fn test_resume_no_value_is_tail_resumptive() {
        let resume = make_resume(None);
        assert!(analyze_tail_resumptive(&resume));
    }

    #[test]
    fn test_literal_not_tail_resumptive() {
        let lit = make_literal();
        assert!(!analyze_tail_resumptive(&lit));
    }

    #[test]
    fn test_block_with_resume_tail_is_tail_resumptive() {
        let resume = make_resume(Some(make_literal()));
        let block = make_block(vec![], Some(resume));
        assert!(analyze_tail_resumptive(&block));
    }

    #[test]
    fn test_block_without_tail_not_tail_resumptive() {
        let block = make_block(vec![], None);
        assert!(!analyze_tail_resumptive(&block));
    }

    #[test]
    fn test_count_resumes_single() {
        let resume = make_resume(Some(make_literal()));
        assert_eq!(count_resumes(&resume), 1);
    }

    #[test]
    fn test_count_resumes_zero() {
        let lit = make_literal();
        assert_eq!(count_resumes(&lit), 0);
    }

    #[test]
    fn test_analyze_resume_mode_tail() {
        let resume = make_resume(Some(make_literal()));
        assert_eq!(
            analyze_resume_mode(&resume, HandlerKind::Deep),
            ResumeMode::Tail
        );
    }

    #[test]
    fn test_analyze_resume_mode_direct() {
        let lit = make_literal();
        assert_eq!(
            analyze_resume_mode(&lit, HandlerKind::Deep),
            ResumeMode::Direct
        );
    }

    #[test]
    fn test_analyze_handler_all_tail_resumptive() {
        let ops = vec![
            OperationImpl {
                operation: crate::hir::DefId::new(1),
                params: vec![],
                body: make_resume(None),
                is_tail_resumptive: true,
            },
            OperationImpl {
                operation: crate::hir::DefId::new(2),
                params: vec![],
                body: make_resume(None),
                is_tail_resumptive: true,
            },
        ];
        assert!(analyze_handler_tail_resumptive(&ops));
    }

    #[test]
    fn test_analyze_handler_not_all_tail_resumptive() {
        let ops = vec![
            OperationImpl {
                operation: crate::hir::DefId::new(1),
                params: vec![],
                body: make_resume(None),
                is_tail_resumptive: true,
            },
            OperationImpl {
                operation: crate::hir::DefId::new(2),
                params: vec![],
                body: make_literal(),
                is_tail_resumptive: false,
            },
        ];
        assert!(!analyze_handler_tail_resumptive(&ops));
    }
}
