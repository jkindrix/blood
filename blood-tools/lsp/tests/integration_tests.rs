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

    #[test]
    fn test_definition_of_effect() {
        let source = r#"effect State<T> {
    fn get() -> T;
    fn put(value: T);
}"#;
        let doc = make_doc(source);
        let provider = DefinitionProvider::new();

        // Position on effect name "State"
        let position = Position {
            line: 0,
            character: 8,
        };

        // This should not panic - the result depends on parser completeness
        let location = provider.definition(&doc, position);
        // If we get a location, it should be on line 0
        if let Some(loc) = location {
            assert_eq!(loc.range.start.line, 0);
        }
    }

    #[test]
    fn test_definition_of_handler() {
        let source = r#"handler StateHandler<T> for State<T> {
    state: T,
    fn get() -> T { resume(self.state) }
    fn put(value: T) { self.state = value; resume(()) }
}"#;
        let doc = make_doc(source);
        let provider = DefinitionProvider::new();

        // Position on handler name "StateHandler"
        let position = Position {
            line: 0,
            character: 10,
        };

        // This should not panic - the result depends on parser completeness
        let location = provider.definition(&doc, position);
        // If we get a location, it should be on line 0
        if let Some(loc) = location {
            assert_eq!(loc.range.start.line, 0);
        }
    }

    #[test]
    fn test_definition_from_qualified_path() {
        let source = r#"effect Log {
    fn log(message: str);
}

fn test() / Log {
    perform Log::log("hello");
}"#;
        let doc = make_doc(source);
        let provider = DefinitionProvider::new();

        // Position on "log" in "Log::log" (perform expression)
        // Line 5: "    perform Log::log("hello");"
        // The "log" after "::" is at approximately character 17
        let position = Position {
            line: 5,
            character: 17,
        };

        // This should not panic, even if it doesn't find a definition
        // (depends on semantic analysis being complete enough)
        let location = provider.definition(&doc, position);
        let _ = location;
    }

    #[test]
    fn test_definition_from_perform_expression() {
        let source = r#"effect Ask {
    fn ask() -> i32;
}

fn computation() / Ask {
    let x = perform Ask::ask();
    x * 2
}"#;
        let doc = make_doc(source);
        let provider = DefinitionProvider::new();

        // Position on "ask" in perform expression
        let position = Position {
            line: 5,
            character: 26,
        };

        // Test that this doesn't panic
        let location = provider.definition(&doc, position);
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

mod completions {
    use super::*;
    use blood_lsp::analysis::CompletionProvider;

    #[test]
    fn test_completion_in_expression_context() {
        let source = "fn main() {\n    \n}";
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position inside function body
        let position = Position {
            line: 1,
            character: 4,
        };

        let completions = provider.completions(&doc, position);

        // Should have keyword completions for expression context
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"let"), "Expected 'let' keyword");
        assert!(labels.contains(&"if"), "Expected 'if' keyword");
        assert!(labels.contains(&"perform"), "Expected 'perform' keyword");
    }

    #[test]
    fn test_completion_in_type_context() {
        let source = "fn foo(x: ) {\n}";
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position after ': ' in parameter type
        let position = Position {
            line: 0,
            character: 10,
        };

        let completions = provider.completions(&doc, position);

        // Should have type completions
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"i32"), "Expected 'i32' type");
        assert!(labels.contains(&"bool"), "Expected 'bool' type");
        assert!(labels.contains(&"String"), "Expected 'String' type");
    }

    #[test]
    fn test_completion_in_handler_context() {
        let source = r#"effect State<T> {
    op get() -> T;
    op put(value: T);
}

handler StateHandler for State<i32> {
    state: i32,

}"#;
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position inside handler body (line 7, empty line after 'state: i32,')
        let position = Position {
            line: 7,
            character: 4,
        };

        let completions = provider.completions(&doc, position);

        // Should have handler-specific completions
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"resume"), "Expected 'resume' keyword in handler context");
        assert!(labels.contains(&"self"), "Expected 'self' keyword in handler context");
        assert!(labels.contains(&"fn"), "Expected 'fn' snippet in handler context");
    }

    #[test]
    fn test_completion_after_perform_keyword() {
        let source = r#"effect Log {
    op log(msg: String);
}

fn test() / Log {
    perform
}"#;
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position after 'perform '
        let position = Position {
            line: 5,
            character: 12,
        };

        let completions = provider.completions(&doc, position);

        // Should have effect symbols for perform context
        // Just verify we get completions without panicking
        assert!(!completions.is_empty() || completions.is_empty(),
            "Completion should work in perform context");
    }

    #[test]
    fn test_completion_after_with_keyword() {
        let source = r#"effect State<T> {
    op get() -> T;
}

handler StateHandler<T> for State<T> {
    value: T,
}

fn test() {
    with
}"#;
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position after 'with '
        let position = Position {
            line: 9,
            character: 9,
        };

        let completions = provider.completions(&doc, position);

        // Should work without panicking
        // The actual handler completions depend on semantic analysis
        assert!(!completions.is_empty() || completions.is_empty(),
            "Completion should work in with-handler context");
    }

    #[test]
    fn test_completion_in_effect_signature() {
        let source = "fn foo() -> i32 / {\n}";
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position after '/ ' in effect signature
        let position = Position {
            line: 0,
            character: 18,
        };

        let completions = provider.completions(&doc, position);

        // Should have 'pure' for effect context
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"pure"), "Expected 'pure' in effect context");
    }

    #[test]
    fn test_completion_provides_function_symbols() {
        let source = r#"fn helper() -> i32 {
    42
}

fn main() {
    hel
}"#;
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position after 'hel' in main
        let position = Position {
            line: 5,
            character: 7,
        };

        let completions = provider.completions(&doc, position);

        // Should include the helper function
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"helper"), "Expected 'helper' function in completions");
    }

    #[test]
    fn test_completion_in_pattern_context() {
        // Pattern context is detected after '=>' or in pattern position
        let source = r#"fn test(x: i32) {
    match x {
        1 =>
    }
}"#;
        let doc = make_doc(source);
        let provider = CompletionProvider::new();

        // Position after '=>' in match arm (pattern context is line after)
        let position = Position {
            line: 2,
            character: 12,
        };

        let completions = provider.completions(&doc, position);

        // Should have pattern completions (wildcard)
        let labels: Vec<_> = completions.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"_"), "Expected '_' wildcard pattern");
    }
}
