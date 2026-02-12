//! Legacy HIR codegen tests.
//!
//! **DEPRECATED**: These tests use the old `compile_crate()` HIR path which does
//! NOT emit generation checks or use escape analysis. They are retained for
//! backwards compatibility but should NOT be extended.
//!
//! **For production-path testing, use `mir_codegen::tests`** which tests:
//! - MIR lowering
//! - Escape analysis integration
//! - Generation validation on dereference
//! - Tier-based allocation
//!
//! Production code path: `compile_definition_to_object()` → `compile_mir_body()`

#![allow(deprecated)] // These tests use compile_crate (HIR path) for backwards compatibility

use super::*;
use crate::hir::{self, Crate, Item, ItemKind, Body, BodyId, Type, Expr, ExprKind, LiteralValue, Local, LocalId, DefId};
use crate::span::Span;
use std::collections::HashMap;

/// Helper to create a simple HIR crate for testing.
fn make_test_crate(body_expr: Expr, return_type: Type) -> Crate {
    let def_id = DefId::new(0);
    let body_id = BodyId::new(0);

    let fn_sig = hir::FnSig {
        inputs: Vec::new(),
        output: return_type.clone(),
        is_const: false,
        is_async: false,
        is_unsafe: false,
        generics: Vec::new(),
        const_generics: Vec::new(),
    };

    let fn_def = hir::FnDef {
        sig: fn_sig,
        body_id: Some(body_id),
        generics: hir::Generics {
            params: Vec::new(),
            predicates: Vec::new(),
        },
    };

    let item = Item {
        name: "test_fn".to_string(),
        kind: ItemKind::Fn(fn_def),
        def_id,
        span: Span::dummy(),
        vis: crate::ast::Visibility::Public,
    };

    // Create the return place local (index 0)
    let return_local = Local {
        id: LocalId::new(0),
        ty: return_type.clone(),
        mutable: false,
        name: None,
        span: Span::dummy(),
    };

    let body = Body {
        locals: vec![return_local],
        param_count: 0,
        expr: body_expr,
        span: Span::dummy(),
        tuple_destructures: HashMap::new(),
    };

    let mut items = HashMap::new();
    items.insert(def_id, item);

    let mut bodies = HashMap::new();
    bodies.insert(body_id, body);

    Crate {
        items,
        bodies,
        entry: None,
        builtin_fns: HashMap::new(),
    }
}

fn i32_type() -> Type {
    Type::i32()
}

fn f64_type() -> Type {
    Type::f64()
}

fn bool_type() -> Type {
    Type::bool()
}

fn unit_type() -> Type {
    Type::unit()
}

fn int_literal(val: i128) -> Expr {
    Expr {
        kind: ExprKind::Literal(LiteralValue::Int(val)),
        ty: i32_type(),
        span: Span::dummy(),
    }
}

fn float_literal(val: f64) -> Expr {
    Expr {
        kind: ExprKind::Literal(LiteralValue::Float(val)),
        ty: f64_type(),
        span: Span::dummy(),
    }
}

fn bool_literal(val: bool) -> Expr {
    Expr {
        kind: ExprKind::Literal(LiteralValue::Bool(val)),
        ty: bool_type(),
        span: Span::dummy(),
    }
}

fn binary_expr(op: crate::ast::BinOp, left: Expr, right: Expr, result_ty: Type) -> Expr {
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty: result_ty,
        span: Span::dummy(),
    }
}

fn unary_expr(op: crate::ast::UnaryOp, operand: Expr, result_ty: Type) -> Expr {
    Expr {
        kind: ExprKind::Unary {
            op,
            operand: Box::new(operand),
        },
        ty: result_ty,
        span: Span::dummy(),
    }
}

// ==================== LITERAL TESTS ====================

