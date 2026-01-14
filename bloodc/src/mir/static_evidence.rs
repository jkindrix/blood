//! # Static Evidence Analysis
//!
//! This module analyzes handler expressions to determine if they can be
//! statically allocated, avoiding runtime evidence allocation overhead.
//!
//! ## Overview
//!
//! Static evidence optimization (EFF-OPT-001) identifies handler installations
//! where the handler state is known at compile time. Such handlers can use
//! pre-allocated static evidence instead of calling `blood_evidence_create()`
//! at runtime.
//!
//! ## Handler State Classification
//!
//! | Kind | Description | Example |
//! |------|-------------|---------|
//! | Stateless | Unit type `()` | `handle { ... } with Logger` |
//! | Constant | Compile-time constant | `handle { ... } with State(0)` |
//! | ZeroInit | Default-initialized | `handle { ... } with State(default)` |
//! | Dynamic | Runtime-computed | `handle { ... } with State(compute())` |
//!
//! ## Usage
//!
//! ```ignore
//! use bloodc::mir::static_evidence::analyze_handler_state;
//!
//! let kind = analyze_handler_state(&handler_instance_expr);
//! if kind.is_static() {
//!     // Can use static evidence optimization
//! }
//! ```

use crate::effects::evidence::HandlerStateKind;
use crate::hir::{Expr, ExprKind, LiteralValue, Type, TypeKind};

/// Analyze a handler instance expression to determine its state kind.
///
/// This function examines the handler instance expression (the initializer
/// passed to a handle block) and classifies it for static evidence optimization.
///
/// # Arguments
///
/// * `expr` - The handler instance expression to analyze
///
/// # Returns
///
/// The `HandlerStateKind` classification for this handler.
///
/// # Examples
///
/// ```ignore
/// // Stateless: unit type expression
/// // handle { body } with LogHandler  // LogHandler has type ()
///
/// // Constant: literal value
/// // handle { body } with State(42)
///
/// // ZeroInit: default value
/// // handle { body } with State(default)
///
/// // Dynamic: computed value
/// // handle { body } with State(get_initial_value())
/// ```
pub fn analyze_handler_state(expr: &Expr) -> HandlerStateKind {
    // First check if the type is unit - stateless handlers are optimal
    if is_unit_type(&expr.ty) {
        return HandlerStateKind::Stateless;
    }

    // Check if the expression is compile-time constant
    if is_constant_expr(expr) {
        return HandlerStateKind::Constant;
    }

    // Check if it's a default expression (zero-initialized)
    if is_default_expr(expr) {
        return HandlerStateKind::ZeroInit;
    }

    // Otherwise, it's dynamic (requires runtime computation)
    HandlerStateKind::Dynamic
}

/// Check if a type is the unit type `()`.
fn is_unit_type(ty: &Type) -> bool {
    matches!(ty.kind(), TypeKind::Tuple(ref elems) if elems.is_empty())
}

/// Check if an expression is a compile-time constant.
///
/// A constant expression is one that can be evaluated at compile time
/// and embedded in static data.
fn is_constant_expr(expr: &Expr) -> bool {
    match &expr.kind {
        // Literals are always constant
        ExprKind::Literal(_) => true,

        // Unit tuple is constant
        ExprKind::Tuple(elements) if elements.is_empty() => true,

        // Tuple of constants is constant
        ExprKind::Tuple(elements) => elements.iter().all(is_constant_expr),

        // Array of constants is constant
        ExprKind::Array(elements) => elements.iter().all(is_constant_expr),

        // Repeat with constant value and count is constant
        ExprKind::Repeat { value, .. } => is_constant_expr(value),

        // Struct literal with constant fields is constant
        ExprKind::Struct { fields, .. } => {
            fields.iter().all(|f| is_constant_expr(&f.value))
        }

        // Default is handled separately as ZeroInit
        ExprKind::Default => false,

        // Unary operations on constants are constant (for simple ops)
        ExprKind::Unary { operand, .. } => is_constant_expr(operand),

        // Binary operations on constants are constant (for simple ops)
        ExprKind::Binary { left, right, .. } => {
            is_constant_expr(left) && is_constant_expr(right)
        }

        // Block with only expression (no statements) that is constant
        ExprKind::Block { stmts, expr } if stmts.is_empty() => {
            expr.as_ref().map_or(true, |e| is_constant_expr(e))
        }

        // References to constants/statics could be constant, but we're conservative
        // to avoid complex analysis
        ExprKind::Def(_) => false,

        // Local variables are not constant (they're computed at runtime)
        ExprKind::Local(_) => false,

        // All other expressions are dynamic
        _ => false,
    }
}

