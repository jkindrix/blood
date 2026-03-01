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
    /// Next token (for one-token lookahead).
    next: Token,
    /// Previous token.
    previous: Token,
    /// Accumulated errors.
    errors: Vec<Diagnostic>,
    /// Whether we're in panic mode (error recovery).
    panic_mode: bool,
    /// Pending `>` token from splitting `>>` or `>>=` in type argument contexts.
    /// When we consume a `>` from `>>`, we set this to the span of the second `>`.
    pending_gt: Option<Span>,
}

impl<'src> Parser<'src> {
    /// Create a new parser for the given source.
    pub fn new(source: &'src str) -> Self {
        Self::with_interner(source, DefaultStringInterner::new())
    }

    /// Create a parser with an existing string interner.
    ///
    /// This is useful for parsing external modules where we want to share
    /// the same symbol table as the parent module.
    pub fn with_interner(source: &'src str, interner: DefaultStringInterner) -> Self {
        let mut lexer = Lexer::new(source);
        let current = lexer.next().unwrap_or(Token::dummy(TokenKind::Error));
        let next = lexer.next().unwrap_or(Token::dummy(TokenKind::Eof));

        Self {
            lexer,
            source,
            interner,
            current,
            next,
            previous: Token::dummy(TokenKind::Error),
            errors: Vec::new(),
            panic_mode: false,
            pending_gt: None,
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

    /// Parse a single expression (used for macro expansion).
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_expr_for_macro(&mut self) -> Result<crate::ast::Expr, Vec<Diagnostic>> {
        let expr = self.parse_expr();

        if self.errors.is_empty() {
            Ok(expr)
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

    /// Check if a token kind is a contextual keyword that can be used as an identifier.
    /// These keywords are reserved but can appear as identifiers in certain contexts
    /// (field names, parameter names, function names, etc.).
    fn is_contextual_keyword(kind: TokenKind) -> bool {
        matches!(
            kind,
            // Effect system keywords
            TokenKind::Default
                | TokenKind::Handle
                | TokenKind::Handler
                | TokenKind::Effect
                | TokenKind::Op
                | TokenKind::Deep
                | TokenKind::Shallow
                | TokenKind::Pure
                | TokenKind::Resume
                | TokenKind::Perform
                // Common field names
                | TokenKind::Type
                | TokenKind::In
                | TokenKind::Fiber
                | TokenKind::Suspend
                | TokenKind::Move
                | TokenKind::Ref
                | TokenKind::With
                | TokenKind::Where
                | TokenKind::Module
                | TokenKind::Finally
        )
    }

    /// Check if the current token is an identifier or a contextual keyword.
    /// Use this instead of `check(TokenKind::Ident)` when parsing names that
    /// should allow contextual keywords.
    fn check_ident(&self) -> bool {
        self.current.kind == TokenKind::Ident || Self::is_contextual_keyword(self.current.kind)
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

        // Shift: current <- next, next <- lexer.next()
        self.current = self.next.clone();

        loop {
            self.next = self.lexer.next().unwrap_or_else(|| {
                Token::new(
                    TokenKind::Eof,
                    Span::new(self.source.len(), self.source.len(), 0, 0),
                )
            });

            // Accept any non-error token, or EOF
            if self.next.kind != TokenKind::Error || self.next.kind == TokenKind::Eof {
                break;
            }

            // Report lexer errors
            self.error_at_current("unexpected character", ErrorCode::UnexpectedCharacter);
        }
        self.previous.clone()
    }

    /// Check if the next token (lookahead) matches the given kind.
    fn check_next(&self, kind: TokenKind) -> bool {
        self.next.kind == kind
    }

    /// Check if the token after next (2-ahead lookahead) matches the given kind.
    /// This clones the lexer to peek ahead without mutating parser state.
    fn check_after_next(&self, kind: TokenKind) -> bool {
        let mut lexer_clone = self.lexer.clone();
        if let Some(token) = lexer_clone.next() {
            token.kind == kind
        } else {
            false
        }
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

    /// Check if the current token is a path separator (either `::` or `.`).
    /// Blood's grammar uses `.` as the universal path separator (GRAMMAR.md v0.4.0+),
    /// but `::` is still accepted during the transition period.
    fn check_path_sep(&self) -> bool {
        self.check(TokenKind::ColonColon) || self.check(TokenKind::Dot)
    }

    /// Try to consume a path separator (either `::` or `.`).
    fn try_consume_path_sep(&mut self) -> bool {
        if self.check(TokenKind::ColonColon) || self.check(TokenKind::Dot) {
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

    /// Get the text of the current token.
    fn current_text(&self) -> &'src str {
        self.text(&self.current.span)
    }

    /// Intern a string and return its symbol.
    fn intern(&mut self, s: &str) -> Symbol {
        self.interner.get_or_intern(s)
    }

    /// Resolve a symbol to its string.
    fn interner_symbol_str(&self, symbol: Symbol) -> &str {
        self.interner.resolve(symbol).unwrap_or("")
    }

    /// Take ownership of the string interner.
    pub fn take_interner(&mut self) -> DefaultStringInterner {
        std::mem::take(&mut self.interner)
    }

    /// Check if there are any parsing errors.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Take ownership of any accumulated errors.
    pub fn take_errors(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.errors)
    }

    /// Create a spanned symbol from the previous token.
    fn spanned_symbol(&mut self) -> Spanned<Symbol> {
        let text = self.text(&self.previous.span);
        let symbol = self.intern(text);
        Spanned::new(symbol, self.previous.span)
    }

    /// Create a spanned symbol from a lifetime token, stripping the leading `'`.
    fn spanned_lifetime_symbol(&mut self) -> Spanned<Symbol> {
        let text = self.text(&self.previous.span);
        let text = text.strip_prefix('\'').unwrap_or(text);
        let symbol = self.intern(text);
        Spanned::new(symbol, self.previous.span)
    }

    // ============================================================
    // Type argument `>` handling (for `>>` disambiguation)
    // ============================================================

    /// Check if we're at a closing angle bracket for type arguments.
    /// This handles `>`, `>>`, and `>>=` tokens, as well as pending `>` from previous splits.
    fn check_closing_angle(&self) -> bool {
        if self.pending_gt.is_some() {
            return true;
        }
        matches!(
            self.current.kind,
            TokenKind::Gt | TokenKind::Shr | TokenKind::ShrEq | TokenKind::GtEq
        )
    }

    /// Consume a single `>` for closing type arguments.
    /// If the current token is `>>`, this splits it and leaves a pending `>`.
    /// If the current token is `>>=`, this splits it and converts current to `>=`.
    /// If the current token is `>=`, this splits it and leaves a pending `=`.
    /// Returns the span of the consumed `>`.
    fn consume_closing_angle(&mut self) -> Option<Span> {
        // First check for pending `>` from a previous split
        if let Some(span) = self.pending_gt.take() {
            self.previous = Token::new(TokenKind::Gt, span);
            return Some(span);
        }

        match self.current.kind {
            TokenKind::Gt => {
                let span = self.current.span;
                self.advance();
                Some(span)
            }
            TokenKind::Shr => {
                // `>>` - consume first `>`, leave second as pending
                let full_span = self.current.span;
                // First `>` is the first byte
                let first_span = Span::new(
                    full_span.start,
                    full_span.start + 1,
                    full_span.start_line,
                    full_span.start_col,
                );
                // Second `>` is the second byte
                let second_span = Span::new(
                    full_span.start + 1,
                    full_span.end,
                    full_span.start_line,
                    full_span.start_col + 1,
                );
                self.pending_gt = Some(second_span);
                self.previous = Token::new(TokenKind::Gt, first_span);
                // Advance past the `>>` token
                self.current = self.next.clone();
                self.next = self.lexer.next().unwrap_or_else(|| {
                    Token::new(TokenKind::Eof, Span::new(self.source.len(), self.source.len(), 0, 0))
                });
                Some(first_span)
            }
            TokenKind::ShrEq => {
                // `>>=` - consume first `>`, convert to `>=`
                let full_span = self.current.span;
                let first_span = Span::new(
                    full_span.start,
                    full_span.start + 1,
                    full_span.start_line,
                    full_span.start_col,
                );
                // Convert current token to `>=`
                let ge_span = Span::new(
                    full_span.start + 1,
                    full_span.end,
                    full_span.start_line,
                    full_span.start_col + 1,
                );
                self.current = Token::new(TokenKind::GtEq, ge_span);
                self.previous = Token::new(TokenKind::Gt, first_span);
                Some(first_span)
            }
            TokenKind::GtEq => {
                // `>=` in type context - consume `>`, leave `=` as pending
                // This handles cases like `Vec<T>=` which would be unusual but possible
                let full_span = self.current.span;
                let gt_span = Span::new(
                    full_span.start,
                    full_span.start + 1,
                    full_span.start_line,
                    full_span.start_col,
                );
                // We don't have a pending_eq mechanism, so just treat this as `>`
                // and let the `=` cause an error if it's unexpected
                self.previous = Token::new(TokenKind::Gt, gt_span);
                // Convert current to `=`
                let eq_span = Span::new(
                    full_span.start + 1,
                    full_span.end,
                    full_span.start_line,
                    full_span.start_col + 1,
                );
                self.current = Token::new(TokenKind::Eq, eq_span);
                Some(gt_span)
            }
            _ => {
                self.error_expected("`>`");
                None
            }
        }
    }

    /// Expect a closing angle bracket `>` for type arguments.
    /// This is like `expect(TokenKind::Gt)` but handles `>>` splitting.
    fn expect_closing_angle(&mut self) -> Option<Span> {
        if self.check_closing_angle() {
            self.consume_closing_angle()
        } else {
            self.error_expected("`>`");
            None
        }
    }

    // ============================================================
    // Error handling
    // ============================================================

    fn error_at_current(&mut self, message: &str, code: ErrorCode) {
        self.error_at(self.current.span, message, code);
    }

    fn error_at(&mut self, span: Span, message: &str, code: ErrorCode) {
        if self.panic_mode {
            return;
        }
        self.panic_mode = true;
        self.errors
            .push(Diagnostic::error(message, span).with_error_code(code));
    }

    /// Emit a warning at the given span.
    /// Unlike errors, warnings don't trigger panic mode.
    fn warn_at(&mut self, span: Span, message: &str, code: ErrorCode) {
        self.errors
            .push(Diagnostic::warning(message, span).with_error_code(code));
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
    ///
    /// A recovery point is either:
    /// 1. A declaration keyword (fn, struct, enum, etc.) that starts a new top-level item
    /// 2. EOF
    ///
    /// The synchronization logic skips tokens until it finds a valid recovery point,
    /// properly handling nested braces to avoid getting stuck inside blocks.
    fn synchronize(&mut self) {
        self.panic_mode = false;

        while !self.is_at_end() {
            // Check if we're at a valid declaration keyword first
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
                | TokenKind::Mod
                | TokenKind::Bridge
                | TokenKind::Extern
                | TokenKind::Macro => return,
                // When we encounter an opening brace, skip the entire block
                // This prevents getting stuck inside function bodies during error recovery
                TokenKind::LBrace => {
                    self.advance(); // consume '{'
                    self.skip_to_closing(TokenKind::RBrace);
                    if self.check(TokenKind::RBrace) {
                        self.advance(); // consume '}'
                    }
                    continue;
                }
                // Skip closing delimiters to avoid infinite loops
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                    self.advance();
                    continue;
                }
                _ => {}
            }

            self.advance();
        }
    }

    /// Skip tokens until we find a closing delimiter, handling nested delimiters.
    /// Returns true if the closing delimiter was found.
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
    #[allow(dead_code)] // Infrastructure for fine-grained error recovery
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
                // Intentionally empty: continue advancing until a sync point is found
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

        // First segment must be an identifier
        // Blood uses dot-separated paths: std.collections.vec
        // NOT Rust's crate:: or super:: syntax
        // Accept contextual keywords (e.g., `module`) and type identifiers as path segments
        if self.check_ident() || self.check(TokenKind::TypeIdent) {
            self.advance();
            segments.push(self.spanned_symbol());
        } else {
            self.error_expected("identifier");
        }

        // Additional segments - accept both `.` and `::` as path separators.
        // Don't consume if followed by `{` or `*` (import target markers handled by caller).
        loop {
            if self.check_path_sep() {
                // Don't consume path separator if followed by `{` or `*` (import syntax)
                if self.check_next(TokenKind::LBrace) || self.check_next(TokenKind::Star) {
                    break;
                }
                // Also don't consume `.` if followed by a non-path-segment token
                // (could be method call or field access in non-import contexts)
                if self.check(TokenKind::Dot)
                    && !self.check_next(TokenKind::Ident)
                    && !self.check_next(TokenKind::TypeIdent)
                    && !Self::is_contextual_keyword(self.next.kind)
                {
                    break;
                }
                self.advance(); // consume path separator
                if self.check_ident() || self.check(TokenKind::TypeIdent) {
                    self.advance();
                    segments.push(self.spanned_symbol());
                } else {
                    self.error_expected("identifier");
                    break;
                }
            } else {
                break;
            }
        }

        ModulePath {
            segments,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse an import statement (private, for top-of-file imports).
    fn parse_import(&mut self) -> Import {
        self.parse_import_with_visibility(Visibility::Private)
    }

    /// Parse an import statement with the given visibility.
    ///
    /// This is used for both top-of-file imports (always private) and
    /// declaration-level imports which can have visibility modifiers (`pub use`).
    fn parse_import_with_visibility(&mut self, visibility: Visibility) -> Import {
        let start = self.current.span;
        self.advance(); // consume 'use'

        let path = self.parse_module_path();

        // Check for path separator before grouped imports, glob imports, or single item imports.
        // Accepts both `::` (legacy) and `.` (spec syntax) as the separator.
        if self.try_consume_path_sep() {
            if self.try_consume(TokenKind::Star) {
                // Glob import
                self.expect(TokenKind::Semi);
                Import::Glob {
                    path,
                    visibility,
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
                    visibility,
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
                    visibility,
                    span: start.merge(self.previous.span),
                }
            } else {
                self.error_expected("`*`, `{`, or identifier");
                self.expect(TokenKind::Semi);
                Import::Simple {
                    path,
                    alias: None,
                    visibility,
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
                visibility,
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

        loop {
            if self.check(TokenKind::DocComment) {
                // Convert doc comment to #[doc = "content"] attribute
                attrs.push(self.parse_doc_comment_as_attribute());
            } else if self.check(TokenKind::Hash) {
                attrs.push(self.parse_attribute());
            } else {
                break;
            }
        }

        attrs
    }

    /// Parse a doc comment (`///`) and convert it to a `#[doc = "..."]` attribute.
    ///
    /// This follows the same approach as Rust's `syn` crate where doc comments
    /// are promoted to attributes. This enables documentation generation and
    /// makes the parser handle doc comments uniformly with other attributes.
    fn parse_doc_comment_as_attribute(&mut self) -> Attribute {
        let span = self.current.span;
        let text = self.text(&span);

        // Doc comment starts with "/// " or just "///"
        // Strip the leading "///" to get the content
        let content = if let Some(stripped) = text.strip_prefix("/// ") {
            stripped
        } else if let Some(stripped) = text.strip_prefix("///") {
            stripped
        } else {
            text
        };

        self.advance(); // consume the doc comment token

        // Create a synthetic #[doc = "content"] attribute
        let doc_sym = self.intern("doc");
        Attribute {
            is_inner: false,
            path: vec![Spanned::new(doc_sym, span)],
            args: Some(AttributeArgs::Eq(Literal {
                kind: LiteralKind::String(content.to_string()),
                span,
            })),
            span,
        }
    }

    fn parse_attribute(&mut self) -> Attribute {
        let start = self.current.span;
        self.advance(); // consume '#'

        // Check for inner attribute: #![...] vs outer: #[...]
        let is_inner = self.try_consume(TokenKind::Not);

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

                let arg = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
                    self.advance();
                    let name = self.spanned_symbol();
                    if self.try_consume(TokenKind::Eq) {
                        AttributeArg::KeyValue(name, self.parse_literal())
                    } else if self.try_consume(TokenKind::LParen) {
                        // Nested attribute like #[repr(align(N))]
                        let inner = self.parse_literal();
                        self.expect(TokenKind::RParen);
                        AttributeArg::Call(name, inner)
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
            is_inner,
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
                let s = self.parse_string_literal(text, span);
                LiteralKind::String(s)
            }
            TokenKind::ByteStringLit => {
                let bytes = self.parse_byte_string_literal(text, span);
                LiteralKind::ByteString(bytes)
            }
            TokenKind::RawStringLit => {
                let s = self.parse_raw_string_literal(text, 0);
                LiteralKind::String(s)
            }
            TokenKind::RawStringLitHash1 => {
                let s = self.parse_raw_string_literal(text, 1);
                LiteralKind::String(s)
            }
            TokenKind::RawStringLitHash2 => {
                let s = self.parse_raw_string_literal(text, 2);
                LiteralKind::String(s)
            }
            TokenKind::CharLit => {
                let c = self.parse_char_literal(text, span);
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
                "isize" => Some(IntSuffix::Isize),
                "u8" => Some(IntSuffix::U8),
                "u16" => Some(IntSuffix::U16),
                "u32" => Some(IntSuffix::U32),
                "u64" => Some(IntSuffix::U64),
                "u128" => Some(IntSuffix::U128),
                "usize" => Some(IntSuffix::Usize),
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

    fn parse_string_literal(&mut self, text: &str, span: Span) -> String {
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
                    Some(c) => {
                        self.errors.push(
                            Diagnostic::error(
                                &format!("unknown escape sequence `\\{c}`"),
                                span,
                            ).with_error_code(ErrorCode::InvalidEscape),
                        );
                        result.push(c);
                    }
                    None => {}
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    /// Parse a byte string literal like b"..."
    fn parse_byte_string_literal(&mut self, text: &str, span: Span) -> Vec<u8> {
        // Remove b prefix and quotes, then process escape sequences
        let inner = &text[2..text.len() - 1];
        let mut result = Vec::new();
        let mut chars = inner.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push(b'\n'),
                    Some('r') => result.push(b'\r'),
                    Some('t') => result.push(b'\t'),
                    Some('\\') => result.push(b'\\'),
                    Some('\'') => result.push(b'\''),
                    Some('"') => result.push(b'"'),
                    Some('0') => result.push(0),
                    Some('x') => {
                        // Hex escape \xNN
                        let mut hex = String::new();
                        for _ in 0..2 {
                            if let Some(h) = chars.next() {
                                hex.push(h);
                            }
                        }
                        if let Ok(n) = u8::from_str_radix(&hex, 16) {
                            result.push(n);
                        }
                    }
                    Some(c) if c.is_ascii() => {
                        self.errors.push(
                            Diagnostic::error(
                                &format!("unknown escape sequence `\\{c}`"),
                                span,
                            ).with_error_code(ErrorCode::InvalidEscape),
                        );
                        result.push(c as u8);
                    }
                    _ => {}
                }
            } else if c.is_ascii() {
                result.push(c as u8);
            }
        }

        result
    }

    /// Parse a raw string literal like r"...", r#"..."#, or r##"..."##
    /// The `hash_count` parameter indicates how many # characters are used as delimiters.
    fn parse_raw_string_literal(&self, text: &str, hash_count: usize) -> String {
        // Raw string format: r"content" or r#"content"# or r##"content"##
        // Strip the r prefix, hash delimiters, and quotes
        // No escape sequence processing - content is taken literally
        let prefix_len = 1 + hash_count + 1; // r + hashes + opening quote
        let suffix_len = 1 + hash_count;     // closing quote + hashes
        let inner = &text[prefix_len..text.len() - suffix_len];
        inner.to_string()
    }

    fn parse_char_literal(&mut self, text: &str, span: Span) -> char {
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
                Some('x') => {
                    // Hex escape \xNN
                    let mut hex = String::new();
                    for _ in 0..2 {
                        if let Some(h) = chars.next() {
                            hex.push(h);
                        }
                    }
                    if let Ok(n) = u8::from_str_radix(&hex, 16) {
                        n as char
                    } else {
                        '\0'
                    }
                }
                Some('u') => {
                    // Unicode escape \u{NNNN}
                    if chars.next() == Some('{') {
                        let mut hex = String::new();
                        for c in chars {
                            if c == '}' {
                                break;
                            }
                            hex.push(c);
                        }
                        if let Ok(n) = u32::from_str_radix(&hex, 16) {
                            char::from_u32(n).unwrap_or('\0')
                        } else {
                            '\0'
                        }
                    } else {
                        '\0'
                    }
                }
                Some(c) => {
                    self.errors.push(
                        Diagnostic::error(
                            &format!("unknown escape sequence `\\{c}`"),
                            span,
                        ).with_error_code(ErrorCode::InvalidEscape),
                    );
                    c
                }
                None => '\0',
            },
            Some(c) => c,
            None => '\0',
        }
    }
}
