//! Fuzz target for the Blood lexer.
//!
//! This target tests that the lexer handles arbitrary input without panicking
//! or producing undefined behavior. The lexer should gracefully handle any
//! input, even malformed or invalid UTF-8 sequences.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Lexer;

fuzz_target!(|data: &str| {
    // Create a lexer for the input
    let lexer = Lexer::new(data);

    // Consume all tokens - should never panic
    for token in lexer {
        // Access token fields to ensure they're valid
        let _ = token.kind;
        let _ = token.span;
    }
});
