//! Parser tests using insta snapshot testing.
//!
//! These tests verify the parser produces correct ASTs for various inputs.
//! Snapshots are stored in `src/parser/snapshots/`.

use super::*;
use crate::ast::*;

/// Helper to parse an expression and return it (for snapshot tests).
#[allow(dead_code)] // Test infrastructure — available for expression-level snapshot tests
fn parse_expr(source: &str) -> Result<Expr, String> {
    let mut parser = Parser::new(source);
    // For expression parsing, we need to wrap in a function body
    // since parse_program expects declarations
    let expr = parser.parse_expr();
    if parser.errors.is_empty() {
        Ok(expr)
    } else {
        Err(format!("{:?}", parser.errors))
    }
}

/// Helper to parse a complete program.
fn parse_program(source: &str) -> Result<Program, String> {
    let mut parser = Parser::new(source);
    match parser.parse_program() {
        Ok(program) => Ok(program),
        Err(errors) => Err(format!("{:?}", errors)),
    }
}

/// Helper to parse and return debug representation.
fn parse_to_debug(source: &str) -> String {
    match parse_program(source) {
        Ok(program) => format!("{:#?}", program),
        Err(e) => format!("PARSE ERROR: {}", e),
    }
}

/// Helper to check if source parses without errors.
fn parse_ok(source: &str) -> bool {
    parse_program(source).is_ok()
}

// ============================================================
// Function Declaration Tests
// ============================================================

