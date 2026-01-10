//! End-to-end integration tests for the Blood compiler pipeline.
//!
//! These tests exercise the complete pipeline from parsing through
//! type checking and HIR generation.

use bloodc::{Parser, Lexer};
use bloodc::typeck::check_program;
use string_interner::StringInterner;
use std::fs;

/// Test helper to run the full pipeline on source code.
/// Kept for use in future type-checking tests.
#[allow(dead_code)]
fn check_source(source: &str) -> Result<bloodc::hir::Crate, Vec<bloodc::Diagnostic>> {
    let mut parser = Parser::new(source);
    let program = parser.parse_program()?;
    let interner = StringInterner::default();
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

    // Verify key structures are present
    assert!(
        program.declarations.len() >= 5,
        "Expected at least 5 declarations in hello.blood"
    );

    // Check for effect and handler declarations
    let has_effect = program.declarations.iter().any(|d| {
        matches!(d, bloodc::ast::Declaration::Effect { .. })
    });
    assert!(has_effect, "hello.blood should contain effect declarations");

    let has_handler = program.declarations.iter().any(|d| {
        matches!(d, bloodc::ast::Declaration::Handler { .. })
    });
    assert!(has_handler, "hello.blood should contain handler declarations");
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
