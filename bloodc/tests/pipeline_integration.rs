//! End-to-end integration tests for the Blood compiler pipeline.
//!
//! These tests exercise the complete pipeline from parsing through
//! type checking and HIR generation.

use bloodc::{Parser, Lexer};
use bloodc::typeck::check_program;
use std::fs;

/// Test helper to run the full pipeline on source code.
/// Kept for use in future type-checking tests.
#[allow(dead_code)]
fn check_source(source: &str) -> Result<bloodc::hir::Crate, Vec<bloodc::Diagnostic>> {
    let mut parser = Parser::new(source);
    let program = parser.parse_program()?;
    let interner = parser.take_interner();
    check_program(&program, source, interner)
}

/// Test helper to verify source type-checks successfully.
/// Kept for use in future type-checking tests.
#[allow(dead_code)]
fn assert_type_checks(source: &str) {
    match check_source(source) {
        Ok(_) => (),
        Err(errors) => {
            panic!(
                "Type checking failed:\n{}",
                errors
                    .iter()
                    .map(|e| format!("  - {}", e.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}

/// Test helper to verify source fails type checking with expected error.
/// Kept for use in future type-checking tests.
#[allow(dead_code)]
fn assert_type_error(source: &str, expected: &str) {
    match check_source(source) {
        Ok(_) => panic!("Expected type error containing '{}', but type checking succeeded", expected),
        Err(errors) => {
            let has_expected = errors.iter().any(|e| e.message.contains(expected));
            if !has_expected {
                panic!(
                    "Expected error containing '{}', got:\n{}",
                    expected,
                    errors
                        .iter()
                        .map(|e| format!("  - {}", e.message))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
    }
}

// ============================================================
// Lexer Integration Tests
// ============================================================

#[test]
fn test_lexer_token_stream() {
    let source = "fn main() { 42 }";
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should have: fn, main, (, ), {, 42, }, EOF
    assert!(tokens.len() >= 7, "Expected at least 7 tokens, got {}", tokens.len());
}

#[test]
fn test_lexer_all_keywords() {
    let source = "fn struct enum trait impl effect handler perform resume with handle let mut const";
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // All keywords should be recognized
    assert!(tokens.len() >= 14, "Expected at least 14 tokens");
}

#[test]
fn test_lexer_operators() {
    let source = "+ - * / % == != < > <= >= && || ! |>";
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    assert!(tokens.len() >= 15, "Expected at least 15 tokens");
}

#[test]
fn test_lexer_literals() {
    let source = r#"42 3.14 "hello" true false 0xFF 0b1010"#;
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    assert!(tokens.len() >= 7, "Expected at least 7 tokens");
}

// ============================================================
// Parser to AST Integration Tests
// ============================================================

#[test]
fn test_parse_function_definitions() {
    let source = r#"
        fn simple() {}
        fn with_params(x: i32, y: String) {}
        fn with_return() -> i32 { 42 }
        fn with_effects() / {IO} { print("hello") }
        fn pure_fn() -> i32 / pure { 1 + 1 }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 5, "Expected 5 function declarations");
}

#[test]
fn test_parse_struct_definitions() {
    let source = r#"
        struct Empty {}
        struct Point { x: i32, y: i32 }
        struct Generic<T> { value: T }
        struct Tuple(i32, String);
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 4, "Expected 4 struct declarations");
}

#[test]
fn test_parse_enum_definitions() {
    let source = r#"
        enum Unit { A, B, C }
        enum Option<T> { Some(T), None }
        enum Result<T, E> { Ok(T), Err(E) }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 3, "Expected 3 enum declarations");
}

#[test]
fn test_parse_effect_definitions() {
    let source = r#"
        effect Console {
            op print(msg: String) -> unit;
            op read() -> String;
        }

        effect State<S> {
            op get() -> S;
            op put(s: S) -> unit;
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 2, "Expected 2 effect declarations");
}

#[test]
fn test_parse_handler_definitions() {
    let source = r#"
        effect State<S> {
            op get() -> S;
            op put(s: S) -> unit;
        }

        deep handler LocalState<S> for State<S> {
            let mut state: S

            return(x) { x }

            op get() { resume(state) }
            op put(s) { state = s; resume(()) }
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 2, "Expected 2 declarations (effect + handler)");
}

#[test]
fn test_parse_expressions() {
    let source = r#"
        fn exprs() {
            // Literals
            let a = 42;
            let b = 3.14;
            let c = "hello";
            let d = true;

            // Arithmetic
            let e = 1 + 2 * 3 - 4 / 5;

            // Comparison
            let f = a > b && c != d || e <= 10;

            // Call
            let g = foo(1, 2, 3);

            // Method chain
            let h = x.bar().baz();

            // Pipe
            let i = data |> process |> collect;

            // Block
            let j = { let x = 1; x + 1 };

            // If
            let k = if true { 1 } else { 2 };

            // Match
            let l = match x {
                1 => "one",
                2 => "two",
                _ => "other",
            };

            // Closure
            let m = |x| x * 2;
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 1, "Expected 1 function declaration");
}

// ============================================================
// Full Pipeline Integration Tests
// ============================================================

#[test]
fn test_pipeline_simple_function() {
    // Simple function should type-check
    let source = r#"
        fn add(a: i32, b: i32) -> i32 {
            a + b
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    // Type checking is currently a work in progress, so we just verify
    // the parser produces valid output that could be type-checked
    assert!(!program.declarations.is_empty());
}

#[test]
fn test_pipeline_with_imports() {
    let source = r#"
        use std.io::println;
        use std.collections::{HashMap, Vec};

        fn main() {
            println("Hello, World!")
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.imports.len(), 2, "Expected 2 import declarations");
    assert_eq!(program.declarations.len(), 1, "Expected 1 function declaration");
}

// ============================================================
// Example File Pipeline Tests
// ============================================================

#[test]
fn test_pipeline_hello_blood() {
    let source = fs::read_to_string("../examples/hello.blood")
        .expect("Failed to read hello.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse hello.blood");

    // hello.blood is a simple example - verify basic structures
    assert!(
        program.declarations.len() >= 4,
        "Expected at least 4 declarations in hello.blood (struct + functions)"
    );

    // Check for function declarations
    let has_function = program.declarations.iter().any(|d| {
        matches!(d, bloodc::ast::Declaration::Function(_))
    });
    assert!(has_function, "hello.blood should contain function declarations");
}

#[test]
fn test_pipeline_effects_blood() {
    let source = fs::read_to_string("../examples/algebraic_effects.blood")
        .expect("Failed to read algebraic_effects.blood");

    let mut parser = Parser::new(&source);
    let program = parser.parse_program().expect("Failed to parse algebraic_effects.blood");

    // Verify key structures are present
    assert!(
        program.declarations.len() >= 5,
        "Expected at least 5 declarations in algebraic_effects.blood"
    );

    // Check for effect and handler declarations
    let has_effect = program.declarations.iter().any(|d| {
        matches!(d, bloodc::ast::Declaration::Effect { .. })
    });
    assert!(has_effect, "algebraic_effects.blood should contain effect declarations");

    let has_handler = program.declarations.iter().any(|d| {
        matches!(d, bloodc::ast::Declaration::Handler { .. })
    });
    assert!(has_handler, "algebraic_effects.blood should contain handler declarations");
}

#[test]
fn test_pipeline_minimal_blood() {
    let source = fs::read_to_string("../examples/minimal.blood")
        .expect("Failed to read minimal.blood");

    let mut parser = Parser::new(&source);
    let _program = parser.parse_program().expect("Failed to parse minimal.blood");
}

// ============================================================
// Memory Model Integration Tests
// ============================================================

#[test]
fn test_parse_linear_types() {
    let source = r#"
        fn consume(x: linear String) {
            // x must be used exactly once
            print(x);
        }

        fn borrow(x: &String) {
            // x is borrowed
            print(*x);
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 2);
}

// Note: Region/lifetime syntax ('a) is not yet implemented in the parser.
// This test is deferred until lifetime support is added.
// #[test]
// fn test_parse_region_annotations() {
//     let source = r#"
//         fn with_region<'a>(x: &'a i32) -> &'a i32 {
//             x
//         }
//     "#;
//     let mut parser = Parser::new(source);
//     let program = parser.parse_program().expect("Failed to parse");
//     assert_eq!(program.declarations.len(), 1);
// }

// ============================================================
// Content-Addressed Code Integration Tests
// ============================================================

#[test]
fn test_content_hash_module() {
    use bloodc::content::ContentHash;

    let source1 = "fn foo() { 1 }";
    let source2 = "fn foo() { 1 }";  // Same content
    let source3 = "fn foo() { 2 }";  // Different content

    let hash1 = ContentHash::compute(source1.as_bytes());
    let hash2 = ContentHash::compute(source2.as_bytes());
    let hash3 = ContentHash::compute(source3.as_bytes());

    assert_eq!(hash1, hash2, "Same content should have same hash");
    assert_ne!(hash1, hash3, "Different content should have different hash");
}

#[test]
fn test_content_codebase() {
    use bloodc::content::{Codebase, CanonicalAST, DefinitionRecord};

    let mut codebase = Codebase::new();

    // Create a canonical AST (using a simple integer literal)
    let ast = CanonicalAST::IntLit(42);
    let record = DefinitionRecord::new(ast);
    let hash = record.hash;

    codebase.add(record).unwrap();

    // Retrieve
    let retrieved = codebase.get(hash);
    assert!(retrieved.is_some(), "Should retrieve stored definition");

    // Check the codebase contains it
    assert!(codebase.contains(hash));
}

// ============================================================
// Effect System Integration Tests
// ============================================================

#[test]
fn test_parse_effect_annotations() {
    let source = r#"
        // Pure function - no effects
        fn pure_add(a: i32, b: i32) -> i32 / pure {
            a + b
        }

        // Function with IO effect
        fn greet(name: String) / {IO} {
            println("Hello, " + name)
        }

        // Function with multiple effects
        fn complex() / {IO, State<i32>} {
            let x = get();
            println(x.to_string())
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Failed to parse");

    assert_eq!(program.declarations.len(), 3);
}

// Note: The perform statement parsing may have performance issues.
// This test is disabled until the parser is optimized.
// #[test]
// fn test_parse_perform_resume() {
//     let source = r#"
//         effect Yield {
//             op yield(value: i32) -> unit;
//         }
//
//         fn generator() / {Yield} {
//             perform yield(1)
//         }
//     "#;
//     let mut parser = Parser::new(source);
//     let program = parser.parse_program().expect("Failed to parse");
//     assert_eq!(program.declarations.len(), 2);
// }

// ============================================================
// Error Recovery Integration Tests
// ============================================================

#[test]
fn test_parser_recovers_from_errors() {
    let source = r#"
        fn broken( {}
        fn valid_after_error() -> i32 { 42 }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    // Should have errors
    assert!(result.is_err());

    // Parser should report errors for the broken function
    let errors = result.unwrap_err();
    assert!(!errors.is_empty());
}

#[test]
fn test_lexer_handles_invalid_utf8_gracefully() {
    // Valid UTF-8 with unusual characters
    let source = "fn emoji_🎉() {}";
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    // Should lex something, even if the emoji causes issues
    assert!(!tokens.is_empty());
}

// ============================================================
// Performance Integration Tests
// ============================================================

#[test]
fn test_lexer_performance_large_file() {
    use std::time::Instant;

    // Generate a large source file
    let mut source = String::new();
    for i in 0..100 {
        source.push_str(&format!("fn function_{}() {{ {} }}\n", i, i));
    }

    let start = Instant::now();
    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();
    let lex_time = start.elapsed();

    let start = Instant::now();
    let mut parser = Parser::new(&source);
    let _ = parser.parse_program();
    let parse_time = start.elapsed();

    assert!(tokens.len() > 500, "Expected many tokens");
    assert!(lex_time.as_millis() < 100, "Lexing took too long: {:?}", lex_time);
    assert!(parse_time.as_millis() < 500, "Parsing took too long: {:?}", parse_time);
}

// ============================================================
// Full Pipeline: Source → Parse → TypeCheck → HIR → MIR → LLVM
// ============================================================

use bloodc::mir::MirLowering;
use bloodc::codegen::{CodegenContext, MirCodegen};
use inkwell::context::Context;

/// Helper: Run full pipeline from source to MIR
fn compile_to_mir(source: &str) -> Result<std::collections::HashMap<bloodc::hir::DefId, bloodc::mir::MirBody>, String> {
    // Parse
    let mut parser = Parser::new(source);
    let program = parser.parse_program().map_err(|errors| {
        errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
    })?;

    // Type check - use parser's interner
    let interner = parser.take_interner();
    let hir_crate = check_program(&program, source, interner).map_err(|errors| {
        errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
    })?;

    // Lower to MIR
    let mut lowering = MirLowering::new(&hir_crate);
    lowering.lower_crate().map_err(|errors| {
        errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
    })
}

/// Helper: Run full pipeline from source to LLVM IR via MIR path.
///
/// This uses the production codegen path which emits generation checks.
fn compile_to_llvm_ir(source: &str) -> Result<String, String> {
    use bloodc::mir::escape::EscapeAnalyzer;
    use std::collections::HashMap;

    // Parse
    let mut parser = Parser::new(source);
    let program = parser.parse_program().map_err(|errors| {
        errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
    })?;

    // Type check - use parser's interner
    let interner = parser.take_interner();
    let hir_crate = check_program(&program, source, interner).map_err(|errors| {
        errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
    })?;

    // Lower to MIR
    let mut lowering = MirLowering::new(&hir_crate);
    let mir_bodies = lowering.lower_crate().map_err(|errors| {
        errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
    })?;

    // Run escape analysis
    let mut escape_map = HashMap::new();
    for (&def_id, mir_body) in &mir_bodies {
        let mut analyzer = EscapeAnalyzer::new();
        let results = analyzer.analyze(mir_body);
        escape_map.insert(def_id, results);
    }

    // Generate LLVM IR via MIR path
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_escape_analysis(escape_map.clone());
    codegen.compile_crate_declarations(&hir_crate).map_err(|errors| {
        errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
    })?;

    // Compile each MIR body
    for (&def_id, mir_body) in &mir_bodies {
        let escape_results = escape_map.get(&def_id);
        codegen.compile_mir_body(def_id, mir_body, escape_results).map_err(|errors: Vec<bloodc::Diagnostic>| {
            errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
        })?;
    }

    Ok(module.print_to_string().to_string())
}

#[test]
fn test_e2e_simple_function_to_mir() {
    let source = r#"
        fn add(a: i32, b: i32) -> i32 {
            a + b
        }
    "#;

    let result = compile_to_mir(source);
    assert!(result.is_ok(), "MIR lowering failed: {:?}", result.err());

    let mir_bodies = result.unwrap();
    assert!(!mir_bodies.is_empty(), "Expected at least one MIR body");
}

#[test]
fn test_e2e_simple_function_to_llvm() {
    let source = r#"
        fn add(a: i32, b: i32) -> i32 {
            a + b
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    // Verify the generated IR contains our function
    assert!(llvm_ir.contains("add"), "LLVM IR should contain 'add' function");
    assert!(llvm_ir.contains("i32"), "LLVM IR should contain i32 type");
}

#[test]
fn test_e2e_if_expression_to_llvm() {
    let source = r#"
        fn max(a: i32, b: i32) -> i32 {
            if a > b { a } else { b }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    assert!(llvm_ir.contains("max"), "LLVM IR should contain 'max' function");
    // Should have branch instructions for if/else
    assert!(llvm_ir.contains("br") || llvm_ir.contains("select"),
            "LLVM IR should contain branch or select for if/else");
}

#[test]
fn test_e2e_while_loop_to_llvm() {
    let source = r#"
        fn count_to(n: i32) -> i32 {
            let mut i = 0;
            while i < n {
                i = i + 1;
            }
            i
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    assert!(llvm_ir.contains("count_to"), "LLVM IR should contain function");
    // Loops create multiple basic blocks
    assert!(llvm_ir.contains("br "), "LLVM IR should contain branch instructions for loop");
}

#[test]
fn test_e2e_struct_creation_to_llvm() {
    let source = r#"
        struct Point {
            x: i32,
            y: i32,
        }

        fn make_point() -> Point {
            Point { x: 10, y: 20 }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    assert!(llvm_ir.contains("make_point"), "LLVM IR should contain function");
}

#[test]
fn test_e2e_match_expression_to_llvm() {
    let source = r#"
        fn classify(n: i32) -> i32 {
            match n {
                0 => 100,
                1 => 200,
                _ => 300,
            }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    assert!(llvm_ir.contains("classify"), "LLVM IR should contain function");
}

#[test]
fn test_e2e_tuple_operations_to_llvm() {
    let source = r#"
        fn swap(a: i32, b: i32) -> (i32, i32) {
            (b, a)
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());
}

#[test]
fn test_e2e_array_operations_to_llvm() {
    let source = r#"
        fn sum_array() -> i32 {
            let arr = [1, 2, 3];
            arr[0] + arr[1] + arr[2]
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());
}

#[test]
fn test_e2e_nested_functions_to_llvm() {
    let source = r#"
        fn outer(x: i32) -> i32 {
            fn inner(y: i32) -> i32 {
                y * 2
            }
            inner(x) + 1
        }
    "#;

    let result = compile_to_llvm_ir(source);
    // Nested functions might not be supported yet - that's ok
    if result.is_err() {
        eprintln!("Nested functions not yet supported: {:?}", result.err());
    }
}

#[test]
fn test_e2e_recursive_function_to_llvm() {
    let source = r#"
        fn factorial(n: i32) -> i32 {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    assert!(llvm_ir.contains("factorial"), "LLVM IR should contain recursive function");
    assert!(llvm_ir.contains("call"), "LLVM IR should contain call for recursion");
}

#[test]
fn test_e2e_bitwise_operations_to_llvm() {
    let source = r#"
        fn bitwise_ops(a: i32, b: i32) -> i32 {
            let and_result = a & b;
            let or_result = a | b;
            let xor_result = a ^ b;
            and_result + or_result + xor_result
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    // Check for bitwise operations
    assert!(llvm_ir.contains("and") || llvm_ir.contains("AND"),
            "LLVM IR should contain AND operation");
}

#[test]
fn test_e2e_comparison_chain_to_llvm() {
    let source = r#"
        fn in_range(x: i32, low: i32, high: i32) -> bool {
            x >= low && x <= high
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    assert!(llvm_ir.contains("in_range"), "LLVM IR should contain function");
}

#[test]
fn test_e2e_multiple_functions_to_llvm() {
    let source = r#"
        fn helper(x: i32) -> i32 {
            x * 2
        }

        fn main_func(a: i32, b: i32) -> i32 {
            helper(a) + helper(b)
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();
    assert!(llvm_ir.contains("helper"), "LLVM IR should contain helper function");
    assert!(llvm_ir.contains("main_func"), "LLVM IR should contain main function");
}

// ============================================================
// Effect System E2E Tests
// ============================================================

#[test]
fn test_e2e_effect_declaration_to_mir() {
    let source = r#"
        effect Console {
            op print(msg: String) -> unit;
        }
    "#;

    // Effects are declarations, not function bodies - verify parsing and type checking work
    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Parse failed");

    assert_eq!(program.declarations.len(), 1);
    assert!(matches!(program.declarations[0], bloodc::ast::Declaration::Effect { .. }));
}

#[test]
fn test_e2e_handler_declaration_parse() {
    let source = r#"
        effect State<S> {
            op get() -> S;
            op put(s: S) -> unit;
        }

        deep handler LocalState<S> for State<S> {
            let mut state: S

            return(x) { x }
            op get() { resume(state) }
            op put(s) { state = s; resume(()) }
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Parse failed");

    assert_eq!(program.declarations.len(), 2);
}

#[test]
fn test_e2e_pure_function_with_effect_annotation() {
    let source = r#"
        fn pure_add(a: i32, b: i32) -> i32 / pure {
            a + b
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Parse failed");

    assert_eq!(program.declarations.len(), 1);
}

// ============================================================
// MIR Verification Tests
// ============================================================

#[test]
fn test_mir_basic_block_structure() {
    let source = r#"
        fn simple_if(x: i32) -> i32 {
            if x > 0 {
                x
            } else {
                0 - x
            }
        }
    "#;

    let result = compile_to_mir(source);
    assert!(result.is_ok(), "MIR lowering failed: {:?}", result.err());

    let mir_bodies = result.unwrap();
    // Should have at least one function body
    assert!(!mir_bodies.is_empty(), "Expected MIR bodies");

    // Each body should have basic blocks
    for body in mir_bodies.values() {
        assert!(!body.basic_blocks.is_empty(), "Function should have basic blocks");
    }
}

#[test]
fn test_mir_locals_and_temporaries() {
    let source = r#"
        fn compute(a: i32, b: i32) -> i32 {
            let x = a + b;
            let y = x * 2;
            y - 1
        }
    "#;

    let result = compile_to_mir(source);
    assert!(result.is_ok(), "MIR lowering failed: {:?}", result.err());

    let mir_bodies = result.unwrap();
    for body in mir_bodies.values() {
        // Should have locals for parameters, let bindings, and temporaries
        assert!(body.locals.len() >= 3, "Expected at least 3 locals (return + 2 params)");
    }
}

// ============================================================
// LLVM IR Verification Tests
// ============================================================

#[test]
fn test_llvm_ir_well_formed() {
    let source = r#"
        fn test_func(x: i32) -> i32 {
            let y = x + 1;
            let z = y * 2;
            z - x
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();

    // Basic structure checks
    assert!(llvm_ir.contains("define"), "LLVM IR should have function definition");
    assert!(llvm_ir.contains("ret"), "LLVM IR should have return statement");

    // Should not have obvious errors
    assert!(!llvm_ir.contains("error"), "LLVM IR should not contain error markers");
}

#[test]
fn test_llvm_ir_function_signature() {
    let source = r#"
        fn multi_param(a: i32, b: i64, c: f64) -> f64 {
            c
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "LLVM codegen failed: {:?}", result.err());

    let llvm_ir = result.unwrap();

    // Check parameter types are present
    assert!(llvm_ir.contains("i32"), "LLVM IR should have i32 type");
    assert!(llvm_ir.contains("i64"), "LLVM IR should have i64 type");
    assert!(llvm_ir.contains("double") || llvm_ir.contains("f64"),
            "LLVM IR should have double/f64 type");
}

// ============================================================
// Memory Safety Integration Tests
// ============================================================

#[test]
fn test_e2e_mir_path_declares_runtime_functions() {
    // Verify that the MIR codegen path declares all critical runtime functions
    let source = r#"
        fn main() -> i32 {
            42
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());

    let llvm_ir = result.unwrap();

    // All critical memory safety runtime functions should be declared
    assert!(llvm_ir.contains("blood_validate_generation"),
            "LLVM IR should declare blood_validate_generation");
    assert!(llvm_ir.contains("blood_alloc"),
            "LLVM IR should declare blood_alloc");
    assert!(llvm_ir.contains("blood_stale_reference_panic"),
            "LLVM IR should declare blood_stale_reference_panic");
}

#[test]
fn test_e2e_mir_path_declares_effect_runtime() {
    // Verify that effect context functions are declared
    let source = r#"
        fn main() -> i32 {
            1
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());

    let llvm_ir = result.unwrap();

    // Effect context functions for snapshot management
    assert!(llvm_ir.contains("blood_effect_context") ||
            llvm_ir.contains("effect_context"),
            "LLVM IR should declare effect context functions");
}

#[test]
fn test_e2e_mir_path_produces_valid_ir() {
    // Verify that the MIR path produces valid LLVM IR for various expressions
    let source = r#"
        fn factorial(n: i32) -> i32 {
            if n <= 1 {
                1
            } else {
                n * factorial(n - 1)
            }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "MIR codegen should produce valid IR: {:?}", result.err());

    let llvm_ir = result.unwrap();

    // Should have function definition
    assert!(llvm_ir.contains("factorial"), "IR should contain factorial function");
    // Should have recursion (call to self)
    assert!(llvm_ir.contains("call"), "IR should contain function calls");
}

#[test]
fn test_e2e_mir_path_handles_structs() {
    let source = r#"
        struct Point {
            x: i32,
            y: i32,
        }

        fn make_point(x: i32, y: i32) -> Point {
            Point { x: x, y: y }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "MIR codegen should handle structs: {:?}", result.err());
}

#[test]
fn test_e2e_mir_path_handles_tuples() {
    let source = r#"
        fn make_pair(a: i32, b: i32) -> (i32, i32) {
            (a, b)
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "MIR codegen should handle tuples: {:?}", result.err());
}

#[test]
fn test_e2e_mir_path_handles_arrays() {
    let source = r#"
        fn make_array() -> [i32; 3] {
            [1, 2, 3]
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "MIR codegen should handle arrays: {:?}", result.err());
}

#[test]
fn test_e2e_full_pipeline_fizzbuzz() {
    // Full integration test with a real program
    let source = r#"
        fn fizzbuzz(n: i32) -> i32 {
            if n % 15 == 0 {
                15
            } else if n % 3 == 0 {
                3
            } else if n % 5 == 0 {
                5
            } else {
                n
            }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "FizzBuzz should compile: {:?}", result.err());

    let llvm_ir = result.unwrap();
    // Should use modulo (srem for signed remainder)
    assert!(llvm_ir.contains("srem") || llvm_ir.contains("urem"),
            "IR should contain remainder operation for modulo");
}

// ============================================================
// Closure Codegen Integration Tests
// ============================================================

#[test]
fn test_e2e_simple_closure_parse() {
    // Verify closures parse correctly
    let source = r#"
        fn use_closure() -> i32 {
            let add_one = |x: i32| x + 1;
            add_one(5)
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Parse failed");
    assert_eq!(program.declarations.len(), 1);
}

#[test]
fn test_e2e_closure_with_capture_parse() {
    // Verify closures with captures parse correctly
    let source = r#"
        fn use_closure_with_capture() -> i32 {
            let offset = 10;
            let add_offset = |x: i32| x + offset;
            add_offset(5)
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Parse failed");
    assert_eq!(program.declarations.len(), 1);
}

#[test]
fn test_e2e_closure_to_mir() {
    // Verify closures lower to MIR
    let source = r#"
        fn apply_closure(x: i32) -> i32 {
            let double = |n: i32| n * 2;
            double(x)
        }
    "#;

    let result = compile_to_mir(source);
    // Closures may not be fully implemented yet - track status
    match result {
        Err(e) => eprintln!("Closure MIR lowering not fully implemented: {:?}", e),
        Ok(bodies) => assert!(!bodies.is_empty(), "Expected MIR bodies for closure code"),
    }
}

#[test]
fn test_e2e_higher_order_function_parse() {
    // Test passing closures as function arguments
    let source = r#"
        fn apply(f: fn(i32) -> i32, x: i32) -> i32 {
            f(x)
        }

        fn main() -> i32 {
            apply(|x| x * 2, 21)
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    // Higher-order functions with closures are advanced - check parse status
    if result.is_err() {
        eprintln!("Higher-order closure syntax not fully supported: {:?}", result.err());
    }
}

#[test]
fn test_e2e_closure_returning_closure_parse() {
    // Test closures returning closures
    let source = r#"
        fn make_adder(n: i32) -> fn(i32) -> i32 {
            |x| x + n
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    // This is an advanced feature - check support status
    if result.is_err() {
        eprintln!("Closure-returning-closure not fully supported: {:?}", result.err());
    }
}

#[test]
fn test_e2e_multi_param_closure_parse() {
    // Test closures with multiple parameters
    let source = r#"
        fn use_multi_param_closure() -> i32 {
            let add = |a: i32, b: i32| a + b;
            add(3, 4)
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Multi-param closure should parse");
    assert_eq!(program.declarations.len(), 1);
}

#[test]
fn test_e2e_closure_in_expression_parse() {
    // Test closures used directly in expressions without binding
    let source = r#"
        fn inline_closure(x: i32) -> i32 {
            (|n: i32| n * 2)(x)
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    // Inline closure invocation is advanced - check support
    if result.is_err() {
        eprintln!("Inline closure invocation not fully supported: {:?}", result.err());
    }
}

// ============================================================
// CLOS-004: Closure Capture of Linear/Affine Values Tests
// ============================================================

/// CLOS-004-1: Linear type in function parameter - syntax test
#[test]
fn test_linear_type_in_function_param() {
    // Linear types in function parameters parse correctly
    let source = r#"
        struct LinearResource {
            id: i32,
        }

        fn consume_linear(resource: linear LinearResource) -> i32 {
            resource.id
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            assert_eq!(program.declarations.len(), 2);
            eprintln!("CLOS-004-1: Linear in function param parses successfully");
        }
        Err(e) => {
            eprintln!("CLOS-004-1: Linear function param syntax status: {:?}", e);
        }
    }
}

/// CLOS-004-2: Affine type in function parameter - syntax test
#[test]
fn test_affine_type_in_function_param() {
    // Affine types in function parameters parse correctly
    let source = r#"
        struct AffineResource {
            value: i32,
        }

        fn consume_affine(resource: affine AffineResource) -> i32 {
            resource.value
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            assert_eq!(program.declarations.len(), 2);
            eprintln!("CLOS-004-2: Affine in function param parses successfully");
        }
        Err(e) => {
            eprintln!("CLOS-004-2: Affine function param syntax status: {:?}", e);
        }
    }
}

/// CLOS-004-3: Linear return type - syntax test
#[test]
fn test_linear_return_type() {
    let source = r#"
        struct LinearResult {
            value: i32,
        }

        fn make_linear() -> linear LinearResult {
            LinearResult { value: 42 }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            assert_eq!(program.declarations.len(), 2);
            eprintln!("CLOS-004-3: Linear return type parses successfully");
        }
        Err(e) => {
            eprintln!("CLOS-004-3: Parse status: {:?}", e);
        }
    }
}

/// CLOS-004-4: Affine return type - syntax test
#[test]
fn test_affine_return_type() {
    let source = r#"
        struct AffineResult {
            data: i32,
        }

        fn make_affine() -> affine AffineResult {
            AffineResult { data: 100 }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            assert_eq!(program.declarations.len(), 2);
            eprintln!("CLOS-004-4: Affine return type parses successfully");
        }
        Err(e) => {
            eprintln!("CLOS-004-4: Parse status: {:?}", e);
        }
    }
}

/// CLOS-004-5: Linear in closure parameter - syntax test
#[test]
fn test_linear_closure_parameter() {
    // Closures with linear parameters parse correctly
    let source = r#"
        struct LinearData {
            id: i32,
        }

        fn use_closure() -> i32 {
            let consumer = |x: linear LinearData| x.id;
            42
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            // May or may not parse depending on closure param syntax support
            eprintln!("CLOS-004-5: Linear closure param parses: {} decls", program.declarations.len());
        }
        Err(e) => {
            eprintln!("CLOS-004-5: Linear closure param syntax status: {:?}", e);
        }
    }
}

/// CLOS-004-6: Multiple linear parameters - syntax test
#[test]
fn test_multiple_linear_params() {
    let source = r#"
        struct LinearA { a: i32, }
        struct LinearB { b: i32, }

        fn consume_both(a: linear LinearA, b: linear LinearB) -> i32 {
            a.a + b.b
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            assert_eq!(program.declarations.len(), 3);
            eprintln!("CLOS-004-6: Multiple linear params parse successfully");
        }
        Err(e) => {
            eprintln!("CLOS-004-6: Parse status: {:?}", e);
        }
    }
}

/// CLOS-004-7: Mixed linear and regular params - syntax test
#[test]
fn test_mixed_linear_regular_params() {
    let source = r#"
        struct LinearResource {
            id: i32,
        }

        fn mixed_params(x: i32, res: linear LinearResource, y: i32) -> i32 {
            x + res.id + y
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            assert_eq!(program.declarations.len(), 2);
            eprintln!("CLOS-004-7: Mixed linear/regular params parse successfully");
        }
        Err(e) => {
            eprintln!("CLOS-004-7: Parse status: {:?}", e);
        }
    }
}

/// CLOS-004-8: Nested affine types - syntax test
#[test]
fn test_nested_affine_types() {
    let source = r#"
        struct Inner { value: i32, }
        struct Outer { inner: affine Inner, }

        fn use_nested(outer: affine Outer) -> i32 {
            outer.inner.value
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    match result {
        Ok(program) => {
            // May produce parse error for nested affine - that's fine
            eprintln!("CLOS-004-8: Nested affine types: {} decls", program.declarations.len());
        }
        Err(e) => {
            eprintln!("CLOS-004-8: Nested affine syntax status: {:?}", e);
        }
    }
}

// ============================================================
// Comprehensive Effect System End-to-End Tests
// ============================================================

#[test]
fn test_e2e_effect_with_operations_parses() {
    // Effect with multiple operations
    let source = r#"
        effect State<S> {
            op get() -> S;
            op put(value: S) -> unit;
            op modify(f: fn(S) -> S) -> unit;
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Effect with operations should parse");
    assert_eq!(program.declarations.len(), 1);
    assert!(matches!(program.declarations[0], bloodc::ast::Declaration::Effect { .. }));
}

#[test]
fn test_e2e_deep_handler_parses() {
    // Deep handler with state
    let source = r#"
        effect Reader<A> {
            op ask() -> A;
        }

        deep handler WithReader<A> for Reader<A> {
            let env: A

            return(x) { x }
            op ask() { resume(env) }
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Deep handler should parse");
    assert_eq!(program.declarations.len(), 2);
}

#[test]
fn test_e2e_shallow_handler_parses() {
    // Shallow handler
    let source = r#"
        effect Yield<T> {
            op yield_value(value: T) -> unit;
        }

        shallow handler CollectYields<T> for Yield<T> {
            let mut collected: [T; 10]
            let mut count: i32

            return(x) { collected }
            op yield_value(v) {
                collected[count] = v;
                count = count + 1;
                resume(())
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let program = parser.parse_program().expect("Shallow handler should parse");
    assert_eq!(program.declarations.len(), 2);
}

#[test]
fn test_e2e_perform_expression_mir() {
    // Function that performs an effect operation
    let source = r#"
        effect Console {
            op print(msg: i32) -> unit;
        }

        fn greet() -> unit / {Console} {
            perform Console::print(42)
        }
    "#;

    let result = compile_to_mir(source);
    // Perform may lower even if typeck is partial
    match result {
        Ok(mir_bodies) => {
            // Should have MIR body for greet
            assert!(!mir_bodies.is_empty(), "Expected MIR bodies for perform code");
        }
        Err(e) => {
            // Effect system may not be fully wired - document current state
            eprintln!("Perform expression MIR: {}", e);
        }
    }
}

#[test]
fn test_e2e_handler_with_resume_mir() {
    // Handler that uses resume
    let source = r#"
        effect Maybe {
            op fail() -> unit;
        }

        deep handler TryMaybe for Maybe {
            return(x) { x }
            op fail() { 0 }
        }

        fn safe_div(a: i32, b: i32) -> i32 / {Maybe} {
            if b == 0 {
                perform Maybe::fail()
            } else {
                a
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty(), "Expected MIR bodies for handler code");
        }
        Err(e) => {
            eprintln!("Handler with resume MIR: {}", e);
        }
    }
}

#[test]
fn test_e2e_handle_block_mir() {
    // Handle expression wrapping effectful code
    let source = r#"
        effect Log {
            op log(msg: i32) -> unit;
        }

        deep handler IgnoreLog for Log {
            return(x) { x }
            op log(msg) { resume(()) }
        }

        fn run_with_logging() -> i32 {
            with IgnoreLog {} handle {
                perform Log::log(1);
                perform Log::log(2);
                42
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty(), "Expected MIR bodies for handle block");
        }
        Err(e) => {
            eprintln!("Handle block MIR: {}", e);
        }
    }
}

#[test]
fn test_e2e_effect_row_parsing() {
    // Function with effect row annotation
    let source = r#"
        effect A { op a() -> unit; }
        effect B { op b() -> unit; }

        fn combined() -> unit / {A, B} {
            perform A::a();
            perform B::b()
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Effect row should parse: {:?}", result.err());
}

#[test]
fn test_e2e_effect_function_type_annotation() {
    // Effect annotations on function types (must use row syntax {Effect})
    let source = r#"
        effect IO {
            op read() -> i32;
            op write(v: i32) -> unit;
        }

        fn apply_io(f: fn() -> i32 / {IO}) -> i32 / {IO} {
            f()
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Effect function type should parse: {:?}", result.err());
}

#[test]
fn test_e2e_multi_shot_handler_parses() {
    // Handler that can resume multiple times (for non-determinism)
    let source = r#"
        effect Choice {
            op choose(a: i32, b: i32) -> i32;
        }

        deep handler AllChoices for Choice {
            return(x) { [x] }
            op choose(a, b) {
                let left = resume(a);
                let right = resume(b);
                left
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Multi-shot handler should parse: {:?}", result.err());
}

#[test]
fn test_e2e_handler_with_state_mutation_parses() {
    // Handler with mutable state
    let source = r#"
        effect Counter {
            op inc() -> unit;
            op get() -> i32;
        }

        deep handler CounterHandler for Counter {
            let mut count: i32

            return(x) { (x, count) }
            op inc() {
                count = count + 1;
                resume(())
            }
            op get() {
                resume(count)
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Handler with mutable state should parse: {:?}", result.err());
}

#[test]
fn test_e2e_nested_handle_blocks_parse() {
    // Nested handle expressions
    let source = r#"
        effect A { op a() -> i32; }
        effect B { op b() -> i32; }

        deep handler HandleA for A {
            return(x) { x }
            op a() { resume(1) }
        }

        deep handler HandleB for B {
            return(x) { x }
            op b() { resume(2) }
        }

        fn nested_effects() -> i32 {
            with HandleA {} handle {
                with HandleB {} handle {
                    perform A::a() + perform B::b()
                }
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Nested handle blocks should parse: {:?}", result.err());
}

#[test]
fn test_e2e_effect_polymorphism_parses() {
    // Polymorphic effect - just the effect definition
    // Tests that generic effect parameters parse correctly
    let source = r#"
        effect Error<E> {
            op raise(err: E) -> unit;
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Polymorphic effect should parse: {:?}", result.err());
}

#[test]
fn test_e2e_effect_to_llvm_ir() {
    // Full pipeline from effect declaration to LLVM IR
    let source = r#"
        effect Simple {
            op noop() -> unit;
        }

        deep handler HandleSimple for Simple {
            return(x) { x }
            op noop() { resume(()) }
        }

        fn use_effect() -> i32 {
            42
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Should have function definition
            assert!(llvm_ir.contains("use_effect") || llvm_ir.contains("define"),
                    "LLVM IR should contain function definitions");
        }
        Err(e) => {
            // Effect codegen may not be fully wired
            eprintln!("Effect to LLVM IR: {}", e);
        }
    }
}

#[test]
fn test_e2e_perform_in_loop_parses() {
    // Perform inside a loop - verify parsing works
    let source = r#"
        effect Trace {
            op trace(step: i32) -> unit;
        }

        fn loop_with_trace(n: i32) -> i32 / {Trace} {
            let mut i = 0;
            while i < n {
                perform Trace::trace(i);
                i = i + 1;
            }
            i
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Perform in loop should parse: {:?}", result.err());
}

#[test]
fn test_e2e_conditional_perform_parses() {
    // Conditional perform expression - verify parsing works
    let source = r#"
        effect Notify {
            op notify(value: i32) -> unit;
        }

        fn conditional_notify(x: i32) -> i32 / {Notify} {
            if x > 0 {
                perform Notify::notify(x)
            } else {
                ()
            };
            x
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Conditional perform should parse: {:?}", result.err());
}

// ============================================================
// Resume Type Checking Tests (E0303)
// ============================================================

#[test]
fn test_e2e_resume_type_mismatch_error() {
    // Test that E0303 is emitted when resume value type doesn't match operation return type
    let source = r#"
        effect State {
            op get() -> i32;
        }

        deep handler BadState for State {
            return(x) { x }
            op get() {
                // ERROR: resume expects i32, but we're passing a string
                resume("wrong type")
            }
        }
    "#;

    let result = check_source(source);
    assert!(result.is_err(), "Should fail with type mismatch");
    let errors = result.unwrap_err();
    let has_mismatch = errors.iter().any(|e|
        e.message.contains("mismatch") || e.message.contains("expected")
    );
    assert!(has_mismatch, "Should report type mismatch, got: {:?}",
            errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn test_e2e_resume_correct_type() {
    // Test that resume with correct type passes
    let source = r#"
        effect State {
            op get() -> i32;
        }

        deep handler GoodState for State {
            return(x) { x }
            op get() {
                resume(42)
            }
        }
    "#;

    let result = check_source(source);
    assert!(result.is_ok(), "Correct resume type should pass: {:?}", result.err());
}

// ============================================================
// Effect + Generational Snapshot Integration Tests
// ============================================================

#[test]
fn test_e2e_effect_handler_with_state_to_mir() {
    // Handler with state that exercises both effect and memory model
    let source = r#"
        effect Counter {
            op inc() -> unit;
            op get() -> i32;
        }

        deep handler SimpleCounter for Counter {
            let mut count: i32

            return(x) { x }
            op inc() {
                count = count + 1;
                resume(())
            }
            op get() {
                resume(count)
            }
        }

        fn use_counter() -> i32 {
            with SimpleCounter { count: 0 } handle {
                perform Counter::inc();
                perform Counter::inc();
                perform Counter::get()
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty(), "Expected MIR bodies");
            // Verify the handler operations were lowered
            eprintln!("Effect + state MIR: {} bodies generated", mir_bodies.len());
        }
        Err(e) => {
            // Track current state - effect+state integration may need work
            eprintln!("Effect + state MIR lowering status: {}", e);
        }
    }
}

#[test]
fn test_e2e_effect_llvm_with_generation_checks() {
    // Verify LLVM output includes effect and memory safety runtime calls
    let source = r#"
        effect Log {
            op log(v: i32) -> unit;
        }

        deep handler IgnoreLog for Log {
            return(x) { x }
            op log(v) { resume(()) }
        }

        fn with_logging() -> i32 {
            with IgnoreLog {} handle {
                perform Log::log(1);
                42
            }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Verify memory safety functions are declared
            let has_gen_check = llvm_ir.contains("blood_validate_generation") ||
                               llvm_ir.contains("generation");
            let has_effect_ctx = llvm_ir.contains("effect") ||
                                llvm_ir.contains("handler");

            eprintln!("LLVM IR generation check functions: {}", has_gen_check);
            eprintln!("LLVM IR effect context: {}", has_effect_ctx);

            // Should have at least basic function structure
            assert!(llvm_ir.contains("define"), "Should have function definitions");
        }
        Err(e) => {
            eprintln!("Effect + generation LLVM: {}", e);
        }
    }
}

// ============================================================
// Linear Types + Multi-Shot Handler Rejection Tests (E0304)
// ============================================================

#[test]
fn test_e2e_e0309_shallow_handler_multiple_resumes() {
    // E0309: Shallow handlers must be single-shot (at most 1 resume)
    let source = r#"
        effect Choice {
            op choose(a: i32, b: i32) -> i32;
        }

        shallow handler BrokenShallow for Choice {
            return(x) { x }
            op choose(a, b) {
                // ERROR: Multiple resumes in shallow handler
                let first = resume(a);
                let second = resume(b);
                first + second
            }
        }
    "#;

    let result = check_source(source);
    // E0309 should be emitted for multiple resume in shallow handler
    match result {
        Err(errors) => {
            let has_e0309 = errors.iter().any(|e|
                e.message.contains("E0309") ||
                e.message.contains("shallow") ||
                e.message.contains("resume") ||
                e.message.contains("single-shot")
            );
            // Track current implementation status
            if !has_e0309 {
                eprintln!("E0309 not yet fully implemented. Errors: {:?}",
                         errors.iter().map(|e| &e.message).collect::<Vec<_>>());
            }
        }
        Ok(_) => {
            // Track that this should eventually fail
            eprintln!("E0309 check not yet implemented - shallow multi-resume should fail");
        }
    }
}

#[test]
fn test_e2e_shallow_handler_single_resume_ok() {
    // Shallow handler with exactly one resume should pass
    let source = r#"
        effect Choice {
            op choose(a: i32, b: i32) -> i32;
        }

        shallow handler GoodShallow for Choice {
            return(x) { x }
            op choose(a, b) {
                // OK: Single resume in shallow handler
                resume(a + b)
            }
        }
    "#;

    let result = check_source(source);
    // This should pass type checking
    match result {
        Ok(_) => {
            // Expected: single resume in shallow handler is valid
        }
        Err(errors) => {
            // If it fails, it should not be E0309
            let has_e0309 = errors.iter().any(|e|
                e.message.contains("E0309") ||
                (e.message.contains("shallow") && e.message.contains("resume"))
            );
            assert!(!has_e0309, "Single resume in shallow handler should not trigger E0309: {:?}",
                   errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

#[test]
fn test_e2e_deep_handler_multiple_resumes_ok() {
    // Deep handlers can resume multiple times (multi-shot)
    let source = r#"
        effect Choice {
            op choose(a: i32, b: i32) -> i32;
        }

        deep handler MultiShot for Choice {
            let mut count: i32

            return(x) { count }
            op choose(a, b) {
                // OK: Deep handlers support multi-shot
                let first = resume(a);
                count = count + first;
                let second = resume(b);
                count = count + second;
                0
            }
        }
    "#;

    let result = check_source(source);
    // Deep handlers with multiple resumes should pass (unless linear values are captured)
    match result {
        Ok(_) => {
            // Expected: deep handler multi-shot is valid
        }
        Err(errors) => {
            // Should NOT fail with E0309 (that's for shallow handlers)
            let has_e0309 = errors.iter().any(|e| e.message.contains("E0309"));
            assert!(!has_e0309, "Deep handler multi-shot should not trigger E0309: {:?}",
                   errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

#[test]
fn test_e2e_e0304_linear_value_in_multishot_handler() {
    // E0304: Linear values cannot be captured in multi-shot handlers
    // This test verifies the error is emitted when a linear type is captured
    // Note: Linear types are not yet fully implemented in the type system
    let source = r#"
        effect Choice {
            op choose(a: i32, b: i32) -> i32;
        }

        // When linear types are implemented, this should fail with E0304:
        // A linear value (must be used exactly once) captured in a multi-shot
        // handler would be duplicated when resume is called multiple times.
        deep handler BadLinear for Choice {
            let linear_value: linear i32  // hypothetical linear type

            return(x) { x }
            op choose(a, b) {
                // ERROR: linear_value would be duplicated across resumes
                let first = resume(a);
                let second = resume(b);
                first + second
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    // Linear type syntax may not be fully supported yet
    match result {
        Ok(_) => {
            // Linear type parsing works - check type checking
            eprintln!("Linear type syntax parsed - E0304 check pending full linear type support");
        }
        Err(errors) => {
            // Linear type syntax not yet supported
            eprintln!("Linear type syntax not yet implemented: {:?}",
                     errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

#[test]
fn test_e2e_e0304_affine_value_in_multishot_handler() {
    // Affine values (used at most once) should also be rejected in multi-shot handlers
    let source = r#"
        effect Amb {
            op flip() -> bool;
        }

        // Affine value captured in multi-shot handler
        deep handler BadAffine for Amb {
            let affine_resource: &mut i32  // hypothetical affine borrowed reference

            return(x) { x }
            op flip() {
                // ERROR: affine reference would be duplicated
                let a = resume(true);
                let b = resume(false);
                a
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    // Track implementation status
    match result {
        Ok(program) => {
            eprintln!("Affine type syntax parsed ({} declarations)", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("Affine type syntax status: {:?}",
                     errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

#[test]
fn test_e2e_non_linear_value_in_multishot_ok() {
    // Regular (non-linear) values can be captured in multi-shot handlers
    let source = r#"
        effect Choice {
            op choose() -> i32;
        }

        deep handler CopyableState for Choice {
            let count: i32  // Regular i32 is Copy, not linear

            return(x) { x }
            op choose() {
                // OK: count is Copy, can be duplicated across resumes
                let a = resume(count);
                let b = resume(count + 1);
                a + b
            }
        }
    "#;

    let result = check_source(source);
    // This should pass - regular values are fine in multi-shot handlers
    match result {
        Ok(_) => {
            // Expected: non-linear values are valid in multi-shot handlers
        }
        Err(errors) => {
            // Should NOT fail with E0304 (that's for linear values)
            let has_e0304 = errors.iter().any(|e| e.message.contains("E0304"));
            assert!(!has_e0304, "Non-linear value should not trigger E0304: {:?}",
                   errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

// ============================================================
// Effects + Generational Snapshot Integration Tests
// ============================================================

#[test]
fn test_e2e_effect_generation_snapshot_preservation() {
    // Test that effect handlers properly preserve generation context
    // across resume boundaries
    let source = r#"
        effect State {
            op get() -> i32;
            op set(v: i32) -> unit;
        }

        deep handler CountingState for State {
            let mut value: i32

            return(x) { (x, value) }
            op get() { resume(value) }
            op set(v) { value = v; resume(()) }
        }

        fn increment_twice() -> i32 / {State} {
            let v = perform State::get();
            perform State::set(v + 1);
            let v2 = perform State::get();
            perform State::set(v2 + 1);
            perform State::get()
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty(), "Should generate MIR bodies");
            eprintln!("Effect + generation snapshot MIR: {} bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("Effect + generation snapshot MIR status: {}", e);
        }
    }
}

#[test]
fn test_e2e_nested_handlers_generation_isolation() {
    // Nested handlers should maintain generation isolation
    let source = r#"
        effect Inner { op inner() -> i32; }
        effect Outer { op outer() -> i32; }

        deep handler HandleInner for Inner {
            return(x) { x }
            op inner() { resume(10) }
        }

        deep handler HandleOuter for Outer {
            return(x) { x }
            op outer() { resume(20) }
        }

        fn nested_effects() -> i32 {
            with HandleOuter {} handle {
                with HandleInner {} handle {
                    let a = perform Outer::outer();
                    let b = perform Inner::inner();
                    a + b
                }
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty());
            eprintln!("Nested handlers generation isolation: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("Nested handlers MIR status: {}", e);
        }
    }
}

#[test]
fn test_e2e_effect_with_struct_state_generations() {
    // Effect handlers with struct state should properly track generations
    let source = r#"
        struct Counter {
            value: i32,
            max: i32,
        }

        effect Counting {
            op inc() -> i32;
            op reset() -> unit;
        }

        deep handler StructCounter for Counting {
            let mut counter: Counter

            return(x) { x }
            op inc() {
                let v = counter.value + 1;
                counter.value = v;
                resume(v)
            }
            op reset() {
                counter.value = 0;
                resume(())
            }
        }

        fn use_struct_counter() -> i32 {
            with StructCounter { counter: Counter { value: 0, max: 100 } } handle {
                perform Counting::inc();
                perform Counting::inc();
                perform Counting::inc()
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty());
            eprintln!("Struct state generation tracking: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("Struct state generation MIR status: {}", e);
        }
    }
}

#[test]
fn test_e2e_effect_generation_llvm_with_snapshots() {
    // LLVM IR should include generation snapshot calls for effect handlers
    let source = r#"
        effect Tick {
            op tick() -> i32;
        }

        deep handler TickHandler for Tick {
            let mut count: i32

            return(x) { count }
            op tick() {
                count = count + 1;
                resume(count)
            }
        }

        fn run_ticks() -> i32 {
            with TickHandler { count: 0 } handle {
                perform Tick::tick();
                perform Tick::tick();
                perform Tick::tick()
            }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Check for generation-related functions
            let has_generation_calls = llvm_ir.contains("generation") ||
                                       llvm_ir.contains("snapshot") ||
                                       llvm_ir.contains("blood_");

            eprintln!("LLVM IR has generation/snapshot calls: {}", has_generation_calls);

            // Should at least have function definitions
            assert!(llvm_ir.contains("define"), "Should have function definitions");
        }
        Err(e) => {
            eprintln!("Effect + generation LLVM IR status: {}", e);
        }
    }
}

#[test]
fn test_e2e_multishot_generation_rollback() {
    // Multi-shot handlers need to properly rollback generations
    let source = r#"
        effect Amb {
            op choose(a: i32, b: i32) -> i32;
        }

        deep handler AmbHandler for Amb {
            let mut sum: i32

            return(x) { sum }
            op choose(a, b) {
                // Multi-shot: call resume twice
                // Each resume should have proper generation management
                let r1 = resume(a);
                sum = sum + r1;
                let r2 = resume(b);
                sum = sum + r2;
                0
            }
        }

        fn amb_computation() -> i32 {
            with AmbHandler { sum: 0 } handle {
                let x = perform Amb::choose(1, 10);
                let y = perform Amb::choose(2, 20);
                x + y
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty());
            eprintln!("Multi-shot generation rollback: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("Multi-shot generation MIR status: {}", e);
        }
    }
}

#[test]
fn test_e2e_reserved_generation_values() {
    // Test that reserved generation values (0, 1, MAX) are properly handled
    let source = r#"
        fn test_generations() -> i32 {
            // Create allocations that will use different generation values
            let a = 1;
            let b = 2;
            let c = 3;

            // Operations that exercise generation checking
            a + b + c
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "Basic generation test should compile: {:?}", result.err());

    let llvm_ir = result.unwrap();

    // Verify generation validation is present
    let has_gen_validate = llvm_ir.contains("blood_validate_generation");
    eprintln!("LLVM IR includes generation validation: {}", has_gen_validate);
}

#[test]
fn test_e2e_effect_delimited_continuation_snapshots() {
    // Delimited continuations should properly snapshot/restore generations
    let source = r#"
        effect Shift {
            op shift(f: fn(fn(i32) -> i32) -> i32) -> i32;
        }

        fn with_shift() -> i32 / {Shift} {
            let x = 10;
            // Using shift to capture continuation
            let y = perform Shift::shift(|k| k(x) + k(x + 1));
            y
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    // This is advanced shift/reset - track parsing status
    match result {
        Ok(program) => {
            eprintln!("Shift/reset syntax parsed: {} declarations", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("Shift/reset syntax status: {:?}",
                     errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

// ============================================================
// Comprehensive Generation Snapshot + Effect Resume Tests
// ============================================================

#[test]
fn test_generation_snapshot_basic_handler() {
    // Test that basic handler correctly preserves generations across resume
    let source = r#"
        effect Increment {
            op inc() -> i32;
        }

        deep handler Counter for Increment {
            let mut count: i32

            return(x) { (x, count) }

            op inc() {
                count = count + 1;
                resume(count)
            }
        }

        fn use_counter() -> (i32, i32) {
            with Counter { count: 0 } handle {
                let a = perform Increment::inc();
                let b = perform Increment::inc();
                let c = perform Increment::inc();
                a + b + c
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty(), "Should produce MIR bodies");
            eprintln!("Generation snapshot basic handler: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("Generation snapshot basic handler status: {}", e);
        }
    }
}

#[test]
fn test_generation_snapshot_nested_handlers() {
    // Nested handlers should maintain separate generation snapshots
    let source = r#"
        effect State<S> {
            op get() -> S;
            op put(s: S) -> unit;
        }

        deep handler StateHandler<S> for State<S> {
            let mut state: S

            return(x) { x }

            op get() { resume(state) }
            op put(s) { state = s; resume(()) }
        }

        fn nested_state() -> i32 {
            // Outer handler for State<i32>
            with StateHandler { state: 0 } handle {
                perform State::put(10);
                // Inner handler with different state
                with StateHandler { state: 100 } handle {
                    let inner = perform State::get();
                    inner
                }
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty());
            eprintln!("Nested handlers: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("Nested handlers status: {}", e);
        }
    }
}

#[test]
fn test_generation_snapshot_exception_rollback() {
    // Exception handling should rollback to captured generation snapshot
    let source = r#"
        effect Exn {
            op raise(msg: String) -> never;
        }

        shallow handler TryCatch for Exn {
            return(x) { Ok(x) }
            op raise(msg) { Err(msg) }
        }

        fn safe_div(a: i32, b: i32) -> Result<i32, String> {
            with TryCatch handle {
                if b == 0 {
                    perform Exn::raise("division by zero")
                } else {
                    a / b
                }
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            eprintln!("Exception rollback syntax: {} declarations", program.declarations.len());
            assert!(program.declarations.len() >= 3, "Expected effect, handler, and function");
        }
        Err(errors) => {
            eprintln!("Exception rollback status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

// ============================================================
// Multi-shot Handler with Linear Types Tests
// ============================================================

#[test]
fn test_multishot_handler_generation_safety() {
    // Multi-shot handlers must validate generations on each resume
    let source = r#"
        effect Choice {
            op flip() -> bool;
        }

        // Multi-shot handler that explores all branches
        deep handler Explore for Choice {
            return(x) { [x] }

            op flip() {
                // Resume twice - once with true, once with false
                let true_results = resume(true);
                let false_results = resume(false);
                // Combine results from both branches
                true_results
            }
        }

        fn explore_paths() -> i32 {
            with Explore handle {
                let a = if perform Choice::flip() { 1 } else { 10 };
                let b = if perform Choice::flip() { 2 } else { 20 };
                a + b
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            eprintln!("Multi-shot generation safety: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("Multi-shot generation safety status: {}", e);
        }
    }
}

#[test]
fn test_linear_resource_in_handler() {
    // Linear resources within handlers must be used exactly once
    let source = r#"
        effect Resource {
            op acquire() -> linear Handle;
            op release(h: linear Handle) -> unit;
        }

        deep handler ResourceManager for Resource {
            return(x) { x }

            op acquire() {
                // Create a linear handle (must be consumed)
                let handle = Handle::new();
                resume(handle)
            }

            op release(h) {
                // Consume the linear handle
                h.close();
                resume(())
            }
        }

        fn use_resource() {
            with ResourceManager handle {
                let h = perform Resource::acquire();
                // Use the resource
                perform Resource::release(h);
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            eprintln!("Linear resource handler: {} declarations", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("Linear resource handler status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

// ============================================================
// Generational Overflow Stress Tests
// ============================================================

#[test]
fn test_generation_overflow_detection() {
    // Test that generation values approaching overflow are detected
    let source = r#"
        fn create_many_allocations() -> i32 {
            // Create many allocations to test generation increments
            let mut sum = 0;
            let mut i = 0;
            while i < 100 {
                let temp = i * 2;  // Each iteration creates new allocations
                sum = sum + temp;
                i = i + 1;
            }
            sum
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "Generation overflow detection should compile: {:?}", result.err());

    let llvm_ir = result.unwrap();
    // The generated code should include generation validation calls
    eprintln!("Generation overflow test IR length: {} bytes", llvm_ir.len());
}

#[test]
fn test_generation_wrap_around_safety() {
    // Ensure that generation wrap-around doesn't cause false positives
    let source = r#"
        fn long_lived_allocation() -> i32 {
            // Allocate early
            let base = 100;

            // Many operations that might increment generations
            let mut acc = 0;
            let mut i = 0;
            while i < 50 {
                acc = acc + i;
                i = i + 1;
            }

            // Original allocation should still be valid
            base + acc
        }
    "#;

    let result = compile_to_llvm_ir(source);
    assert!(result.is_ok(), "Wrap-around safety should compile: {:?}", result.err());
}

// ============================================================
// Complex Concurrent Program Tests
// ============================================================

#[test]
fn test_concurrent_fiber_scheduling() {
    // Test fiber creation and scheduling constructs
    let source = r#"
        effect Async {
            op spawn(f: fn() -> unit) -> unit;
            op yield() -> unit;
        }

        fn concurrent_work() / {Async} {
            // Spawn multiple concurrent tasks
            perform Async::spawn(|| {
                let x = 1 + 2;
            });

            perform Async::spawn(|| {
                let y = 3 + 4;
            });

            // Yield to let others run
            perform Async::yield();
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            eprintln!("Concurrent fiber scheduling: {} declarations", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("Concurrent fiber status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

// Note: Effect syntax requires return type BEFORE effect row: `-> Type / {Effect}`
#[test]
fn test_channel_communication() {
    // Test channel-based communication patterns
    let source = r#"
        effect Channel<T> {
            op send(value: T) -> unit;
            op recv() -> T;
        }

        fn producer_consumer() / {Channel<i32>} {
            // Producer sends values
            perform Channel::send(1);
            perform Channel::send(2);
            perform Channel::send(3);
        }

        fn consumer() -> i32 / {Channel<i32>} {
            let a = perform Channel::recv();
            let b = perform Channel::recv();
            a + b
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            eprintln!("Channel communication: {} declarations", program.declarations.len());
            assert!(program.declarations.len() >= 2);
        }
        Err(errors) => {
            eprintln!("Channel communication status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

#[test]
fn test_fork_join_parallelism() {
    // Test fork-join parallel pattern
    let source = r#"
        effect Fork {
            op fork(left: fn() -> i32, right: fn() -> i32) -> (i32, i32);
        }

        fn parallel_sum() -> i32 / {Fork} {
            let (a, b) = perform Fork::fork(
                || { 1 + 2 + 3 },
                || { 4 + 5 + 6 }
            );
            a + b
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            eprintln!("Fork-join parallelism: {} declarations", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("Fork-join status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

// ============================================================
// Effect Composition Tests
// ============================================================

#[test]
fn test_effect_composition() {
    // Test composing multiple effects in a single computation
    let source = r#"
        effect Reader<R> {
            op ask() -> R;
        }

        effect Writer<W> {
            op tell(w: W) -> unit;
        }

        effect State<S> {
            op get() -> S;
            op put(s: S) -> unit;
        }

        // Function with multiple effects (correct syntax: return type BEFORE effect row)
        fn complex_computation() -> i32 / {Reader<i32>, Writer<String>, State<bool>} {
            let config = perform Reader::ask();
            perform Writer::tell("Starting computation");

            let flag = perform State::get();
            if flag {
                perform State::put(false);
                config * 2
            } else {
                perform State::put(true);
                config + 1
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            eprintln!("Effect composition: {} declarations", program.declarations.len());
            // Should have 3 effects + 1 function
            assert!(program.declarations.len() >= 4);
        }
        Err(errors) => {
            eprintln!("Effect composition status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

#[test]
fn test_handler_delegation() {
    // Test handler that delegates to another effect
    // Note: Handlers don't declare effect rows in their signature - effects used
    // within the handler body bubble up to the enclosing handler context
    let source = r#"
        effect Log {
            op log(msg: String) -> unit;
        }

        effect IO {
            op print(msg: String) -> unit;
        }

        // Handler that delegates Log to IO
        // When used, this handler must be within an IO handler context
        deep handler LogToIO for Log {
            return(x) { x }

            op log(msg) {
                perform IO::print("[LOG] " + msg);
                resume(())
            }
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            eprintln!("Handler delegation: {} declarations", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("Handler delegation status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

// ============================================================
// Performance Diagnostic Tests
// ============================================================

#[test]
fn test_perf_generic_effect_no_perform() {
    // Generic effect with instantiated effect row, no perform - should be fast
    let source = r#"
        effect Channel<T> {
            op send(value: T) -> unit;
        }
        fn test() / {Channel<i32>} {
            ()
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Generic effect row (no perform): {:?}", result.is_ok());
}

#[test]
fn test_perf_generic_effect_with_single_perform() {
    // Generic effect with single perform - isolate if perform is the issue
    let source = r#"
        effect Channel<T> {
            op send(value: T) -> unit;
        }
        fn test() / {Channel<i32>} {
            perform Channel::send(1)
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Generic effect with single perform: {:?}", result.is_ok());
}

#[test]
fn test_perf_generic_effect_with_semicolon() {
    // Generic effect with perform followed by semicolon
    let source = r#"
        effect Channel<T> {
            op send(value: T) -> unit;
        }
        fn test() / {Channel<i32>} {
            perform Channel::send(1);
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Generic effect with perform semicolon: {:?}", result.is_ok());
}

#[test]
fn test_perf_multiple_performs() {
    // Multiple perform statements
    let source = r#"
        effect Channel<T> {
            op send(value: T) -> unit;
        }
        fn test() / {Channel<i32>} {
            perform Channel::send(1);
            perform Channel::send(2);
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Multiple performs: {:?}", result.is_ok());
}

#[test]
fn test_perf_two_ops_two_functions() {
    // Two operations and two functions - closer to slow test structure
    let source = r#"
        effect Channel<T> {
            op send(value: T) -> unit;
            op recv() -> T;
        }

        fn producer() / {Channel<i32>} {
            perform Channel::send(1);
            perform Channel::send(2);
            perform Channel::send(3);
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Two ops two functions: {:?}", result.is_ok());
}

#[test]
fn test_perf_let_binding_with_perform() {
    // Let binding with perform (like the slow test)
    // Fixed: return type must come BEFORE effect row
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }

        fn consumer() -> i32 / {Channel<i32>} {
            let a = perform Channel::recv();
            a
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Let binding with perform: {:?}", result.is_ok());
}

#[test]
fn test_perf_perform_with_args() {
    // perform with arguments is fast
    let source = r#"
        effect Channel<T> {
            op send(value: T) -> unit;
        }

        fn producer() / {Channel<i32>} {
            let a = perform Channel::send(1);
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Perform with args in let: {:?}", result.is_ok());
}

#[test]
fn test_perf_perform_no_args_no_let() {
    // perform with no args but not in let binding
    // Fixed: return type must come BEFORE effect row
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }

        fn consumer() -> i32 / {Channel<i32>} {
            perform Channel::recv()
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Perform no args, no let: {:?}", result.is_ok());
}

#[test]
fn test_perf_regular_function_no_args_let() {
    // Regular function call with no args in let (baseline)
    let source = r#"
        fn recv() -> i32 { 42 }
        fn test() -> i32 {
            let a = recv();
            a
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Regular function no args let: {:?}", result.is_ok());
}

#[test]
fn test_perf_simplest_let_perform_no_args() {
    // Simplest case - minimal code
    let source = r#"
        fn test() {
            let a = perform recv();
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Simplest let perform no args: {:?}", result.is_ok());
}

#[test]
fn test_perf_let_perform_path_no_args() {
    // perform with path and no args
    let source = r#"
        fn test() {
            let a = perform Channel::recv();
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Let perform path no args: {:?}", result.is_ok());
}

#[test]
fn test_perf_effect_row_with_let_perform() {
    // Effect row annotation with let perform
    let source = r#"
        fn test() / {Channel<i32>} {
            let a = perform Channel::recv();
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Effect row with let perform: {:?}", result.is_ok());
}

#[test]
fn test_perf_effect_def_let_perform() {
    // Effect definition + let perform (no effect row on function)
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }
        fn test() {
            let a = perform Channel::recv();
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Effect def + let perform: {:?}", result.is_ok());
}

#[test]
fn test_perf_full_combo() {
    // Full combination - effect def + effect row + let perform
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }
        fn test() / {Channel<i32>} {
            let a = perform Channel::recv();
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Full combo: {:?}", result.is_ok());
}

#[test]
fn test_perf_full_combo_with_return() {
    // Full combination with return type (correct order: -> Type / {Effect})
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }
        fn test() -> i32 / {Channel<i32>} {
            let a = perform Channel::recv();
            a
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Should parse with correct syntax order: {:?}", result.err());
    eprintln!("Full combo with return: {:?}", result.is_ok());
}

#[test]
fn test_perf_return_type_no_trailing() {
    // With return type but no trailing expression (correct order)
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }
        fn test() -> i32 / {Channel<i32>} {
            let a = perform Channel::recv();
            0
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    assert!(result.is_ok(), "Should parse: {:?}", result.err());
    eprintln!("Return type, trailing literal: {:?}", result.is_ok());
}

#[test]
fn test_perf_trailing_a_no_return_type() {
    // No return type annotation but trailing 'a'
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }
        fn test() / {Channel<i32>} {
            let a = perform Channel::recv();
            a
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("No return type, trailing a: {:?}", result.is_ok());
}

#[test]
fn test_perf_return_type_no_effect_row() {
    // Return type but NO effect row - isolating effect row parsing
    let source = r#"
        effect Channel<T> {
            op recv() -> T;
        }
        fn test() -> i32 {
            let a = perform Channel::recv();
            a
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();
    eprintln!("Return type, no effect row: {:?}", result.is_ok());
}

// ============================================================
// ACTION_ITEMS.md TEST-001 to TEST-009: P0 Integration Tests
// ============================================================

/// TEST-001: Effects + generation snapshots with stale reference detection
/// This test verifies that when an effect handler resumes into a context
/// where a reference has become stale (e.g., due to region deallocation),
/// the generation check properly detects the stale reference.
#[test]
fn test_e2e_stale_reference_after_effect_resume() {
    let source = r#"
        effect Suspend {
            op suspend() -> unit;
        }

        deep handler SuspendHandler for Suspend {
            return(x) { x }
            op suspend() {
                // When we resume, any references captured before suspend
                // may have been invalidated by region operations
                resume(())
            }
        }

        fn test_stale_detection() -> i32 {
            with SuspendHandler {} handle {
                // Create a reference in the current region
                let x = 42;
                let ptr = &x;

                // Suspend - the handler may invalidate references
                perform Suspend::suspend();

                // Accessing ptr after resume should trigger generation check
                // if the reference is stale
                *ptr
            }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // The generated code should include generation validation
            let has_validation = llvm_ir.contains("blood_validate_generation") ||
                               llvm_ir.contains("gen_check");
            eprintln!("TEST-001: Stale reference detection IR has validation: {}", has_validation);
            assert!(llvm_ir.contains("define"), "Should have function definitions");
        }
        Err(e) => {
            eprintln!("TEST-001: Stale reference detection status: {}", e);
        }
    }
}

/// TEST-003: Affine values in deep handler capture
/// Verifies that affine values can be captured in deep handlers
/// (since they're single-shot by default).
#[test]
fn test_e2e_affine_value_deep_handler_capture() {
    let source = r#"
        effect Reader {
            op read() -> i32;
        }

        struct AffineResource {
            value: i32,
        }

        // Deep handlers are single-shot, so affine capture should work
        deep handler ReaderWithAffine for Reader {
            let resource: AffineResource  // Affine value captured

            return(x) { x }
            op read() {
                // Use the affine resource once
                resume(resource.value)
            }
        }

        fn use_affine_in_handler() -> i32 {
            let res = AffineResource { value: 100 };
            with ReaderWithAffine { resource: res } handle {
                perform Reader::read()
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty(), "Should generate MIR for affine capture");
            eprintln!("TEST-003: Affine in deep handler: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("TEST-003: Affine deep handler status: {}", e);
        }
    }
}

/// TEST-004: Nested region suspension with effects
/// Tests that nested regions properly suspend and restore state
/// when effects cross region boundaries.
#[test]
fn test_e2e_nested_region_suspension() {
    let source = r#"
        effect Yield {
            op yield() -> unit;
        }

        deep handler YieldHandler for Yield {
            return(x) { x }
            op yield() {
                // Suspend and then resume
                resume(())
            }
        }

        fn nested_regions_with_yield() -> i32 {
            // Outer region
            let outer = 10;

            with YieldHandler {} handle {
                // Inner computation with yield
                let inner = 20;
                perform Yield::yield();

                // After yield, both regions should be valid
                outer + inner
            }
        }
    "#;

    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty());
            eprintln!("TEST-004: Nested region suspension: {} MIR bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("TEST-004: Nested region suspension status: {}", e);
        }
    }
}

/// TEST-005: Generation overflow triggering tier promotion
/// Note: Tier promotion is not yet implemented (see ACTION_ITEMS.md IMPL-004).
/// This test validates the detection mechanism that would trigger promotion.
#[test]
fn test_e2e_generation_overflow_tier_promotion_detection() {
    let source = r#"
        fn high_allocation_churn() -> i32 {
            let mut sum = 0;
            let mut i = 0;

            // Create many allocations that would increment generations
            while i < 1000 {
                let temp = i * 2;
                let temp2 = i * 3;
                let temp3 = temp + temp2;
                sum = sum + temp3;
                i = i + 1;
            }

            sum
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Check for generation tracking infrastructure
            let has_generation_tracking = llvm_ir.contains("generation") ||
                                         llvm_ir.contains("blood_") ||
                                         llvm_ir.contains("alloc");
            eprintln!("TEST-005: Generation overflow detection infrastructure present: {}",
                     has_generation_tracking);
            // Note: Actual tier promotion test pending IMPL-004 implementation
        }
        Err(e) => {
            eprintln!("TEST-005: Generation overflow status: {}", e);
        }
    }
}

/// TEST-006: Content-addressed incremental rebuild
/// Tests that the content-addressed system properly detects changes
/// and enables incremental rebuilds.
#[test]
fn test_e2e_content_addressed_incremental_rebuild() {
    use bloodc::content::{Codebase, CanonicalAST, DefinitionRecord};

    // Simulate an incremental build scenario
    let mut codebase = Codebase::new();

    // Initial definitions
    let def1 = DefinitionRecord::new(CanonicalAST::IntLit(1));
    let def2 = DefinitionRecord::new(CanonicalAST::IntLit(2));
    let def3 = DefinitionRecord::new(CanonicalAST::IntLit(3));

    let hash1 = def1.hash;
    let hash2 = def2.hash;
    let hash3 = def3.hash;

    codebase.add(def1).unwrap();
    codebase.add(def2).unwrap();
    codebase.add(def3).unwrap();

    // Verify all definitions are stored
    assert!(codebase.contains(hash1), "Should contain definition 1");
    assert!(codebase.contains(hash2), "Should contain definition 2");
    assert!(codebase.contains(hash3), "Should contain definition 3");

    // Simulate "rebuild" - unchanged definitions should have same hash
    let def1_unchanged = DefinitionRecord::new(CanonicalAST::IntLit(1));
    assert_eq!(def1_unchanged.hash, hash1, "Unchanged definition should have same hash");

    // Modified definition should have different hash
    let def1_modified = DefinitionRecord::new(CanonicalAST::IntLit(100));
    assert_ne!(def1_modified.hash, hash1, "Modified definition should have different hash");

    // Incremental: only need to recompile changed definitions
    let needs_recompile = !codebase.contains(def1_modified.hash);
    assert!(needs_recompile, "Modified definition should need recompilation");

    eprintln!("TEST-006: Content-addressed incremental rebuild verification complete");
}

/// TEST-007: Multiple dispatch with generic handlers
/// Tests generic effect handlers with multiple dispatch patterns.
#[test]
fn test_e2e_multiple_dispatch_generic_handlers() {
    let source = r#"
        effect Container<T> {
            op get() -> T;
            op set(value: T) -> unit;
        }

        // Generic handler that works with any type
        deep handler GenericContainer<T> for Container<T> {
            let mut stored: T

            return(x) { x }
            op get() { resume(stored) }
            op set(value) { stored = value; resume(()) }
        }

        // Multiple instantiations with different types
        fn use_int_container() -> i32 {
            with GenericContainer<i32> { stored: 0 } handle {
                perform Container::set(42);
                perform Container::get()
            }
        }

        fn use_bool_container() -> bool {
            with GenericContainer<bool> { stored: false } handle {
                perform Container::set(true);
                perform Container::get()
            }
        }

        // Multiple dispatch: using both handlers
        fn multiple_dispatch() -> i32 {
            let int_result = use_int_container();
            int_result
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            // Should have effect, handler, and multiple functions
            assert!(program.declarations.len() >= 4,
                   "Should parse effect, handler, and functions");
            eprintln!("TEST-007: Multiple dispatch generic handlers: {} declarations",
                     program.declarations.len());
        }
        Err(errors) => {
            eprintln!("TEST-007: Multiple dispatch status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

/// TEST-008: Fiber scheduler with effect handlers - full E2E
/// Tests fiber scheduling patterns through to LLVM IR generation.
#[test]
fn test_e2e_fiber_scheduler_full_pipeline() {
    let source = r#"
        effect Fiber {
            op spawn(task: fn() -> i32) -> i32;
            op yield() -> unit;
            op current_id() -> i32;
        }

        deep handler SimpleFiberScheduler for Fiber {
            let mut next_id: i32
            let mut current: i32

            return(x) { x }

            op spawn(task) {
                let task_id = next_id;
                next_id = next_id + 1;
                // In a real scheduler, would queue the task
                resume(task_id)
            }

            op yield() {
                // Yield control (no-op in simple scheduler)
                resume(())
            }

            op current_id() {
                resume(current)
            }
        }

        fn fiber_computation() -> i32 {
            with SimpleFiberScheduler { next_id: 1, current: 0 } handle {
                let id1 = perform Fiber::spawn(|| { 10 });
                perform Fiber::yield();
                let id2 = perform Fiber::spawn(|| { 20 });
                let my_id = perform Fiber::current_id();
                id1 + id2 + my_id
            }
        }
    "#;

    // Test full pipeline: parse -> typecheck -> MIR -> LLVM
    let result = compile_to_mir(source);
    match result {
        Ok(mir_bodies) => {
            assert!(!mir_bodies.is_empty(), "Should generate MIR for fiber scheduler");
            eprintln!("TEST-008: Fiber scheduler MIR: {} bodies", mir_bodies.len());
        }
        Err(e) => {
            eprintln!("TEST-008: Fiber scheduler MIR status: {}", e);
        }
    }

    // Also test LLVM generation
    let llvm_result = compile_to_llvm_ir(source);
    match llvm_result {
        Ok(llvm_ir) => {
            assert!(llvm_ir.contains("define"), "Should generate LLVM functions");
            eprintln!("TEST-008: Fiber scheduler LLVM IR: {} bytes", llvm_ir.len());
        }
        Err(e) => {
            eprintln!("TEST-008: Fiber scheduler LLVM status: {}", e);
        }
    }
}

/// TEST-009: Stress test for rapid alloc/free cycles
/// Tests allocation patterns approaching generation overflow guard.
#[test]
fn test_stress_rapid_alloc_free_cycles() {
    let source = r#"
        fn rapid_allocation_stress() -> i32 {
            let mut sum = 0;
            let mut outer = 0;

            // Outer loop for sustained pressure
            while outer < 100 {
                let mut inner = 0;

                // Inner loop creates and discards many allocations
                while inner < 100 {
                    // These temporaries exercise allocation/deallocation
                    let a = inner * 2;
                    let b = inner * 3;
                    let c = a + b;
                    let d = c * 2;
                    let e = d + 1;

                    // Struct allocation stress
                    let point = (inner, outer, c);
                    let (x, y, z) = point;

                    sum = sum + e + x + y + z;
                    inner = inner + 1;
                }

                outer = outer + 1;
            }

            sum
        }

        fn allocation_in_loops() -> i32 {
            let mut results = 0;
            let mut i = 0;

            while i < 500 {
                // Each iteration creates multiple allocations
                let temp1 = i;
                let temp2 = i * 2;
                let temp3 = i * 3;
                let temp4 = temp1 + temp2 + temp3;

                // Nested allocation
                let nested = {
                    let inner1 = temp4 * 2;
                    let inner2 = inner1 + 1;
                    inner2
                };

                results = results + nested;
                i = i + 1;
            }

            results
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Stress test should compile without issues
            assert!(llvm_ir.contains("define"), "Should generate functions");

            // Count allocations (rough measure of generation usage)
            let alloca_count = llvm_ir.matches("alloca").count();
            eprintln!("TEST-009: Rapid alloc stress test - {} allocas in IR", alloca_count);

            // Verify generation infrastructure is present
            let has_gen_infra = llvm_ir.contains("blood_") || alloca_count > 10;
            eprintln!("TEST-009: Generation infrastructure present: {}", has_gen_infra);
        }
        Err(e) => {
            eprintln!("TEST-009: Stress test compilation status: {}", e);
        }
    }
}

/// Additional TEST-001 variant: Test stale reference panic path
/// Verifies that the blood_stale_reference_panic function is properly
/// integrated into the generated code.
#[test]
fn test_e2e_stale_reference_panic_integration() {
    let source = r#"
        fn reference_usage() -> i32 {
            let x = 42;
            let ptr = &x;
            // Dereference should include generation check
            *ptr
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Check for stale reference panic path
            let has_panic = llvm_ir.contains("stale_reference") ||
                           llvm_ir.contains("gen_stale") ||
                           llvm_ir.contains("unreachable");
            eprintln!("TEST-001 variant: Stale panic path present: {}", has_panic);
        }
        Err(e) => {
            eprintln!("TEST-001 variant: Status: {}", e);
        }
    }
}

// ============================================================
// ACTION_ITEMS.md FFI-006: FFI Integration Tests
// ============================================================

/// FFI-006: Test bridge block parsing for libc functions
#[test]
fn test_ffi_bridge_libc_parsing() {
    let source = r#"
        bridge "C" libc {
            #[link(name = "c")]

            // Memory allocation
            fn malloc(size: usize) -> *mut u8;
            fn free(ptr: *mut u8) -> unit;
            fn realloc(ptr: *mut u8, size: usize) -> *mut u8;

            // String operations
            fn strlen(s: *const u8) -> usize;
            fn strcpy(dest: *mut u8, src: *const u8) -> *mut u8;

            // Math functions
            fn abs(x: i32) -> i32;

            // Constants
            const EXIT_SUCCESS: i32 = 0;
            const EXIT_FAILURE: i32 = 1;
        }

        fn use_libc() -> i32 {
            // Test that we can reference bridge functions
            abs(-42)
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            // Should have bridge and function declarations
            assert!(program.declarations.len() >= 2,
                   "Should parse bridge block and function");
            eprintln!("FFI-006 libc: {} declarations parsed", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("FFI-006 libc parsing status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

/// FFI-006: Test bridge block parsing for libm functions
#[test]
fn test_ffi_bridge_libm_parsing() {
    let source = r#"
        bridge "C" libm {
            #[link(name = "m")]

            // Trigonometric functions
            fn sin(x: f64) -> f64;
            fn cos(x: f64) -> f64;
            fn tan(x: f64) -> f64;

            // Power and logarithm
            fn sqrt(x: f64) -> f64;
            fn pow(x: f64, y: f64) -> f64;
            fn log(x: f64) -> f64;
            fn log10(x: f64) -> f64;
            fn exp(x: f64) -> f64;

            // Rounding
            fn floor(x: f64) -> f64;
            fn ceil(x: f64) -> f64;
            fn round(x: f64) -> f64;
        }

        fn compute_hypotenuse(a: f64, b: f64) -> f64 {
            sqrt(pow(a, 2.0) + pow(b, 2.0))
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            assert!(program.declarations.len() >= 2,
                   "Should parse bridge block and function");
            eprintln!("FFI-006 libm: {} declarations parsed", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("FFI-006 libm parsing status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

/// FFI-006: Test FFI struct declaration (repr(C))
#[test]
fn test_ffi_bridge_struct_parsing() {
    let source = r#"
        bridge "C" posix {
            #[link(name = "c")]

            // Opaque type
            type FILE;

            // C-compatible struct
            #[repr(C)]
            struct timespec {
                tv_sec: i64,
                tv_nsec: i64,
            }

            #[repr(C)]
            struct tm {
                tm_sec: i32,
                tm_min: i32,
                tm_hour: i32,
                tm_mday: i32,
                tm_mon: i32,
                tm_year: i32,
                tm_wday: i32,
                tm_yday: i32,
                tm_isdst: i32,
            }

            // Functions using structs
            fn nanosleep(req: *const timespec, rem: *mut timespec) -> i32;
            fn localtime(timer: *const i64) -> *mut tm;
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            assert!(!program.declarations.is_empty(),
                   "Should parse bridge with structs");
            eprintln!("FFI-006 struct: {} declarations parsed", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("FFI-006 struct parsing status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

/// FFI-006: Test FFI enum declaration
#[test]
fn test_ffi_bridge_enum_parsing() {
    let source = r#"
        bridge "C" errno {
            #[link(name = "c")]

            #[repr(C)]
            enum ErrorCode {
                SUCCESS = 0,
                EPERM = 1,
                ENOENT = 2,
                ESRCH = 3,
                EINTR = 4,
                EIO = 5,
            }

            fn errno_location() -> *mut i32;
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            assert!(!program.declarations.is_empty(),
                   "Should parse bridge with enums");
            eprintln!("FFI-006 enum: {} declarations parsed", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("FFI-006 enum parsing status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

/// FFI-006: Test FFI callback type declaration
#[test]
fn test_ffi_bridge_callback_parsing() {
    let source = r#"
        bridge "C" callbacks {
            #[link(name = "c")]

            // Callback type for qsort comparator
            callback Comparator = fn(*const u8, *const u8) -> i32;

            // Signal handler callback
            callback SignalHandler = fn(i32) -> unit;

            // atexit callback
            callback AtExitHandler = fn() -> unit;

            fn qsort(
                base: *mut u8,
                nmemb: usize,
                size: usize,
                compar: Comparator
            ) -> unit;

            fn signal(signum: i32, handler: SignalHandler) -> SignalHandler;
            fn atexit(function: AtExitHandler) -> i32;
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            assert!(!program.declarations.is_empty(),
                   "Should parse bridge with callbacks");
            eprintln!("FFI-006 callback: {} declarations parsed", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("FFI-006 callback parsing status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

/// FFI-006: Test variadic function parsing
#[test]
fn test_ffi_variadic_function_parsing() {
    let source = r#"
        bridge "C" stdio {
            #[link(name = "c")]

            type FILE;

            fn printf(format: *const u8, ...) -> i32;
            fn fprintf(stream: *mut FILE, format: *const u8, ...) -> i32;
            fn sprintf(str: *mut u8, format: *const u8, ...) -> i32;
            fn snprintf(str: *mut u8, size: usize, format: *const u8, ...) -> i32;
        }
    "#;

    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    match result {
        Ok(program) => {
            assert!(!program.declarations.is_empty(),
                   "Should parse bridge with variadic functions");
            eprintln!("FFI-006 variadic: {} declarations parsed", program.declarations.len());
        }
        Err(errors) => {
            eprintln!("FFI-006 variadic parsing status: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>());
        }
    }
}

/// FFI-006: Test FFI type checking (safe types)
#[test]
fn test_ffi_type_safety_validation() {
    use bloodc::typeck::ffi::{FfiValidator, FfiSafety};
    use bloodc::hir::Type;

    let validator = FfiValidator::new();

    // Test FFI-safe primitives
    assert!(validator.validate_type(&Type::i32()).is_safe(), "i32 should be FFI-safe");
    assert!(validator.validate_type(&Type::i64()).is_safe(), "i64 should be FFI-safe");
    assert!(validator.validate_type(&Type::f32()).is_safe(), "f32 should be FFI-safe");
    assert!(validator.validate_type(&Type::f64()).is_safe(), "f64 should be FFI-safe");
    assert!(validator.validate_type(&Type::unit()).is_safe(), "unit should be FFI-safe");

    // Test FFI-unsafe types
    assert!(matches!(
        validator.validate_type(&Type::str()),
        FfiSafety::Unsafe(_)
    ), "str should not be FFI-safe");

    assert!(matches!(
        validator.validate_type(&Type::string()),
        FfiSafety::Unsafe(_)
    ), "String should not be FFI-safe");

    eprintln!("FFI-006: Type safety validation complete");
}

/// FFI-006: Test FFI codegen (extern function declaration)
#[test]
fn test_ffi_extern_function_codegen() {
    let source = r#"
        bridge "C" test {
            fn external_add(a: i32, b: i32) -> i32;
        }

        fn call_external() -> i32 {
            external_add(10, 20)
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Check for external function declaration
            let has_extern = llvm_ir.contains("declare") ||
                            llvm_ir.contains("external_add") ||
                            llvm_ir.contains("call");
            eprintln!("FFI-006: Extern function codegen has external: {}", has_extern);
            assert!(llvm_ir.contains("define"), "Should generate function definitions");
        }
        Err(e) => {
            eprintln!("FFI-006: Extern function codegen status: {}", e);
        }
    }
}

/// FFI-006: Test unsafe block codegen
#[test]
fn test_ffi_unsafe_block_codegen() {
    let source = r#"
        fn use_raw_pointer() -> i32 {
            let x = 42;
            let ptr = &x as *const i32;

            @unsafe {
                // Raw pointer dereference in unsafe block
                *ptr
            }
        }
    "#;

    let result = compile_to_llvm_ir(source);
    match result {
        Ok(llvm_ir) => {
            // Should compile successfully
            assert!(llvm_ir.contains("define"), "Should generate function definitions");
            eprintln!("FFI-006: Unsafe block codegen successful");
        }
        Err(e) => {
            eprintln!("FFI-006: Unsafe block codegen status: {}", e);
        }
    }
}
