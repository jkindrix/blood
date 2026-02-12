//! Fuzz target for expression parsing specifically.
//!
//! This target wraps input in a function body to specifically test the
//! expression parser, which uses Pratt parsing and has complex precedence
//! handling that could potentially have edge cases.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Parser;

fuzz_target!(|data: &str| {
    // Wrap the input in a function body to test expression parsing
    let source = format!("fn fuzz() {{ {} }}", data);

    // Create a parser for the wrapped input
    let mut parser = Parser::new(&source);

    // Attempt to parse - may succeed or fail, but should never panic
    let _ = parser.parse_program();
});
