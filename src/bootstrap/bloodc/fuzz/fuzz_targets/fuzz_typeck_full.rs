//! Full type checker fuzz target with grammar-generated programs.
//!
//! This target uses structured grammar generators to create syntactically
//! valid Blood programs, then exercises the full type checking pipeline.
//! The type checker should handle all inputs without panicking.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Parser;
use bloodc::typeck::TypeContext;
use bloodc_fuzz::FuzzProgram;

fuzz_target!(|program: FuzzProgram| {
    // Convert the structured program to source code
    let source = program.to_source();

    // Parse the generated source
    let mut parser = Parser::new(&source);
    let ast = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return, // Parsing failed, skip type checking
    };

    // Take interner from parser
    let interner = parser.take_interner();

    // Run full type checking - should never panic
    let mut ctx = TypeContext::new(&source, interner);

    // Name resolution phase
    if ctx.resolve_program(&ast).is_err() {
        return; // Resolution failed, that's expected for some inputs
    }

    // Type inference and checking phase
    let _ = ctx.check_all_bodies();

    // Even if type checking fails (semantic errors), the checker should
    // handle it gracefully without panicking
});
