//! Parser for Blood.
//!
//! This module implements a hand-written recursive descent parser with
//! Pratt parsing for expressions. The parser produces an AST according
//! to the grammar defined in GRAMMAR.md.
//!
//! # Parser Architecture
//!
//! The parser is organized into several submodules:
//!
//! - `expr` - Expression parsing with Pratt parsing for precedence
//! - `item` - Declaration/item parsing (functions, structs, etc.)
//! - `pattern` - Pattern parsing for match arms and let bindings
//! - `types` - Type expression parsing
//!
//! # Example
//!
//! ```rust
//! use bloodc::Parser;
//! use bloodc::ast::Declaration;
//!
//! let source = "fn add(a: Int, b: Int) -> Int { a + b }";
//! let mut parser = Parser::new(source);
//!
//! let program = parser.parse_program().expect("parse failed");
//! assert_eq!(program.declarations.len(), 1);
//!
//! match &program.declarations[0] {
//!     Declaration::Function(f) => {
//!         assert_eq!(f.params.len(), 2);
//!     }
//!     _ => panic!("expected function"),
//! }
//! ```
//!
//! # Error Recovery
//!
//! The parser implements panic-mode error recovery. When an error is
//! encountered, it enters "panic mode" and skips tokens until it finds
//! a synchronization point (like a semicolon or a keyword that starts
//! a new declaration).

mod expr;
mod item;
mod pattern;
mod types;

#[cfg(test)]
mod tests;

use crate::ast::*;
use crate::diagnostics::{Diagnostic, ErrorCode};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::span::{Span, Spanned};
use string_interner::DefaultStringInterner;

pub use self::expr::Precedence;

/// Format a list of expected items in natural English.
///
/// - Empty list: ""
/// - Single item: "X"
/// - Two items: "X or Y"
/// - Multiple items: "X, Y, or Z"
fn format_expected_list(items: &[&str]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].to_string(),
        2 => format!("{} or {}", items[0], items[1]),
        _ => {
            // Safe: we're in the _ arm so items.len() >= 3, meaning split_last() returns Some
            let (last, rest) = items.split_last()
                .expect("BUG: items.len() >= 3 but split_last() returned None");
            format!("{}, or {}", rest.join(", "), last)
        }
    }
}

/// The Blood parser.
pub struct Parser<'src> {
    /// The lexer producing tokens.
    lexer: Lexer<'src>,
    /// The source text (for extracting lexemes).
    source: &'src str,
    /// String interner for symbols.
    interner: DefaultStringInterner,
    /// Current token.
    current: Token,
    /// Previous token.
    previous: Token,
    /// Accumulated errors.
    errors: Vec<Diagnostic>,
    /// Whether we're in panic mode (error recovery).
    panic_mode: bool,
}

impl<'src> Parser<'src> {
    /// Create a new parser for the given source.
    pub fn new(source: &'src str) -> Self {
        let mut lexer = Lexer::new(source);
        let current = lexer.next().unwrap_or(Token::dummy(TokenKind::Error));

        Self {
            lexer,
            source,
            interner: DefaultStringInterner::new(),
            current,
            previous: Token::dummy(TokenKind::Error),
            errors: Vec::new(),
            panic_mode: false,
        }
    }

    /// Parse a complete program.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_program(&mut self) -> Result<Program, Vec<Diagnostic>> {
        let start = self.current.span;
        let mut module = None;
        let mut imports = Vec::new();
        let mut declarations = Vec::new();

        // Parse optional module declaration
        if self.check(TokenKind::Module) {
            module = Some(self.parse_module_decl());
        }

        // Parse imports
        while self.check(TokenKind::Use) {
            imports.push(self.parse_import());
        }

        // Parse declarations
        while !self.is_at_end() {
            if let Some(decl) = self.parse_declaration() {
                declarations.push(decl);
            }
        }

