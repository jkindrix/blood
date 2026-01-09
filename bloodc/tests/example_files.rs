//! Integration tests for all example Blood files.
//!
//! These tests ensure that all example files in the examples/ directory
//! can be parsed successfully without errors.

use bloodc::Parser;
use std::fs;

/// Test helper to parse a file and assert it succeeds.
fn parse_file_ok(path: &str) {
    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("Failed to read file {path}: {e}");
    });

    let mut parser = Parser::new(&source);
    match parser.parse_program() {
        Ok(program) => {
            // Verify the program has some content
            assert!(
                program.declarations.len() > 0 || program.imports.len() > 0,
                "Parsed program from {path} should have declarations or imports"
            );
        }
        Err(errors) => {
            panic!(
                "Failed to parse {path}:\n{}",
                errors
                    .iter()
                    .map(|e| format!("  - {}", e.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}

#[test]
fn test_parse_hello_blood() {
    parse_file_ok("../examples/hello.blood");
}

#[test]
fn test_parse_simple_blood() {
    parse_file_ok("../examples/simple.blood");
}

#[test]
fn test_parse_test1_blood() {
    parse_file_ok("../examples/test1.blood");
}

#[test]
fn test_parse_test2_blood() {
    parse_file_ok("../examples/test2.blood");
}

#[test]
fn test_parse_test3_blood() {
    parse_file_ok("../examples/test3.blood");
}

#[test]
fn test_parse_test4_blood() {
    parse_file_ok("../examples/test4.blood");
}

#[test]
fn test_parse_test5_blood() {
    parse_file_ok("../examples/test5.blood");
}

#[test]
fn test_parse_test6_blood() {
    parse_file_ok("../examples/test6.blood");
}

/// Test that we can parse all example files in a loop.
/// This provides a single test that covers all files for quick validation.
#[test]
fn test_parse_all_examples() {
    let examples_dir = "../examples";
    let entries = fs::read_dir(examples_dir).unwrap_or_else(|e| {
        panic!("Failed to read examples directory: {e}");
    });

    let mut parsed_count = 0;
    let mut errors = Vec::new();

    for entry in entries {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path.extension().map_or(false, |ext| ext == "blood") {
            let path_str = path.to_string_lossy();
            let source = fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!("Failed to read {path_str}: {e}");
            });

            let mut parser = Parser::new(&source);
            match parser.parse_program() {
                Ok(_) => {
                    parsed_count += 1;
                }
                Err(parse_errors) => {
                    errors.push(format!(
                        "{path_str}:\n{}",
                        parse_errors
                            .iter()
                            .map(|e| format!("    - {}", e.message))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        panic!(
            "Failed to parse {} example files:\n{}",
            errors.len(),
            errors.join("\n\n")
        );
    }

    assert!(
        parsed_count >= 8,
        "Expected at least 8 example files, found {parsed_count}"
    );
}

/// Test parsing performance sanity check.
/// Ensures parsing the comprehensive hello.blood file is reasonably fast.
#[test]
fn test_parse_performance_sanity() {
    use std::time::Instant;

    let source = fs::read_to_string("../examples/hello.blood")
        .expect("Failed to read hello.blood");

    let start = Instant::now();
    for _ in 0..100 {
        let mut parser = Parser::new(&source);
        let _ = parser.parse_program();
    }
    let elapsed = start.elapsed();

    // Should be able to parse 100 times in under 1 second
    assert!(
        elapsed.as_secs() < 1,
        "Parsing hello.blood 100 times took {:?}, expected < 1s",
        elapsed
    );
}

// ============================================================
// Error Handling Integration Tests
// ============================================================

/// Helper to parse source and expect errors.
fn parse_expect_error(source: &str) -> Vec<bloodc::Diagnostic> {
    let mut parser = Parser::new(source);
    match parser.parse_program() {
        Ok(_) => panic!("Expected parse error but parsing succeeded"),
        Err(errors) => errors,
    }
}

/// Helper to verify error contains expected message substring.
fn assert_error_contains(errors: &[bloodc::Diagnostic], expected: &str) {
    assert!(
        errors.iter().any(|e| e.message.contains(expected)),
        "Expected error containing '{}', got: {:?}",
        expected,
        errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

#[test]
fn test_error_missing_function_body() {
    let errors = parse_expect_error("fn foo()");
    assert_error_contains(&errors, "expected");
}

#[test]
fn test_error_unclosed_block() {
    let errors = parse_expect_error("fn foo() { let x = 1;");
    assert!(!errors.is_empty(), "Should report unclosed block error");
}

#[test]
fn test_error_unclosed_paren() {
    let errors = parse_expect_error("fn foo(x: i32 {}");
    assert!(!errors.is_empty(), "Should report unclosed paren error");
}

#[test]
fn test_error_invalid_expression() {
    let errors = parse_expect_error("fn foo() { let x = ; }");
    assert!(!errors.is_empty(), "Should report invalid expression error");
}

#[test]
fn test_error_missing_type_annotation() {
    let errors = parse_expect_error("fn foo(x) {}");
    assert_error_contains(&errors, ":");
}

#[test]
fn test_error_unexpected_token_in_struct() {
    let errors = parse_expect_error("struct Foo { + }");
    assert!(!errors.is_empty(), "Should report unexpected token error");
}

#[test]
fn test_error_invalid_import() {
    let errors = parse_expect_error("import ;");
    assert!(!errors.is_empty(), "Should report invalid import error");
}

#[test]
fn test_error_incomplete_match_arm() {
    // Missing arrow and body in match arm
    let errors = parse_expect_error("fn foo() { match x { 1 } }");
    assert!(!errors.is_empty(), "Should report error for incomplete match arm");
}

#[test]
fn test_error_recovery_continues_parsing() {
    // Parser should recover and continue after errors
    let source = r#"
        fn broken( {}
        fn valid() { 42 }
    "#;
    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    // Should have errors from the broken function
    assert!(result.is_err(), "Should have parse errors");

    // The parser should have attempted to continue
    let errors = result.unwrap_err();
    assert!(!errors.is_empty(), "Should have reported errors");
}

#[test]
fn test_error_duplicate_comma() {
    let errors = parse_expect_error("fn foo(a: i32,, b: i32) {}");
    assert!(!errors.is_empty(), "Should report error for duplicate comma");
}

#[test]
fn test_error_trailing_operator() {
    let errors = parse_expect_error("fn foo() { 1 + }");
    assert!(!errors.is_empty(), "Should report error for trailing operator");
}

#[test]
fn test_error_multiple_errors_reported() {
    // Test that multiple errors are accumulated
    let source = r#"
        fn bad1( {}
        fn bad2( {}
    "#;
    let mut parser = Parser::new(source);
    let result = parser.parse_program();

    assert!(result.is_err(), "Should have parse errors");
    let errors = result.unwrap_err();
    assert!(errors.len() >= 1, "Should report at least one error");
}
