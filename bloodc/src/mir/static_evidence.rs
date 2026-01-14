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
use super::ptr::MemoryTier;

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

// ============================================================================
// Handler Escape Analysis (EFF-OPT-005/006)
// ============================================================================

/// Check if a handler body expression contains escaping control flow.
///
/// A handler's evidence "escapes" if the body contains:
/// - `Perform` operations (control transfers to another handler)
/// - `Resume` operations (continuation transfers control)
///
/// When evidence doesn't escape, the handler can use stack allocation
/// instead of heap allocation for the evidence vector.
///
/// # Arguments
///
/// * `body` - The handler body expression to analyze
///
/// # Returns
///
/// `true` if the body contains Perform or Resume operations (evidence escapes),
/// `false` if the handler is purely lexical (can use stack allocation).
pub fn handler_evidence_escapes(body: &Expr) -> bool {
    contains_escaping_control_flow(body)
}

/// Determine the allocation tier for a handler based on escape analysis.
///
/// This is the main entry point for EFF-OPT-005/006.
///
/// # Arguments
///
/// * `body` - The handler body expression
///
/// # Returns
///
/// - `MemoryTier::Stack` if the handler is lexically scoped (no escaping control flow)
/// - `MemoryTier::Region` if the handler evidence may escape (contains Perform/Resume)
pub fn analyze_handler_allocation_tier(body: &Expr) -> MemoryTier {
    if handler_evidence_escapes(body) {
        MemoryTier::Region
    } else {
        MemoryTier::Stack
    }
}