        if self.errors.is_empty() {
            let end = self.previous.span;
            Ok(Program {
                module,
                imports,
                declarations,
                span: start.merge(end),
            })
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    // ============================================================
    // Token handling
    // ============================================================

    /// Check if the current token matches the given kind.
    fn check(&self, kind: TokenKind) -> bool {
        self.current.kind == kind
    }

    /// Check if we've reached the end of input.
    fn is_at_end(&self) -> bool {
        self.current.kind == TokenKind::Eof
    }

    /// Advance to the next token, returning the previous.
    fn advance(&mut self) -> Token {
        self.previous = self.current.clone();

        // Don't advance past EOF
        if self.current.kind == TokenKind::Eof {
            return self.previous.clone();
        }

        loop {
            self.current = self.lexer.next().unwrap_or_else(|| {
                Token::new(
                    TokenKind::Eof,
                    Span::new(self.source.len(), self.source.len(), 0, 0),
                )
            });

            // Accept any non-error token, or EOF
            if self.current.kind != TokenKind::Error || self.current.kind == TokenKind::Eof {
                break;
            }

            // Report lexer errors
            self.error_at_current("unexpected character", ErrorCode::UnexpectedCharacter);
        }
        self.previous.clone()
    }

    /// Consume a token of the expected kind, or error.
    fn expect(&mut self, kind: TokenKind) -> Option<Token> {
        if self.check(kind) {
            Some(self.advance())
        } else {
            self.error_expected(kind.description());
            None
        }
    }

    /// Try to consume a token of the expected kind.
    fn try_consume(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Get the text of a span.
    fn text(&self, span: &Span) -> &'src str {
        &self.source[span.start..span.end]
    }

    /// Intern a string and return its symbol.
    fn intern(&mut self, s: &str) -> Symbol {
        self.interner.get_or_intern(s)
    }

    /// Take ownership of the string interner.
    pub fn take_interner(&mut self) -> DefaultStringInterner {
        std::mem::take(&mut self.interner)
    }

    /// Create a spanned symbol from the previous token.
    fn spanned_symbol(&mut self) -> Spanned<Symbol> {
        let text = self.text(&self.previous.span);
        let symbol = self.intern(text);
        Spanned::new(symbol, self.previous.span)
    }

    // ============================================================
    // Error handling
    // ============================================================

    fn error_at_current(&mut self, message: &str, code: ErrorCode) {
        self.error_at(self.current.span, message, code);
    }

    #[allow(dead_code)]
    fn error_at_previous(&mut self, message: &str, code: ErrorCode) {
        self.error_at(self.previous.span, message, code);
    }

    fn error_at(&mut self, span: Span, message: &str, code: ErrorCode) {
        if self.panic_mode {
            return;
        }
        self.panic_mode = true;
        self.errors
            .push(Diagnostic::error(message, span).with_error_code(code));
    }

    fn error_expected(&mut self, expected: &str) {
        let found = self.current.kind.description();
        let message = format!("expected {}, found {}", expected, found);
        let code = if self.is_at_end() {
            ErrorCode::UnexpectedEof
        } else {
            ErrorCode::UnexpectedToken
        };
        self.error_at_current(&message, code);
    }

    /// Report an error expecting one of several things.
    fn error_expected_one_of(&mut self, expected: &[&str]) {
        let found = self.current.kind.description();
        let expected_msg = format_expected_list(expected);
        let message = format!("expected {}, found {}", expected_msg, found);
        let code = if self.is_at_end() {
            ErrorCode::UnexpectedEof
        } else {
            ErrorCode::UnexpectedToken
        };
        self.error_at_current(&message, code);
    }

    /// Synchronize after an error by skipping to a recovery point.
    fn synchronize(&mut self) {
        self.panic_mode = false;

        while !self.is_at_end() {
            if self.previous.kind == TokenKind::Semi {
                return;
            }

            match self.current.kind {
                TokenKind::Fn
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Effect
                | TokenKind::Handler
                | TokenKind::Trait
                | TokenKind::Impl
                | TokenKind::Type
                | TokenKind::Const
                | TokenKind::Static
                | TokenKind::Pub
                | TokenKind::Use
                | TokenKind::Module => return,
                _ => {}
            }

            self.advance();
        }
    }

    /// Skip tokens until we find a closing delimiter, handling nested delimiters.
    /// Returns true if the closing delimiter was found.
    #[allow(dead_code)]
    fn skip_to_closing(&mut self, closing: TokenKind) -> bool {
        let opening = match closing {
            TokenKind::RParen => TokenKind::LParen,
            TokenKind::RBracket => TokenKind::LBracket,
            TokenKind::RBrace => TokenKind::LBrace,
            _ => return false,
        };

        let mut depth = 1;
        while !self.is_at_end() && depth > 0 {
            if self.current.kind == opening {
                depth += 1;
            } else if self.current.kind == closing {
                depth -= 1;
                if depth == 0 {
                    return true;
                }
            }
            self.advance();
        }
        false
    }

    /// Synchronize within a statement/expression context.
    /// Skips to semicolon, comma, or closing delimiter.
    #[allow(dead_code)]
    fn synchronize_local(&mut self) {
        self.panic_mode = false;

        while !self.is_at_end() {
            match self.current.kind {
                // Statement terminators
                TokenKind::Semi | TokenKind::Comma => {
                    return;
                }
                // Closing delimiters
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    return;
                }
                // Start of new statement
                TokenKind::Let | TokenKind::Return | TokenKind::If | TokenKind::While
                | TokenKind::For | TokenKind::Loop | TokenKind::Break | TokenKind::Continue
                | TokenKind::Match => {
                    return;
                }
                _ => {}
            }
            self.advance();
        }
    }

