//! Integration tests for the Blood LSP server.

use blood_lsp::analysis::{DefinitionProvider, HoverProvider, SemanticAnalyzer};
use blood_lsp::diagnostics::DiagnosticEngine;
use blood_lsp::document::Document;
use tower_lsp::lsp_types::*;

/// Creates a test document from source code.
fn make_doc(source: &str) -> Document {
    let uri = Url::parse("file:///test.blood").expect("valid URL");
    Document::new(uri, 1, source.to_string())
}

mod diagnostics {
    use super::*;

    #[test]
    fn test_no_errors_on_valid_code() {
        let source = r#"fn main() {
    let x: i32 = 42;
}"#;
        let doc = make_doc(source);
        let engine = DiagnosticEngine::new();
        let diagnostics = engine.check(&doc);

        // Filter out lint warnings (TODO, FIXME, etc.)
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .collect();

        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_parse_error_reported() {
        let source = r#"
fn main() {
    let x =
}
"#;
        let doc = make_doc(source);
        let engine = DiagnosticEngine::new();
        let diagnostics = engine.check(&doc);

        // Should have at least one error
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .collect();

        assert!(!errors.is_empty(), "Expected parse error");
    }

    #[test]
    fn test_lint_warnings() {
        let source = r#"
// TODO: implement this
fn main() {
    42
}
"#;
        let doc = make_doc(source);
        let engine = DiagnosticEngine::new();
        let diagnostics = engine.check(&doc);

        // Should have TODO warning
        let todo_warnings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.message.contains("TODO"))
            .collect();

        assert!(!todo_warnings.is_empty(), "Expected TODO warning");
    }
}

mod analysis {
    use super::*;

    #[test]
    fn test_collect_function_symbols() {
        let source = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    add(1, 2)
}
"#;
        let doc = make_doc(source);
        let analyzer = SemanticAnalyzer::new();
        let result = analyzer.analyze(&doc).expect("analysis should succeed");

        // Should have at least the two functions
        let fn_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::FUNCTION)
            .collect();

        assert!(fn_symbols.len() >= 2, "Expected at least 2 functions, got {:?}", fn_symbols);

        // Check that 'add' function is found with correct signature
        let add_fn = fn_symbols.iter().find(|s| s.name == "add");
        assert!(add_fn.is_some(), "Expected 'add' function");

        let add = add_fn.unwrap();
        assert!(
            add.description.contains("fn add"),
            "Expected 'fn add' in description, got: {}",
            add.description
        );
    }

    #[test]
    fn test_collect_struct_symbols() {
        let source = r#"
struct Point {
    x: i32,
    y: i32,
}
"#;
        let doc = make_doc(source);
        let analyzer = SemanticAnalyzer::new();
        let result = analyzer.analyze(&doc).expect("analysis should succeed");

        let struct_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::STRUCT)
            .collect();

        assert_eq!(struct_symbols.len(), 1, "Expected 1 struct");
        assert_eq!(struct_symbols[0].name, "Point");
    }

    #[test]
    fn test_collect_enum_symbols() {
        let source = r#"
enum Option<T> {
    Some(T),
    None,
}
"#;
        let doc = make_doc(source);
        let analyzer = SemanticAnalyzer::new();
        let result = analyzer.analyze(&doc).expect("analysis should succeed");

        let enum_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::ENUM)
            .collect();

        assert_eq!(enum_symbols.len(), 1, "Expected 1 enum");
        assert_eq!(enum_symbols[0].name, "Option");

        // Check variants
        let variant_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::ENUM_MEMBER)
            .collect();

        assert!(variant_symbols.len() >= 2, "Expected at least 2 variants");
    }

    #[test]
    fn test_collect_effect_symbols() {
        let source = r#"
effect Console {
    op print(msg: String) -> ();
    op read() -> String;
}
"#;
        let doc = make_doc(source);
        let analyzer = SemanticAnalyzer::new();
        let result = analyzer.analyze(&doc).expect("analysis should succeed");

        let effect_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::INTERFACE)
            .collect();

        assert_eq!(effect_symbols.len(), 1, "Expected 1 effect");
        assert_eq!(effect_symbols[0].name, "Console");

        // Check operations
        let op_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::METHOD)
            .collect();

        assert!(op_symbols.len() >= 2, "Expected at least 2 operations");
    }

    #[test]
    fn test_collect_variable_symbols() {
        let source = r#"
fn main() {
    let x: i32 = 42;
    let y = x + 1;
    y
}
"#;
        let doc = make_doc(source);
        let analyzer = SemanticAnalyzer::new();
        let result = analyzer.analyze(&doc).expect("analysis should succeed");

        let var_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::VARIABLE && (s.name == "x" || s.name == "y"))
            .collect();

        assert!(var_symbols.len() >= 2, "Expected at least 2 variables");
    }
}

