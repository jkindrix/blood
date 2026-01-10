//! Core Formatter Implementation
//!
//! The main formatting engine for Blood source code.

use crate::config::Config;
use crate::printer::Printer;
use crate::tokens::{Token, TokenKind, Tokenizer};
use crate::{FormatError, FormatResult};

/// The Blood code formatter.
pub struct Formatter {
    config: Config,
}

impl Formatter {
    /// Creates a new formatter with the given configuration.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Formats Blood source code.
    pub fn format(&self, source: &str) -> FormatResult<String> {
        let tokens = self.tokenize(source)?;
        let formatted = self.format_tokens(&tokens)?;
        Ok(formatted)
    }

    /// Tokenizes the source code.
    fn tokenize(&self, source: &str) -> FormatResult<Vec<Token>> {
        let tokenizer = Tokenizer::new(source);
        Ok(tokenizer.collect())
    }

    /// Formats a sequence of tokens.
    fn format_tokens(&self, tokens: &[Token]) -> FormatResult<String> {
        let mut printer = Printer::new(&self.config);

        let mut i = 0;
        while i < tokens.len() {
            let token = &tokens[i];

            match token.kind {
                TokenKind::Newline => {
                    printer.newline();
                }

                TokenKind::Whitespace => {
                    // Handled by spacing rules
                }

                TokenKind::Comment => {
                    // Normalize comment spacing
                    let text = &token.text;
                    if text.starts_with("//") && !text.starts_with("// ") && text.len() > 2 {
                        let rest = &text[2..];
                        if !rest.starts_with('/') && !rest.starts_with('!') {
                            printer.write("// ");
                            printer.write(rest.trim_start());
                        } else {
                            printer.write(text);
                        }
                    } else {
                        printer.write(text);
                    }
                }

                TokenKind::Keyword => {
                    self.format_keyword(&mut printer, token, tokens, &mut i)?;
                }

                TokenKind::Identifier => {
                    printer.write(&token.text);
                    self.maybe_space_after(&mut printer, tokens, i);
                }

                TokenKind::Number | TokenKind::String | TokenKind::Char => {
                    printer.write(&token.text);
                    self.maybe_space_after(&mut printer, tokens, i);
                }

                TokenKind::Operator => {
                    self.format_operator(&mut printer, token, tokens, i);
                }

                TokenKind::OpenBrace => {
                    if self.config.style.brace_same_line {
                        printer.write(" {");
                    } else {
                        printer.newline();
                        printer.write("{");
                    }
                    printer.increase_indent();
                    printer.newline();
                }

                TokenKind::CloseBrace => {
                    printer.decrease_indent();
                    printer.newline_if_needed();
                    printer.write("}");
                }

                TokenKind::OpenParen => {
                    printer.write("(");
                }

                TokenKind::CloseParen => {
                    printer.write(")");
                    self.maybe_space_after(&mut printer, tokens, i);
                }

                TokenKind::OpenBracket => {
                    printer.write("[");
                }

                TokenKind::CloseBracket => {
                    printer.write("]");
                    self.maybe_space_after(&mut printer, tokens, i);
                }

                TokenKind::Comma => {
                    printer.write(",");
                    if !self.next_is(&tokens, i, TokenKind::Newline)
                        && !self.next_is(&tokens, i, TokenKind::CloseParen)
                        && !self.next_is(&tokens, i, TokenKind::CloseBracket)
                        && !self.next_is(&tokens, i, TokenKind::CloseBrace)
                    {
                        printer.write(" ");
                    }
                }

                TokenKind::Semicolon => {
                    printer.write(";");
                }

                TokenKind::Colon => {
                    if self.config.style.space_before_colon {
                        printer.write(" ");
                    }
                    printer.write(":");
                    if self.config.style.space_after_colon {
                        printer.write(" ");
                    }
                }

                TokenKind::DoubleColon => {
                    printer.write("::");
                }

                TokenKind::Arrow => {
                    printer.write(" -> ");
                }

                TokenKind::FatArrow => {
                    printer.write(" => ");
                }

                TokenKind::Slash => {
                    // Effect annotation
                    if self.config.effects.space_before_slash {
                        printer.write(" ");
                    }
                    printer.write("/");
                    if self.config.effects.space_after_slash {
                        printer.write(" ");
                    }
                }

                TokenKind::Dot => {
                    printer.write(".");
                }

                TokenKind::Unknown => {
                    printer.write(&token.text);
                }
            }

            i += 1;
        }

        Ok(printer.finish())
    }