    // ============================================================
    // Module and Import parsing
    // ============================================================

    fn parse_module_decl(&mut self) -> ModuleDecl {
        let start = self.current.span;
        self.advance(); // consume 'module'

        let path = self.parse_module_path();
        self.expect(TokenKind::Semi);

        ModuleDecl {
            path,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_module_path(&mut self) -> ModulePath {
        let start = self.current.span;
        let mut segments = Vec::new();

        // First segment
        if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            segments.push(self.spanned_symbol());
        } else {
            self.error_expected("identifier");
        }

        // Additional segments
        while self.try_consume(TokenKind::Dot) {
            if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
                self.advance();
                segments.push(self.spanned_symbol());
            } else {
                self.error_expected("identifier");
                break;
            }
        }

        ModulePath {
            segments,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_import(&mut self) -> Import {
        let start = self.current.span;
        self.advance(); // consume 'use'

        let path = self.parse_module_path();

        // Check for `::` for grouped imports or single item imports
        if self.try_consume(TokenKind::ColonColon) {
            if self.try_consume(TokenKind::Star) {
                // Glob import
                self.expect(TokenKind::Semi);
                Import::Glob {
                    path,
                    span: start.merge(self.previous.span),
                }
            } else if self.try_consume(TokenKind::LBrace) {
                // Group import
                let mut items = Vec::new();

                loop {
                    if self.check(TokenKind::RBrace) {
                        break;
                    }

                    if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
                        self.advance();
                        let name = self.spanned_symbol();
                        let alias = if self.try_consume(TokenKind::As) {
                            self.expect(TokenKind::Ident);
                            Some(self.spanned_symbol())
                        } else {
                            None
                        };
                        items.push(ImportItem { name, alias });
                    } else {
                        self.error_expected("identifier");
                        break;
                    }

                    if !self.try_consume(TokenKind::Comma) {
                        break;
                    }
                }

                self.expect(TokenKind::RBrace);
                self.expect(TokenKind::Semi);

                Import::Group {
                    path,
                    items,
                    span: start.merge(self.previous.span),
                }
            } else if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
                // Single item import: use path::item;
                self.advance();
                let name = self.spanned_symbol();
                let alias = if self.try_consume(TokenKind::As) {
                    self.expect(TokenKind::Ident);
                    Some(self.spanned_symbol())
                } else {
                    None
                };
                self.expect(TokenKind::Semi);

                Import::Group {
                    path,
                    items: vec![ImportItem { name, alias }],
                    span: start.merge(self.previous.span),
                }
            } else {
                self.error_expected("`*`, `{`, or identifier");
                self.expect(TokenKind::Semi);
                Import::Simple {
                    path,
                    alias: None,
                    span: start.merge(self.previous.span),
                }
            }
        } else {
            // Simple import with optional alias
            let alias = if self.try_consume(TokenKind::As) {
                self.expect(TokenKind::Ident);
                Some(self.spanned_symbol())
            } else {
                None
            };

            self.expect(TokenKind::Semi);

            Import::Simple {
                path,
                alias,
                span: start.merge(self.previous.span),
            }
        }
    }

    // ============================================================
    // Visibility
    // ============================================================