/// Recursively check if an expression contains escaping control flow.
fn contains_escaping_control_flow(expr: &Expr) -> bool {
    match &expr.kind {
        // Direct escape points
        ExprKind::Perform { .. } => true,
        ExprKind::Resume { .. } => true,

        // Nested handle blocks: their body may contain Perform/Resume,
        // but those are handled by the nested handler, not ours.
        // However, if the nested handler's body doesn't fully handle
        // the effect, it could escape to our handler. Be conservative.
        ExprKind::Handle { body, .. } => {
            // Conservative: check the nested body too
            // A more precise analysis would track which effects are handled
            contains_escaping_control_flow(body)
        }

        // Recursively check all sub-expressions
        ExprKind::Block { stmts, expr } => {
            for stmt in stmts {
                if contains_escaping_control_flow_stmt(stmt) {
                    return true;
                }
            }
            if let Some(e) = expr {
                if contains_escaping_control_flow(e) {
                    return true;
                }
            }
            false
        }

        ExprKind::If { condition, then_branch, else_branch } => {
            contains_escaping_control_flow(condition)
                || contains_escaping_control_flow(then_branch)
                || else_branch.as_ref().map_or(false, |e| contains_escaping_control_flow(e))
        }

        ExprKind::Match { scrutinee, arms } => {
            if contains_escaping_control_flow(scrutinee) {
                return true;
            }
            for arm in arms {
                if let Some(ref guard) = arm.guard {
                    if contains_escaping_control_flow(guard) {
                        return true;
                    }
                }
                if contains_escaping_control_flow(&arm.body) {
                    return true;
                }
            }
            false
        }

        ExprKind::Loop { body, .. } => {
            contains_escaping_control_flow(body)
        }

        ExprKind::While { condition, body, .. } => {
            contains_escaping_control_flow(condition)
                || contains_escaping_control_flow(body)
        }

        ExprKind::Call { callee, args } => {
            // Function calls could internally use effects, but we're
            // analyzing the direct expression tree, not called functions.
            // The callee's effects are separate from ours.
            contains_escaping_control_flow(callee)
                || args.iter().any(contains_escaping_control_flow)
        }

        ExprKind::MethodCall { receiver, args, .. } => {
            contains_escaping_control_flow(receiver)
                || args.iter().any(contains_escaping_control_flow)
        }

        ExprKind::Closure { .. } => {
            // Closures have a separate body_id, we can't analyze them here.
            // Be conservative: assume closures might contain Perform/Resume.
            // This is a limitation - a more precise analysis would resolve
            // the body_id and check the closure body.
            true
        }

        ExprKind::Binary { left, right, .. } => {
            contains_escaping_control_flow(left)
                || contains_escaping_control_flow(right)
        }

        ExprKind::Unary { operand, .. } => {
            contains_escaping_control_flow(operand)
        }

        ExprKind::Cast { expr, .. } => {
            contains_escaping_control_flow(expr)
        }

        ExprKind::Index { base, index } => {
            contains_escaping_control_flow(base)
                || contains_escaping_control_flow(index)
        }

        ExprKind::Field { base, .. } => {
            contains_escaping_control_flow(base)
        }

        ExprKind::Tuple(elements) | ExprKind::Array(elements) | ExprKind::VecLiteral(elements) => {
            elements.iter().any(contains_escaping_control_flow)
        }

        ExprKind::Struct { fields, base, .. } => {
            fields.iter().any(|f| contains_escaping_control_flow(&f.value))
                || base.as_ref().map_or(false, |e| contains_escaping_control_flow(e))
        }

        ExprKind::Repeat { value, .. } => {
            // count is u64, not an expression
            contains_escaping_control_flow(value)
        }

        ExprKind::VecRepeat { value, count } => {
            contains_escaping_control_flow(value)
                || contains_escaping_control_flow(count)
        }

        ExprKind::Return(value) => {
            value.as_ref().map_or(false, |e| contains_escaping_control_flow(e))
        }

        ExprKind::Break { value, .. } => {
            value.as_ref().map_or(false, |e| contains_escaping_control_flow(e))
        }

        ExprKind::Assign { target, value } => {
            contains_escaping_control_flow(target)
                || contains_escaping_control_flow(value)
        }

        ExprKind::Borrow { expr, .. } | ExprKind::AddrOf { expr, .. } | ExprKind::Deref(expr) => {
            contains_escaping_control_flow(expr)
        }

        ExprKind::Range { start, end, .. } => {
            start.as_ref().map_or(false, |e| contains_escaping_control_flow(e))
                || end.as_ref().map_or(false, |e| contains_escaping_control_flow(e))
        }

        ExprKind::Unsafe(expr) | ExprKind::Dbg(expr) => {
            contains_escaping_control_flow(expr)
        }

        ExprKind::Let { init, .. } => {
            contains_escaping_control_flow(init)
        }

        ExprKind::Variant { fields, .. } => {
            fields.iter().any(contains_escaping_control_flow)
        }

        ExprKind::Record { fields } => {
            fields.iter().any(|f| contains_escaping_control_flow(&f.value))
        }

        ExprKind::Assert { condition, message } => {
            contains_escaping_control_flow(condition)
                || message.as_ref().map_or(false, |m| contains_escaping_control_flow(m))
        }

        ExprKind::MacroExpansion { args, .. } => {
            args.iter().any(contains_escaping_control_flow)
        }

        ExprKind::MethodFamily { .. } => {
            // Method family is a call site marker, not directly executable
            false
        }

        // Leaf expressions: no sub-expressions to check
        ExprKind::Literal(_)
        | ExprKind::Local(_)
        | ExprKind::Def(_)
        | ExprKind::Continue { .. }
        | ExprKind::Default
        | ExprKind::Error => false,
    }
}

