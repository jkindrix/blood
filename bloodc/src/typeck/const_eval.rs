//! Compile-time constant evaluation.
//!
//! This module implements constant evaluation for expressions that must
//! be known at compile time, such as array sizes.
//!
//! Supported expressions:
//! - Integer and boolean literals
//! - Simple binary operations (+, -, *, /, %, comparisons)
//! - Unary operations (-, !)
//! - Parenthesized expressions
//!
//! Future work:
//! - Const items and const functions
//! - More complex const generics

use crate::ast::{self, BinOp, UnaryOp};
use crate::span::Span;
use crate::typeck::{TypeError, TypeErrorKind};

/// Result of const evaluation - an integer value.
#[derive(Debug, Clone, Copy)]
pub enum ConstResult {
    /// A signed integer value.
    Int(i128),
    /// An unsigned integer value.
    Uint(u128),
    /// A boolean value.
    Bool(bool),
}

impl ConstResult {
    /// Try to get as u64 for array sizes.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            ConstResult::Int(v) if *v >= 0 && *v <= u64::MAX as i128 => Some(*v as u64),
            ConstResult::Uint(v) if *v <= u64::MAX as u128 => Some(*v as u64),
            _ => None,
        }
    }

    /// Get the signed integer value.
    pub fn as_i128(&self) -> Option<i128> {
        match self {
            ConstResult::Int(v) => Some(*v),
            ConstResult::Uint(v) if *v <= i128::MAX as u128 => Some(*v as i128),
            _ => None,
        }
    }
}

/// Evaluate an AST expression as a compile-time constant.
///
/// Returns `Ok(value)` if the expression can be evaluated at compile time,
/// or an error if it cannot.
pub fn eval_const_expr(expr: &ast::Expr) -> Result<ConstResult, TypeError> {
    match &expr.kind {
        ast::ExprKind::Literal(lit) => eval_literal(lit, expr.span),

        ast::ExprKind::Binary { left, op, right } => {
            let left_val = eval_const_expr(left)?;
            let right_val = eval_const_expr(right)?;
            eval_binary_op(*op, left_val, right_val, expr.span)
        }

        ast::ExprKind::Unary { op, operand } => {
            let val = eval_const_expr(operand)?;
            eval_unary_op(*op, val, expr.span)
        }

        ast::ExprKind::Paren(inner) => eval_const_expr(inner),

        ast::ExprKind::If { condition, then_branch, else_branch } => {
            let cond = eval_const_expr(condition)?;
            match cond {
                ConstResult::Bool(true) => eval_block(then_branch),
                ConstResult::Bool(false) => {
                    if let Some(else_expr) = else_branch {
                        eval_else_branch(else_expr)
                    } else {
                        Err(TypeError::new(
                            TypeErrorKind::ConstEvalError {
                                reason: "if expression without else branch in const context".to_string(),
                            },
                            expr.span,
                        ))
                    }
                }
                _ => Err(TypeError::new(
                    TypeErrorKind::ConstEvalError {
                        reason: "condition must be a boolean".to_string(),
                    },
                    condition.span,
                )),
            }
        }

        ast::ExprKind::Block(block) => {
            // Only pure expression blocks without statements are allowed
            eval_block(block)
        }

        _ => Err(TypeError::new(
            TypeErrorKind::ConstEvalError {
                reason: "expression cannot be evaluated at compile time".to_string(),
            },
            expr.span,
        )),
    }
}

/// Evaluate a block expression.
fn eval_block(block: &ast::Block) -> Result<ConstResult, TypeError> {
    if block.statements.is_empty() {
        if let Some(final_expr) = &block.expr {
            eval_const_expr(final_expr)
        } else {
            Err(TypeError::new(
                TypeErrorKind::ConstEvalError {
                    reason: "empty block in const context".to_string(),
                },
                block.span,
            ))
        }
    } else {
        Err(TypeError::new(
            TypeErrorKind::ConstEvalError {
                reason: "const expressions cannot contain statements".to_string(),
            },
            block.span,
        ))
    }
}

/// Evaluate an else branch.
fn eval_else_branch(branch: &ast::ElseBranch) -> Result<ConstResult, TypeError> {
    match branch {
        ast::ElseBranch::Block(block) => eval_block(block),
        ast::ElseBranch::If(if_expr) => eval_const_expr(if_expr),
    }
}

/// Evaluate a literal.
fn eval_literal(lit: &ast::Literal, span: Span) -> Result<ConstResult, TypeError> {
    match &lit.kind {
        ast::LiteralKind::Int { value, suffix } => {
            // Determine if unsigned based on suffix
            let is_unsigned = matches!(
                suffix,
                Some(ast::IntSuffix::U8)
                    | Some(ast::IntSuffix::U16)
                    | Some(ast::IntSuffix::U32)
                    | Some(ast::IntSuffix::U64)
                    | Some(ast::IntSuffix::U128)
            );

            if is_unsigned {
                Ok(ConstResult::Uint(*value))
            } else {
                Ok(ConstResult::Int(*value as i128))
            }
        }
        ast::LiteralKind::Bool(b) => Ok(ConstResult::Bool(*b)),
        _ => Err(TypeError::new(
            TypeErrorKind::ConstEvalError {
                reason: "only integer and boolean literals are supported in const contexts".to_string(),
            },
            span,
        )),
    }
}