    fn parse_visibility(&mut self) -> Visibility {
        if !self.try_consume(TokenKind::Pub) {
            return Visibility::Private;
        }

        if self.try_consume(TokenKind::LParen) {
            let vis = if self.try_consume(TokenKind::Crate) {
                Visibility::PublicCrate
            } else if self.try_consume(TokenKind::Super) {
                Visibility::PublicSuper
            } else if self.try_consume(TokenKind::SelfLower) {
                Visibility::PublicSelf
            } else {
                self.error_expected("`crate`, `super`, or `self`");
                Visibility::Public
            };
            self.expect(TokenKind::RParen);
            vis
        } else {
            Visibility::Public
        }
    }

    // ============================================================
    // Attributes
    // ============================================================

    fn parse_attributes(&mut self) -> Vec<Attribute> {
        let mut attrs = Vec::new();

        while self.check(TokenKind::Hash) {
            attrs.push(self.parse_attribute());
        }

        attrs
    }

    fn parse_attribute(&mut self) -> Attribute {
        let start = self.current.span;
        self.advance(); // consume '#'

        self.expect(TokenKind::LBracket);

        let mut path = Vec::new();

        // Parse attribute path
        if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            path.push(self.spanned_symbol());

            while self.try_consume(TokenKind::ColonColon) {
                if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
                    self.advance();
                    path.push(self.spanned_symbol());
                } else {
                    self.error_expected("identifier");
                    break;
                }
            }
        } else {
            self.error_expected("identifier");
        }

        // Parse optional arguments
        let args = if self.try_consume(TokenKind::Eq) {
            Some(AttributeArgs::Eq(self.parse_literal()))
        } else if self.try_consume(TokenKind::LParen) {
            let mut args = Vec::new();

            loop {
                if self.check(TokenKind::RParen) {
                    break;
                }

                let arg = if self.check(TokenKind::Ident) {
                    self.advance();
                    let name = self.spanned_symbol();
                    if self.try_consume(TokenKind::Eq) {
                        AttributeArg::KeyValue(name, self.parse_literal())
                    } else {
                        AttributeArg::Ident(name)
                    }
                } else {
                    AttributeArg::Literal(self.parse_literal())
                };

                args.push(arg);

                if !self.try_consume(TokenKind::Comma) {
                    break;
                }
            }

            self.expect(TokenKind::RParen);
            Some(AttributeArgs::List(args))
        } else {
            None
        };

        self.expect(TokenKind::RBracket);

        Attribute {
            path,
            args,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Literal parsing
    // ============================================================

    fn parse_literal(&mut self) -> Literal {
        let span = self.current.span;
        let text = self.text(&span);

        let kind = match self.current.kind {
            TokenKind::IntLit => {
                let (value, suffix) = self.parse_int_literal(text, span);
                LiteralKind::Int { value, suffix }
            }
            TokenKind::FloatLit => {
                let (value, suffix) = self.parse_float_literal(text, span);
                LiteralKind::Float { value: value.into(), suffix }
            }
            TokenKind::StringLit => {
                let s = self.parse_string_literal(text);
                LiteralKind::String(s)
            }
            TokenKind::CharLit => {
                let c = self.parse_char_literal(text);
                LiteralKind::Char(c)
            }
            TokenKind::True => LiteralKind::Bool(true),
            TokenKind::False => LiteralKind::Bool(false),
            _ => {
                self.error_expected("literal");
                LiteralKind::Int {
                    value: 0,
                    suffix: None,
                }
            }
        };

        self.advance();
        Literal { kind, span }
    }

    fn parse_int_literal(&mut self, text: &str, span: Span) -> (u128, Option<IntSuffix>) {
        let text = text.replace('_', "");

        // Check for suffix
        let (num_text, suffix) = if let Some(pos) = text.find(|c: char| c.is_alphabetic() && c != 'x' && c != 'o' && c != 'b' && c != 'a' && c != 'c' && c != 'd' && c != 'e' && c != 'f' && c != 'A' && c != 'B' && c != 'C' && c != 'D' && c != 'E' && c != 'F') {
            let (n, s) = text.split_at(pos);
            let suffix = match s {
                "i8" => Some(IntSuffix::I8),
                "i16" => Some(IntSuffix::I16),
                "i32" => Some(IntSuffix::I32),
                "i64" => Some(IntSuffix::I64),
                "i128" => Some(IntSuffix::I128),
                "u8" => Some(IntSuffix::U8),
                "u16" => Some(IntSuffix::U16),
                "u32" => Some(IntSuffix::U32),
                "u64" => Some(IntSuffix::U64),
                "u128" => Some(IntSuffix::U128),
                _ => {
                    self.error_at(span, &format!("invalid integer suffix '{}'", s), ErrorCode::InvalidInteger);
                    None
                }
            };
            (n, suffix)
        } else {
            (text.as_str(), None)
        };

        let value = if num_text.starts_with("0x") || num_text.starts_with("0X") {
            match u128::from_str_radix(&num_text[2..], 16) {
                Ok(v) => v,
                Err(_) => {
                    self.error_at(span, "invalid hexadecimal integer literal", ErrorCode::InvalidInteger);
                    0
                }
            }
        } else if num_text.starts_with("0o") || num_text.starts_with("0O") {
            match u128::from_str_radix(&num_text[2..], 8) {
                Ok(v) => v,
                Err(_) => {
                    self.error_at(span, "invalid octal integer literal", ErrorCode::InvalidInteger);
                    0
                }
            }
        } else if num_text.starts_with("0b") || num_text.starts_with("0B") {
            match u128::from_str_radix(&num_text[2..], 2) {
                Ok(v) => v,
                Err(_) => {
                    self.error_at(span, "invalid binary integer literal", ErrorCode::InvalidInteger);
                    0
                }
            }
        } else {
            match num_text.parse() {
                Ok(v) => v,
                Err(_) => {
                    self.error_at(span, "invalid decimal integer literal", ErrorCode::InvalidInteger);
                    0
                }
            }
        };

        (value, suffix)
    }

    fn parse_float_literal(&mut self, text: &str, span: Span) -> (f64, Option<FloatSuffix>) {
        let text = text.replace('_', "");

        let (num_text, suffix) = if text.ends_with("f32") {
            (&text[..text.len() - 3], Some(FloatSuffix::F32))
        } else if text.ends_with("f64") {
            (&text[..text.len() - 3], Some(FloatSuffix::F64))
        } else {
            (text.as_str(), None)
        };

        let value = match num_text.parse() {
            Ok(v) => v,
            Err(_) => {
                self.error_at(span, "invalid floating-point literal", ErrorCode::InvalidFloat);
                0.0
            }
        };
        (value, suffix)
    }

    fn parse_string_literal(&self, text: &str) -> String {
        // Remove quotes and process escape sequences
        let inner = &text[1..text.len() - 1];
        let mut result = String::new();
        let mut chars = inner.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('r') => result.push('\r'),
                    Some('t') => result.push('\t'),
                    Some('\\') => result.push('\\'),
                    Some('\'') => result.push('\''),
                    Some('"') => result.push('"'),
                    Some('0') => result.push('\0'),
                    Some('x') => {
                        // Hex escape \xNN
                        let mut hex = String::new();
                        for _ in 0..2 {
                            if let Some(h) = chars.next() {
                                hex.push(h);
                            }
                        }
                        if let Ok(n) = u8::from_str_radix(&hex, 16) {
                            result.push(n as char);
                        }
                    }
                    Some('u') => {
                        // Unicode escape \u{NNNN}
                        if chars.next() == Some('{') {
                            let mut hex = String::new();
                            while let Some(&c) = chars.peek() {
                                if c == '}' {
                                    chars.next();
                                    break;
                                }
                                // Safe: we just peeked Some(&c), so next() must return Some(c)
                                hex.push(chars.next()
                                    .expect("BUG: peek() returned Some but next() returned None"));
                            }
                            if let Ok(n) = u32::from_str_radix(&hex, 16) {
                                if let Some(c) = char::from_u32(n) {
                                    result.push(c);
                                }
                            }
                        }
                    }
                    Some(c) => result.push(c),
                    None => {}
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    fn parse_char_literal(&self, text: &str) -> char {
        let inner = &text[1..text.len() - 1];
        let mut chars = inner.chars();

        match chars.next() {
            Some('\\') => match chars.next() {
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('\\') => '\\',
                Some('\'') => '\'',
                Some('"') => '"',
                Some('0') => '\0',
                Some(c) => c,
                None => '\0',
            },
            Some(c) => c,
            None => '\0',
        }
    }
}