#[test]
fn test_simple_function() {
    let source = "fn foo() {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_function_with_params() {
    let source = "fn add(a: i32, b: i32) -> i32 { a + b }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_function_with_effects() {
    let source = "fn greet(name: String) / {IO} { println(name) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_pure_function() {
    let source = "fn pure_add(x: i32, y: i32) -> i32 / pure { x + y }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_async_function() {
    let source = "async fn fetch_data(url: String) -> Data / {IO, Async} { data }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_const_function() {
    let source = "const fn max(a: i32, b: i32) -> i32 { if a > b { a } else { b } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_generic_function() {
    let source = "fn identity<T>(x: T) -> T { x }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_function_with_where_clause() {
    let source = "fn process<T>(x: T) -> T where T: Clone + Debug { x.clone() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Struct Tests
// ============================================================

#[test]
fn test_unit_struct() {
    let source = "struct Empty;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_tuple_struct() {
    let source = "struct Point(i32, i32);";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_record_struct() {
    let source = r#"
struct Person {
    name: String,
    age: u32,
    email: Option<String>,
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_generic_struct() {
    let source = "struct Box<T> { value: T }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_pub_struct() {
    let source = r#"
pub struct PublicPerson {
    pub name: String,
    age: u32,
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Enum Tests
// ============================================================

#[test]
fn test_simple_enum() {
    let source = r#"
enum Color {
    Red,
    Green,
    Blue,
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_enum_with_data() {
    let source = r#"
enum Option<T> {
    Some(T),
    None,
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_enum_with_records() {
    let source = r#"
enum Message {
    Quit,
    Move { x: i32, y: i32 },
    Write(String),
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Effect Tests
// ============================================================

#[test]
fn test_simple_effect() {
    let source = r#"
effect Console {
    op print(msg: String) -> unit;
    op read_line() -> String;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_generic_effect() {
    let source = r#"
effect State<S> {
    op get() -> S;
    op put(s: S) -> unit;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_effect_with_extends() {
    let source = r#"
effect Reader<R> extends State<R> {
    op ask() -> R;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Handler Tests
// ============================================================

#[test]
fn test_deep_handler() {
    let source = r#"
deep handler CounterHandler for State<i32> {
    let mut count: i32 = 0

    return(x) { x }

    op get() { resume(count) }
    op put(s) { count = s; resume(()) }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_shallow_handler() {
    let source = r#"
shallow handler OnceReader<R> for State<R> {
    let value: R

    return(x) { x }

    op get() { resume(value) }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Expression Tests
// ============================================================

#[test]
fn test_arithmetic_expr() {
    let source = "fn f() { 1 + 2 * 3 - 4 / 2 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_comparison_expr() {
    let source = "fn f() { a == b && c != d || e < f }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_pipe_expr() {
    let source = "fn f() { data |> transform |> filter |> collect }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_method_chain() {
    let source = "fn f() { x.foo().bar(1, 2).baz() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_closure() {
    let source = "fn f() { |x, y| x + y }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_move_closure() {
    let source = "fn f() { move |x| x * 2 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_if_else() {
    let source = r#"
fn f(x: i32) -> i32 {
    if x > 0 {
        1
    } else if x < 0 {
        -1
    } else {
        0
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_match_expr() {
    let source = r#"
fn f(opt: Option<i32>) -> i32 {
    match opt {
        Option::Some(x) => x,
        Option::None => 0,
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_for_loop() {
    let source = "fn f() { for i in 0..10 { println(i) } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_while_loop() {
    let source = "fn f() { while x > 0 { x = x - 1 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_loop_with_break() {
    let source = "fn f() { loop { if done { break 42 } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_array_literal() {
    let source = "fn f() { [1, 2, 3, 4, 5] }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_array_repeat() {
    let source = "fn f() { [0; 100] }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_tuple_expr() {
    let source = "fn f() { (1, \"hello\", true) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_struct_construction() {
    let source = "fn f() { Point { x: 10, y: 20 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_field_init_shorthand() {
    let source = "fn f(x: i32, y: i32) { Point { x, y } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_anonymous_record_literal() {
    let source = "fn f() { { x: 10, y: 20 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_range_expr() {
    let source = "fn f() { 0..10 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_inclusive_range() {
    let source = "fn f() { 0..=10 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_perform_effect() {
    let source = "fn f() / {State<i32>} { let x = perform State.get(); x }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_with_handle() {
    let source = r#"
fn f() {
    with handler handle {
        perform State.get()
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Pattern Tests
// ============================================================

#[test]
fn test_wildcard_pattern() {
    let source = "fn f() { match x { _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Escape Sequence Tests
// ============================================================

#[test]
fn test_hex_escape_in_string() {
    let source = r#"fn f() { let s = "\x00\x7F\xFF"; }"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_hex_escape_in_char() {
    let source = r"fn f() { let c = '\x41'; }"; // ASCII 'A'
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_unicode_escape_in_string() {
    let source = r#"fn f() { let s = "\u{1F600}"; }"#; // 😀
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_unicode_escape_in_char() {
    let source = r"fn f() { let c = '\u{03B1}'; }"; // Greek alpha α
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_literal_pattern() {
    let source = "fn f() { match x { 42 => true, _ => false } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_tuple_pattern() {
    let source = "fn f() { match pair { (a, b) => a + b } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_struct_pattern() {
    let source = "fn f() { match point { Point { x, y } => x + y } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_or_pattern() {
    let source = "fn f() { match x { 1 | 2 | 3 => true, _ => false } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_range_pattern() {
    let source = "fn f() { match x { 0..10 => true, _ => false } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_guard_pattern() {
    let source = "fn f() { match x { n if n > 0 => true, _ => false } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_binding_pattern() {
    let source = "fn f() { match opt { Some(x @ 1..10) => x, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_slice_pattern_with_rest() {
    // Test slice pattern with binding rest pattern (rest @ ..)
    let source = "fn f() { match arr { [first, rest @ .., last] => first + last, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_slice_pattern_ref_rest() {
    // Test slice pattern with ref binding rest pattern (ref rest @ ..)
    let source = "fn f() { match arr { [first, ref rest @ ..] => rest.len(), _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Type Tests
// ============================================================

#[test]
fn test_reference_type() {
    let source = "fn f(x: &i32, y: &mut i32) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_lifetime_type() {
    let source = "fn f<'a>(x: &'a str) -> &'a str { x }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_array_type() {
    let source = "fn f(arr: [i32; 10]) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_slice_type() {
    let source = "fn f(slice: &[i32]) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_function_type() {
    let source = "fn f(callback: fn(i32) -> bool) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_tuple_type() {
    let source = "fn f() -> (i32, String, bool) { (1, s, true) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_linear_type() {
    let source = "fn f(resource: linear Handle) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_affine_type() {
    let source = "fn f(resource: affine Handle) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Import Tests
// ============================================================

#[test]
fn test_simple_import() {
    let source = "use std.io;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_import_item() {
    let source = "use std.io::println;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_group_import() {
    let source = "use std.collections::{Vec, HashMap, HashSet};";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_glob_import() {
    let source = "use std.ops::*;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_aliased_import() {
    let source = "use std.collections::HashMap as Map;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_import_with_colons_separator() {
    // Both `.` and `::` can be used as module path separators
    let source = "use std::collections::{HashMap, HashSet};";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_import_with_mixed_separators() {
    // Mix of `.` and `::` in module path
    let source = "use std.collections::map::HashMap;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_import_single_item_rust_style() {
    // Rust-style single item import with all `::` separators
    let source = "use std::io::println;";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Module Tests
// ============================================================

#[test]
fn test_module_declaration() {
    let source = "module myproject.utils.helpers;";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Trait and Impl Tests
// ============================================================

#[test]
fn test_trait_declaration() {
    let source = r#"
trait Printable {
    fn print(&self);
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_trait_with_default() {
    let source = r#"
trait Debug {
    fn debug(&self) -> String {
        "Debug".to_string()
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_impl_block() {
    let source = r#"
impl Point {
    fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_trait_impl() {
    let source = r#"
impl Printable for Point {
    fn print(&self) {
        println(self.x);
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Attribute Tests
// ============================================================

#[test]
fn test_simple_attribute() {
    let source = r#"
#[inline]
fn fast() {}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_attribute_with_args() {
    let source = r#"
#[cfg(target_os = "linux")]
fn linux_only() {}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Const and Static Tests
// ============================================================

#[test]
fn test_const_decl() {
    let source = "const PI: f64 = 3.14159;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_static_decl() {
    let source = "static mut COUNTER: i32 = 0;";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Complex Program Tests
// ============================================================

#[test]
fn test_hello_world() {
    let source = r#"
module examples.hello;

use std.io::println;

fn main() / {IO} {
    println("Hello, World!")
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_effect_example() {
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

fn counter() -> i32 / {State<i32>} {
    let current = perform State.get();
    perform State.put(current + 1);
    current
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_fizzbuzz() {
    let source = r#"
fn fizzbuzz(n: i32) -> String / pure {
    if n % 15 == 0 {
        "FizzBuzz"
    } else if n % 3 == 0 {
        "Fizz"
    } else if n % 5 == 0 {
        "Buzz"
    } else {
        n.to_string()
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Const Generics and Lifetime Parameter Tests
// ============================================================

#[test]
fn test_const_generic_array() {
    let source = "struct Array<T, const N: usize> { data: T }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_const_generic_function() {
    let source = "fn create_array<T, const N: usize>() -> Array<T, N> { Array { data: default() } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_lifetime_parameter() {
    let source = "fn longest<'a>(x: &'a str, y: &'a str) -> &'a str { if x.len() > y.len() { x } else { y } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_lifetime_bounds() {
    let source = "struct Ref<'a, 'b: 'a, T> { r: &'a &'b T }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_mixed_generic_params() {
    let source = "fn process<'a, T: Clone, const N: usize>(data: &'a [T; N]) -> T { data[0].clone() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// >> Disambiguation Tests (nested generics)
// ============================================================

#[test]
fn test_nested_generic_type_args() {
    // Tests that `>>` is correctly split into two `>` in type argument context
    let source = "fn process(data: Vec<Vec<i32>>) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_deeply_nested_generics() {
    // Tests deeply nested generics like `Option<Result<Vec<T>, E>>`
    let source = "fn foo() -> Option<Result<Vec<String>, Error>> { None }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_generics_in_let() {
    // Tests `>>` disambiguation in let bindings
    let source = "fn foo() { let x: HashMap<String, Vec<i32>> = HashMap::new(); }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_generics_triple() {
    // Tests `>>>` (three nested generics)
    let source = "fn foo() -> Vec<Vec<Vec<i32>>> { [] }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Extern Block Tests
// ============================================================

#[test]
fn test_extern_block_c() {
    let source = r#"
extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_extern_block_without_abi() {
    // Should default to "C"
    let source = r#"
extern {
    fn puts(s: *const i8) -> i32;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_extern_block_with_multiple_functions() {
    let source = r#"
extern "C" {
    fn sqlite3_open(filename: *const i8, ppDb: *mut *mut c_void) -> i32;
    fn sqlite3_close(db: *mut c_void) -> i32;
    fn sqlite3_exec(
        db: *mut c_void,
        sql: *const i8,
        callback: *const c_void,
        arg: *mut c_void,
        errmsg: *mut *mut i8
    ) -> i32;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Error Recovery Tests
// ============================================================

#[test]
fn test_missing_semicolon() {
    let source = "fn f() { let x = 1 let y = 2; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_unclosed_brace() {
    let source = "fn f() { if true { 1 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_unexpected_token() {
    let source = "fn f() { + }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_macro_call_error() {
    // Should produce a clear error message about macros not being supported
    let source = "fn f() { println!(\"hello\"); }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_macro_call_vec() {
    // vec![] style macros should also produce clear error
    let source = "fn f() { let x = vec![1, 2, 3]; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_unsafe_block_error() {
    // Rust-style `unsafe { }` should produce clear error suggesting @unsafe
    let source = "fn f() { let x = unsafe { foo() }; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_at_unsafe_block() {
    // Blood-style @unsafe { } should work correctly
    let source = "fn f() { let x = @unsafe { foo() }; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Reference Pattern Tests
// ============================================================

#[test]
fn test_ref_pattern_in_match() {
    let source = "fn f() { match x { &Some(v) => v, &None => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_ref_pattern_struct_field() {
    let source = "fn f() { match p { Point { ref x, ref y } => *x + *y } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_ref_mut_pattern() {
    let source = "fn f() { match p { Point { ref mut x, y } => { *x = y } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_ref_pattern() {
    let source = "fn f() { match x { &Option::Some(&Point { x, y }) => x + y, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Match Arm Block Body Tests (regression for issue #4)
// ============================================================

#[test]
fn test_block_body_then_ref_pattern() {
    // Block body followed by &pattern - regression test
    let source = r#"
fn f(k: &Kind) -> i32 {
    match k {
        &Kind::First { val } => {
            val
        }
        &Kind::Second { inner } => inner,
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_block_body_then_ref_wildcard() {
    let source = r#"
fn f(k: &Kind) -> i32 {
    match k {
        &Kind::First { val } => {
            val
        }
        &_ => 0,
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_multiple_block_bodies_with_ref() {
    let source = r#"
fn f(k: &Kind) -> i32 {
    match k {
        &Kind::A => { 1 }
        &Kind::B => { 2 }
        &Kind::C => { 3 }
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// If Let / While Let Tests
// ============================================================

#[test]
fn test_if_let_simple() {
    let source = "fn f() { if let Some(x) = opt { x } else { 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_if_let_no_else() {
    let source = "fn f() { if let Some(x) = opt { println(x) } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_if_let_ref_pattern() {
    let source = "fn f() { if let &Some(ref x) = opt { *x } else { 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_while_let() {
    let source = "fn f() { while let Some(x) = iter.next() { sum = sum + x } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_if_let_else_if_let() {
    let source = r#"
fn f() {
    if let Some(x) = a {
        x
    } else if let Some(y) = b {
        y
    } else {
        0
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Expression Combination Tests
// ============================================================

#[test]
fn test_closure_in_match_arm() {
    let source = "fn f() { match x { 0 => |y| y * 2, _ => |y| y } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_match_in_closure() {
    let source = "fn f() { let f = |x| match x { 0 => 1, n => n }; f(5) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_loops() {
    let source = r#"
fn f() {
    for i in 0..10 {
        for j in 0..10 {
            while k < 5 { k = k + 1 }
        }
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_block_as_binary_operand() {
    let source = "fn f() { { 1 + 2 } + { 3 + 4 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Operator Precedence Tests
// ============================================================

#[test]
fn test_mixed_logical_operators() {
    let source = "fn f() { a && b || c && !d || e }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_mixed_bitwise_operators() {
    let source = "fn f() { x & 0xFF | y ^ 0x0F }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_comparison_chain() {
    let source = "fn f() { a < b && b < c && c < d }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Complex Type Tests
// ============================================================

#[test]
fn test_deeply_nested_option_result() {
    let source = "fn f() -> Option<Result<Option<i32>, String>> { None }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_function_type_complex() {
    let source = "fn f(callback: fn(&str, i32) -> Option<bool>) -> bool { true }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_ref_to_ref_type() {
    let source = "fn f<'a, 'b>(x: &'a &'b str) -> &'a str { *x }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_mutable_slice_type() {
    let source = "fn f(slice: &mut [i32]) { slice[0] = 1 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Pattern Edge Cases
// ============================================================

#[test]
fn test_or_pattern_with_guard() {
    let source = "fn f() { match x { 1 | 2 | 3 if x % 2 == 0 => true, _ => false } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_struct_pattern() {
    let source = "fn f() { match c { Container::Nested { inner: Point { x, y } } => x + y, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_tuple_struct_pattern_ref() {
    let source = "fn f() { match w { &Wrapper(ref x) => *x, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Type Alias Tests
// ============================================================

#[test]
fn test_type_alias_simple() {
    let source = "type MyInt = i32;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_type_alias_generic() {
    let source = "type StringMap<V> = HashMap<String, V>;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_type_alias_complex() {
    let source = "type Handler<T, E> = fn(T) -> Result<T, E>;";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Visibility Modifier Tests
// ============================================================

#[test]
fn test_pub_crate_visibility() {
    let source = "pub(crate) fn internal() {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_pub_super_visibility() {
    let source = "pub(super) struct ParentVisible { x: i32 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_pub_self_visibility() {
    let source = "pub(self) const PRIVATE: i32 = 0;";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Pointer Type Tests
// ============================================================

#[test]
fn test_const_pointer_type() {
    let source = "fn f(p: *const i32) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_mut_pointer_type() {
    let source = "fn f(p: *mut u8) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_pointer_type() {
    let source = "fn f(p: *mut *const i32) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Effect Expression Tests
// ============================================================

#[test]
fn test_perform_simple() {
    let source = "fn f() / {Console} { perform print(\"hello\") }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_perform_qualified() {
    let source = "fn f() / {State<i32>} { perform State.get() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_try_with_block() {
    let source = r#"
fn f() {
    try {
        perform read_line()
    } with {
        Console.print(s) => { resume(()) },
        Console.read_line() => { resume("input") },
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_region_block() {
    let source = "fn f() { region { let x = alloc(10); x } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_region_labeled() {
    let source = "fn f() { region 'heap { alloc(100) } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_resume_expr() {
    let source = r#"
deep handler H for State<i32> {
    return(x) { x }
    op get() { resume(42) }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Call Expression Tests
// ============================================================

#[test]
fn test_named_arguments() {
    let source = "fn f() { create(width: 10, height: 20) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_mixed_positional_named_args() {
    let source = "fn f() { draw(canvas, x: 10, y: 20, color: red) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_turbofish_method() {
    let source = "fn f() { vec.iter().collect::<Vec<_>>() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_turbofish_function() {
    let source = "fn f() { parse::<i32>(\"42\") }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Ownership Qualifier Tests
// ============================================================

#[test]
fn test_linear_type_param() {
    let source = "fn consume(linear handle: Handle) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_affine_type_param() {
    let source = "fn maybe_use(affine resource: Resource) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Pattern Rest/Spread Tests
// ============================================================

#[test]
fn test_tuple_pattern_rest_start() {
    let source = "fn f() { let (.., last) = tuple; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_tuple_pattern_rest_middle() {
    let source = "fn f() { let (first, .., last) = tuple; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_struct_pattern_rest() {
    let source = "fn f() { let Point { x, .. } = p; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_slice_pattern_rest_middle() {
    let source = "fn f() { match arr { [first, .., last] => first + last, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_slice_pattern_rest_named() {
    let source = "fn f() { match arr { [first, middle @ .., last] => first, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Negative Literal Pattern Tests
// ============================================================

#[test]
fn test_negative_int_pattern() {
    let source = "fn f() { match x { -1 => true, 0 => false, 1 => true, _ => false } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_negative_float_pattern() {
    let source = "fn f() { match x { -1.0 => neg, 0.0 => zero, 1.0 => pos, _ => other } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Parenthesized Pattern Tests
// ============================================================

#[test]
fn test_parenthesized_pattern() {
    let source = "fn f() { match x { (Some(v)) => v, (None) => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_parenthesized_or_pattern() {
    let source = "fn f() { match x { (1 | 2 | 3) => true, _ => false } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Attribute Tests
// ============================================================

#[test]
fn test_attribute_multiple() {
    let source = r#"
#[inline]
#[must_use]
fn important() -> i32 { 42 }
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_attribute_nested() {
    let source = r#"
#[cfg(all(target_os = "linux", feature = "extra"))]
fn linux_extra() {}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_inner_attribute() {
    let source = r#"
fn f() {
    #![allow(unused)]
    let x = 1;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Loop Label Tests
// ============================================================

#[test]
fn test_labeled_loop() {
    let source = "fn f() { 'outer: loop { break 'outer } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_labeled_while() {
    let source = "fn f() { 'outer: while true { continue 'outer } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_labeled_for() {
    let source = "fn f() { 'outer: for i in 0..10 { if i == 5 { break 'outer } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_break_with_value() {
    let source = "fn f() -> i32 { loop { break 42 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_labeled_break_with_value() {
    let source = "fn f() -> i32 { 'result: loop { break 'result 42 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Try Operator Tests
// ============================================================

#[test]
fn test_try_operator() {
    let source = "fn f() -> Result<i32, E> { let x = fallible()?; Ok(x) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_try_operator_chain() {
    let source = "fn f() -> Result<i32, E> { a()?.b()?.c() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Range Expression Tests
// ============================================================

#[test]
fn test_range_from() {
    let source = "fn f() { for i in 0.. { if i > 10 { break } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_range_to() {
    let source = "fn f() { let x = arr[..5]; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_range_to_inclusive() {
    let source = "fn f() { let x = arr[..=5]; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_range_full() {
    let source = "fn f() { let x = arr[..]; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Struct Update Syntax Tests
// ============================================================

#[test]
fn test_struct_update() {
    let source = "fn f() { Point { x: 10, ..default } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_struct_update_single_field() {
    let source = "fn f() { Config { debug: true, ..Config::default() } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Tuple Index Tests
// ============================================================

#[test]
fn test_tuple_index() {
    let source = "fn f() { let x = pair.0; let y = pair.1; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_tuple_index() {
    let source = "fn f() { nested.0.1.2 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Module Inline Tests
// ============================================================

#[test]
fn test_inline_module() {
    let source = r#"
mod inner {
    fn private() {}
    pub fn public() {}
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_nested_modules() {
    let source = r#"
mod outer {
    mod inner {
        fn deep() {}
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Self Parameter Variations
// ============================================================

#[test]
fn test_self_by_value() {
    let source = "impl Point { fn consume(self) {} }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_self_mut_by_value() {
    let source = "impl Point { fn modify(mut self) -> Self { self.x += 1; self } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_self_ref() {
    let source = "impl Point { fn get_x(&self) -> i32 { self.x } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_self_mut_ref() {
    let source = "impl Point { fn set_x(&mut self, x: i32) { self.x = x } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Generic Impl Tests
// ============================================================

#[test]
fn test_generic_impl() {
    let source = "impl<T> Container<T> { fn new(val: T) -> Self { Container { val } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_generic_trait_impl() {
    let source = "impl<T> Default for Container<T> where T: Default { fn default() -> Self { Container { val: T::default() } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Associated Type Tests
// ============================================================

#[test]
fn test_trait_associated_type() {
    let source = r#"
trait Iterator {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Effect Row Tests
// ============================================================

#[test]
fn test_pure_effect() {
    let source = "fn pure_fn(x: i32) -> i32 / pure { x * 2 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_empty_effect_row() {
    let source = "fn no_effects() / {} { 42 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_multiple_effects() {
    let source = "fn effectful() / {IO, State<i32>, Console} { perform print(\"hi\") }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Extern Block Tests (Extended)
// ============================================================

#[test]
fn test_extern_variadic() {
    let source = r#"
extern "C" {
    fn printf(fmt: *const i8, ...) -> i32;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// String Literal Variations
// ============================================================

#[test]
fn test_raw_string() {
    let source = r#"fn f() { let s = r"raw \n string"; }"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_raw_string_hashes() {
    let source = r##"fn f() { let s = r#"contains "quotes""#; }"##;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_byte_string() {
    let source = r#"fn f() { let b = b"bytes"; }"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Numeric Literal Variations
// ============================================================

#[test]
fn test_hex_literal() {
    let source = "fn f() { let x = 0xDEADBEEF; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_octal_literal() {
    let source = "fn f() { let x = 0o755; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_binary_literal() {
    let source = "fn f() { let x = 0b1010_1010; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_float_exponent() {
    let source = "fn f() { let x = 1.5e10; let y = 2.0E-5; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_numeric_suffix() {
    let source = "fn f() { let a = 42u32; let b = 3.14f64; let c = 100i8; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Complex Generic Bounds
// ============================================================

#[test]
fn test_multiple_trait_bounds() {
    let source = "fn f<T: Clone + Debug + Default>(x: T) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_lifetime_bounds_extended() {
    let source = "fn f<'a, 'b: 'a>(x: &'a str, y: &'b str) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_where_clause_complex() {
    let source = r#"
fn complex<'a, T, U>(x: &'a T, y: U) -> U
where
    T: Clone + 'a,
    U: From<T> + Default,
{
    U::default()
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// Const Generic Tests
// ============================================================

#[test]
fn test_const_generic_in_type() {
    let source = "fn f(arr: [i32; 10]) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_const_generic_param() {
    let source = "struct Buffer<const N: usize> { data: [u8; N] }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_const_generic_expr() {
    let source = "fn f<const N: usize>() -> [i32; N] { [0; N] }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// COMBINATORIAL TESTS
// Tests that combine multiple features
// ============================================================

#[test]
fn test_combo_generic_effect_where() {
    // Generic function with effects and where clause
    let source = r#"
fn process<'a, T, U>(input: &'a T) -> Result<U, Error> / {IO, State<U>}
where
    T: Clone + Debug,
    U: Default + From<T>,
{
    U::default()
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_match_with_all_pattern_types() {
    // Match with various pattern combinations
    let source = r#"
fn complex_match(x: Value) -> i32 {
    match x {
        Value::Int(0) => 0,
        Value::Int(n) if n < 0 => -1,
        Value::Int(n @ 1..=10) => n,
        Value::Pair(a, b) => a + b,
        Value::Record { x, y: ref yval, .. } => x + *yval,
        Value::Nested(&inner) => inner,
        a | b | c => 999,
        _ => -999,
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_closure_with_effects_and_types() {
    // Complex closure with type annotations and effects
    let source = "fn f() { let c = move |x: i32, y: &str| -> Result<i32, E> / {IO} { Ok(x) }; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_deeply_nested_generics_and_refs() {
    // Deeply nested types with references
    let source = "fn f(x: &Option<Result<Vec<&mut [i32]>, String>>) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_method_chain_with_closures() {
    // Method chaining with closures and turbofish
    let source = "fn f() { items.iter().filter(|x| x > 0).map::<Vec<_>, _>(|x| x * 2).collect() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_nested_control_flow() {
    // Deeply nested control flow
    let source = r#"
fn f() {
    'outer: for i in 0..10 {
        if i % 2 == 0 {
            match i {
                0 => continue 'outer,
                n if n > 5 => {
                    while n > 0 {
                        if n == 3 { break 'outer }
                    }
                }
                _ => loop {
                    break i
                },
            }
        }
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_impl_with_all_features() {
    // Impl block with generics, where clause, various method types
    let source = r#"
impl<'a, T: Clone> Container<'a, T>
where
    T: Debug + Default,
{
    const SIZE: usize = 10;

    fn new() -> Self { Container { data: T::default() } }
    fn get(&self) -> &T { &self.data }
    fn set(&mut self, val: T) { self.data = val }
    fn consume(self) -> T { self.data }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_handler_with_state_and_ops() {
    // Complex handler with multiple operations and state
    let source = r#"
deep handler ComplexHandler<T> for State<T>
where T: Default
{
    let mut state: T = T::default()
    let count: i32 = 0

    return(x) {
        if count > 0 { x } else { T::default() }
    }

    op get() {
        let current = state;
        resume(current)
    }

    op put(new_state) {
        state = new_state;
        resume(())
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_combo_extern_with_attributes() {
    // Extern block with attributes
    let source = r#"
#[link(name = "mylib", kind = "static")]
extern "C" {
    #[doc = "Allocates memory"]
    fn malloc(size: usize) -> *mut u8;

    fn free(ptr: *mut u8);
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// EDGE CASE / BOUNDARY TESTS
// ============================================================

#[test]
fn test_edge_empty_struct() {
    let source = "struct Empty {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_empty_enum() {
    let source = "enum Never {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_empty_trait() {
    let source = "trait Marker {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_empty_impl() {
    let source = "impl Empty {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_empty_block() {
    let source = "fn f() {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_empty_match() {
    let source = "fn f() { match x {} }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_single_element_tuple() {
    let source = "fn f() { let x = (42,); }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_single_element_array() {
    let source = "fn f() { let x = [42]; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_unit_return() {
    let source = "fn f() -> () { () }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_never_return() {
    let source = "fn f() -> ! { loop {} }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_trailing_comma_everywhere() {
    let source = r#"
fn f(
    a: i32,
    b: i32,
) -> (i32, i32,) {
    let arr = [1, 2, 3,];
    let tuple = (a, b,);
    match x {
        (a, b,) => (a, b,),
    }
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_operator_at_line_start() {
    // Binary operator at line start
    let source = r#"
fn f() {
    let x = 1
        + 2
        + 3;
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_deep_nesting_parens() {
    let source = "fn f() { (((((((x))))))) }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_deep_nesting_blocks() {
    let source = "fn f() { { { { { { { x } } } } } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_struct_vs_block_disambiguation() {
    // Ambiguity: is { x } a block or struct?
    let source = r#"
fn f() {
    if cond { x }        // block
    let s = Point { x }; // struct
    { x: 10 }            // anonymous record
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_less_than_vs_generic() {
    // < as comparison vs generic
    let source = "fn f() { if a < b { 1 } else { 2 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_pipe_vs_closure() {
    // | as or-pattern vs closure param delimiter
    let source = "fn f() { let c = |a| a | b; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_ref_pattern_vs_bitand() {
    // & as reference pattern vs bitwise and
    let source = "fn f() { match x { &y => y, _ => x & 0xFF } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_range_vs_struct_update() {
    // .. as range vs struct update
    let source = "fn f() { let r = 0..10; let s = Point { x: 0, ..p }; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_precedence_not_vs_method() {
    // !x.method() - is it (!x).method() or !(x.method())?
    let source = "fn f() { !x.is_empty() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_precedence_deref_vs_field() {
    // *x.field - is it (*x).field or *(x.field)?
    let source = "fn f() { *ptr.data }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_precedence_ref_vs_method() {
    // &x.field - is it (&x).field or &(x.field)?
    let source = "fn f() { &obj.field }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_as_chain() {
    // Multiple as casts
    let source = "fn f() { x as i32 as i64 as usize }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_try_chain_with_method() {
    // ? with method chains
    let source = "fn f() { a?.b?.c()?.d }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_index_chain() {
    // Multiple index operations
    let source = "fn f() { arr[0][1][2] }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_call_chain() {
    // Multiple function calls
    let source = "fn f() { a()()(x)() }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_generic_in_expr_position() {
    // Generics in expression that could be confused with comparisons
    let source = "fn f() { let x = Vec::<i32>::new(); }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_keywords_as_field_names() {
    // Using contextual keywords as field names
    let source = r#"
struct S {
    effect: i32,
    handler: i32,
    pure: bool,
}
"#;
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_edge_self_type_in_return() {
    let source = "impl Point { fn clone(&self) -> Self { Self { x: self.x, y: self.y } } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// EXPANDED ERROR RECOVERY TESTS
// ============================================================

#[test]
fn test_error_missing_type_annotation() {
    let source = "fn f(x) {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_missing_return_type() {
    let source = "fn f() -> { 42 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_unclosed_paren() {
    let source = "fn f() { let x = (1 + 2; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_unclosed_bracket() {
    let source = "fn f() { let x = [1, 2, 3; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_unclosed_generic() {
    let source = "fn f() { let x: Vec<i32 = vec; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_double_comma() {
    let source = "fn f() { let x = (1,, 2); }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_missing_match_arm_body() {
    let source = "fn f() { match x { 1 =>, _ => 0 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_invalid_operator() {
    let source = "fn f() { x +* y }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_let_without_pattern() {
    let source = "fn f() { let = 1; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_fn_without_name() {
    let source = "fn () {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_struct_without_name() {
    let source = "struct { x: i32 }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_enum_without_name() {
    let source = "enum { A, B }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_impl_without_type() {
    let source = "impl { fn f() {} }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_reserved_keyword_as_ident() {
    let source = "fn f() { let fn = 1; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_invalid_pattern_in_let() {
    let source = "fn f() { let 1 + 2 = 3; }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_missing_else_branch() {
    // if without else when used as expression
    let source = "fn f() -> i32 { if true { 1 } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_stray_closing_brace() {
    let source = "fn f() { let x = 1; } }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_multiple_visibility() {
    let source = "pub pub fn f() {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_empty_generic_params() {
    let source = "fn f<>() {}";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_trailing_operator() {
    let source = "fn f() { x + }";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_error_leading_operator() {
    let source = "fn f() { * x }";
    insta::assert_snapshot!(parse_to_debug(source));
}

// ============================================================
// PROPERTY-BASED / FUZZ TESTS
// ============================================================

mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    // Strategy for generating valid identifiers
    fn ident_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,10}".prop_filter("not keyword", |s| {
            !matches!(
                s.as_str(),
                "fn" | "let" | "if" | "else" | "match" | "for" | "while" | "loop"
                    | "return" | "break" | "continue" | "struct" | "enum" | "trait"
                    | "impl" | "pub" | "mod" | "use" | "const" | "static" | "mut"
                    | "ref" | "self" | "Self" | "true" | "false" | "as" | "in"
                    | "where" | "type" | "effect" | "handler" | "perform" | "resume"
                    | "try" | "with" | "pure" | "async" | "move" | "dyn" | "unsafe"
            )
        })
    }

    // Strategy for generating integer literals
    fn int_literal_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            (0i64..1000).prop_map(|n| n.to_string()),
            (0u32..256).prop_map(|n| format!("0x{:X}", n)),
            (0u32..64).prop_map(|n| format!("0o{:o}", n)),
            (0u32..16).prop_map(|n| format!("0b{:b}", n)),
        ]
    }

    // Strategy for generating simple types
    fn simple_type_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("i32".to_string()),
            Just("i64".to_string()),
            Just("u32".to_string()),
            Just("u64".to_string()),
            Just("bool".to_string()),
            Just("char".to_string()),
            Just("f32".to_string()),
            Just("f64".to_string()),
            Just("String".to_string()),
            Just("()".to_string()),
        ]
    }

    // Strategy for generating simple expressions
    fn simple_expr_strategy() -> BoxedStrategy<String> {
        prop_oneof![
            int_literal_strategy(),
            Just("true".to_string()),
            Just("false".to_string()),
            ident_strategy(),
            Just("()".to_string()),
        ]
        .boxed()
    }

    // Strategy for generating binary expressions
    fn binary_expr_strategy() -> impl Strategy<Value = String> {
        let ops = prop_oneof![
            Just("+"),
            Just("-"),
            Just("*"),
            Just("/"),
            Just("%"),
            Just("=="),
            Just("!="),
            Just("<"),
            Just(">"),
            Just("<="),
            Just(">="),
            Just("&&"),
            Just("||"),
            Just("&"),
            Just("|"),
            Just("^"),
        ];

        (simple_expr_strategy(), ops, simple_expr_strategy())
            .prop_map(|(left, op, right)| format!("{} {} {}", left, op, right))
    }

    // Strategy for generating let statements
    fn let_stmt_strategy() -> impl Strategy<Value = String> {
        (ident_strategy(), simple_type_strategy(), simple_expr_strategy()).prop_map(
            |(name, ty, expr)| format!("let {}: {} = {};", name, ty, expr),
        )
    }

    // Strategy for generating function signatures
    fn fn_sig_strategy() -> impl Strategy<Value = String> {
        (
            ident_strategy(),
            prop::collection::vec((ident_strategy(), simple_type_strategy()), 0..3),
            prop::option::of(simple_type_strategy()),
        )
            .prop_map(|(name, params, ret)| {
                let params_str = params
                    .into_iter()
                    .map(|(n, t)| format!("{}: {}", n, t))
                    .collect::<Vec<_>>()
                    .join(", ");
                match ret {
                    Some(r) => format!("fn {}({}) -> {}", name, params_str, r),
                    None => format!("fn {}({})", name, params_str),
                }
            })
    }

    // Property: any valid function should parse without panic
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_fn_parses_without_panic(
            name in ident_strategy(),
            body_expr in simple_expr_strategy()
        ) {
            let source = format!("fn {}() {{ {} }}", name, body_expr);
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_let_stmt_parses_without_panic(
            stmt in let_stmt_strategy()
        ) {
            let source = format!("fn test() {{ {} }}", stmt);
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_binary_expr_parses_without_panic(
            expr in binary_expr_strategy()
        ) {
            let source = format!("fn test() {{ {} }}", expr);
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_struct_parses_without_panic(
            name in ident_strategy(),
            fields in prop::collection::vec(
                (ident_strategy(), simple_type_strategy()),
                0..5
            )
        ) {
            let fields_str = fields
                .into_iter()
                .map(|(n, t)| format!("    {}: {},", n, t))
                .collect::<Vec<_>>()
                .join("\n");
            let source = if fields_str.is_empty() {
                format!("struct {} {{}}", name)
            } else {
                format!("struct {} {{\n{}\n}}", name, fields_str)
            };
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_enum_parses_without_panic(
            name in ident_strategy(),
            variants in prop::collection::vec(ident_strategy(), 1..5)
        ) {
            let variants_str = variants.join(",\n    ");
            let source = format!("enum {} {{\n    {}\n}}", name, variants_str);
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_nested_parens_parse_without_panic(
            depth in 1usize..10,
            inner in simple_expr_strategy()
        ) {
            let open = "(".repeat(depth);
            let close = ")".repeat(depth);
            let source = format!("fn test() {{ {}{}{} }}", open, inner, close);
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_chained_binary_ops_parse_without_panic(
            exprs in prop::collection::vec(simple_expr_strategy(), 2..6)
        ) {
            let source = format!("fn test() {{ {} }}", exprs.join(" + "));
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_match_arms_parse_without_panic(
            arms in prop::collection::vec(
                (int_literal_strategy(), simple_expr_strategy()),
                1..5
            )
        ) {
            let arms_str = arms
                .into_iter()
                .map(|(pat, body)| format!("        {} => {},", pat, body))
                .collect::<Vec<_>>()
                .join("\n");
            let source = format!(
                "fn test() {{\n    match x {{\n{}\n        _ => 0,\n    }}\n}}",
                arms_str
            );
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_if_else_chain_parses_without_panic(
            conditions in prop::collection::vec(simple_expr_strategy(), 1..4),
            bodies in prop::collection::vec(simple_expr_strategy(), 2..5)
        ) {
            let mut source = String::from("fn test() { ");
            for (i, cond) in conditions.iter().enumerate() {
                if i == 0 {
                    source.push_str(&format!("if {} {{ {} }}", cond, bodies.get(i).unwrap_or(&"0".to_string())));
                } else {
                    source.push_str(&format!(" else if {} {{ {} }}", cond, bodies.get(i).unwrap_or(&"0".to_string())));
                }
            }
            source.push_str(&format!(" else {{ {} }} }}", bodies.last().unwrap_or(&"0".to_string())));
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_array_literal_parses_without_panic(
            elements in prop::collection::vec(int_literal_strategy(), 0..10)
        ) {
            let source = format!("fn test() {{ [{}] }}", elements.join(", "));
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_tuple_literal_parses_without_panic(
            elements in prop::collection::vec(simple_expr_strategy(), 0..6)
        ) {
            let tuple_str = if elements.len() == 1 {
                format!("({},)", elements[0])
            } else {
                format!("({})", elements.join(", "))
            };
            let source = format!("fn test() {{ {} }}", tuple_str);
            // Should not panic
            let _ = parse_program(&source);
        }

        #[test]
        fn prop_closure_parses_without_panic(
            params in prop::collection::vec(ident_strategy(), 0..4),
            body in simple_expr_strategy()
        ) {
            let params_str = params.join(", ");
            let source = format!("fn test() {{ |{}| {} }}", params_str, body);
            // Should not panic
            let _ = parse_program(&source);
        }
    }

    // Property: parsing should be deterministic
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        #[test]
        fn prop_parsing_is_deterministic(source in fn_sig_strategy()) {
            let full_source = format!("{} {{}}", source);
            let result1 = parse_program(&full_source);
            let result2 = parse_program(&full_source);
            // Both parses should produce the same result
            prop_assert_eq!(
                result1.is_ok(),
                result2.is_ok(),
                "Parsing should be deterministic"
            );
        }
    }
}

// ============================================================
// CONTEXTUAL KEYWORD TESTS
// ============================================================

#[test]
fn test_op_keyword_in_struct_pattern() {
    // Issue #5: op keyword should work in struct patterns
    let source = r#"
        struct Test { op: i32 }
        fn main() -> i32 {
            let t = Test { op: 42 };
            match t {
                Test { op } => op,
            }
        }
    "#;
    let result = parse_program(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
}

#[test]
fn test_contextual_keywords_in_struct_patterns() {
    // All contextual keywords should work as struct field patterns
    assert!(parse_ok(r#"
        struct Config { default: i32 }
        fn test(c: Config) -> i32 {
            match c { Config { default } => default }
        }
    "#));
}

#[test]
fn test_op_keyword_in_struct_field_binding_syntax() {
    // Issue #6: field: binding syntax should work
    assert!(parse_ok(r#"
        struct Test { x: i32 }
        fn main() -> i32 {
            let t = Test { x: 42 };
            match t {
                Test { x: value } => value,
            }
        }
    "#));
}
