//! MIR lowering fuzz target with grammar-generated programs.
//!
//! This target exercises the full pipeline from parsing through MIR lowering:
//! Parse → Type Check → HIR → MIR. This catches bugs in the lowering from
//! high-level typed IR to mid-level IR.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Parser;
use bloodc::typeck::TypeContext;
use bloodc::mir::MirLowering;
use bloodc_fuzz::FuzzProgram;

fuzz_target!(|program: FuzzProgram| {
    // Convert the structured program to source code
    let source = program.to_source();

    // Parse the generated source
    let mut parser = Parser::new(&source);
    let ast = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return, // Parsing failed, skip further stages
    };

    // Take interner from parser
    let interner = parser.take_interner();

    // Run type checking
    let mut ctx = TypeContext::new(&source, interner);

    // Name resolution phase
    if ctx.resolve_program(&ast).is_err() {
        return; // Resolution failed
    }

    // Type inference and checking phase
    if ctx.check_all_bodies().is_err() {
        return; // Type checking failed
    }

    // Convert to HIR
    let hir_crate = ctx.into_hir();

    // Run MIR lowering - should never panic
    let mut lowering = MirLowering::new(&hir_crate);
    let _ = lowering.lower_crate();

    // Even if MIR lowering fails (semantic errors), it should handle
    // gracefully without panicking
});
