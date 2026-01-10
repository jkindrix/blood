//! Diagnostic Engine
//!
//! Provides error and warning diagnostics for Blood source files.

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

        // TODO: Integrate with bloodc for real parsing and type checking
        // For now, provide basic syntactic checks

        self.check_balanced_delimiters(&text, &mut diagnostics);
        self.check_basic_syntax(&text, &mut diagnostics);

        if self.enable_lints {
            self.check_lints(&text, &mut diagnostics);
        }

        diagnostics
    }

    /// Checks for balanced delimiters (braces, parens, brackets).
    fn check_balanced_delimiters(&self, text: &str, diagnostics: &mut Vec<Diagnostic>) {
        let mut stack: Vec<(char, usize, usize)> = Vec::new(); // (char, line, col)
        let mut line = 0u32;
        let mut col = 0u32;
        let mut in_string = false;
        let mut in_char = false;
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        let mut prev_char = '\0';

        for c in text.chars() {
            // Handle newlines
            if c == '\n' {
                line += 1;
                col = 0;
                in_line_comment = false;
                prev_char = c;
                continue;
            }

            // Handle comments
            if !in_string && !in_char {
                if prev_char == '/' && c == '/' {
                    in_line_comment = true;
                    col += 1;
                    prev_char = c;
                    continue;
                }
                if prev_char == '/' && c == '*' {
                    in_block_comment = true;
                    col += 1;
                    prev_char = c;
                    continue;
                }
                if prev_char == '*' && c == '/' && in_block_comment {
                    in_block_comment = false;
                    col += 1;
                    prev_char = c;
                    continue;
                }
            }

            if in_line_comment || in_block_comment {
                col += 1;
                prev_char = c;
                continue;
            }

            // Handle strings
            if c == '"' && prev_char != '\\' && !in_char {
                in_string = !in_string;
            }
            if c == '\'' && prev_char != '\\' && !in_string {
                in_char = !in_char;
            }

            if !in_string && !in_char {
                match c {
                    '(' | '[' | '{' => {
                        stack.push((c, line as usize, col as usize));
                    }
                    ')' | ']' | '}' => {
                        let expected = match c {
                            ')' => '(',
                            ']' => '[',
                            '}' => '{',
                            _ => unreachable!(),
                        };
                        match stack.pop() {
                            Some((open, _, _)) if open == expected => {}
                            Some((open, open_line, open_col)) => {
                                diagnostics.push(Diagnostic {
                                    range: Range {
                                        start: Position {
                                            line,
                                            character: col,
                                        },
                                        end: Position {
                                            line,
                                            character: col + 1,
                                        },
                                    },
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: Some(NumberOrString::String("E0001".to_string())),
                                    source: Some("blood".to_string()),
                                    message: format!(
                                        "Mismatched delimiter: expected '{}' to close '{}' at {}:{}",
                                        match open {
                                            '(' => ')',
                                            '[' => ']',
                                            '{' => '}',
                                            _ => '?',
                                        },
                                        open,
                                        open_line + 1,
                                        open_col + 1
                                    ),
                                    ..Default::default()
                                });
                            }
                            None => {
                                diagnostics.push(Diagnostic {
                                    range: Range {
                                        start: Position {
                                            line,
                                            character: col,
                                        },
                                        end: Position {
                                            line,
                                            character: col + 1,
                                        },
                                    },
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: Some(NumberOrString::String("E0002".to_string())),
                                    source: Some("blood".to_string()),
                                    message: format!("Unmatched closing delimiter '{}'", c),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }

            col += 1;
            prev_char = c;
        }

        // Report unclosed delimiters
        for (open, open_line, open_col) in stack {
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position {
                        line: open_line as u32,
                        character: open_col as u32,
                    },
                    end: Position {
                        line: open_line as u32,
                        character: open_col as u32 + 1,
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("E0003".to_string())),
                source: Some("blood".to_string()),
                message: format!("Unclosed delimiter '{}'", open),
                ..Default::default()
            });
        }
    }

    /// Performs basic syntax checks.
    fn check_basic_syntax(&self, text: &str, diagnostics: &mut Vec<Diagnostic>) {
        for (line_num, line) in text.lines().enumerate() {
            let trimmed = line.trim();

            // Check for common typos in keywords
            let typos = [
                ("funciton", "fn"),
                ("fucntion", "fn"),
                ("funtion", "fn"),
                ("stuct", "struct"),
                ("strcut", "struct"),
                ("imlp", "impl"),
                ("implment", "impl"),
                ("hadnler", "handler"),
                ("hander", "handler"),
                ("efect", "effect"),
                ("peform", "perform"),
                ("preform", "perform"),
                ("reusme", "resume"),
                ("resum", "resume"),
            ];

            for (typo, correct) in typos {
                if let Some(col) = trimmed.find(typo) {
                    // Make sure it's a word boundary
                    let before_ok = col == 0
                        || !trimmed.chars().nth(col - 1).unwrap_or(' ').is_alphanumeric();
                    let after_ok = col + typo.len() >= trimmed.len()
                        || !trimmed
                            .chars()
                            .nth(col + typo.len())
                            .unwrap_or(' ')
                            .is_alphanumeric();

                    if before_ok && after_ok {
                        let actual_col = line.find(typo).unwrap_or(0);
                        diagnostics.push(Diagnostic {
                            range: Range {
                                start: Position {
                                    line: line_num as u32,
                                    character: actual_col as u32,
                                },
                                end: Position {
                                    line: line_num as u32,
                                    character: (actual_col + typo.len()) as u32,
                                },
                            },
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: Some(NumberOrString::String("E0010".to_string())),
                            source: Some("blood".to_string()),
                            message: format!("Unknown keyword '{}', did you mean '{}'?", typo, correct),
                            ..Default::default()
                        });
                    }
                }
            }
        }
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
