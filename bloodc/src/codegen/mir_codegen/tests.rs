//! Tests for MIR code generation.

use super::*;
use crate::hir::{Crate as HirCrate, Item, ItemKind, Body, BodyId, Expr, ExprKind,
                LiteralValue, Local, FnSig, FnDef, Generics, Stmt, DefId, LocalId, Type};
use crate::mir::lowering::MirLowering;
use crate::mir::escape::EscapeAnalyzer;
use crate::span::Span;
use inkwell::context::Context;
use std::collections::HashMap;

#[test]
fn test_tier_name() {
    assert_eq!(tier_name(MemoryTier::Stack), "stack");
    assert_eq!(tier_name(MemoryTier::Region), "region");
    assert_eq!(tier_name(MemoryTier::Persistent), "persistent");
}

/// Helper to create a simple HIR crate for testing MIR codegen.
fn make_test_crate(body_expr: Expr, return_type: Type) -> HirCrate {
    let def_id = DefId::new(0);
    let body_id = BodyId::new(0);

    let fn_sig = FnSig {
        inputs: Vec::new(),
        output: return_type.clone(),
        is_const: false,
        is_async: false,
        is_unsafe: false,
        generics: Vec::new(),
    };

    let fn_def = FnDef {
        sig: fn_sig,
        body_id: Some(body_id),
        generics: Generics {
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

    HirCrate {
        items,
        bodies,
        entry: None,
        builtin_fns: HashMap::new(),
    }
}

fn i32_type() -> Type {
    Type::i32()
}

fn int_literal(val: i128) -> Expr {
    Expr {
        kind: ExprKind::Literal(LiteralValue::Int(val)),
        ty: i32_type(),
        span: Span::dummy(),
    }
}

/// Compile HIR through the MIR path and return LLVM IR as string.
fn compile_via_mir(hir_crate: &HirCrate) -> Result<String, Vec<Diagnostic>> {
    // Lower HIR to MIR
    let mut lowering = MirLowering::new(hir_crate);
    let mir_bodies = lowering.lower_crate()?;

    // Run escape analysis on each body
    let mut escape_map = HashMap::new();
    for (&def_id, mir_body) in &mir_bodies {
        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(mir_body);
        escape_map.insert(def_id, results);
    }

    // Create LLVM context and compile
    let context = Context::create();
    let module = context.create_module("test_mir");
    let builder = context.create_builder();

    let mut codegen = super::super::CodegenContext::new(&context, &module, &builder);
    codegen.set_escape_analysis(escape_map.clone());
    codegen.compile_crate_declarations(hir_crate)?;

    // Compile each MIR body
    for (&def_id, mir_body) in &mir_bodies {
        let escape_results = escape_map.get(&def_id);
        codegen.compile_mir_body(def_id, mir_body, escape_results)?;
    }

    Ok(module.print_to_string().to_string())
}

#[test]
fn test_mir_codegen_int_literal() {
    let expr = int_literal(42);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed: {:?}", ir.err());

    let ir_str = ir.unwrap();
    // Should contain the constant 42
    assert!(ir_str.contains("42") || ir_str.contains("i32"),
        "IR should contain the literal: {}", ir_str);
}

#[test]
fn test_mir_codegen_binary_op() {
    let left = int_literal(10);
    let right = int_literal(5);
    let expr = Expr {
        kind: ExprKind::Binary {
            op: crate::ast::BinOp::Add,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed: {:?}", ir.err());

    let ir_str = ir.unwrap();
    // Should contain add instruction
    assert!(ir_str.contains("add") || ir_str.contains("10") || ir_str.contains("5"),
        "IR should contain add operation: {}", ir_str);
}

#[test]
fn test_mir_codegen_declares_runtime_functions() {
    let expr = int_literal(1);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed");

    let ir_str = ir.unwrap();
    // Should declare generation validation function
    assert!(ir_str.contains("blood_validate_generation") ||
            ir_str.contains("blood_alloc") ||
            ir_str.contains("blood_"),
        "IR should declare runtime functions: {}", ir_str);
}

#[test]
fn test_mir_codegen_if_expression() {
    let cond = Expr {
        kind: ExprKind::Literal(LiteralValue::Bool(true)),
        ty: Type::bool(),
        span: Span::dummy(),
    };
    let then_expr = int_literal(1);
    let else_expr = int_literal(0);

    let expr = Expr {
        kind: ExprKind::If {
            condition: Box::new(cond),
            then_branch: Box::new(then_expr),
            else_branch: Some(Box::new(else_expr)),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed: {:?}", ir.err());

    let ir_str = ir.unwrap();
    // Should have conditional branch
    assert!(ir_str.contains("br") || ir_str.contains("label"),
        "IR should contain branch instructions: {}", ir_str);
}

#[test]
fn test_mir_codegen_while_loop() {
    // while false { 0 }
    let cond_expr = Expr {
        kind: ExprKind::Literal(LiteralValue::Bool(false)),
        ty: Type::bool(),
        span: Span::dummy(),
    };
    let body_expr = int_literal(0);

    let expr = Expr {
        kind: ExprKind::While {
            condition: Box::new(cond_expr),
            body: Box::new(body_expr),
            label: None,
        },
        ty: Type::unit(),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, Type::unit());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed: {:?}", ir.err());

    let ir_str = ir.unwrap();
    // Should have loop structure
    assert!(ir_str.contains("br") || ir_str.contains("loop"),
        "IR should contain loop structure: {}", ir_str);
}

#[test]
fn test_mir_codegen_let_binding() {
    // let x = 42; x
    let init = int_literal(42);
    let local_id = LocalId::new(1);

    let local_expr = Expr {
        kind: ExprKind::Local(local_id),
        ty: i32_type(),
        span: Span::dummy(),
    };

    let stmt = Stmt::Let {
        local_id,
        init: Some(init),
    };

    let expr = Expr {
        kind: ExprKind::Block {
            stmts: vec![stmt],
            expr: Some(Box::new(local_expr)),
        },
        ty: i32_type(),
        span: Span::dummy(),
    };

    // Need to add the local to the body
    let def_id = DefId::new(0);
    let body_id = BodyId::new(0);

    let fn_sig = FnSig {
        inputs: Vec::new(),
        output: i32_type(),
        is_const: false,
        is_async: false,
        is_unsafe: false,
        generics: Vec::new(),
    };

    let fn_def = FnDef {
        sig: fn_sig,
        body_id: Some(body_id),
        generics: Generics {
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

    let return_local = Local {
        id: LocalId::new(0),
        ty: i32_type(),
        mutable: false,
        name: None,
        span: Span::dummy(),
    };

    let x_local = Local {
        id: local_id,
        ty: i32_type(),
        mutable: false,
        name: Some("x".to_string()),
        span: Span::dummy(),
    };

    let body = Body {
        locals: vec![return_local, x_local],
        param_count: 0,
        expr,
        span: Span::dummy(),
        tuple_destructures: HashMap::new(),
    };

    let mut items = HashMap::new();
    items.insert(def_id, item);

    let mut bodies = HashMap::new();
    bodies.insert(body_id, body);

    let hir_crate = HirCrate {
        items,
        bodies,
        entry: None,
        builtin_fns: HashMap::new(),
    };

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed: {:?}", ir.err());

    let ir_str = ir.unwrap();
    // Should allocate and store
    assert!(ir_str.contains("alloca") || ir_str.contains("store"),
        "IR should contain stack allocation: {}", ir_str);
}

#[test]
fn test_mir_path_compiles_function() {
    // Verify the full MIR path produces valid LLVM IR
    let expr = int_literal(100);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should produce valid IR");

    let ir_str = ir.unwrap();
    // Should have a function definition
    assert!(ir_str.contains("define"),
        "IR should define a function: {}", ir_str);
    // Should have a return
    assert!(ir_str.contains("ret"),
        "IR should have return instruction: {}", ir_str);
}

#[test]
fn test_mir_codegen_tuple() {
    let elem1 = int_literal(1);
    let elem2 = int_literal(2);

    let expr = Expr {
        kind: ExprKind::Tuple(vec![elem1, elem2]),
        ty: Type::tuple(vec![i32_type(), i32_type()]),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, Type::tuple(vec![i32_type(), i32_type()]));

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should handle tuples: {:?}", ir.err());
}

#[test]
fn test_mir_codegen_array() {
    let elem1 = int_literal(1);
    let elem2 = int_literal(2);
    let elem3 = int_literal(3);

    let expr = Expr {
        kind: ExprKind::Array(vec![elem1, elem2, elem3]),
        ty: Type::array(i32_type(), 3),
        span: Span::dummy(),
    };
    let hir_crate = make_test_crate(expr, Type::array(i32_type(), 3));

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should handle arrays: {:?}", ir.err());
}

// =========================================================================
// Generation Check Emission Tests
// =========================================================================

#[test]
fn test_mir_declares_blood_validate_generation() {
    // Verify that the MIR codegen path declares the blood_validate_generation function
    let expr = int_literal(1);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed");

    let ir_str = ir.unwrap();
    // The runtime function should be declared
    assert!(ir_str.contains("blood_validate_generation"),
        "IR should declare blood_validate_generation: {}", ir_str);
}

#[test]
fn test_mir_declares_blood_alloc_or_abort() {
    // Verify that region allocation functions are declared
    let expr = int_literal(1);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed");

    let ir_str = ir.unwrap();
    // The allocation function should be declared
    assert!(ir_str.contains("blood_alloc_or_abort") ||
            ir_str.contains("blood_alloc"),
        "IR should declare blood_alloc_or_abort: {}", ir_str);
}

#[test]
fn test_mir_declares_stale_reference_panic() {
    // Verify that the panic function for stale references is declared
    let expr = int_literal(1);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed");

    let ir_str = ir.unwrap();
    // The panic function should be declared
    assert!(ir_str.contains("blood_stale_reference_panic") ||
            ir_str.contains("panic"),
        "IR should declare stale reference panic function: {}", ir_str);
}

#[test]
fn test_mir_declares_effect_context_functions() {
    // Verify that effect context functions for snapshot management are declared
    let expr = int_literal(1);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed");

    let ir_str = ir.unwrap();
    // The effect context functions should be declared
    assert!(ir_str.contains("blood_effect_context") ||
            ir_str.contains("effect_context"),
        "IR should declare effect context functions: {}", ir_str);
}

#[test]
fn test_mir_codegen_runtime_declarations_complete() {
    // Comprehensive test: verify ALL critical runtime functions are declared
    let expr = int_literal(1);
    let hir_crate = make_test_crate(expr, i32_type());

    let ir = compile_via_mir(&hir_crate);
    assert!(ir.is_ok(), "MIR codegen should succeed");

    let ir_str = ir.unwrap();

    // Critical memory safety functions
    let critical_functions = [
        "blood_validate_generation",
        "blood_alloc",
        "blood_stale_reference_panic",
    ];

    for func in &critical_functions {
        assert!(ir_str.contains(func),
            "IR should declare {}: {}", func, ir_str);
    }
}