    /// Formats a keyword token.
    fn format_keyword(
        &self,
        printer: &mut Printer,
        token: &Token,
        tokens: &[Token],
        i: &mut usize,
    ) -> FormatResult<()> {
        let keyword = &token.text;

        match keyword.as_str() {
            "fn" | "pub" | "let" | "mut" | "const" | "static" | "type" | "struct" | "enum"
            | "trait" | "impl" | "mod" | "use" | "effect" | "handler" | "deep" | "shallow" => {
                printer.write(keyword);
                printer.write(" ");
            }

            "if" | "else" | "match" | "while" | "for" | "loop" | "handle" | "with" => {
                printer.write(keyword);
                printer.write(" ");
            }

            "return" | "break" | "continue" | "perform" | "resume" => {
                printer.write(keyword);
                // Only add space if there's something after
                if !self.next_is(tokens, *i, TokenKind::Semicolon)
                    && !self.next_is(tokens, *i, TokenKind::Newline)
                    && !self.next_is(tokens, *i, TokenKind::CloseBrace)
                {
                    printer.write(" ");
                }
            }

            "in" | "as" | "where" => {
                printer.write(" ");
                printer.write(keyword);
                printer.write(" ");
            }

            "pure" | "linear" => {
                printer.write(keyword);
                self.maybe_space_after(printer, tokens, *i);
            }

            _ => {
                printer.write(keyword);
                self.maybe_space_after(printer, tokens, *i);
            }
        }

        Ok(())
    }

    /// Formats an operator token.
    fn format_operator(&self, printer: &mut Printer, token: &Token, tokens: &[Token], i: usize) {
        let op = &token.text;

        // Binary operators that need spacing
        let binary_ops = [
            "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||", "&", "|", "^",
            "<<", ">>", "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=",
        ];

        if binary_ops.contains(&op.as_str()) {
            // Check if this is a unary operator (at start or after open delimiter)
            let is_unary = i == 0
                || self.prev_is(tokens, i, TokenKind::OpenParen)
                || self.prev_is(tokens, i, TokenKind::OpenBracket)
                || self.prev_is(tokens, i, TokenKind::Comma)
                || self.prev_is(tokens, i, TokenKind::Operator);

            if is_unary && (op == "-" || op == "+" || op == "*" || op == "&" || op == "!") {
                printer.write(op);
            } else {
                printer.write(" ");
                printer.write(op);
                printer.write(" ");
            }
        } else if op == "!" {
            printer.write(op);
        } else {
            printer.write(op);
        }
    }

    /// Checks if the next significant token is of the given kind.
    fn next_is(&self, tokens: &[Token], i: usize, kind: TokenKind) -> bool {
        tokens
            .get(i + 1)
            .map(|t| t.kind == kind || (t.kind == TokenKind::Whitespace && self.next_is(tokens, i + 1, kind)))
            .unwrap_or(false)
    }

    /// Checks if the previous significant token is of the given kind.
    fn prev_is(&self, tokens: &[Token], i: usize, kind: TokenKind) -> bool {
        if i == 0 {
            return false;
        }
        tokens
            .get(i - 1)
            .map(|t| {
                t.kind == kind
                    || (t.kind == TokenKind::Whitespace && i > 1 && self.prev_is(tokens, i - 1, kind))
            })
            .unwrap_or(false)
    }

    /// Adds a space after the current token if appropriate.
    fn maybe_space_after(&self, printer: &mut Printer, tokens: &[Token], i: usize) {
        if let Some(next) = tokens.get(i + 1) {
            match next.kind {
                TokenKind::OpenParen
                | TokenKind::OpenBracket
                | TokenKind::Comma
                | TokenKind::Semicolon
                | TokenKind::Colon
                | TokenKind::DoubleColon
                | TokenKind::Dot
                | TokenKind::CloseParen
                | TokenKind::CloseBracket
                | TokenKind::CloseBrace
                | TokenKind::Newline => {}

                _ => {
                    if next.kind != TokenKind::Whitespace {
                        // Don't add space if next token will add it
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_formatting() {
        let formatter = Formatter::new(Config::default());
        let source = "fn main(){\nlet x=1+2\n}";
        let result = formatter.format(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_effect_annotation() {
        let formatter = Formatter::new(Config::default());
        let source = "fn foo()->i32/pure{42}";
        let result = formatter.format(source);
        assert!(result.is_ok());
    }
}
