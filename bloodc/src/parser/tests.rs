//! Parser tests using insta snapshot testing.
//!
//! These tests verify the parser produces correct ASTs for various inputs.
//! Snapshots are stored in `src/parser/snapshots/`.

use super::*;
use crate::ast::*;

/// Helper to parse an expression and return it (for snapshot tests).
#[allow(dead_code)]
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

#[test]
fn test_pub_use_reexport() {
    // Public re-export: `pub use` makes an import visible to external modules
    let source = "pub use std.io::println;";
    insta::assert_snapshot!(parse_to_debug(source));
}

#[test]
fn test_pub_use_glob_reexport() {
    // Public glob re-export
    let source = "pub use std.prelude::*;";
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
