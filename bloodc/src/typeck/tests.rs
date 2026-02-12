//! Comprehensive type checker tests.
//!
//! Tests organized by category:
//! - Type inference
//! - Generic instantiation
//! - Trait resolution
//! - Effect system
//! - Error detection

use crate::parser::Parser;
use crate::typeck::check_program;
use crate::diagnostics::Diagnostic;

/// Helper to parse and type-check source code.
fn typecheck(source: &str) -> Result<(), Vec<Diagnostic>> {
    let mut parser = Parser::new(source);
    let program = parser.parse_program().map_err(|e| {
        e.into_iter().map(|err| {
            Diagnostic::error(err.message, crate::span::Span::default())
        }).collect::<Vec<_>>()
    })?;

    // Use the interner from the parser - it has all symbols populated
    let interner = parser.take_interner();
    check_program(&program, source, interner).map(|_| ())
}

/// Helper to assert type checking succeeds.
fn assert_typechecks(source: &str) {
    match typecheck(source) {
        Ok(()) => {}
        Err(errors) => {
            panic!(
                "Expected type checking to succeed, but got {} error(s):\n{}",
                errors.len(),
                errors.iter()
                    .map(|e| format!("  - {}", e.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}

/// Helper to assert type checking fails with specific error.
fn assert_type_error(source: &str, expected_substring: &str) {
    match typecheck(source) {
        Ok(()) => {
            panic!(
                "Expected type error containing '{}', but type checking succeeded",
                expected_substring
            );
        }
        Err(errors) => {
            let has_expected = errors.iter().any(|e| e.message.contains(expected_substring));
            if !has_expected {
                panic!(
                    "Expected error containing '{}', but got:\n{}",
                    expected_substring,
                    errors.iter()
                        .map(|e| format!("  - {}", e.message))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
    }
}

/// Helper to assert type checking fails.
fn assert_type_fails(source: &str) {
    match typecheck(source) {
        Ok(()) => {
            panic!("Expected type checking to fail, but it succeeded");
        }
        Err(_) => {}
    }
}

// ============================================================
// BASIC TYPE INFERENCE TESTS
// ============================================================

#[test]
fn test_infer_integer_literal() {
    assert_typechecks("fn main() { let x = 42; }");
}

#[test]
fn test_infer_float_literal() {
    assert_typechecks("fn main() { let x = 3.14; }");
}

#[test]
fn test_infer_bool_literal() {
    assert_typechecks("fn main() { let x = true; let y = false; }");
}

#[test]
fn test_infer_string_literal() {
    assert_typechecks(r#"fn main() { let x = "hello"; }"#);
}

#[test]
fn test_infer_from_annotation() {
    assert_typechecks("fn main() { let x: i32 = 42; }");
}

#[test]
fn test_infer_from_function_return() {
    assert_typechecks("fn foo() -> i32 { 42 } fn main() { let x = foo(); }");
}

#[test]
fn test_infer_arithmetic() {
    assert_typechecks("fn main() { let x = 1 + 2 * 3 - 4 / 2; }");
}

#[test]
fn test_infer_comparison() {
    assert_typechecks("fn main() { let x = 1 < 2; let y = 3 >= 4; }");
}

#[test]
fn test_infer_logical() {
    assert_typechecks("fn main() { let x = true && false || !true; }");
}

// ============================================================
// TYPE ANNOTATION TESTS
// ============================================================

#[test]
fn test_explicit_type_annotation() {
    assert_typechecks("fn main() { let x: i64 = 42; }");
}

#[test]
fn test_type_annotation_mismatch() {
    assert_type_fails("fn main() { let x: bool = 42; }");
}

#[test]
fn test_return_type_mismatch() {
    assert_type_fails("fn foo() -> bool { 42 }");
}

#[test]
fn test_parameter_type_mismatch() {
    assert_type_fails("fn foo(x: i32) {} fn main() { foo(true); }");
}

// ============================================================
// FUNCTION TYPE TESTS
// ============================================================

#[test]
fn test_function_no_params() {
    assert_typechecks("fn foo() {} fn main() { foo(); }");
}

#[test]
fn test_function_with_params() {
    assert_typechecks("fn add(a: i32, b: i32) -> i32 { a + b } fn main() { add(1, 2); }");
}

#[test]
fn test_function_wrong_arg_count() {
    assert_type_fails("fn foo(x: i32) {} fn main() { foo(1, 2); }");
}

#[test]
fn test_function_missing_args() {
    assert_type_fails("fn foo(x: i32, y: i32) {} fn main() { foo(1); }");
}

#[test]
fn test_recursive_function() {
    assert_typechecks(r#"
        fn factorial(n: i32) -> i32 {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
    "#);
}

// ============================================================
// STRUCT TYPE TESTS
// ============================================================

#[test]
fn test_struct_construction() {
    assert_typechecks(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 10, y: 20 }; }
    "#);
}

#[test]
fn test_struct_field_access() {
    assert_typechecks(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 10, y: 20 }; let x = p.x; }
    "#);
}

#[test]
fn test_struct_missing_field() {
    assert_type_fails(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 10 }; }
    "#);
}

#[test]
fn test_struct_unknown_field() {
    assert_type_fails(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 10, y: 20, z: 30 }; }
    "#);
}

#[test]
fn test_struct_field_wrong_type() {
    assert_type_fails(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: true, y: 20 }; }
    "#);
}

#[test]
fn test_struct_access_nonexistent_field() {
    assert_type_fails(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 10, y: 20 }; let z = p.z; }
    "#);
}

// ============================================================
// ENUM TYPE TESTS
// ============================================================

#[test]
fn test_enum_unit_variant() {
    assert_typechecks(r#"
        enum Color { Red, Green, Blue }
        fn main() { let c = Color::Red; }
    "#);
}

#[test]
fn test_enum_tuple_variant() {
    assert_typechecks(r#"
        enum MyOption { Some(i32), None }
        fn main() { let x = MyOption::Some(42); let y = MyOption::None; }
    "#);
}

#[test]
fn test_enum_struct_variant() {
    assert_typechecks(r#"
        enum Message { Move { x: i32, y: i32 }, Quit }
        fn main() { let m = Message::Move { x: 10, y: 20 }; }
    "#);
}

#[test]
fn test_enum_variant_wrong_args() {
    assert_type_fails(r#"
        enum MyOption { Some(i32), None }
        fn main() { let x = MyOption::Some(true); }
    "#);
}

// ============================================================
// PATTERN MATCHING TYPE TESTS
// ============================================================

#[test]
fn test_match_exhaustive() {
    assert_typechecks(r#"
        enum Bool { True, False }
        fn main() {
            let b = Bool::True;
            match b {
                Bool::True => 1,
                Bool::False => 0,
            };
        }
    "#);
}

#[test]
fn test_match_with_wildcard() {
    assert_typechecks(r#"
        fn main() {
            let x = 5;
            match x {
                0 => 0,
                1 => 1,
                _ => 2,
            };
        }
    "#);
}

#[test]
fn test_match_binding_type() {
    assert_typechecks(r#"
        enum MyOption { Some(i32), None }
        fn main() {
            let opt = MyOption::Some(42);
            match opt {
                MyOption::Some(x) => x,
                MyOption::None => 0,
            };
        }
    "#);
}

#[test]
fn test_match_arm_type_mismatch() {
    assert_type_fails(r#"
        fn main() {
            let x = true;
            match x {
                true => 1,
                false => "no",
            };
        }
    "#);
}

// ============================================================
// IF EXPRESSION TYPE TESTS
// ============================================================

#[test]
fn test_if_condition_must_be_bool() {
    assert_type_fails("fn main() { if 42 { 1 } else { 2 }; }");
}

#[test]
fn test_if_branches_same_type() {
    assert_typechecks("fn main() { let x = if true { 1 } else { 2 }; }");
}

#[test]
fn test_if_branches_different_types() {
    assert_type_fails(r#"fn main() { let x = if true { 1 } else { "no" }; }"#);
}

#[test]
fn test_if_no_else_is_unit() {
    assert_typechecks("fn main() { if true { let x = 1; } }");
}

// ============================================================
// LOOP TYPE TESTS
// ============================================================

#[test]
fn test_while_condition_must_be_bool() {
    assert_type_fails("fn main() { while 42 {} }");
}

#[test]
fn test_for_loop_iterator() {
    assert_typechecks("fn main() { for i in 0..10 { let x = i; } }");
}

#[test]
fn test_loop_break_value() {
    assert_typechecks("fn main() { let x: i32 = loop { break 42; }; }");
}

// ============================================================
// REFERENCE TYPE TESTS
// ============================================================

#[test]
fn test_reference_creation() {
    assert_typechecks("fn main() { let x = 42; let r = &x; }");
}

#[test]
fn test_mutable_reference() {
    assert_typechecks("fn main() { let mut x = 42; let r = &mut x; }");
}

#[test]
fn test_dereference() {
    assert_typechecks("fn main() { let x = 42; let r = &x; let y = *r; }");
}

#[test]
fn test_reference_parameter() {
    assert_typechecks("fn foo(x: &i32) -> i32 { *x } fn main() { let x = 42; foo(&x); }");
}

// ============================================================
// GENERIC TYPE TESTS
// ============================================================

#[test]
fn test_generic_function() {
    assert_typechecks("fn identity<T>(x: T) -> T { x } fn main() { identity(42); }");
}

#[test]
fn test_generic_struct() {
    assert_typechecks(r#"
        struct MyBox<T> { value: T }
        fn main() { let b = MyBox { value: 42 }; }
    "#);
}

#[test]
fn test_generic_enum() {
    assert_typechecks(r#"
        enum MyOption<T> { Some(T), None }
        fn main() { let x = MyOption::Some(42); }
    "#);
}

#[test]
fn test_generic_inference() {
    assert_typechecks(r#"
        fn first<T>(a: T, b: T) -> T { a }
        fn main() { let x = first(1, 2); }
    "#);
}

#[test]
fn test_generic_type_mismatch() {
    assert_type_fails(r#"
        fn first<T>(a: T, b: T) -> T { a }
        fn main() { let x = first(1, true); }
    "#);
}

// ============================================================
// CLOSURE TYPE TESTS
// ============================================================

#[test]
fn test_closure_inference() {
    assert_typechecks("fn main() { let f = |x| x + 1; f(42); }");
}

#[test]
fn test_closure_with_annotation() {
    assert_typechecks("fn main() { let f = |x: i32| -> i32 { x + 1 }; f(42); }");
}

#[test]
fn test_closure_captures() {
    assert_typechecks("fn main() { let y = 10; let f = |x| x + y; f(42); }");
}

#[test]
fn test_closure_as_parameter() {
    assert_typechecks(r#"
        fn apply(f: fn(i32) -> i32, x: i32) -> i32 { f(x) }
        fn main() { apply(|x| x * 2, 21); }
    "#);
}

// ============================================================
// ARRAY AND SLICE TYPE TESTS
// ============================================================

#[test]
fn test_array_literal() {
    assert_typechecks("fn main() { let arr = [1, 2, 3]; }");
}

#[test]
fn test_array_type_annotation() {
    assert_typechecks("fn main() { let arr: [i32; 3] = [1, 2, 3]; }");
}

#[test]
fn test_array_heterogeneous_error() {
    assert_type_fails("fn main() { let arr = [1, true, 3]; }");
}

#[test]
fn test_array_indexing() {
    assert_typechecks("fn main() { let arr = [1, 2, 3]; let x = arr[0]; }");
}

#[test]
fn test_slice_parameter() {
    assert_typechecks("fn sum(s: &[i32]) -> i32 { 0 } fn main() { let arr = [1, 2, 3]; sum(&arr); }");
}

// ============================================================
// TUPLE TYPE TESTS
// ============================================================

#[test]
fn test_tuple_literal() {
    assert_typechecks("fn main() { let t = (1, true, 3.14); }");
}

#[test]
fn test_tuple_destructuring() {
    assert_typechecks("fn main() { let (a, b) = (1, 2); }");
}

#[test]
fn test_tuple_index() {
    assert_typechecks("fn main() { let t = (1, true); let x = t.0; let y = t.1; }");
}

#[test]
fn test_unit_type() {
    assert_typechecks("fn foo() {} fn main() { let x: () = foo(); }");
}

// ============================================================
// METHOD CALL TYPE TESTS
// ============================================================

#[test]
fn test_impl_method() {
    assert_typechecks(r#"
        struct Point { x: i32, y: i32 }
        impl Point {
            fn new(x: i32, y: i32) -> Point { Point { x, y } }
        }
        fn main() { let p = Point::new(1, 2); }
    "#);
}

#[test]
fn test_self_method() {
    assert_typechecks(r#"
        struct Point { x: i32, y: i32 }
        impl Point {
            fn get_x(&self) -> i32 { self.x }
        }
        fn main() { let p = Point { x: 1, y: 2 }; let x = p.get_x(); }
    "#);
}

#[test]
fn test_mut_self_method() {
    assert_typechecks(r#"
        struct Counter { value: i32 }
        impl Counter {
            fn increment(&mut self) { self.value = self.value + 1; }
        }
        fn main() { let mut c = Counter { value: 0 }; c.increment(); }
    "#);
}

// ============================================================
// TRAIT TYPE TESTS
// ============================================================

#[test]
fn test_trait_implementation() {
    assert_typechecks(r#"
        trait Printable { fn print(&self); }
        struct Point { x: i32, y: i32 }
        impl Printable for Point { fn print(&self) {} }
    "#);
}

#[test]
fn test_trait_bound() {
    // Test basic trait bound syntax
    // Note: Full method dispatch with Self type has known limitations
    assert_typechecks(r#"
        trait Display { fn show(&self) -> i32; }
        fn process<T: Display>(x: T) -> i32 { 42 }
    "#);
}

// ============================================================
// EFFECT SYSTEM TYPE TESTS
// ============================================================

#[test]
fn test_effect_declaration() {
    assert_typechecks(r#"
        effect Console {
            op print(msg: i32) -> ();
        }
    "#);
}

#[test]
fn test_function_with_effect() {
    assert_typechecks(r#"
        effect Console {
            op print(msg: i32) -> ();
        }
        fn greet() / {Console} {
            perform Console.print(42)
        }
    "#);
}

#[test]
fn test_pure_function() {
    assert_typechecks(r#"
        fn add(a: i32, b: i32) -> i32 / pure { a + b }
    "#);
}

#[test]
fn test_handler_basic() {
    assert_typechecks(r#"
        effect State {
            op get() -> i32;
            op put(x: i32) -> ();
        }

        deep handler Counter for State {
            return(x) { x }
            op get() { resume(0) }
            op put(x) { resume(()) }
        }
    "#);
}

// ============================================================
// CONST AND STATIC TYPE TESTS
// ============================================================

#[test]
fn test_const_declaration() {
    assert_typechecks("const PI: i32 = 3;");
}

#[test]
fn test_static_declaration() {
    assert_typechecks("static COUNTER: i32 = 0;");
}

#[test]
fn test_const_in_expression() {
    assert_typechecks("const SIZE: i32 = 10; fn main() { let x = SIZE + 1; }");
}

// ============================================================
// TYPE ALIAS TESTS
// ============================================================

#[test]
fn test_type_alias() {
    assert_typechecks("type Int = i32; fn main() { let x: Int = 42; }");
}

#[test]
fn test_generic_type_alias() {
    assert_typechecks(r#"
        struct MyBox<T> { value: T }
        type IntBox = MyBox<i32>;
        fn main() { let b: IntBox = MyBox { value: 42 }; }
    "#);
}

// ============================================================
// NEVER TYPE TESTS
// ============================================================

#[test]
fn test_never_type_panic() {
    assert_typechecks(r#"
        fn diverge() -> ! { loop {} }
        fn main() { let x: i32 = if true { 42 } else { diverge() }; }
    "#);
}

#[test]
fn test_never_in_match() {
    assert_typechecks(r#"
        fn diverge() -> ! { loop {} }
        fn main() {
            let x: i32 = match true {
                true => 42,
                false => diverge(),
            };
        }
    "#);
}

// ============================================================
// COERCION TESTS
// ============================================================

#[test]
fn test_deref_coercion() {
    assert_typechecks(r#"
        fn takes_ref(x: &i32) {}
        fn main() {
            let x = 42;
            let r = &x;
            takes_ref(r);
        }
    "#);
}

// ============================================================
// AS CAST TESTS
// ============================================================

#[test]
fn test_numeric_cast() {
    assert_typechecks("fn main() { let x = 42i32 as i64; }");
}

#[test]
fn test_bool_to_int_cast() {
    assert_typechecks("fn main() { let x = true as i32; }");
}

// ============================================================
// DIVERGING EXPRESSION TESTS
// ============================================================

#[test]
fn test_match_with_return_diverge() {
    // This tests that match arms with diverging expressions (return, panic, etc.)
    // have type ! (never) which unifies with any type
    assert_typechecks(r#"
        fn example(opt: Option<i32>) -> i32 {
            let val = match opt {
                Some(x) => x,
                None => {
                    return 0;
                }
            };
            val
        }
    "#);
}

#[test]
fn test_match_with_return_no_block() {
    // Return without block wrapper
    assert_typechecks(r#"
        fn example(opt: Option<i32>) -> i32 {
            match opt {
                Some(x) => x,
                None => return 0,
            }
        }
    "#);
}

#[test]
fn test_if_with_return_diverge() {
    // If-else where one branch returns early
    assert_typechecks(r#"
        fn example(x: i32) -> i32 {
            let val = if x > 0 {
                x
            } else {
                return -1;
            };
            val * 2
        }
    "#);
}

#[test]
fn test_block_with_return_statement() {
    // Block that diverges via return statement
    assert_typechecks(r#"
        fn example() -> i32 {
            let x: i32 = {
                return 42;
            };
            x
        }
    "#);
}

#[test]
fn test_match_all_arms_diverge() {
    // All arms diverge - match has type !
    assert_typechecks(r#"
        fn example(x: i32) -> i32 {
            match x {
                0 => return 0,
                _ => return 1,
            }
        }
    "#);
}

#[test]
fn test_loop_break_with_value_diverges() {
    // Loop with break-with-value in match arm
    assert_typechecks(r#"
        fn example(opt: Option<i32>) -> i32 {
            loop {
                match opt {
                    Some(x) => break x,
                    None => {
                        break 0;
                    }
                }
            }
        }
    "#);
}

#[test]
fn test_nested_block_diverge() {
    // Nested block with diverging inner block
    assert_typechecks(r#"
        fn example() -> i32 {
            let x: i32 = {
                let y = 10;
                {
                    return y;
                }
            };
            x
        }
    "#);
}
