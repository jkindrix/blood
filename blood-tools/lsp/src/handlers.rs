//! Additional LSP Handlers
//!
//! Extended handler implementations for Blood-specific features.

use std::collections::HashMap;

use tower_lsp::lsp_types::*;

use crate::analysis::{AnalysisResult, SemanticAnalyzer};
use crate::document::Document;

/// Provides inlay hints for Blood source files.
pub struct InlayHintProvider {
    analyzer: SemanticAnalyzer,
}

impl InlayHintProvider {
    /// Creates a new inlay hint provider.
    pub fn new() -> Self {
        Self {
            analyzer: SemanticAnalyzer::new(),
        }
    }

    /// Provides inlay hints for a document range.
    pub fn provide(&self, doc: &Document, range: Range) -> Vec<InlayHint> {
        let mut hints = Vec::new();
        let text = doc.text();

        // Try to get inferred types from type checking
        let analysis = self.analyzer.analyze(doc);
        let inferred_types = analysis.as_ref()
            .map(|a| &a.inferred_types)
            .cloned()
            .unwrap_or_default();

        for (line_idx, line) in text.lines().enumerate() {
            let line_num = line_idx as u32;

            // Skip lines outside the requested range
            if line_num < range.start.line || line_num > range.end.line {
                continue;
            }

            // Look for let bindings without type annotations
            if let Some(hint) = self.check_let_binding_with_types(line, line_num, &text, &inferred_types) {
                hints.push(hint);
            }

            // Look for function parameters that could use type hints
            if let Some(mut param_hints) = self.check_function_params_with_analysis(line, line_num, analysis.as_ref()) {
                hints.append(&mut param_hints);
            }

            // Look for effect annotations
            if let Some(effect_hint) = self.check_effect_annotation_with_analysis(line, line_num, analysis.as_ref()) {
                hints.push(effect_hint);
            }
        }

        hints
    }

