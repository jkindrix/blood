//! Fuzz target for effect handler parsing and type checking.
//!
//! This target tests that effect declarations and handlers are parsed and
//! type-checked without panicking, even with arbitrary/malformed input.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Parser;
use bloodc::typeck::TypeContext;

fuzz_target!(|data: &str| {
    // Wrap input in effect context to increase coverage of effect code paths
    let effect_wrapper = format!(
        r#"
        effect TestEffect {{
            op test_op() -> i32;
        }}

        {}
        "#,
        data
    );

    // Parse the wrapped input
    let mut parser = Parser::new(&effect_wrapper);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return, // Parsing failed, that's fine
    };

    // Take interner from parser
    let interner = parser.take_interner();

    // Attempt to type check - may succeed or fail, but should never panic
    let mut ctx = TypeContext::new(&effect_wrapper, interner);
    let _ = ctx.resolve_program(&program);
    // Note: we don't call check_all_bodies as it requires successful resolution
});
