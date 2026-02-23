//! Pattern matching fuzz target.
//!
//! This target exercises pattern parsing and type checking with structured
//! match expressions. Patterns have complex semantics (binding, guards,
//! exhaustiveness) and deserve dedicated fuzzing coverage.

#![no_main]

use libfuzzer_sys::fuzz_target;
use bloodc::Parser;
use bloodc::typeck::TypeContext;
use bloodc_fuzz::FuzzMatch;
use arbitrary::Arbitrary;

/// A wrapper that generates a function containing a match expression.
#[derive(Debug)]
struct MatchWrapper {
    match_expr: FuzzMatch,
}

impl<'a> Arbitrary<'a> for MatchWrapper {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(MatchWrapper {
            match_expr: FuzzMatch::arbitrary(u)?,
        })
    }
}

impl MatchWrapper {
    fn to_source(&self) -> String {
        format!(
            r#"
fn test_match(x: i32) -> i32 {{
    {}
}}
"#,
            self.match_expr.to_source()
        )
    }
}

fuzz_target!(|wrapper: MatchWrapper| {
    let source = wrapper.to_source();

    // Parse the generated source
    let mut parser = Parser::new(&source);
    let ast = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return, // Parsing failed, skip further stages
    };

    // Take interner from parser
    let interner = parser.take_interner();

    // Run type checking - patterns exercise binding analysis and exhaustiveness
    let mut ctx = TypeContext::new(&source, interner);

    // Name resolution phase
    if ctx.resolve_program(&ast).is_err() {
        return; // Resolution failed
    }

    // Type inference and checking phase
    let _ = ctx.check_all_bodies();

    // Even if type checking fails, it should not panic
});
