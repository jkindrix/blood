//! Token Definitions and Tokenizer
//!
//! Provides tokenization for Blood source code.

use std::iter::Peekable;
use std::str::Chars;

/// Token kinds for Blood source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A keyword (fn, let, struct, etc.)
    Keyword,
    /// An identifier
    Identifier,
    /// A number literal
    Number,
    /// A string literal
    String,
    /// A character literal
    Char,
    /// An operator (+, -, *, etc.)
    Operator,
    /// Opening brace `{`
    OpenBrace,
    /// Closing brace `}`
    CloseBrace,
    /// Opening parenthesis `(`
    OpenParen,
    /// Closing parenthesis `)`
    CloseParen,
    /// Opening bracket `[`
    OpenBracket,
    /// Closing bracket `]`
    CloseBracket,
    /// Comma `,`
    Comma,
    /// Semicolon `;`
    Semicolon,
    /// Colon `:`
    Colon,
    /// Double colon `::`
    DoubleColon,
    /// Arrow `->`
    Arrow,
    /// Fat arrow `=>`
    FatArrow,
    /// Slash `/` (for effect annotations)
    Slash,
    /// Dot `.`
    Dot,
    /// Whitespace (spaces, tabs)
    Whitespace,
    /// Newline
    Newline,
    /// Comment (line or block)
    Comment,
    /// Unknown character
    Unknown,
}

/// A token in Blood source code.
#[derive(Debug, Clone)]
pub struct Token {
    /// The kind of token.
    pub kind: TokenKind,
    /// The token text.
    pub text: String,
    /// Start position in source.
    pub start: usize,
    /// End position in source.
    pub end: usize,
}

impl Token {
    /// Creates a new token.
    pub fn new(kind: TokenKind, text: String, start: usize, end: usize) -> Self {
        Self {
            kind,
            text,
            start,
            end,
        }
    }
}

/// Blood source code tokenizer.
pub struct Tokenizer<'a> {
    source: &'a str,
    chars: Peekable<Chars<'a>>,
    position: usize,
}

