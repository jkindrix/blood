//! Utility functions for MIR lowering.
//!
//! This module provides shared utility functions for HIR→MIR lowering.
//! These functions are used by both `FunctionLowering` and `ClosureLowering`
//! to avoid code duplication.
//!
//! ## Functions
//!
//! - [`convert_binop`]: Convert AST binary operator to MIR binary operator
//! - [`convert_unop`]: Convert AST unary operator to MIR unary operator
//! - [`lower_literal_to_constant`]: Convert HIR literal to MIR constant
//! - [`is_irrefutable_pattern`]: Check if a pattern always matches

use crate::ast::{BinOp, UnaryOp};
use crate::hir::{LiteralValue, Pattern, PatternKind, Type};
use crate::mir::types::{BinOp as MirBinOp, UnOp as MirUnOp, Constant, ConstantKind};

// ============================================================================
// Operator Conversion
// ============================================================================

/// Convert HIR binary op to MIR binary op.
pub fn convert_binop(op: BinOp) -> MirBinOp {
    match op {
        BinOp::Add => MirBinOp::Add,
        BinOp::Sub => MirBinOp::Sub,
        BinOp::Mul => MirBinOp::Mul,
        BinOp::Div => MirBinOp::Div,
        BinOp::Rem => MirBinOp::Rem,
        BinOp::BitAnd => MirBinOp::BitAnd,
        BinOp::BitOr => MirBinOp::BitOr,
        BinOp::BitXor => MirBinOp::BitXor,
        BinOp::Shl => MirBinOp::Shl,
        BinOp::Shr => MirBinOp::Shr,
        BinOp::Eq => MirBinOp::Eq,
        BinOp::Ne => MirBinOp::Ne,
        BinOp::Lt => MirBinOp::Lt,
        BinOp::Le => MirBinOp::Le,
        BinOp::Gt => MirBinOp::Gt,
        BinOp::Ge => MirBinOp::Ge,
        BinOp::And => MirBinOp::BitAnd, // Logical and
        BinOp::Or => MirBinOp::BitOr,   // Logical or
        BinOp::Pipe => MirBinOp::BitOr, // Pipe operator (placeholder)
    }
}

/// Convert HIR unary op to MIR unary op.
///
/// Returns `None` for operators that require special handling:
/// - `Deref`: Creates a dereferenced place projection
/// - `Ref`/`RefMut`: Creates a reference to a place
///
/// These operators are handled directly in the lowering code.
pub fn convert_unop(op: UnaryOp) -> Option<MirUnOp> {
    match op {
        UnaryOp::Neg => Some(MirUnOp::Neg),
        UnaryOp::Not => Some(MirUnOp::Not),
        // These require special place-based handling
        UnaryOp::Deref | UnaryOp::Ref | UnaryOp::RefMut => None,
    }
}

// ============================================================================
// Literal Conversion
// ============================================================================

/// Convert a literal value to a MIR constant.
///
/// This is a pure utility function used during expression lowering
/// to convert HIR literal values into MIR constants.
pub fn lower_literal_to_constant(lit: &LiteralValue, ty: &Type) -> Constant {
    let kind = match lit {
        LiteralValue::Int(v) => ConstantKind::Int(*v),
        LiteralValue::Uint(v) => ConstantKind::Int(*v as i128),
        LiteralValue::Float(v) => ConstantKind::Float(*v),
        LiteralValue::Bool(v) => ConstantKind::Bool(*v),
        LiteralValue::Char(v) => ConstantKind::Char(*v),
        LiteralValue::String(v) => ConstantKind::String(v.clone()),
    };
    Constant::new(ty.clone(), kind)
}

// ============================================================================
// Pattern Analysis
// ============================================================================