    /// Checks for let bindings that could use type hints, with real type inference.
    fn check_let_binding_with_types(
        &self,
        line: &str,
        line_num: u32,
        full_text: &str,
        inferred_types: &HashMap<usize, String>,
    ) -> Option<InlayHint> {
        let trimmed = line.trim();

        // Match "let name = " or "let mut name = " patterns
        if let Some(after_let) = trimmed.strip_prefix("let ") {
            let rest = after_let.strip_prefix("mut ").unwrap_or(after_let);

            // Find the variable name
            let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
            let var_name = &rest[..name_end];

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

                // Calculate the byte offset for this line/column
                let line_offset = self.line_to_offset(full_text, line_num as usize);
                let var_start = line_offset + line.find(var_name).unwrap_or(0);

                // Try to get the inferred type from type checking
                let type_str = inferred_types.get(&var_start)
                    .map(|t| format!(": {}", t))
                    .unwrap_or_else(|| ": <inferred>".to_string());

                let tooltip = if type_str == ": <inferred>" {
                    "Type annotation can be added explicitly (type checking incomplete)"
                } else {
                    "Type annotation can be added explicitly"
                };

                return Some(InlayHint {
                    position: Position {
                        line: line_num,
                        character: col as u32,
                    },
                    label: InlayHintLabel::String(type_str),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::String(tooltip.to_string())),
                    padding_left: Some(false),
                    padding_right: Some(true),
                    data: None,
                });
            }
        }

        None
    }

    /// Calculates the byte offset for the start of a given line.
    fn line_to_offset(&self, text: &str, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        // Sum up the byte lengths of all lines before the target line
        text.lines()
            .take(line)
            .map(|l| l.len() + 1) // +1 for newline
            .sum()
    }

    /// Checks for let bindings that could use type hints (legacy version).
    #[allow(dead_code)]
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

    /// Checks function parameters for hints, using analysis data to get parameter names.
    fn check_function_params_with_analysis(
        &self,
        line: &str,
        line_num: u32,
        analysis: Option<&AnalysisResult>,
    ) -> Option<Vec<InlayHint>> {
        let trimmed = line.trim();
        let mut hints = Vec::new();

        // Look for function calls with arguments
        // Pattern: identifier(arg1, arg2, ...)
        if let Some(paren_start) = trimmed.find('(') {
            if paren_start > 0 {
                let before_paren = &trimmed[..paren_start];

                // Extract the function name (last identifier before the paren)
                let fn_name = before_paren
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>();

                if fn_name.is_empty() {
                    return None;
                }

                // Try to find the function signature from the analysis
                let param_names = analysis.and_then(|a| {
                    a.symbols.iter().find_map(|sym| {
                        if sym.name == fn_name && sym.kind == SymbolKind::FUNCTION {
                            // Parse parameter names from the description
                            // Format: "fn name(param1: Type1, param2: Type2) -> ReturnType"
                            self.extract_param_names(&sym.description)
                        } else {
                            None
                        }
                    })
                });

                // If we found parameter names, provide hints
                if let Some(names) = param_names {
                    // Find arguments in the call
                    if let Some(paren_end) = trimmed.rfind(')') {
                        let args_str = &trimmed[paren_start + 1..paren_end];

                        // Parse arguments (simple comma-splitting for now)
                        let args: Vec<&str> = self.split_arguments(args_str);

                        // Provide hints for each argument
                        let call_start = line.find(&fn_name).unwrap_or(0);
                        let args_offset = call_start + fn_name.len() + 1; // +1 for '('

                        let mut current_pos = args_offset;
                        for (i, arg) in args.iter().enumerate() {
                            if let Some(param_name) = names.get(i) {
                                // Skip if the argument already has a label (e.g., "name: value")
                                if !arg.contains(':') {
                                    hints.push(InlayHint {
                                        position: Position {
                                            line: line_num,
                                            character: (current_pos + arg.len() - arg.trim_start().len()) as u32,
                                        },
                                        label: InlayHintLabel::String(format!("{}: ", param_name)),
                                        kind: Some(InlayHintKind::PARAMETER),
                                        text_edits: None,
                                        tooltip: Some(InlayHintTooltip::String(
                                            format!("Parameter name: {}", param_name),
                                        )),
                                        padding_left: Some(false),
                                        padding_right: Some(false),
                                        data: None,
                                    });
                                }
                            }
                            current_pos += arg.len() + 2; // +2 for ", "
                        }
                    }
                }
            }
        }

        if hints.is_empty() {
            None
        } else {
            Some(hints)
        }
    }

    /// Extracts parameter names from a function signature description.
    fn extract_param_names(&self, description: &str) -> Option<Vec<String>> {
        // Parse: "fn name(param1: Type1, param2: Type2) -> ReturnType"
        let paren_start = description.find('(')?;
        let paren_end = description.find(')')?;
        let params_str = &description[paren_start + 1..paren_end];

        if params_str.trim().is_empty() {
            return Some(Vec::new());
        }

        let mut names = Vec::new();
        for param in params_str.split(',') {
            let param = param.trim();
            // Handle "name: Type" or just "Type"
            if let Some(colon_pos) = param.find(':') {
                let name = param[..colon_pos].trim();
                // Skip 'self' parameters
                if name != "self" && name != "&self" && name != "&mut self" {
                    names.push(name.to_string());
                }
            }
        }

        Some(names)
    }

    /// Splits function call arguments, handling nested parentheses.
    fn split_arguments<'a>(&self, args_str: &'a str) -> Vec<&'a str> {
        let mut args = Vec::new();
        let mut depth = 0;
        let mut start = 0;

        for (i, c) in args_str.char_indices() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(&args_str[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }

        // Don't forget the last argument
        if start < args_str.len() {
            args.push(&args_str[start..]);
        }

        args
    }

    /// Checks function parameters for hints (legacy version).
    #[allow(dead_code)]
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
                    // Requires integration with bloodc
                }
            }
        }

        None
    }

    /// Checks for missing effect annotations, using analysis to get real effect info.
    fn check_effect_annotation_with_analysis(
        &self,
        line: &str,
        line_num: u32,
        analysis: Option<&AnalysisResult>,
    ) -> Option<InlayHint> {
        let trimmed = line.trim();

        // Look for function definitions without effect annotations
        // Pattern: fn name(...) -> Type {
        // Should have: fn name(...) -> Type / Effects {
        if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") {
            // Check if there's already an effect annotation (contains " / ")
            if trimmed.contains(" / ") {
                return None;
            }

            // Extract function name
            let fn_start = if trimmed.starts_with("pub fn ") { 7 } else { 3 };
            let after_fn = &trimmed[fn_start..];
            let name_end = after_fn.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(after_fn.len());
            let fn_name = &after_fn[..name_end];

            // Try to find the function's effect annotation from analysis
            let effect_str = analysis.and_then(|a| {
                a.symbols.iter().find_map(|sym| {
                    if sym.name == fn_name && sym.kind == SymbolKind::FUNCTION {
                        // Extract effect from description: "fn name(...) -> Type / Effects"
                        self.extract_effect_from_description(&sym.description)
                    } else {
                        None
                    }
                })
            });

            // Look for the return type or opening brace to place the hint
            if let Some(arrow_pos) = trimmed.find("->") {
                let after_arrow = &trimmed[arrow_pos + 2..];
                if let Some(brace_pos) = after_arrow.find('{') {
                    let insert_pos = arrow_pos + 2 + brace_pos;
                    let col = line.find("fn ").unwrap() + insert_pos;

                    // Use real effect if available, otherwise "pure"
                    let has_effect = effect_str.is_some();
                    let effect_label = effect_str
                        .map(|e| format!("/ {}", e))
                        .unwrap_or_else(|| "/ pure".to_string());

                    let tooltip = if has_effect {
                        "Inferred effect annotation from type checking"
                    } else {
                        "Inferred effect annotation (type checking incomplete)"
                    };

                    return Some(InlayHint {
                        position: Position {
                            line: line_num,
                            character: col as u32,
                        },
                        label: InlayHintLabel::String(effect_label),
                        kind: Some(InlayHintKind::TYPE),
                        text_edits: None,
                        tooltip: Some(InlayHintTooltip::String(tooltip.to_string())),
                        padding_left: Some(true),
                        padding_right: Some(true),
                        data: None,
                    });
                }
            }
        }

        None
    }

    /// Extracts effect annotation from a function description.
    fn extract_effect_from_description(&self, description: &str) -> Option<String> {
        // Parse: "fn name(...) -> Type / Effects"
        if let Some(slash_pos) = description.rfind(" / ") {
            let effect = description[slash_pos + 3..].trim();
            if !effect.is_empty() && !effect.contains('(') { // Avoid picking up return type
                return Some(effect.to_string());
            }
        }
        // If no explicit effect annotation, function is implicitly pure
        None
    }

    /// Checks for missing effect annotations (legacy version).
    #[allow(dead_code)]
    fn check_effect_annotation(&self, line: &str, line_num: u32) -> Option<InlayHint> {
        let trimmed = line.trim();

        // Look for function definitions without effect annotations
        if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") {
            if !trimmed.contains(" / ") {
                if let Some(arrow_pos) = trimmed.find("->") {
                    let after_arrow = &trimmed[arrow_pos + 2..];
                    if let Some(brace_pos) = after_arrow.find('{') {
                        let insert_pos = arrow_pos + 2 + brace_pos;
                        let col = line.find("fn ").unwrap() + insert_pos;

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