mod hover {
    use super::*;

    #[test]
    fn test_hover_on_function() {
        let source = r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}"#;
        let doc = make_doc(source);
        let provider = HoverProvider::new();

        // Position on 'add' function name
        let position = Position {
            line: 0,
            character: 3,
        };

        let hover = provider.hover(&doc, position);
        assert!(hover.is_some(), "Expected hover info for function");

        let info = hover.unwrap();
        if let HoverContents::Markup(content) = info.contents {
            assert!(
                content.value.contains("fn add"),
                "Expected function signature in hover, got: {}",
                content.value
            );
        }
    }

    #[test]
    fn test_hover_on_struct() {
        let source = r#"struct Point {
    x: i32,
    y: i32,
}"#;
        let doc = make_doc(source);
        let provider = HoverProvider::new();

        // Position on 'Point' struct name
        let position = Position {
            line: 0,
            character: 8,
        };

        let hover = provider.hover(&doc, position);
        assert!(hover.is_some(), "Expected hover info for struct");

        let info = hover.unwrap();
        if let HoverContents::Markup(content) = info.contents {
            assert!(
                content.value.contains("struct Point"),
                "Expected struct in hover, got: {}",
                content.value
            );
        }
    }

    #[test]
    fn test_no_hover_on_whitespace() {
        let source = "fn main() { }";
        let doc = make_doc(source);
        let provider = HoverProvider::new();

        // Position on whitespace
        let position = Position {
            line: 0,
            character: 10,
        };

        let hover = provider.hover(&doc, position);
        // May or may not have hover depending on position - just ensure no panic
        let _ = hover;
    }
}

mod definition {
    use super::*;

    #[test]
    fn test_definition_of_function() {
        let source = r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}"#;
        let doc = make_doc(source);
        let provider = DefinitionProvider::new();

        // Position on 'add' function name
        let position = Position {
            line: 0,
            character: 3,
        };

        let location = provider.definition(&doc, position);
        assert!(location.is_some(), "Expected definition location");

        let loc = location.unwrap();
        assert_eq!(loc.uri.as_str(), "file:///test.blood");
        // Definition should point to the function name
        assert_eq!(loc.range.start.line, 0);
    }

    #[test]
    fn test_definition_of_struct() {
        let source = r#"struct Point {
    x: i32,
    y: i32,
}"#;
        let doc = make_doc(source);
        let provider = DefinitionProvider::new();

        // Position on 'Point' struct name
        let position = Position {
            line: 0,
            character: 8,
        };

        let location = provider.definition(&doc, position);
        assert!(location.is_some(), "Expected definition location for struct");

        let loc = location.unwrap();
        assert_eq!(loc.range.start.line, 0);
    }

    #[test]
    fn test_no_definition_on_whitespace() {
        let source = "fn main() { }";
        let doc = make_doc(source);
        let provider = DefinitionProvider::new();

        // Position on whitespace between braces
        let position = Position {
            line: 0,
            character: 11,
        };

        let location = provider.definition(&doc, position);
        // No definition expected on whitespace - just ensure no panic
        let _ = location;
    }
}

mod document {
    use super::*;

    #[test]
    fn test_document_position_conversion() {
        let source = "fn main() {\n    let x = 42;\n}";
        let doc = make_doc(source);

        // Test round-trip conversion
        let pos = Position {
            line: 1,
            character: 8,
        };

        let offset = doc.position_to_offset(pos);
        assert!(offset.is_some(), "Position should be valid");

        let back = doc.offset_to_position(offset.unwrap());
        assert_eq!(pos.line, back.line);
        assert_eq!(pos.character, back.character);
    }

    #[test]
    fn test_document_word_at_position() {
        let source = "let foo = 42";
        let doc = make_doc(source);

        let word = doc.word_at_position(Position {
            line: 0,
            character: 5,
        });

        assert!(word.is_some(), "Word should be found");
        assert_eq!(word.unwrap().text, "foo");
    }

    #[test]
    fn test_document_apply_change() {
        let source = "fn main() { }";
        let mut doc = make_doc(source);

        // Apply incremental change - insert 'x' at position
        doc.apply_change(
            2,
            TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position {
                        line: 0,
                        character: 11,
                    },
                    end: Position {
                        line: 0,
                        character: 11,
                    },
                }),
                range_length: None,
                text: "let x = 1; ".to_string(),
            },
        );

        let new_text = doc.text();
        assert!(new_text.contains("let x = 1"), "Change should be applied");
        assert_eq!(doc.version(), 2);
    }
}