/// Check if a pattern is irrefutable (always matches).
///
/// An irrefutable pattern is one that will match any value of its type.
/// This includes:
/// - Wildcard patterns (`_`)
/// - Simple bindings (`x`)
/// - Tuple patterns with all irrefutable sub-patterns
/// - Reference patterns with irrefutable inner patterns
/// - Struct patterns with all irrefutable field patterns
/// - Slice patterns with a rest element (`..`)
///
/// Refutable patterns (which may not match) include:
/// - Literal patterns (`1`, `"hello"`)
/// - Or patterns (`a | b`)
/// - Variant patterns (`Some(x)`)
/// - Range patterns (`1..10`)
pub fn is_irrefutable_pattern(pattern: &Pattern) -> bool {
    match &pattern.kind {
        PatternKind::Wildcard => true,
        PatternKind::Binding { subpattern, .. } => {
            subpattern.as_ref().map_or(true, |p| is_irrefutable_pattern(p))
        }
        PatternKind::Tuple(pats) => pats.iter().all(is_irrefutable_pattern),
        PatternKind::Ref { inner, .. } => is_irrefutable_pattern(inner),
        // These patterns are refutable (may not match)
        PatternKind::Literal(_) => false,
        PatternKind::Or(_) => false,
        PatternKind::Variant { .. } => false,
        PatternKind::Range { .. } => false,
        // Struct patterns are irrefutable if all field patterns are irrefutable
        PatternKind::Struct { fields, .. } => {
            fields.iter().all(|f| is_irrefutable_pattern(&f.pattern))
        }
        // Slice patterns with a rest element (..) are irrefutable
        PatternKind::Slice { prefix, slice, suffix } => {
            slice.is_some() &&
            prefix.iter().all(is_irrefutable_pattern) &&
            suffix.iter().all(is_irrefutable_pattern)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{DefId, FieldPattern, LocalId as HirLocalId};
    use crate::span::Span;

    #[test]
    fn test_convert_binop() {
        assert_eq!(convert_binop(BinOp::Add), MirBinOp::Add);
        assert_eq!(convert_binop(BinOp::Sub), MirBinOp::Sub);
        assert_eq!(convert_binop(BinOp::Eq), MirBinOp::Eq);
        assert_eq!(convert_binop(BinOp::And), MirBinOp::BitAnd);
        assert_eq!(convert_binop(BinOp::Or), MirBinOp::BitOr);
    }

    #[test]
    fn test_convert_unop() {
        assert_eq!(convert_unop(UnaryOp::Neg), Some(MirUnOp::Neg));
        assert_eq!(convert_unop(UnaryOp::Not), Some(MirUnOp::Not));
        // Special operators return None
        assert_eq!(convert_unop(UnaryOp::Deref), None);
        assert_eq!(convert_unop(UnaryOp::Ref), None);
        assert_eq!(convert_unop(UnaryOp::RefMut), None);
    }

    #[test]
    fn test_lower_literal_to_constant() {
        let int_lit = LiteralValue::Int(42);
        let int_const = lower_literal_to_constant(&int_lit, &Type::i64());
        assert!(matches!(int_const.kind, ConstantKind::Int(42)));

        let bool_lit = LiteralValue::Bool(true);
        let bool_const = lower_literal_to_constant(&bool_lit, &Type::bool());
        assert!(matches!(bool_const.kind, ConstantKind::Bool(true)));

        let string_lit = LiteralValue::String("hello".to_string());
        let string_const = lower_literal_to_constant(&string_lit, &Type::string());
        assert!(matches!(string_const.kind, ConstantKind::String(ref s) if s == "hello"));
    }

    fn make_pattern(kind: PatternKind) -> Pattern {
        Pattern {
            kind,
            ty: Type::i64(),
            span: Span::dummy(),
        }
    }

    #[test]
    fn test_is_irrefutable_wildcard() {
        let pat = make_pattern(PatternKind::Wildcard);
        assert!(is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_binding() {
        // Simple binding is irrefutable
        let pat = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(1),
            mutable: false,
            subpattern: None,
        });
        assert!(is_irrefutable_pattern(&pat));

        // Binding with irrefutable subpattern is irrefutable
        let subpat = Box::new(make_pattern(PatternKind::Wildcard));
        let pat = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(2),
            mutable: false,
            subpattern: Some(subpat),
        });
        assert!(is_irrefutable_pattern(&pat));

        // Binding with refutable subpattern is refutable
        let subpat = Box::new(make_pattern(PatternKind::Literal(LiteralValue::Int(42))));
        let pat = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(3),
            mutable: false,
            subpattern: Some(subpat),
        });
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_tuple() {
        // Empty tuple is irrefutable
        let pat = make_pattern(PatternKind::Tuple(vec![]));
        assert!(is_irrefutable_pattern(&pat));

        // Tuple with all irrefutable patterns is irrefutable
        let pat = make_pattern(PatternKind::Tuple(vec![
            make_pattern(PatternKind::Wildcard),
            make_pattern(PatternKind::Binding {
                local_id: HirLocalId::new(1),
                mutable: false,
                subpattern: None,
            }),
        ]));
        assert!(is_irrefutable_pattern(&pat));

        // Tuple with any refutable pattern is refutable
        let pat = make_pattern(PatternKind::Tuple(vec![
            make_pattern(PatternKind::Wildcard),
            make_pattern(PatternKind::Literal(LiteralValue::Int(42))),
        ]));
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_refutable_literal() {
        let pat = make_pattern(PatternKind::Literal(LiteralValue::Int(42)));
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_refutable_variant() {
        let pat = make_pattern(PatternKind::Variant {
            def_id: DefId::new(1),
            variant_idx: 0,
            fields: vec![],
        });
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_struct() {
        // Struct with all irrefutable field patterns is irrefutable
        let pat = make_pattern(PatternKind::Struct {
            def_id: DefId::new(1),
            fields: vec![
                FieldPattern {
                    field_idx: 0,
                    pattern: make_pattern(PatternKind::Wildcard),
                },
            ],
        });
        assert!(is_irrefutable_pattern(&pat));

        // Struct with refutable field pattern is refutable
        let pat = make_pattern(PatternKind::Struct {
            def_id: DefId::new(1),
            fields: vec![
                FieldPattern {
                    field_idx: 0,
                    pattern: make_pattern(PatternKind::Literal(LiteralValue::Int(42))),
                },
            ],
        });
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_slice() {
        // Slice with rest element is irrefutable
        let pat = make_pattern(PatternKind::Slice {
            prefix: vec![make_pattern(PatternKind::Wildcard)],
            slice: Some(Box::new(make_pattern(PatternKind::Wildcard))),
            suffix: vec![],
        });
        assert!(is_irrefutable_pattern(&pat));

        // Slice without rest element is refutable (must match exact length)
        let pat = make_pattern(PatternKind::Slice {
            prefix: vec![make_pattern(PatternKind::Wildcard)],
            slice: None,
            suffix: vec![],
        });
        assert!(!is_irrefutable_pattern(&pat));
    }
}