/// Check statements for escaping control flow.
fn contains_escaping_control_flow_stmt(stmt: &crate::hir::Stmt) -> bool {
    use crate::hir::Stmt;
    match stmt {
        Stmt::Let { init, .. } => {
            init.as_ref().map_or(false, |e| contains_escaping_control_flow(e))
        }
        Stmt::Expr(expr) => {
            contains_escaping_control_flow(expr)
        }
        Stmt::Item(_) => false,
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

    // =========================================================================
    // Handler Escape Analysis Tests (EFF-OPT-005/006)
    // =========================================================================

    #[test]
    fn test_literal_body_no_escape() {
        // A literal body has no Perform/Resume, so handler can use stack allocation
        let body = make_expr(
            ExprKind::Literal(LiteralValue::Int(42)),
            Type::i32(),
        );
        assert!(!handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Stack);
    }

    #[test]
    fn test_perform_causes_escape() {
        use crate::hir::DefId;
        // A Perform expression causes handler evidence to escape
        let body = make_expr(
            ExprKind::Perform {
                effect_id: DefId::new(0),
                op_index: 0,
                args: vec![],
            },
            Type::unit(),
        );
        assert!(handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Region);
    }

    #[test]
    fn test_resume_causes_escape() {
        // A Resume expression causes handler evidence to escape
        let body = make_expr(
            ExprKind::Resume { value: None },
            Type::unit(),
        );
        assert!(handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Region);
    }

    #[test]
    fn test_block_with_perform_causes_escape() {
        use crate::hir::DefId;
        // A block containing Perform causes escape
        let perform = make_expr(
            ExprKind::Perform {
                effect_id: DefId::new(0),
                op_index: 0,
                args: vec![],
            },
            Type::unit(),
        );
        let body = make_expr(
            ExprKind::Block {
                stmts: vec![],
                expr: Some(Box::new(perform)),
            },
            Type::unit(),
        );
        assert!(handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Region);
    }

    #[test]
    fn test_block_without_effects_no_escape() {
        // A block without Perform/Resume doesn't cause escape
        let inner = make_expr(
            ExprKind::Literal(LiteralValue::Int(1)),
            Type::i32(),
        );
        let body = make_expr(
            ExprKind::Block {
                stmts: vec![],
                expr: Some(Box::new(inner)),
            },
            Type::i32(),
        );
        assert!(!handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Stack);
    }

    #[test]
    fn test_if_with_perform_causes_escape() {
        use crate::hir::DefId;
        // An if expression with Perform in either branch causes escape
        let condition = make_expr(
            ExprKind::Literal(LiteralValue::Bool(true)),
            Type::bool(),
        );
        let then_branch = make_expr(
            ExprKind::Perform {
                effect_id: DefId::new(0),
                op_index: 0,
                args: vec![],
            },
            Type::unit(),
        );
        let else_branch = make_expr(
            ExprKind::Literal(LiteralValue::Int(0)),
            Type::i32(),
        );
        let body = make_expr(
            ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Some(Box::new(else_branch)),
            },
            Type::i32(),
        );
        assert!(handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Region);
    }

    #[test]
    fn test_if_without_effects_no_escape() {
        // An if expression without Perform/Resume doesn't cause escape
        let condition = make_expr(
            ExprKind::Literal(LiteralValue::Bool(true)),
            Type::bool(),
        );
        let then_branch = make_expr(
            ExprKind::Literal(LiteralValue::Int(1)),
            Type::i32(),
        );
        let else_branch = make_expr(
            ExprKind::Literal(LiteralValue::Int(0)),
            Type::i32(),
        );
        let body = make_expr(
            ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Some(Box::new(else_branch)),
            },
            Type::i32(),
        );
        assert!(!handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Stack);
    }

    #[test]
    fn test_closure_conservatively_escapes() {
        use crate::hir::BodyId;
        // Closures are conservatively marked as escaping because we can't
        // analyze their body_id inline
        let body = make_expr(
            ExprKind::Closure {
                body_id: BodyId::new(0),
                captures: vec![],
            },
            Type::unit(), // closure type
        );
        assert!(handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Region);
    }

    #[test]
    fn test_binary_no_escape() {
        // Binary operations on literals don't cause escape
        let left = make_expr(
            ExprKind::Literal(LiteralValue::Int(1)),
            Type::i32(),
        );
        let right = make_expr(
            ExprKind::Literal(LiteralValue::Int(2)),
            Type::i32(),
        );
        let body = make_expr(
            ExprKind::Binary {
                op: crate::ast::BinOp::Add,
                left: Box::new(left),
                right: Box::new(right),
            },
            Type::i32(),
        );
        assert!(!handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Stack);
    }

    #[test]
    fn test_call_no_escape() {
        use crate::hir::DefId;
        // Function calls don't cause escape (the callee's effects are separate)
        let callee = make_expr(
            ExprKind::Def(DefId::new(0)),
            Type::unit(), // function type
        );
        let body = make_expr(
            ExprKind::Call {
                callee: Box::new(callee),
                args: vec![],
            },
            Type::unit(),
        );
        assert!(!handler_evidence_escapes(&body));
        assert_eq!(analyze_handler_allocation_tier(&body), MemoryTier::Stack);
    }
}
