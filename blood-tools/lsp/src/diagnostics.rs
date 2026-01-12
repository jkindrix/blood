//! Diagnostic Engine
//!
//! Provides error and warning diagnostics for Blood source files by integrating
//! with the bloodc compiler for parsing and type checking.

use bloodc::{Parser, Span};
use tower_lsp::lsp_types::*;

use crate::document::Document;

/// The diagnostic engine that checks Blood source files.
pub struct DiagnosticEngine {
    /// Whether to enable all lints.
    enable_lints: bool,
}

impl DiagnosticEngine {
    /// Creates a new diagnostic engine.
    pub fn new() -> Self {
        Self { enable_lints: true }
    }

    /// Checks a document and returns diagnostics.
    pub fn check(&self, doc: &Document) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let text = doc.text();

        // Use bloodc parser for real diagnostics
        self.check_with_bloodc(&text, &mut diagnostics);

        // Add lint checks
        if self.enable_lints {
            self.check_lints(&text, &mut diagnostics);
        }

        diagnostics
    }

    /// Parse and type-check using bloodc.
    fn check_with_bloodc(&self, text: &str, diagnostics: &mut Vec<Diagnostic>) {
        let mut parser = Parser::new(text);

        match parser.parse_program() {
            Ok(program) => {
                // Parsing succeeded, try type checking
                let interner = parser.take_interner();
                if let Err(type_errors) = bloodc::typeck::check_program(&program, text, interner) {
                    for error in type_errors {
                        diagnostics.push(self.bloodc_diagnostic_to_lsp(&error, text));
                    }
                }
            }
            Err(parse_errors) => {
                // Report parse errors
                for error in parse_errors {
                    diagnostics.push(self.bloodc_diagnostic_to_lsp(&error, text));
                }
            }
        }
    }

    /// Convert a bloodc Diagnostic to an LSP Diagnostic.
    fn bloodc_diagnostic_to_lsp(
        &self,
        diag: &bloodc::Diagnostic,
        text: &str,
    ) -> Diagnostic {
        let range = self.span_to_range(&diag.span, text);

        let severity = match diag.kind {
            bloodc::DiagnosticKind::Error => DiagnosticSeverity::ERROR,
            bloodc::DiagnosticKind::Warning => DiagnosticSeverity::WARNING,
            bloodc::DiagnosticKind::Note => DiagnosticSeverity::INFORMATION,
            bloodc::DiagnosticKind::Help => DiagnosticSeverity::HINT,
        };

        let code = diag.code.clone().map(NumberOrString::String);

        let related_information = if diag.labels.is_empty() {
            None
        } else {
            Some(
                diag.labels
                    .iter()
                    .map(|label| DiagnosticRelatedInformation {
                        location: Location {
                            uri: Url::parse("file:///unknown").unwrap(),
                            range: self.span_to_range(&label.span, text),
                        },
                        message: label.message.clone(),
                    })
                    .collect(),
            )
        };

        Diagnostic {
            range,
            severity: Some(severity),
            code,
            source: Some("blood".to_string()),
            message: diag.message.clone(),
            related_information,
            ..Default::default()
        }
    }

    /// Convert a bloodc Span to an LSP Range.
    fn span_to_range(&self, span: &Span, text: &str) -> Range {
        let start = self.offset_to_position(span.start, text);
        let end = self.offset_to_position(span.end, text);
        Range { start, end }
    }

    /// Convert a byte offset to an LSP Position.
    fn offset_to_position(&self, offset: usize, text: &str) -> Position {
        let mut line = 0u32;
        let mut col = 0u32;
        let mut current = 0;

        for ch in text.chars() {
            if current >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            current += ch.len_utf8();
        }

        Position { line, character: col }
    }

    /// Performs lint checks.
    fn check_lints(&self, text: &str, diagnostics: &mut Vec<Diagnostic>) {
        for (line_num, line) in text.lines().enumerate() {
            // Warn about TODO comments
            if let Some(col) = line.find("TODO") {
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: line_num as u32,
                            character: col as u32,
                        },
                        end: Position {
                            line: line_num as u32,
                            character: (col + 4) as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::INFORMATION),
                    code: Some(NumberOrString::String("W0001".to_string())),
                    source: Some("blood".to_string()),
                    message: "TODO comment found".to_string(),
                    tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                    ..Default::default()
                });
            }

            // Warn about FIXME comments
            if let Some(col) = line.find("FIXME") {
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: line_num as u32,
                            character: col as u32,
                        },
                        end: Position {
                            line: line_num as u32,
                            character: (col + 5) as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("W0002".to_string())),
                    source: Some("blood".to_string()),
                    message: "FIXME comment found".to_string(),
                    ..Default::default()
                });
            }

            // Warn about trailing whitespace
            if line.ends_with(' ') || line.ends_with('\t') {
                let trimmed_len = line.trim_end().len();
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: line_num as u32,
                            character: trimmed_len as u32,
                        },
                        end: Position {
                            line: line_num as u32,
                            character: line.len() as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::HINT),
                    code: Some(NumberOrString::String("W0003".to_string())),
                    source: Some("blood".to_string()),
                    message: "Trailing whitespace".to_string(),
                    tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                    ..Default::default()
                });
            }
        }
    }
}

impl Default for DiagnosticEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Diagnostic severity levels for Blood.
pub mod severity {
    /// Error codes for Blood diagnostics.
    pub mod codes {
        // Syntax errors (E0xxx)
        pub const MISMATCHED_DELIMITER: &str = "E0001";
        pub const UNMATCHED_CLOSING: &str = "E0002";
        pub const UNCLOSED_DELIMITER: &str = "E0003";
        pub const UNKNOWN_KEYWORD: &str = "E0010";

        // Type errors (E1xxx)
        pub const TYPE_MISMATCH: &str = "E1001";
        pub const UNDEFINED_TYPE: &str = "E1002";
        pub const MISSING_TYPE_ANNOTATION: &str = "E1003";

        // Effect errors (E2xxx)
        pub const UNHANDLED_EFFECT: &str = "E2001";
        pub const EFFECT_MISMATCH: &str = "E2002";
        pub const MISSING_HANDLER: &str = "E2003";

        // Linearity errors (E3xxx)
        pub const USE_AFTER_MOVE: &str = "E3001";
        pub const DOUBLE_USE: &str = "E3002";
        pub const UNCONSUMED_LINEAR: &str = "E3003";

        // Warnings (W0xxx)
        pub const TODO_COMMENT: &str = "W0001";
        pub const FIXME_COMMENT: &str = "W0002";
        pub const TRAILING_WHITESPACE: &str = "W0003";
        pub const UNUSED_VARIABLE: &str = "W0010";
        pub const UNUSED_IMPORT: &str = "W0011";
    }
}