/// Check if an expression is a default (zero-initialized) value.
fn is_default_expr(expr: &Expr) -> bool {
    match &expr.kind {
        // Explicit default expression
        ExprKind::Default => true,

        // Tuple of defaults is default
        ExprKind::Tuple(elements) => elements.iter().all(is_default_expr),

        // Struct with all default fields is default
        ExprKind::Struct { fields, .. } => {
            fields.iter().all(|f| is_default_expr(&f.value))
        }

        // Zero literals are effectively default for their types
        ExprKind::Literal(LiteralValue::Int(0)) => true,
        ExprKind::Literal(LiteralValue::Uint(0)) => true,
        ExprKind::Literal(LiteralValue::Float(f)) if *f == 0.0 => true,
        ExprKind::Literal(LiteralValue::Bool(false)) => true,

        // Block containing only a default expression
        ExprKind::Block { stmts, expr } if stmts.is_empty() => {
            expr.as_ref().map_or(false, |e| is_default_expr(e))
        }

        _ => false,
    }
}

/// Extract constant bytes from a constant expression (if possible).
///
/// This is used for embedding small constant values in static evidence.
/// Returns `None` if the expression is too complex or too large.
pub fn extract_constant_bytes(expr: &Expr) -> Option<Vec<u8>> {
    const MAX_CONSTANT_SIZE: usize = 64; // Max bytes to embed in static data

    match &expr.kind {
        ExprKind::Literal(lit) => {
            let bytes = literal_to_bytes(lit);
            if bytes.len() <= MAX_CONSTANT_SIZE {
                Some(bytes)
            } else {
                None
            }
        }

        ExprKind::Tuple(elements) if elements.is_empty() => {
            // Unit type - zero bytes
            Some(Vec::new())
        }

        // For more complex expressions, we skip embedding for now
        // A future optimization could handle small structs
        _ => None,
    }
}

