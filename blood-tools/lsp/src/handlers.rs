//! Additional LSP Handlers
//!
//! Extended handler implementations for Blood-specific features.

use tower_lsp::lsp_types::*;

use crate::document::Document;

/// Provides inlay hints for Blood source files.
pub struct InlayHintProvider;

impl InlayHintProvider {
    /// Creates a new inlay hint provider.
    pub fn new() -> Self {
        Self
    }

    /// Provides inlay hints for a document range.
    pub fn provide(&self, doc: &Document, range: Range) -> Vec<InlayHint> {
        let mut hints = Vec::new();
        let text = doc.text();

        // TODO: Integrate with bloodc for actual type inference
        // For now, provide hints for let bindings without type annotations

        for (line_idx, line) in text.lines().enumerate() {
            let line_num = line_idx as u32;

            // Skip lines outside the requested range
            if line_num < range.start.line || line_num > range.end.line {
                continue;
            }

            // Look for let bindings without type annotations
            if let Some(hint) = self.check_let_binding(line, line_num) {
                hints.push(hint);
            }

            // Look for function parameters that could use type hints
            if let Some(mut param_hints) = self.check_function_params(line, line_num) {
                hints.append(&mut param_hints);
            }

            // Look for effect annotations
            if let Some(effect_hint) = self.check_effect_annotation(line, line_num) {
                hints.push(effect_hint);
            }
        }

        hints
    }

    /// Checks for let bindings that could use type hints.
    fn check_let_binding(&self, line: &str, line_num: u32) -> Option<InlayHint> {
        let trimmed = line.trim();

        // Match "let name = " or "let mut name = " patterns
        if let Some(after_let) = trimmed.strip_prefix("let ") {
            let rest = after_let.strip_prefix("mut ").unwrap_or(after_let);

            // Find the variable name
            let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
            let _name = &rest[..name_end];

            // Check if there's already a type annotation
            let after_name = rest[name_end..].trim();
            if after_name.starts_with(':') {
                // Already has type annotation
                return None;
            }

            if after_name.starts_with('=') {
                // No type annotation, could add hint
                let col = line.find("let ").unwrap() + 4;
                let col = if line[col..].starts_with("mut ") {
                    col + 4 + name_end
                } else {
                    col + name_end
                };

                // TODO: Get actual inferred type from bloodc
                return Some(InlayHint {
                    position: Position {
                        line: line_num,
                        character: col as u32,
                    },
                    label: InlayHintLabel::String(": <inferred>".to_string()),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::String(
                        "Type annotation can be added explicitly".to_string(),
                    )),
                    padding_left: Some(false),
                    padding_right: Some(true),
                    data: None,
                });
            }
        }

        None
    }

    /// Checks function parameters for hints.
    fn check_function_params(&self, line: &str, _line_num: u32) -> Option<Vec<InlayHint>> {
        let trimmed = line.trim();

        // Look for function calls with arguments
        // Pattern: identifier(arg1, arg2, ...)
        if let Some(paren_start) = trimmed.find('(') {
            if paren_start > 0 {
                let before_paren = &trimmed[..paren_start];

                // Make sure it's a function call (ends with identifier)
                if before_paren
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                {
                    // TODO: Look up function signature and provide parameter name hints
                    // This requires integration with bloodc
                }
            }
        }

        None
    }

    /// Checks for missing effect annotations.
    fn check_effect_annotation(&self, line: &str, line_num: u32) -> Option<InlayHint> {
        let trimmed = line.trim();

        // Look for function definitions without effect annotations
        // Pattern: fn name(...) -> Type {
        // Should have: fn name(...) -> Type / Effects {
        if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") {
            // Check if there's an effect annotation (contains " / ")
            if !trimmed.contains(" / ") {
                // Look for the return type or opening brace
                if let Some(arrow_pos) = trimmed.find("->") {
                    // Find where to insert the effect annotation
                    let after_arrow = &trimmed[arrow_pos + 2..];
                    if let Some(brace_pos) = after_arrow.find('{') {
                        let insert_pos = arrow_pos + 2 + brace_pos;
                        let col = line.find("fn ").unwrap() + insert_pos;

                        // TODO: Get actual effect from bloodc analysis
                        return Some(InlayHint {
                            position: Position {
                                line: line_num,
                                character: col as u32,
                            },
                            label: InlayHintLabel::String("/ pure".to_string()),
                            kind: Some(InlayHintKind::TYPE),
                            text_edits: None,
                            tooltip: Some(InlayHintTooltip::String(
                                "Inferred effect annotation".to_string(),
                            )),
                            padding_left: Some(true),
                            padding_right: Some(true),
                            data: None,
                        });
                    }
                }
            }
        }

        None
    }
}