#[test]
fn test_codegen_int_literal() {
    let expr = int_literal(42);
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Integer literal codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_float_literal() {
    let expr = float_literal(2.5);
    let hir_crate = make_test_crate(expr, f64_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Float literal codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_bool_literal() {
    let expr = bool_literal(true);
    let hir_crate = make_test_crate(expr, bool_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Bool literal codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_string_literal() {
    let expr = Expr {
        kind: ExprKind::Literal(LiteralValue::String("hello".to_string())),
        ty: Type::str(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, Type::str());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "String literal codegen failed: {:?}", result.err());
}

// ==================== BINARY OPERATION TESTS ====================

#[test]
fn test_codegen_int_add() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Add, int_literal(1), int_literal(2), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Int add codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_int_sub() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Sub, int_literal(5), int_literal(3), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Int sub codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_int_mul() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Mul, int_literal(4), int_literal(5), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Int mul codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_int_div() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Div, int_literal(10), int_literal(2), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Int div codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_int_compare() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Lt, int_literal(1), int_literal(2), bool_type());
    let hir_crate = make_test_crate(expr, bool_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Int compare codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_float_add() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Add, float_literal(1.5), float_literal(2.5), f64_type());
    let hir_crate = make_test_crate(expr, f64_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Float add codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_float_mul() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Mul, float_literal(2.0), float_literal(3.0), f64_type());
    let hir_crate = make_test_crate(expr, f64_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Float mul codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_float_compare() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Gt, float_literal(2.5), float_literal(2.71), bool_type());
    let hir_crate = make_test_crate(expr, bool_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Float compare codegen failed: {:?}", result.err());
}

// ==================== UNARY OPERATION TESTS ====================

#[test]
fn test_codegen_int_neg() {
    use crate::ast::UnaryOp;
    let expr = unary_expr(UnaryOp::Neg, int_literal(42), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Int neg codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_float_neg() {
    use crate::ast::UnaryOp;
    let expr = unary_expr(UnaryOp::Neg, float_literal(2.5), f64_type());
    let hir_crate = make_test_crate(expr, f64_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Float neg codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_int_not() {
    use crate::ast::UnaryOp;
    let expr = unary_expr(UnaryOp::Not, int_literal(0xFF), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Int not codegen failed: {:?}", result.err());
}

// ==================== CONTROL FLOW TESTS ====================

#[test]
fn test_codegen_if_expr() {
    let condition = bool_literal(true);
    let then_branch = int_literal(1);
    let else_branch = int_literal(0);

    let expr = Expr {
        kind: ExprKind::If {
            condition: Box::new(condition),
            then_branch: Box::new(then_branch),
            else_branch: Some(Box::new(else_branch)),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "If expr codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_while_loop() {
    let condition = bool_literal(false); // Loop that never executes
    let body = Expr {
        kind: ExprKind::Tuple(Vec::new()),
        ty: unit_type(),
        span: Span::dummy(),
    };

    let while_expr = Expr {
        kind: ExprKind::While {
            condition: Box::new(condition),
            body: Box::new(body),
            label: None,
        },
        ty: unit_type(),
        span: Span::dummy(),
    };

    let block_expr = Expr {
        kind: ExprKind::Block {
            stmts: Vec::new(),
            expr: Some(Box::new(while_expr)),
        },
        ty: unit_type(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(block_expr, unit_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "While loop codegen failed: {:?}", result.err());
}

// ==================== BLOCK AND LET TESTS ====================

#[test]
fn test_codegen_block_with_let() {
    let init_expr = int_literal(42);
    let local_id = LocalId { index: 0 };

    let let_stmt = hir::Stmt::Let {
        local_id,
        init: Some(init_expr),
    };

    let load_expr = Expr {
        kind: ExprKind::Local(local_id),
        ty: i32_type(),
        span: Span::dummy(),
    };

    let block_expr = Expr {
        kind: ExprKind::Block {
            stmts: vec![let_stmt],
            expr: Some(Box::new(load_expr)),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(block_expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Block with let codegen failed: {:?}", result.err());
}

// ==================== TUPLE TESTS ====================

#[test]
fn test_codegen_tuple_empty() {
    let expr = Expr {
        kind: ExprKind::Tuple(Vec::new()),
        ty: unit_type(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(expr, unit_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Empty tuple codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_tuple_with_values() {
    let tuple_ty = Type::tuple(vec![i32_type(), bool_type()]);

    let expr = Expr {
        kind: ExprKind::Tuple(vec![int_literal(42), bool_literal(true)]),
        ty: tuple_ty.clone(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(expr, tuple_ty);

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Tuple with values codegen failed: {:?}", result.err());
}

// ==================== ARRAY TESTS ====================

#[test]
fn test_codegen_array_literal() {
    let arr_ty = Type::array(i32_type(), 3);

    let expr = Expr {
        kind: ExprKind::Array(vec![int_literal(1), int_literal(2), int_literal(3)]),
        ty: arr_ty.clone(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(expr, arr_ty);

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Array literal codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_array_empty() {
    let arr_ty = Type::array(i32_type(), 0);

    let expr = Expr {
        kind: ExprKind::Array(Vec::new()),
        ty: arr_ty.clone(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(expr, arr_ty);

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Empty array codegen failed: {:?}", result.err());
}

// ==================== TYPE LOWERING TESTS ====================

#[test]
fn test_lower_primitive_types() {
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let codegen = CodegenContext::new(&context, &module, &builder);

    // Test various primitive types
    let i32_t = codegen.lower_type(&i32_type());
    assert!(i32_t.is_int_type());

    let f64_t = codegen.lower_type(&f64_type());
    assert!(f64_t.is_float_type());

    let bool_t = codegen.lower_type(&bool_type());
    assert!(bool_t.is_int_type()); // bool is i1

    let unit_t = codegen.lower_type(&unit_type());
    assert!(unit_t.is_int_type()); // unit is i8 placeholder
}

#[test]
fn test_lower_tuple_type() {
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let codegen = CodegenContext::new(&context, &module, &builder);

    let tuple_ty = Type::tuple(vec![i32_type(), f64_type()]);

    let lowered = codegen.lower_type(&tuple_ty);
    assert!(lowered.is_struct_type());
}

#[test]
fn test_lower_array_type() {
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let codegen = CodegenContext::new(&context, &module, &builder);

    let arr_ty = Type::array(i32_type(), 5);

    let lowered = codegen.lower_type(&arr_ty);
    assert!(lowered.is_array_type());
}

// ==================== COMPLEX EXPRESSION TESTS ====================

#[test]
fn test_codegen_nested_binary_ops() {
    use crate::ast::BinOp;
    // (1 + 2) * 3
    let add_expr = binary_expr(BinOp::Add, int_literal(1), int_literal(2), i32_type());
    let mul_expr = binary_expr(BinOp::Mul, add_expr, int_literal(3), i32_type());

    let hir_crate = make_test_crate(mul_expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Nested binary ops codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_nested_if() {
    let inner_if = Expr {
        kind: ExprKind::If {
            condition: Box::new(bool_literal(true)),
            then_branch: Box::new(int_literal(1)),
            else_branch: Some(Box::new(int_literal(2))),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };

    let outer_if = Expr {
        kind: ExprKind::If {
            condition: Box::new(bool_literal(false)),
            then_branch: Box::new(int_literal(0)),
            else_branch: Some(Box::new(inner_if)),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };

    let hir_crate = make_test_crate(outer_if, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Nested if codegen failed: {:?}", result.err());
}

// ==================== BITWISE OPERATION TESTS ====================

#[test]
fn test_codegen_bitwise_and() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::BitAnd, int_literal(0xFF), int_literal(0x0F), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Bitwise AND codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_bitwise_or() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::BitOr, int_literal(0xF0), int_literal(0x0F), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Bitwise OR codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_shift_left() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Shl, int_literal(1), int_literal(4), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Shift left codegen failed: {:?}", result.err());
}

#[test]
fn test_codegen_shift_right() {
    use crate::ast::BinOp;
    let expr = binary_expr(BinOp::Shr, int_literal(16), int_literal(2), i32_type());
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Shift right codegen failed: {:?}", result.err());
}

// ========================================================================
// Effects System Codegen Tests
// ========================================================================

/// Test perform expression generates blood_perform runtime call
///
/// With the evidence passing model (ICFP'21), handler lookup is deferred to runtime.
/// Compilation succeeds and emits a call to blood_perform, which will dispatch
/// to the appropriate handler at runtime.
#[test]
fn test_codegen_perform_basic() {
    let effect_id = DefId::new(100);
    let expr = Expr {
        kind: ExprKind::Perform {
            effect_id,
            op_index: 0,
            args: vec![int_literal(42)],
            type_args: vec![],
        },
        ty: i32_type(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    // With runtime dispatch, compilation succeeds - handler lookup happens at runtime
    assert!(result.is_ok(), "Perform codegen should succeed: {:?}", result.err());

    // Verify blood_perform function is declared
    assert!(module.get_function("blood_perform").is_some(),
        "blood_perform should be declared");
}

/// Test perform with no arguments generates correct runtime call
#[test]
fn test_codegen_perform_no_args() {
    let effect_id = DefId::new(101);
    let expr = Expr {
        kind: ExprKind::Perform {
            effect_id,
            op_index: 1,
            args: vec![],
            type_args: vec![],
        },
        ty: unit_type(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, unit_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    // With runtime dispatch, compilation succeeds even without handlers
    assert!(result.is_ok(), "Perform codegen should succeed: {:?}", result.err());
}

/// Test resume expression (tail-resumptive)
#[test]
fn test_codegen_resume_with_value() {
    let expr = Expr {
        kind: ExprKind::Resume {
            value: Some(Box::new(int_literal(42))),
        },
        ty: Type::never(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, unit_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Resume codegen failed: {:?}", result.err());
}

/// Test resume without value (unit resume)
#[test]
fn test_codegen_resume_unit() {
    let expr = Expr {
        kind: ExprKind::Resume { value: None },
        ty: Type::never(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, unit_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Resume unit codegen failed: {:?}", result.err());
}

/// Test handle expression wraps body
#[test]
fn test_codegen_handle_basic() {
    let handler_id = DefId::new(200);
    let body = int_literal(42);
    // Create a placeholder handler instance (empty struct instantiation)
    let handler_instance = Expr {
        kind: ExprKind::Tuple(Vec::new()),
        ty: unit_type(),
        span: Span::dummy(),
    };
    let expr = Expr {
        kind: ExprKind::Handle {
            body: Box::new(body),
            handler_id,
            handler_instance: Box::new(handler_instance),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, i32_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Handle codegen failed: {:?}", result.err());
}

/// Test handle with unit body
#[test]
fn test_codegen_handle_unit() {
    let handler_id = DefId::new(201);
    let body = Expr {
        kind: ExprKind::Tuple(Vec::new()),
        ty: unit_type(),
        span: Span::dummy(),
    };
    // Create a placeholder handler instance
    let handler_instance = Expr {
        kind: ExprKind::Tuple(Vec::new()),
        ty: unit_type(),
        span: Span::dummy(),
    };
    let expr = Expr {
        kind: ExprKind::Handle {
            body: Box::new(body),
            handler_id,
            handler_instance: Box::new(handler_instance),
        },
        ty: unit_type(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, unit_type());

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    let result = codegen.compile_crate(&hir_crate);

    assert!(result.is_ok(), "Handle unit codegen failed: {:?}", result.err());
}