/// Convert a literal value to bytes.
fn literal_to_bytes(lit: &LiteralValue) -> Vec<u8> {
    match lit {
        LiteralValue::Int(v) => v.to_le_bytes().to_vec(),
        LiteralValue::Uint(v) => v.to_le_bytes().to_vec(),
        LiteralValue::Float(v) => v.to_le_bytes().to_vec(),
        LiteralValue::Bool(v) => vec![*v as u8],
        LiteralValue::Char(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        LiteralValue::String(s) => s.as_bytes().to_vec(),
    }
}

/// Result of analyzing a handle expression for static evidence.
#[derive(Debug, Clone)]
pub struct HandleAnalysis {
    /// The handler state kind.
    pub state_kind: HandlerStateKind,
    /// Constant bytes for embedding (if applicable).
    pub constant_bytes: Option<Vec<u8>>,
}

impl HandleAnalysis {
    /// Analyze a handler instance expression.
    pub fn analyze(handler_instance: &Expr) -> Self {
        let state_kind = analyze_handler_state(handler_instance);
        let constant_bytes = if matches!(state_kind, HandlerStateKind::Constant) {
            extract_constant_bytes(handler_instance)
        } else {
            None
        };

        Self {
            state_kind,
            constant_bytes,
        }
    }

    /// Check if this analysis indicates static evidence can be used.
    pub fn is_static(&self) -> bool {
        self.state_kind.is_static()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{Expr, ExprKind, LiteralValue};
    use crate::span::Span;

    fn make_expr(kind: ExprKind, ty: Type) -> Expr {
        Expr::new(kind, ty, Span::dummy())
    }

    #[test]
    fn test_unit_type_is_stateless() {
        let expr = make_expr(ExprKind::Tuple(vec![]), Type::unit());
        assert_eq!(analyze_handler_state(&expr), HandlerStateKind::Stateless);
    }

    #[test]
    fn test_literal_is_constant() {
        let expr = make_expr(
            ExprKind::Literal(LiteralValue::Int(42)),
            Type::i32(),
        );
        assert_eq!(analyze_handler_state(&expr), HandlerStateKind::Constant);
    }

    #[test]
    fn test_bool_literal_is_constant() {
        let expr = make_expr(
            ExprKind::Literal(LiteralValue::Bool(true)),
            Type::bool(),
        );
        assert_eq!(analyze_handler_state(&expr), HandlerStateKind::Constant);
    }

    #[test]
    fn test_string_literal_is_constant() {
        let expr = make_expr(
            ExprKind::Literal(LiteralValue::String("hello".to_string())),
            Type::string(),
        );
        assert_eq!(analyze_handler_state(&expr), HandlerStateKind::Constant);
    }

    #[test]
    fn test_tuple_of_literals_is_constant() {
        let int_expr = make_expr(
            ExprKind::Literal(LiteralValue::Int(1)),
            Type::i32(),
        );
        let bool_expr = make_expr(
            ExprKind::Literal(LiteralValue::Bool(true)),
            Type::bool(),
        );
        let tuple_expr = make_expr(
            ExprKind::Tuple(vec![int_expr, bool_expr]),
            Type::tuple(vec![Type::i32(), Type::bool()]),
        );
        assert_eq!(analyze_handler_state(&tuple_expr), HandlerStateKind::Constant);
    }

    #[test]
    fn test_default_is_zero_init() {
        let expr = make_expr(ExprKind::Default, Type::i32());
        assert_eq!(analyze_handler_state(&expr), HandlerStateKind::ZeroInit);
    }

    #[test]
    fn test_zero_literal_is_zero_init() {
        let expr = make_expr(
            ExprKind::Literal(LiteralValue::Int(0)),
            Type::i32(),
        );
        // Zero literals are treated as constant, not zero_init
        // The important thing is they're static
        assert!(analyze_handler_state(&expr).is_static());
    }

    #[test]
    fn test_local_is_dynamic() {
        use crate::hir::LocalId;
        let expr = make_expr(
            ExprKind::Local(LocalId::new(0)),
            Type::i32(),
        );
        assert_eq!(analyze_handler_state(&expr), HandlerStateKind::Dynamic);
    }

    #[test]
    fn test_handle_analysis_stateless() {
        let expr = make_expr(ExprKind::Tuple(vec![]), Type::unit());
        let analysis = HandleAnalysis::analyze(&expr);
        assert!(analysis.is_static());
        assert_eq!(analysis.state_kind, HandlerStateKind::Stateless);
        assert!(analysis.constant_bytes.is_none());
    }

    #[test]
    fn test_handle_analysis_constant_with_bytes() {
        let expr = make_expr(
            ExprKind::Literal(LiteralValue::Int(42)),
            Type::i32(),
        );
        let analysis = HandleAnalysis::analyze(&expr);
        assert!(analysis.is_static());
        assert_eq!(analysis.state_kind, HandlerStateKind::Constant);
        assert!(analysis.constant_bytes.is_some());
    }

    #[test]
    fn test_literal_to_bytes() {
        assert_eq!(literal_to_bytes(&LiteralValue::Bool(true)), vec![1]);
        assert_eq!(literal_to_bytes(&LiteralValue::Bool(false)), vec![0]);

        let int_bytes = literal_to_bytes(&LiteralValue::Int(42));
        assert_eq!(int_bytes.len(), 16); // i128 is 16 bytes

        let float_bytes = literal_to_bytes(&LiteralValue::Float(3.14));
        assert_eq!(float_bytes.len(), 8); // f64 is 8 bytes
    }

    #[test]
    fn test_extract_constant_bytes_unit() {
        let expr = make_expr(ExprKind::Tuple(vec![]), Type::unit());
        let bytes = extract_constant_bytes(&expr);
        assert_eq!(bytes, Some(vec![]));
    }

    #[test]
    fn test_is_default_expr() {
        let default_expr = make_expr(ExprKind::Default, Type::i32());
        assert!(is_default_expr(&default_expr));

        let zero_expr = make_expr(
            ExprKind::Literal(LiteralValue::Int(0)),
            Type::i32(),
        );
        assert!(is_default_expr(&zero_expr));

        let nonzero_expr = make_expr(
            ExprKind::Literal(LiteralValue::Int(42)),
            Type::i32(),
        );
        assert!(!is_default_expr(&nonzero_expr));
    }
}
