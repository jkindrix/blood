//! Grammar-based fuzz target for the Blood parser.
//!
//! This target uses structured grammar generators to create syntactically
//! plausible Blood programs, providing better coverage than random byte
//! fuzzing alone.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Parser;
use bloodc_fuzz::FuzzProgram;

fuzz_target!(|program: FuzzProgram| {
    // Convert the structured program to source code
    let source = program.to_source();

    // Parse the generated source - should never panic
    let mut parser = Parser::new(&source);
    let _ = parser.parse_program();

    // Even if parsing fails (some generated programs may be semantically
    // invalid), the parser should handle it gracefully without panicking
});