impl<'a> Tokenizer<'a> {
    /// Creates a new tokenizer for the given source.
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.chars().peekable(),
            position: 0,
        }
    }

    /// Returns the next character without advancing.
    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    /// Advances and returns the next character.
    fn advance(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        self.position += c.len_utf8();
        Some(c)
    }

    /// Advances while the predicate is true.
    fn advance_while<F>(&mut self, predicate: F) -> String
    where
        F: Fn(char) -> bool,
    {
        let mut result = String::new();
        while let Some(c) = self.peek() {
            if predicate(c) {
                result.push(c);
                self.advance();
            } else {
                break;
            }
        }
        result
    }

    /// Tokenizes an identifier or keyword.
    fn tokenize_identifier(&mut self, first: char) -> Token {
        let start = self.position - first.len_utf8();
        let mut text = first.to_string();
        text.push_str(&self.advance_while(|c| c.is_alphanumeric() || c == '_'));
        let end = self.position;

        let kind = if Self::is_keyword(&text) {
            TokenKind::Keyword
        } else {
            TokenKind::Identifier
        };

        Token::new(kind, text, start, end)
    }

    /// Checks if a string is a Blood keyword.
    fn is_keyword(s: &str) -> bool {
        matches!(
            s,
            "fn" | "let"
                | "mut"
                | "if"
                | "else"
                | "match"
                | "loop"
                | "while"
                | "for"
                | "in"
                | "break"
                | "continue"
                | "return"
                | "struct"
                | "enum"
                | "trait"
                | "impl"
                | "type"
                | "where"
                | "pub"
                | "mod"
                | "use"
                | "as"
                | "self"
                | "Self"
                | "super"
                | "crate"
                | "const"
                | "static"
                | "async"
                | "await"
                | "move"
                | "ref"
                | "true"
                | "false"
                | "effect"
                | "handler"
                | "perform"
                | "resume"
                | "handle"
                | "with"
                | "deep"
                | "shallow"
                | "pure"
                | "linear"
                | "op"
        )
    }

    /// Tokenizes a number literal.
    fn tokenize_number(&mut self, first: char) -> Token {
        let start = self.position - first.len_utf8();
        let mut text = first.to_string();

        // Handle hex, octal, binary prefixes
        if first == '0' {
            if let Some(prefix) = self.peek() {
                if prefix == 'x' || prefix == 'X' {
                    text.push(self.advance().unwrap());
                    text.push_str(&self.advance_while(|c| c.is_ascii_hexdigit() || c == '_'));
                } else if prefix == 'o' || prefix == 'O' {
                    text.push(self.advance().unwrap());
                    text.push_str(&self.advance_while(|c| ('0'..='7').contains(&c) || c == '_'));
                } else if prefix == 'b' || prefix == 'B' {
                    text.push(self.advance().unwrap());
                    text.push_str(&self.advance_while(|c| c == '0' || c == '1' || c == '_'));
                }
            }
        }

        // Integer part
        text.push_str(&self.advance_while(|c| c.is_ascii_digit() || c == '_'));

        // Decimal part
        if self.peek() == Some('.') {
            // Look ahead to distinguish from method calls
            let chars: Vec<char> = self.source[self.position..].chars().take(2).collect();
            if chars.len() >= 2 && chars[1].is_ascii_digit() {
                text.push(self.advance().unwrap()); // '.'
                text.push_str(&self.advance_while(|c| c.is_ascii_digit() || c == '_'));
            }
        }

        // Exponent part
        if let Some(e) = self.peek() {
            if e == 'e' || e == 'E' {
                text.push(self.advance().unwrap());
                if let Some(sign) = self.peek() {
                    if sign == '+' || sign == '-' {
                        text.push(self.advance().unwrap());
                    }
                }
                text.push_str(&self.advance_while(|c| c.is_ascii_digit() || c == '_'));
            }
        }

        // Type suffix
        text.push_str(&self.advance_while(|c| c.is_alphabetic()));

        let end = self.position;
        Token::new(TokenKind::Number, text, start, end)
    }

    /// Tokenizes a string literal.
    fn tokenize_string(&mut self) -> Token {
        let start = self.position - 1; // Already consumed opening quote
        let mut text = "\"".to_string();
        let mut escaped = false;

        while let Some(c) = self.advance() {
            text.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                break;
            }
        }

        let end = self.position;
        Token::new(TokenKind::String, text, start, end)
    }

    /// Tokenizes a character literal.
    fn tokenize_char(&mut self) -> Token {
        let start = self.position - 1;
        let mut text = "'".to_string();
        let mut escaped = false;

        while let Some(c) = self.advance() {
            text.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '\'' {
                break;
            }
        }

        let end = self.position;
        Token::new(TokenKind::Char, text, start, end)
    }

    /// Tokenizes a line comment.
    fn tokenize_line_comment(&mut self) -> Token {
        let start = self.position - 2; // Already consumed "//"
        let mut text = "//".to_string();
        text.push_str(&self.advance_while(|c| c != '\n'));
        let end = self.position;
        Token::new(TokenKind::Comment, text, start, end)
    }

    /// Tokenizes a block comment.
    fn tokenize_block_comment(&mut self) -> Token {
        let start = self.position - 2; // Already consumed "/*"
        let mut text = "/*".to_string();
        let mut depth = 1;

        while depth > 0 {
            match self.advance() {
                Some('*') => {
                    text.push('*');
                    if self.peek() == Some('/') {
                        text.push(self.advance().unwrap());
                        depth -= 1;
                    }
                }
                Some('/') => {
                    text.push('/');
                    if self.peek() == Some('*') {
                        text.push(self.advance().unwrap());
                        depth += 1;
                    }
                }
                Some(c) => text.push(c),
                None => break,
            }
        }

        let end = self.position;
        Token::new(TokenKind::Comment, text, start, end)
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.position;
        let c = self.advance()?;

        let token = match c {
            // Whitespace
            ' ' | '\t' => {
                let text = format!("{}{}", c, self.advance_while(|c| c == ' ' || c == '\t'));
                Token::new(TokenKind::Whitespace, text, start, self.position)
            }

            // Newline
            '\n' => Token::new(TokenKind::Newline, "\n".to_string(), start, self.position),
            '\r' => {
                if self.peek() == Some('\n') {
                    self.advance();
                    Token::new(TokenKind::Newline, "\r\n".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Newline, "\r".to_string(), start, self.position)
                }
            }

            // Identifiers and keywords
            c if c.is_alphabetic() || c == '_' => self.tokenize_identifier(c),

            // Numbers
            c if c.is_ascii_digit() => self.tokenize_number(c),

            // Strings
            '"' => self.tokenize_string(),

            // Characters
            '\'' => self.tokenize_char(),

            // Delimiters
            '{' => Token::new(TokenKind::OpenBrace, "{".to_string(), start, self.position),
            '}' => Token::new(TokenKind::CloseBrace, "}".to_string(), start, self.position),
            '(' => Token::new(TokenKind::OpenParen, "(".to_string(), start, self.position),
            ')' => Token::new(TokenKind::CloseParen, ")".to_string(), start, self.position),
            '[' => Token::new(TokenKind::OpenBracket, "[".to_string(), start, self.position),
            ']' => Token::new(TokenKind::CloseBracket, "]".to_string(), start, self.position),

            // Punctuation
            ',' => Token::new(TokenKind::Comma, ",".to_string(), start, self.position),
            ';' => Token::new(TokenKind::Semicolon, ";".to_string(), start, self.position),
            '.' => Token::new(TokenKind::Dot, ".".to_string(), start, self.position),

            // Colon
            ':' => {
                if self.peek() == Some(':') {
                    self.advance();
                    Token::new(TokenKind::DoubleColon, "::".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Colon, ":".to_string(), start, self.position)
                }
            }

            // Arrow and minus
            '-' => {
                if self.peek() == Some('>') {
                    self.advance();
                    Token::new(TokenKind::Arrow, "->".to_string(), start, self.position)
                } else if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "-=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "-".to_string(), start, self.position)
                }
            }

            // Fat arrow and equals
            '=' => {
                if self.peek() == Some('>') {
                    self.advance();
                    Token::new(TokenKind::FatArrow, "=>".to_string(), start, self.position)
                } else if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "==".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "=".to_string(), start, self.position)
                }
            }

            // Slash and comments
            '/' => {
                if self.peek() == Some('/') {
                    self.advance();
                    self.tokenize_line_comment()
                } else if self.peek() == Some('*') {
                    self.advance();
                    self.tokenize_block_comment()
                } else if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "/=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Slash, "/".to_string(), start, self.position)
                }
            }

            // Multi-character operators
            '+' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "+=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "+".to_string(), start, self.position)
                }
            }

            '*' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "*=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "*".to_string(), start, self.position)
                }
            }

            '%' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "%=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "%".to_string(), start, self.position)
                }
            }

            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "!=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "!".to_string(), start, self.position)
                }
            }

            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "<=".to_string(), start, self.position)
                } else if self.peek() == Some('<') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::new(TokenKind::Operator, "<<=".to_string(), start, self.position)
                    } else {
                        Token::new(TokenKind::Operator, "<<".to_string(), start, self.position)
                    }
                } else {
                    Token::new(TokenKind::Operator, "<".to_string(), start, self.position)
                }
            }

            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, ">=".to_string(), start, self.position)
                } else if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::new(TokenKind::Operator, ">>=".to_string(), start, self.position)
                    } else {
                        Token::new(TokenKind::Operator, ">>".to_string(), start, self.position)
                    }
                } else {
                    Token::new(TokenKind::Operator, ">".to_string(), start, self.position)
                }
            }

            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    Token::new(TokenKind::Operator, "&&".to_string(), start, self.position)
                } else if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "&=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "&".to_string(), start, self.position)
                }
            }

            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    Token::new(TokenKind::Operator, "||".to_string(), start, self.position)
                } else if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "|=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "|".to_string(), start, self.position)
                }
            }

            '^' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::new(TokenKind::Operator, "^=".to_string(), start, self.position)
                } else {
                    Token::new(TokenKind::Operator, "^".to_string(), start, self.position)
                }
            }

            '#' => Token::new(TokenKind::Operator, "#".to_string(), start, self.position),
            '@' => Token::new(TokenKind::Operator, "@".to_string(), start, self.position),
            '~' => Token::new(TokenKind::Operator, "~".to_string(), start, self.position),
            '?' => Token::new(TokenKind::Operator, "?".to_string(), start, self.position),

            // Unknown
            _ => Token::new(TokenKind::Unknown, c.to_string(), start, self.position),
        };

        Some(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_keywords() {
        let tokenizer = Tokenizer::new("fn let effect handler");
        let tokens: Vec<_> = tokenizer.collect();

        assert_eq!(tokens[0].kind, TokenKind::Keyword);
        assert_eq!(tokens[0].text, "fn");
    }

    #[test]
    fn test_tokenize_numbers() {
        let tokenizer = Tokenizer::new("42 3.14 0xFF 0b1010");
        let tokens: Vec<_> = tokenizer.filter(|t| t.kind == TokenKind::Number).collect();

        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].text, "42");
        assert_eq!(tokens[1].text, "3.14");
    }

    #[test]
    fn test_tokenize_effect_annotation() {
        let tokenizer = Tokenizer::new("fn foo() -> i32 / pure");
        let tokens: Vec<_> = tokenizer.collect();

        let slash = tokens.iter().find(|t| t.kind == TokenKind::Slash);
        assert!(slash.is_some());
    }
}
