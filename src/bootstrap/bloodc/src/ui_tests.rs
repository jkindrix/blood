//! UI Snapshot Tests for Error Messages
//!
//! These tests verify that compiler error messages are:
//! 1. Clear and helpful to users
//! 2. Consistent across versions
//! 3. Properly formatted with spans and suggestions
//!
//! Uses `insta` for snapshot testing.

use crate::parser::Parser;
use crate::typeck::check_program;
use crate::diagnostics::Diagnostic;

/// Helper to parse and type-check source, collecting diagnostics.
fn get_diagnostics(source: &str) -> Vec<Diagnostic> {
    let mut parser = Parser::new(source);
    match parser.parse_program() {
        Err(errors) => {
            return errors.into_iter().map(|e| {
                Diagnostic::error(e.message, crate::span::Span::default())
            }).collect();
        }
        Ok(program) => {
            let interner = parser.take_interner();
            match check_program(&program, source, interner) {
                Err(errors) => errors,
                Ok(_) => vec![],
            }
        }
    }
}

/// Format diagnostics as a string for snapshot comparison.
fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return "No errors".to_string();
    }

    diagnostics.iter().enumerate().map(|(i, d)| {
        let mut output = format!("Error {}: {}", i + 1, d.message);
        if let Some(code) = &d.code {
            output = format!("[{}] {}", code, output);
        }
        if !d.suggestions.is_empty() {
            output.push_str("\n  Suggestions:");
            for s in &d.suggestions {
                output.push_str(&format!("\n    - {}", s));
            }
        }
        output
    }).collect::<Vec<_>>().join("\n\n")
}

// ============================================================
// TYPE MISMATCH ERRORS
// ============================================================

