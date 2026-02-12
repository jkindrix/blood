//! Fuzz target for the Blood parser.
//!
//! This target tests that the parser handles arbitrary input without panicking.
//! The parser should gracefully return errors for invalid input while never
//! crashing or producing undefined behavior.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Parser;

fuzz_target!(|data: &str| {
    // Create a parser for the input
    let mut parser = Parser::new(data);

    // Attempt to parse - may succeed or fail, but should never panic
    let _ = parser.parse_program();
});