/// Evaluate a binary operation.
fn eval_binary_op(
    op: BinOp,
    left: ConstResult,
    right: ConstResult,
    span: Span,
) -> Result<ConstResult, TypeError> {
    // Convert both to i128 for simplicity
    let (l, r) = match (left.as_i128(), right.as_i128()) {
        (Some(l), Some(r)) => (l, r),
        _ => {
            return Err(TypeError::new(
                TypeErrorKind::ConstEvalError {
                    reason: "cannot perform operation on these values".to_string(),
                },
                span,
            ));
        }
    };

    let result = match op {
        BinOp::Add => l.checked_add(r),
        BinOp::Sub => l.checked_sub(r),
        BinOp::Mul => l.checked_mul(r),
        BinOp::Div => {
            if r == 0 {
                return Err(TypeError::new(
                    TypeErrorKind::ConstEvalError {
                        reason: "division by zero in const expression".to_string(),
                    },
                    span,
                ));
            }
            l.checked_div(r)
        }
        BinOp::Rem => {
            if r == 0 {
                return Err(TypeError::new(
                    TypeErrorKind::ConstEvalError {
                        reason: "remainder by zero in const expression".to_string(),
                    },
                    span,
                ));
            }
            l.checked_rem(r)
        }
        BinOp::BitAnd => Some(l & r),
        BinOp::BitOr => Some(l | r),
        BinOp::BitXor => Some(l ^ r),
        BinOp::Shl => {
            if !(0..=127).contains(&r) {
                return Err(TypeError::new(
                    TypeErrorKind::ConstEvalError {
                        reason: "shift amount out of range".to_string(),
                    },
                    span,
                ));
            }
            l.checked_shl(r as u32)
        }
        BinOp::Shr => {
            if !(0..=127).contains(&r) {
                return Err(TypeError::new(
                    TypeErrorKind::ConstEvalError {
                        reason: "shift amount out of range".to_string(),
                    },
                    span,
                ));
            }
            l.checked_shr(r as u32)
        }
        // Comparison operators return bool
        BinOp::Eq => return Ok(ConstResult::Bool(l == r)),
        BinOp::Ne => return Ok(ConstResult::Bool(l != r)),
        BinOp::Lt => return Ok(ConstResult::Bool(l < r)),
        BinOp::Le => return Ok(ConstResult::Bool(l <= r)),
        BinOp::Gt => return Ok(ConstResult::Bool(l > r)),
        BinOp::Ge => return Ok(ConstResult::Bool(l >= r)),
        // Logical operators work on bools
        BinOp::And | BinOp::Or => {
            let (lb, rb) = match (left, right) {
                (ConstResult::Bool(lb), ConstResult::Bool(rb)) => (lb, rb),
                _ => {
                    return Err(TypeError::new(
                        TypeErrorKind::ConstEvalError {
                            reason: "logical operators require boolean operands".to_string(),
                        },
                        span,
                    ));
                }
            };
            let result = match op {
                BinOp::And => lb && rb,
                BinOp::Or => lb || rb,
                _ => unreachable!(),
            };
            return Ok(ConstResult::Bool(result));
        }
        _ => {
            return Err(TypeError::new(
                TypeErrorKind::ConstEvalError {
                    reason: format!("operator {:?} not supported in const expressions", op),
                },
                span,
            ));
        }
    };

    result
        .map(ConstResult::Int)
        .ok_or_else(|| {
            TypeError::new(
                TypeErrorKind::ConstEvalError {
                    reason: "arithmetic overflow in const expression".to_string(),
                },
                span,
            )
        })
}

/// Evaluate a unary operation.
fn eval_unary_op(op: UnaryOp, val: ConstResult, span: Span) -> Result<ConstResult, TypeError> {
    match op {
        UnaryOp::Neg => {
            let v = val.as_i128().ok_or_else(|| {
                TypeError::new(
                    TypeErrorKind::ConstEvalError {
                        reason: "cannot negate this value".to_string(),
                    },
                    span,
                )
            })?;
            v.checked_neg()
                .map(ConstResult::Int)
                .ok_or_else(|| {
                    TypeError::new(
                        TypeErrorKind::ConstEvalError {
                            reason: "arithmetic overflow in const expression".to_string(),
                        },
                        span,
                    )
                })
        }
        UnaryOp::Not => match val {
            ConstResult::Bool(b) => Ok(ConstResult::Bool(!b)),
            ConstResult::Int(v) => Ok(ConstResult::Int(!v)),
            ConstResult::Uint(v) => Ok(ConstResult::Uint(!v)),
        },
        _ => Err(TypeError::new(
            TypeErrorKind::ConstEvalError {
                reason: format!("operator {:?} not supported in const expressions", op),
            },
            span,
        )),
    }
}