#[test]
fn test_ui_type_mismatch_return() {
    let source = r#"
fn foo() -> i32 {
    "not an int"
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("type_mismatch_return", formatted);
}

#[test]
fn test_ui_type_mismatch_let() {
    let source = r#"
fn main() {
    let x: bool = 42;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("type_mismatch_let", formatted);
}

#[test]
fn test_ui_type_mismatch_function_arg() {
    let source = r#"
fn takes_int(x: i32) {}
fn main() {
    takes_int("hello");
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("type_mismatch_function_arg", formatted);
}

#[test]
fn test_ui_type_mismatch_if_branches() {
    let source = r#"
fn main() {
    let x = if true { 1 } else { "no" };
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("type_mismatch_if_branches", formatted);
}

// ============================================================
// UNDEFINED NAME ERRORS
// ============================================================

#[test]
fn test_ui_undefined_variable() {
    let source = r#"
fn main() {
    let x = undefined_var;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("undefined_variable", formatted);
}

#[test]
fn test_ui_undefined_function() {
    let source = r#"
fn main() {
    unknown_function();
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("undefined_function", formatted);
}

#[test]
fn test_ui_undefined_type() {
    let source = r#"
fn main() {
    let x: UnknownType = 42;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("undefined_type", formatted);
}

// ============================================================
// STRUCT ERRORS
// ============================================================

#[test]
fn test_ui_struct_missing_field() {
    let source = r#"
struct Point { x: i32, y: i32 }
fn main() {
    let p = Point { x: 10 };
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("struct_missing_field", formatted);
}

#[test]
fn test_ui_struct_unknown_field() {
    let source = r#"
struct Point { x: i32, y: i32 }
fn main() {
    let p = Point { x: 10, y: 20, z: 30 };
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("struct_unknown_field", formatted);
}

#[test]
fn test_ui_struct_field_type_mismatch() {
    let source = r#"
struct Point { x: i32, y: i32 }
fn main() {
    let p = Point { x: "wrong", y: 20 };
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("struct_field_type_mismatch", formatted);
}

// ============================================================
// FUNCTION ERRORS
// ============================================================

#[test]
fn test_ui_wrong_arg_count() {
    let source = r#"
fn add(a: i32, b: i32) -> i32 { a + b }
fn main() {
    add(1);
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("wrong_arg_count", formatted);
}

#[test]
fn test_ui_too_many_args() {
    let source = r#"
fn single(x: i32) {}
fn main() {
    single(1, 2, 3);
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("too_many_args", formatted);
}

// ============================================================
// PARSE ERRORS
// ============================================================

#[test]
fn test_ui_parse_missing_semicolon() {
    let source = r#"
fn main() {
    let x = 42
    let y = 43;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("parse_missing_semicolon", formatted);
}

#[test]
fn test_ui_parse_unexpected_token() {
    let source = r#"
fn main() {
    let = 42;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("parse_unexpected_token", formatted);
}

#[test]
fn test_ui_parse_unclosed_brace() {
    let source = r#"
fn main() {
    if true {
        let x = 1;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("parse_unclosed_brace", formatted);
}

// ============================================================
// MATCH ERRORS
// ============================================================

#[test]
fn test_ui_match_arm_type_mismatch() {
    let source = r#"
fn main() {
    let x = 1;
    let y = match x {
        0 => 0,
        1 => "one",
        _ => 2,
    };
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("match_arm_type_mismatch", formatted);
}

// ============================================================
// GENERIC ERRORS
// ============================================================

#[test]
fn test_ui_generic_inference_failure() {
    let source = r#"
fn identity<T>(x: T) -> T { x }
fn main() {
    // Can't infer T with no arguments
    let f = identity;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("generic_inference_failure", formatted);
}

#[test]
fn test_ui_generic_type_mismatch() {
    let source = r#"
fn first<T>(a: T, b: T) -> T { a }
fn main() {
    first(1, "two");
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("generic_type_mismatch", formatted);
}

// ============================================================
// CONTROL FLOW ERRORS
// ============================================================

#[test]
fn test_ui_if_condition_not_bool() {
    let source = r#"
fn main() {
    if 42 {
        let x = 1;
    }
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("if_condition_not_bool", formatted);
}

#[test]
fn test_ui_while_condition_not_bool() {
    let source = r#"
fn main() {
    while "forever" {
        let x = 1;
    }
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("while_condition_not_bool", formatted);
}

// ============================================================
// OPERATOR ERRORS
// ============================================================

#[test]
fn test_ui_invalid_binary_op() {
    let source = r#"
fn main() {
    let x = "hello" + 42;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("invalid_binary_op", formatted);
}

#[test]
fn test_ui_invalid_unary_op() {
    let source = r#"
fn main() {
    let x = -"hello";
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("invalid_unary_op", formatted);
}

// ============================================================
// ARRAY ERRORS
// ============================================================

#[test]
fn test_ui_array_heterogeneous() {
    let source = r#"
fn main() {
    let arr = [1, "two", 3];
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("array_heterogeneous", formatted);
}

// ============================================================
// DUPLICATE DEFINITION ERRORS
// ============================================================

#[test]
fn test_ui_duplicate_function() {
    let source = r#"
fn foo() {}
fn foo() {}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("duplicate_function", formatted);
}

#[test]
fn test_ui_duplicate_struct() {
    let source = r#"
struct Point { x: i32 }
struct Point { y: i32 }
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("duplicate_struct", formatted);
}

// ============================================================
// REFERENCE ERRORS
// ============================================================

#[test]
fn test_ui_mutate_immutable() {
    let source = r#"
fn main() {
    let x = 42;
    x = 43;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("mutate_immutable", formatted);
}

#[test]
fn test_ui_mut_ref_to_immutable() {
    let source = r#"
fn main() {
    let x = 42;
    let r = &mut x;
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("mut_ref_to_immutable", formatted);
}

// ============================================================
// CLOSURE ERRORS
// ============================================================

#[test]
fn test_ui_closure_param_type_mismatch() {
    let source = r#"
fn apply(f: fn(i32) -> i32, x: i32) -> i32 { f(x) }
fn main() {
    apply(|x: bool| 42, 10);
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("closure_param_type_mismatch", formatted);
}

// ============================================================
// EFFECT ERRORS
// ============================================================

#[test]
fn test_ui_undefined_effect() {
    let source = r#"
fn foo() / UnknownEffect {
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("undefined_effect", formatted);
}

// ============================================================
// MODULE ERRORS
// ============================================================

#[test]
fn test_ui_undefined_module_path() {
    let source = r#"
fn main() {
    unknown_module::foo();
}
"#;
    let diagnostics = get_diagnostics(source);
    let formatted = format_diagnostics(&diagnostics);
    insta::assert_snapshot!("undefined_module_path", formatted);
}
