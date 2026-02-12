//! Fuzz target for handler declaration parsing and type checking.
//!
//! This target tests handler declarations with various forms:
//! - deep handlers
//! - shallow handlers
//! - with/without return clauses
//! - with/without state fields

#![no_main]

use libfuzzer_sys::fuzz_target;
use libfuzzer_sys::arbitrary::{Arbitrary, Unstructured};
use bloodc::Parser;
use bloodc::typeck::TypeContext;

#[derive(Debug)]
struct HandlerInput {
    handler_kind: HandlerKind,
    effect_name: String,
    handler_name: String,
    has_return_clause: bool,
    op_body: String,
}

#[derive(Debug, Clone, Copy)]
enum HandlerKind {
    Deep,
    Shallow,
}

impl<'a> Arbitrary<'a> for HandlerInput {
    fn arbitrary(u: &mut Unstructured<'a>) -> libfuzzer_sys::arbitrary::Result<Self> {
        let handler_kind = if u.arbitrary::<bool>()? {
            HandlerKind::Deep
        } else {
            HandlerKind::Shallow
        };

        // Generate simple identifiers (avoid parse errors from bad chars)
        let effect_name = generate_ident(u, "Eff")?;
        let handler_name = generate_ident(u, "Handler")?;
        let has_return_clause = u.arbitrary()?;

        // Generate a simple body that might be valid
        let op_body = generate_simple_body(u)?;

        Ok(Self {
            handler_kind,
            effect_name,
            handler_name,
            has_return_clause,
            op_body,
        })
    }
}

fn generate_ident(u: &mut Unstructured, prefix: &str) -> libfuzzer_sys::arbitrary::Result<String> {
    let suffix: u8 = u.arbitrary()?;
    Ok(format!("{}{}", prefix, suffix % 100))
}

fn generate_simple_body(u: &mut Unstructured) -> libfuzzer_sys::arbitrary::Result<String> {
    let choice: u8 = u.arbitrary()?;
    Ok(match choice % 5 {
        0 => "0".to_string(),
        1 => "resume(0)".to_string(),
        2 => "resume(x + 1)".to_string(),
        3 => "{ let y = 42; resume(y) }".to_string(),
        _ => "x".to_string(),
    })
}

fuzz_target!(|input: HandlerInput| {
    let kind = match input.handler_kind {
        HandlerKind::Deep => "deep",
        HandlerKind::Shallow => "shallow",
    };

    let return_clause = if input.has_return_clause {
        "return(x) { x }"
    } else {
        ""
    };

    let source = format!(
        r#"
        effect {effect} {{
            op test_op(x: i32) -> i32;
        }}

        {kind} handler {handler} for {effect} {{
            {return_clause}
            op test_op(x) {{ {body} }}
        }}

        fn main() -> i32 {{
            with {handler} {{
                test_op(42)
            }}
        }}
        "#,
        effect = input.effect_name,
        handler = input.handler_name,
        kind = kind,
        return_clause = return_clause,
        body = input.op_body,
    );

    // Parse
    let mut parser = Parser::new(&source);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return,
    };

    let interner = parser.take_interner();

    // Type check
    let mut ctx = TypeContext::new(&source, interner);
    let _ = ctx.resolve_program(&program);
});