impl Default for InlayHintProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Provides code lens items for Blood source files.
pub struct CodeLensProvider;

impl CodeLensProvider {
    /// Creates a new code lens provider.
    pub fn new() -> Self {
        Self
    }

    /// Provides code lenses for a document.
    pub fn provide(&self, doc: &Document) -> Vec<CodeLens> {
        let mut lenses = Vec::new();
        let text = doc.text();

        for (line_idx, line) in text.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Add "Run" lens for main function
            if trimmed.starts_with("fn main(") || trimmed.starts_with("pub fn main(") {
                lenses.push(CodeLens {
                    range: Range {
                        start: Position {
                            line: line_num,
                            character: 0,
                        },
                        end: Position {
                            line: line_num,
                            character: line.len() as u32,
                        },
                    },
                    command: Some(Command {
                        title: "Run".to_string(),
                        command: "blood.run".to_string(),
                        arguments: None,
                    }),
                    data: None,
                });
            }

            // Add "Test" lens for test functions
            if trimmed.contains("#[test]") || trimmed.starts_with("fn test_") {
                lenses.push(CodeLens {
                    range: Range {
                        start: Position {
                            line: line_num,
                            character: 0,
                        },
                        end: Position {
                            line: line_num,
                            character: line.len() as u32,
                        },
                    },
                    command: Some(Command {
                        title: "Run Test".to_string(),
                        command: "blood.runTest".to_string(),
                        arguments: None,
                    }),
                    data: None,
                });
            }

            // Add "References" lens for effect declarations
            if trimmed.starts_with("effect ") || trimmed.starts_with("pub effect ") {
                lenses.push(CodeLens {
                    range: Range {
                        start: Position {
                            line: line_num,
                            character: 0,
                        },
                        end: Position {
                            line: line_num,
                            character: line.len() as u32,
                        },
                    },
                    command: Some(Command {
                        title: "Find Handlers".to_string(),
                        command: "blood.findHandlers".to_string(),
                        arguments: None,
                    }),
                    data: None,
                });
            }

            // Add "Implementations" lens for handler declarations
            if trimmed.contains("handler ") && trimmed.contains(" for ") {
                lenses.push(CodeLens {
                    range: Range {
                        start: Position {
                            line: line_num,
                            character: 0,
                        },
                        end: Position {
                            line: line_num,
                            character: line.len() as u32,
                        },
                    },
                    command: Some(Command {
                        title: "Go to Effect".to_string(),
                        command: "blood.goToEffect".to_string(),
                        arguments: None,
                    }),
                    data: None,
                });
            }
        }

        lenses
    }
}

impl Default for CodeLensProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Provides folding ranges for Blood source files.
pub struct FoldingRangeProvider;

impl FoldingRangeProvider {
    /// Creates a new folding range provider.
    pub fn new() -> Self {
        Self
    }

