//! Expression code generation helpers.
//!
//! This module provides utilities for expression compilation.
//! The main expression compilation is in CodegenContext.

use crate::hir::{Expr, ExprKind};

/// Check if an expression has side effects.
pub fn has_side_effects(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Literal(_) | ExprKind::Local(_) | ExprKind::Def(_) => false,
        ExprKind::Binary { left, right, .. } => {
            has_side_effects(left) || has_side_effects(right)
        }
        ExprKind::Unary { operand, .. } => has_side_effects(operand),
        ExprKind::Call { .. } => true,
        ExprKind::Assign { .. } => true,
        ExprKind::Block { stmts, expr } | ExprKind::Region { stmts, expr, .. } => {
            !stmts.is_empty() || expr.as_ref().map(|e| has_side_effects(e)).unwrap_or(false)
        }
        _ => true,
    }
}

/// Check if an expression is a constant that can be evaluated at compile time.
pub fn is_const(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Literal(_) => true,
        ExprKind::Binary { left, right, .. } => is_const(left) && is_const(right),
        ExprKind::Unary { operand, .. } => is_const(operand),
        _ => false,
    }
}