    /// Provides folding ranges for a document.
    pub fn provide(&self, doc: &Document) -> Vec<FoldingRange> {
        let mut ranges = Vec::new();
        let text = doc.text();
        let mut brace_stack: Vec<(u32, FoldingRangeKind)> = Vec::new();
        let mut in_block_comment = false;
        let mut comment_start = 0u32;

        for (line_idx, line) in text.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Block comments
            if trimmed.starts_with("/*") && !in_block_comment {
                in_block_comment = true;
                comment_start = line_num;
            }
            if trimmed.ends_with("*/") && in_block_comment {
                in_block_comment = false;
                if line_num > comment_start {
                    ranges.push(FoldingRange {
                        start_line: comment_start,
                        start_character: None,
                        end_line: line_num,
                        end_character: None,
                        kind: Some(FoldingRangeKind::Comment),
                        collapsed_text: Some("/* ... */".to_string()),
                    });
                }
            }

            // Doc comments (consecutive /// lines)
            if trimmed.starts_with("///") {
                // Look ahead to find the end of doc comments
                // Handled separately for simplicity
            }

            // Imports (use statements)
            if trimmed.starts_with("use ") && trimmed.ends_with('{') {
                brace_stack.push((line_num, FoldingRangeKind::Imports));
            }

            // Region markers (custom for Blood)
            if trimmed.starts_with("// region:") {
                brace_stack.push((line_num, FoldingRangeKind::Region));
            }
            if trimmed.starts_with("// endregion") {
                if let Some((start, FoldingRangeKind::Region)) = brace_stack.pop() {
                    ranges.push(FoldingRange {
                        start_line: start,
                        start_character: None,
                        end_line: line_num,
                        end_character: None,
                        kind: Some(FoldingRangeKind::Region),
                        collapsed_text: None,
                    });
                }
            }

            // Brace-delimited blocks
            for c in line.chars() {
                match c {
                    '{' => {
                        let kind = if trimmed.starts_with("use ") {
                            FoldingRangeKind::Imports
                        } else {
                            FoldingRangeKind::Region
                        };
                        brace_stack.push((line_num, kind));
                    }
                    '}' => {
                        if let Some((start, kind)) = brace_stack.pop() {
                            if line_num > start {
                                ranges.push(FoldingRange {
                                    start_line: start,
                                    start_character: None,
                                    end_line: line_num,
                                    end_character: None,
                                    kind: Some(kind),
                                    collapsed_text: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        ranges
    }
}

impl Default for FoldingRangeProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Hover information builder for Blood.
pub struct HoverBuilder;

impl HoverBuilder {
    /// Builds hover information for a keyword.
    pub fn keyword_hover(keyword: &str) -> Option<Hover> {
        let docs = match keyword {
            "fn" => "Declares a function.\n\n```blood\nfn name(params) -> ReturnType / Effects {\n    body\n}\n```",
            "let" => "Declares a variable binding.\n\n```blood\nlet name: Type = value;\nlet mut name = value;  // mutable\n```",
            "effect" => "Declares an algebraic effect.\n\n```blood\neffect Name {\n    op operation(params) -> ReturnType;\n}\n```",
            "handler" => "Declares an effect handler.\n\n```blood\ndeep handler Name for Effect {\n    return(x) { x }\n    op operation(params) { resume(result) }\n}\n```",
            "perform" => "Performs an effect operation.\n\n```blood\nlet result = perform operation(args);\n```",
            "resume" => "Resumes a continuation in a handler.\n\n```blood\nop read() { resume(value) }\n```",
            "handle" => "Handles effects in an expression.\n\n```blood\nhandle expr with handler { ... }\n```",
            "pure" => "Annotation indicating no effects.\n\n```blood\nfn pure_fn() -> i32 / pure { 42 }\n```",
            "linear" => "Annotation for linear types that must be used exactly once.",
            "struct" => "Declares a struct type.\n\n```blood\nstruct Name {\n    field: Type,\n}\n```",
            "enum" => "Declares an enum type.\n\n```blood\nenum Name {\n    Variant1,\n    Variant2(Type),\n}\n```",
            "trait" => "Declares a trait.\n\n```blood\ntrait Name {\n    fn method(&self) -> Type;\n}\n```",
            "impl" => "Implements methods or traits.\n\n```blood\nimpl Type {\n    fn method(&self) { ... }\n}\n```",
            "match" => "Pattern matching expression.\n\n```blood\nmatch value {\n    Pattern1 => result1,\n    Pattern2 => result2,\n    _ => default,\n}\n```",
            _ => return None,
        };

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: docs.to_string(),
            }),
            range: None,
        })
    }

    /// Builds hover information for a type.
    pub fn type_hover(type_name: &str) -> Option<Hover> {
        let docs = match type_name {
            "Option" => "A type that represents an optional value.\n\n```blood\nenum Option<T> {\n    Some(T),\n    None,\n}\n```",
            "Result" => "A type that represents success or failure.\n\n```blood\nenum Result<T, E> {\n    Ok(T),\n    Err(E),\n}\n```",
            "Box" => "A heap-allocated value with ownership semantics.",
            "Vec" => "A growable array type.",
            "String" => "A heap-allocated UTF-8 string.",
            "Frozen" => "A deeply immutable wrapper type.\n\n```blood\nlet frozen_data: Frozen<Data> = freeze(data);\n```",
            _ => return None,
        };

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: docs.to_string(),
            }),
            range: None,
        })
    }
}
