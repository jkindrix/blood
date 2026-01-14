//! Lexical analysis for Blood.
//!
//! This module tokenizes Blood source code into a stream of tokens.
//! It handles all lexical elements defined in GRAMMAR.md including:
//!
//! - Keywords and identifiers
//! - Integer literals (decimal, hex, octal, binary)
//! - Float literals
//! - String and character literals with escape sequences
//! - Operators and punctuation
//! - Comments (line and nested block)
//! - Attributes
//!
//! # Example
//!
//! ```rust
//! use bloodc::{Lexer, TokenKind};
//!
//! let source = "let x = 42;";
//! let lexer = Lexer::new(source);
//! let tokens: Vec<_> = lexer.collect();
//!
//! assert_eq!(tokens[0].kind, TokenKind::Let);
//! assert_eq!(tokens[1].kind, TokenKind::Ident);
//! assert_eq!(tokens[2].kind, TokenKind::Eq);
//! assert_eq!(tokens[3].kind, TokenKind::IntLit);
//! assert_eq!(tokens[4].kind, TokenKind::Semi);
//! ```

use crate::span::Span;
use logos::Logos;

/// Token kinds for the Blood lexer.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[logos(skip r"[ \t\r\n]+")]
pub enum TokenKind {
    // ============================================================
    // Keywords
    // ============================================================
    #[token("as")]
    As,
    #[token("async")]
    Async,
    #[token("await")]
    Await,
    #[token("break")]
    Break,
    #[token("const")]
    Const,
    #[token("continue")]
    Continue,
    #[token("crate")]
    Crate,
    #[token("deep")]
    Deep,
    #[token("dyn")]
    Dyn,
    #[token("effect")]
    Effect,
    #[token("else")]
    Else,
    #[token("enum")]
    Enum,
    #[token("extends")]
    Extends,
    #[token("extern")]
    Extern,
    #[token("false")]
    False,
    #[token("fn")]
    Fn,
    #[token("for")]
    For,
    #[token("forall")]
    Forall,
    #[token("handler")]
    Handler,
    #[token("if")]
    If,
    #[token("impl")]
    Impl,
    #[token("in")]
    In,
    #[token("let")]
    Let,
    #[token("linear")]
    Linear,
    #[token("loop")]
    Loop,
    #[token("match")]
    Match,
    #[token("mod")]
    Mod,
    #[token("module")]
    Module,
    #[token("move")]
    Move,
    #[token("mut")]
    Mut,
    #[token("op")]
    Op,
    #[token("perform")]
    Perform,
    #[token("pub")]
    Pub,
    #[token("pure")]
    Pure,
    #[token("ref")]
    Ref,
    #[token("region")]
    Region,
    #[token("resume")]
    Resume,
    #[token("return")]
    Return,
    #[token("self")]
    SelfLower,
    #[token("Self")]
    SelfUpper,
    #[token("shallow")]
    Shallow,
    #[token("static")]
    Static,
    #[token("struct")]
    Struct,
    #[token("super")]
    Super,
    #[token("trait")]
    Trait,
    #[token("true")]
    True,
    #[token("type")]
    Type,
    #[token("use")]
    Use,
    #[token("where")]
    Where,
    #[token("while")]
    While,
    #[token("with")]
    With,
    #[token("handle")]
    Handle,
    #[token("affine")]
    Affine,
    #[token("bridge")]
    Bridge,
    // Note: `callback` is a contextual keyword, only valid inside bridge blocks
    // It is parsed as an identifier and checked contextually in the bridge parser

    // Reserved for future use
    #[token("abstract")]
    Abstract,
    #[token("become")]
    Become,
    #[token("box")]
    Box,
    #[token("do")]
    Do,
    #[token("final")]
    Final,
    #[token("macro")]
    Macro,
    #[token("override")]
    Override,
    #[token("priv")]
    Priv,
    #[token("typeof")]
    Typeof,
    #[token("unsized")]
    Unsized,
    #[token("virtual")]
    Virtual,
    #[token("yield")]
    Yield,
    #[token("try")]
    Try,
    #[token("catch")]
    Catch,
    #[token("finally")]
    Finally,
    #[token("throw")]
    Throw,
    #[token("union")]
    Union,
    #[token("default")]
    Default,
    #[token("unsafe")]
    Unsafe,

    // ============================================================
    // Literals
    // ============================================================
    /// Integer literal (decimal, hex, octal, or binary) with optional type suffix
    /// Suffixes: i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, isize, usize
    #[regex(r"0x[0-9a-fA-F_]+(i8|i16|i32|i64|i128|u8|u16|u32|u64|u128|isize|usize)?")]
    #[regex(r"0o[0-7_]+(i8|i16|i32|i64|i128|u8|u16|u32|u64|u128|isize|usize)?")]
    #[regex(r"0b[01_]+(i8|i16|i32|i64|i128|u8|u16|u32|u64|u128|isize|usize)?")]
    #[regex(r"[0-9][0-9_]*(i8|i16|i32|i64|i128|u8|u16|u32|u64|u128|isize|usize)?")]
    IntLit,

    /// Float literal (with optional f32/f64 suffix)
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9_]+)?(f32|f64)?")]
    FloatLit,

    /// String literal
    #[regex(r#""([^"\\]|\\.)*""#)]
    StringLit,

    /// Byte string literal (b"...")
    #[regex(r#"b"([^"\\]|\\.)*""#)]
    ByteStringLit,

    /// Raw string literal (r"..." or r#"..."# etc.)
    /// Simple form: r"..." - no escape processing
    #[regex(r#"r"[^"]*""#)]
    RawStringLit,

    /// Raw string literal with # delimiters: r#"..."#
    /// Content can have quotes as long as they're not followed by #
    /// For simplicity, we support r#"..."# (1 hash) and r##"..."## (2 hashes)
    #[regex(r##"r#"([^"]|"[^#])*"#"##)]
    RawStringLitHash1,

    #[regex(r###"r##"([^"]|"[^#]|"#[^#])*"##"###)]
    RawStringLitHash2,

    /// Character literal
    #[regex(r"'([^'\\]|\\.)'")]
    CharLit,

    // ============================================================
    // Identifiers and Lifetimes
    // ============================================================
    /// Identifier starting with lowercase or underscore
    #[regex(r"[a-z_][a-zA-Z0-9_]*")]
    Ident,

    /// Type identifier starting with uppercase
    #[regex(r"[A-Z][a-zA-Z0-9_]*")]
    TypeIdent,

    /// Lifetime identifier ('a, 'static, '_)
    #[regex(r"'[a-zA-Z_][a-zA-Z0-9_]*", priority = 1)]
    #[token("'static", priority = 2)]
    #[token("'_", priority = 2)]
    Lifetime,

    // ============================================================
    // Operators
    // ============================================================
    // Arithmetic
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,

    // Comparison
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,

    // Logical
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("!")]
    Not,

    // Bitwise
    #[token("&")]
    And,
    #[token("|")]
    Or,
    #[token("^")]
    Caret,
    #[token("<<")]
    Shl,
    #[token(">>")]
    Shr,

    // Assignment
    #[token("=")]
    Eq,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("/=")]
    SlashEq,
    #[token("%=")]
    PercentEq,
    #[token("&=")]
    AndEq,
    #[token("|=")]
    OrEq,
    #[token("^=")]
    CaretEq,
    #[token("<<=")]
    ShlEq,
    #[token(">>=")]
    ShrEq,

    // ============================================================
    // Punctuation
    // ============================================================
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,

    #[token(",")]
    Comma,
    #[token(";")]
    Semi,
    #[token(":")]
    Colon,
    #[token("::")]
    ColonColon,
    #[token(".")]
    Dot,
    #[token("..")]
    DotDot,
    #[token("..=")]
    DotDotEq,

    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,
    #[token("|>")]
    Pipe,
    #[token("@")]
    At,
    #[token("#")]
    Hash,
    #[token("?")]
    Question,
    #[token("$")]
    Dollar,

    // ============================================================
    // Special
    // ============================================================
    /// @unsafe block marker
    #[token("@unsafe")]
    AtUnsafe,

    /// @heap allocation marker
    #[token("@heap")]
    AtHeap,

    /// @stack allocation marker
    #[token("@stack")]
    AtStack,

    // ============================================================
    // Comments (handled specially)
    // ============================================================
    /// Doc comment (///) - not skipped, attached to items
    /// Matches /// followed by content until newline (but not //// which is a line comment)
    #[regex(r"///[^/\n][^\n]*", priority = 3)]
    #[regex(r"///", priority = 2)]
    DocComment,

    /// Line comment (skipped)
    /// Matches // followed by non-/ content, or //// and more
    #[regex(r"//[^/\n][^\n]*", logos::skip)]
    #[regex(r"//\n", logos::skip)]
    #[regex(r"////[^\n]*", logos::skip)]
    LineComment,

    /// Block comment (with nesting support via callback)
    #[token("/*", block_comment)]
    BlockComment,

    /// Unclosed block comment (error token)
    UnclosedBlockComment,

    // ============================================================
    // Special
    // ============================================================
    /// End of file marker (not produced by logos, added by Lexer wrapper)
    Eof,

    /// Lexer error
    Error,
}

/// Callback for nested block comment parsing.
/// Returns Skip for properly closed comments, or UnclosedBlockComment for unclosed ones.
fn block_comment(lexer: &mut logos::Lexer<TokenKind>) -> logos::Filter<TokenKind> {
    let mut depth = 1;
    let remainder = lexer.remainder();

    let mut chars = remainder.chars().peekable();
    let mut consumed = 0;

    while depth > 0 {
        match chars.next() {
            Some('/') if chars.peek() == Some(&'*') => {
                chars.next();
                consumed += 2;
                depth += 1;
            }
            Some('*') if chars.peek() == Some(&'/') => {
                chars.next();
                consumed += 2;
                depth -= 1;
            }
            Some(c) => {
                consumed += c.len_utf8();
            }
            None => {
                // Unclosed block comment - emit as an error token
                lexer.bump(consumed);
                return logos::Filter::Emit(TokenKind::UnclosedBlockComment);
            }
        }
    }

    lexer.bump(consumed);
    logos::Filter::Skip
}

impl TokenKind {
    /// Returns true if this token kind carries text that should be displayed.
    pub fn has_text(&self) -> bool {
        matches!(
            self,
            TokenKind::IntLit
                | TokenKind::FloatLit
                | TokenKind::StringLit
                | TokenKind::ByteStringLit
                | TokenKind::RawStringLit
                | TokenKind::RawStringLitHash1
                | TokenKind::RawStringLitHash2
                | TokenKind::CharLit
                | TokenKind::Ident
                | TokenKind::TypeIdent
                | TokenKind::Lifetime
        )
    }

    /// Returns the keyword string if this is a keyword token.
    pub fn as_keyword_str(&self) -> Option<&'static str> {
        match self {
            TokenKind::As => Some("as"),
            TokenKind::Async => Some("async"),
            TokenKind::Await => Some("await"),
            TokenKind::Break => Some("break"),
            TokenKind::Const => Some("const"),
            TokenKind::Continue => Some("continue"),
            TokenKind::Crate => Some("crate"),
            TokenKind::Deep => Some("deep"),
            TokenKind::Dyn => Some("dyn"),
            TokenKind::Effect => Some("effect"),
            TokenKind::Else => Some("else"),
            TokenKind::Enum => Some("enum"),
            TokenKind::Extends => Some("extends"),
            TokenKind::Extern => Some("extern"),
            TokenKind::False => Some("false"),
            TokenKind::Fn => Some("fn"),
            TokenKind::For => Some("for"),
            TokenKind::Handler => Some("handler"),
            TokenKind::If => Some("if"),
            TokenKind::Impl => Some("impl"),
            TokenKind::In => Some("in"),
            TokenKind::Let => Some("let"),
            TokenKind::Linear => Some("linear"),
            TokenKind::Loop => Some("loop"),
            TokenKind::Match => Some("match"),
            TokenKind::Mod => Some("mod"),
            TokenKind::Module => Some("module"),
            TokenKind::Move => Some("move"),
            TokenKind::Mut => Some("mut"),
            TokenKind::Op => Some("op"),
            TokenKind::Perform => Some("perform"),
            TokenKind::Pub => Some("pub"),
            TokenKind::Pure => Some("pure"),
            TokenKind::Ref => Some("ref"),
            TokenKind::Region => Some("region"),
            TokenKind::Resume => Some("resume"),
            TokenKind::Return => Some("return"),
            TokenKind::SelfLower => Some("self"),
            TokenKind::SelfUpper => Some("Self"),
            TokenKind::Shallow => Some("shallow"),
            TokenKind::Static => Some("static"),
            TokenKind::Struct => Some("struct"),
            TokenKind::Super => Some("super"),
            TokenKind::Trait => Some("trait"),
            TokenKind::True => Some("true"),
            TokenKind::Type => Some("type"),
            TokenKind::Use => Some("use"),
            TokenKind::Where => Some("where"),
            TokenKind::While => Some("while"),
            TokenKind::With => Some("with"),
            TokenKind::Handle => Some("handle"),
            TokenKind::Affine => Some("affine"),
            TokenKind::Bridge => Some("bridge"),
            _ => None,
        }
    }

    /// Returns a human-readable description of the token kind.
    pub fn description(&self) -> &'static str {
        match self {
            TokenKind::As => "keyword `as`",
            TokenKind::Async => "keyword `async`",
            TokenKind::Await => "keyword `await`",
            TokenKind::Break => "keyword `break`",
            TokenKind::Const => "keyword `const`",
            TokenKind::Continue => "keyword `continue`",
            TokenKind::Crate => "keyword `crate`",
            TokenKind::Deep => "keyword `deep`",
            TokenKind::Dyn => "keyword `dyn`",
            TokenKind::Effect => "keyword `effect`",
            TokenKind::Else => "keyword `else`",
            TokenKind::Enum => "keyword `enum`",
            TokenKind::Extends => "keyword `extends`",
            TokenKind::Extern => "keyword `extern`",
            TokenKind::False => "keyword `false`",
            TokenKind::Fn => "keyword `fn`",
            TokenKind::For => "keyword `for`",
            TokenKind::Handler => "keyword `handler`",
            TokenKind::If => "keyword `if`",
            TokenKind::Impl => "keyword `impl`",
            TokenKind::In => "keyword `in`",
            TokenKind::Let => "keyword `let`",
            TokenKind::Linear => "keyword `linear`",
            TokenKind::Loop => "keyword `loop`",
            TokenKind::Match => "keyword `match`",
            TokenKind::Mod => "keyword `mod`",
            TokenKind::Module => "keyword `module`",
            TokenKind::Move => "keyword `move`",
            TokenKind::Mut => "keyword `mut`",
            TokenKind::Op => "keyword `op`",
            TokenKind::Perform => "keyword `perform`",
            TokenKind::Pub => "keyword `pub`",
            TokenKind::Pure => "keyword `pure`",
            TokenKind::Ref => "keyword `ref`",
            TokenKind::Region => "keyword `region`",
            TokenKind::Resume => "keyword `resume`",
            TokenKind::Return => "keyword `return`",
            TokenKind::SelfLower => "keyword `self`",
            TokenKind::SelfUpper => "keyword `Self`",
            TokenKind::Shallow => "keyword `shallow`",
            TokenKind::Static => "keyword `static`",
            TokenKind::Struct => "keyword `struct`",
            TokenKind::Super => "keyword `super`",
            TokenKind::Trait => "keyword `trait`",
            TokenKind::True => "keyword `true`",
            TokenKind::Type => "keyword `type`",
            TokenKind::Use => "keyword `use`",
            TokenKind::Where => "keyword `where`",
            TokenKind::While => "keyword `while`",
            TokenKind::With => "keyword `with`",
            TokenKind::Handle => "keyword `handle`",
            TokenKind::Affine => "keyword `affine`",
            TokenKind::Bridge => "keyword `bridge`",
            TokenKind::Forall => "keyword `forall`",
            TokenKind::Abstract => "reserved keyword `abstract`",
            TokenKind::Become => "reserved keyword `become`",
            TokenKind::Box => "reserved keyword `box`",
            TokenKind::Do => "reserved keyword `do`",
            TokenKind::Final => "reserved keyword `final`",
            TokenKind::Macro => "reserved keyword `macro`",
            TokenKind::Override => "reserved keyword `override`",
            TokenKind::Priv => "reserved keyword `priv`",
            TokenKind::Typeof => "reserved keyword `typeof`",
            TokenKind::Unsized => "reserved keyword `unsized`",
            TokenKind::Virtual => "reserved keyword `virtual`",
            TokenKind::Yield => "reserved keyword `yield`",
            TokenKind::Try => "reserved keyword `try`",
            TokenKind::Catch => "reserved keyword `catch`",
            TokenKind::Finally => "reserved keyword `finally`",
            TokenKind::Throw => "reserved keyword `throw`",
            TokenKind::Union => "contextual keyword `union`",
            TokenKind::Default => "contextual keyword `default`",
            TokenKind::Unsafe => "reserved keyword `unsafe`",
            TokenKind::IntLit => "integer literal",
            TokenKind::FloatLit => "float literal",
            TokenKind::StringLit => "string literal",
            TokenKind::ByteStringLit => "byte string literal",
            TokenKind::RawStringLit => "raw string literal",
            TokenKind::RawStringLitHash1 => "raw string literal",
            TokenKind::RawStringLitHash2 => "raw string literal",
            TokenKind::CharLit => "character literal",
            TokenKind::Ident => "identifier",
            TokenKind::TypeIdent => "type identifier",
            TokenKind::Lifetime => "lifetime",
            TokenKind::Plus => "`+`",
            TokenKind::Minus => "`-`",
            TokenKind::Star => "`*`",
            TokenKind::Slash => "`/`",
            TokenKind::Percent => "`%`",
            TokenKind::EqEq => "`==`",
            TokenKind::NotEq => "`!=`",
            TokenKind::Lt => "`<`",
            TokenKind::Gt => "`>`",
            TokenKind::LtEq => "`<=`",
            TokenKind::GtEq => "`>=`",
            TokenKind::AndAnd => "`&&`",
            TokenKind::OrOr => "`||`",
            TokenKind::Not => "`!`",
            TokenKind::And => "`&`",
            TokenKind::Or => "`|`",
            TokenKind::Caret => "`^`",
            TokenKind::Shl => "`<<`",
            TokenKind::Shr => "`>>`",
            TokenKind::Eq => "`=`",
            TokenKind::PlusEq => "`+=`",
            TokenKind::MinusEq => "`-=`",
            TokenKind::StarEq => "`*=`",
            TokenKind::SlashEq => "`/=`",
            TokenKind::PercentEq => "`%=`",
            TokenKind::AndEq => "`&=`",
            TokenKind::OrEq => "`|=`",
            TokenKind::CaretEq => "`^=`",
            TokenKind::ShlEq => "`<<=`",
            TokenKind::ShrEq => "`>>=`",
            TokenKind::LParen => "`(`",
            TokenKind::RParen => "`)`",
            TokenKind::LBrace => "`{`",
            TokenKind::RBrace => "`}`",
            TokenKind::LBracket => "`[`",
            TokenKind::RBracket => "`]`",
            TokenKind::Comma => "`,`",
            TokenKind::Semi => "`;`",
            TokenKind::Colon => "`:`",
            TokenKind::ColonColon => "`::`",
            TokenKind::Dot => "`.`",
            TokenKind::DotDot => "`..`",
            TokenKind::DotDotEq => "`..=`",
            TokenKind::Arrow => "`->`",
            TokenKind::FatArrow => "`=>`",
            TokenKind::Pipe => "`|>`",
            TokenKind::At => "`@`",
            TokenKind::Hash => "`#`",
            TokenKind::Question => "`?`",
            TokenKind::AtUnsafe => "`@unsafe`",
            TokenKind::AtHeap => "`@heap`",
            TokenKind::AtStack => "`@stack`",
            TokenKind::DocComment => "doc comment",
            TokenKind::LineComment => "line comment",
            TokenKind::BlockComment => "block comment",
            TokenKind::UnclosedBlockComment => "unclosed block comment",
            TokenKind::Eof => "end of file",
            TokenKind::Error => "error",
            TokenKind::Dollar => "`$`",
        }
    }

    /// Check if this token can start an expression.
    pub fn can_start_expr(&self) -> bool {
        matches!(
            self,
            TokenKind::IntLit
                | TokenKind::FloatLit
                | TokenKind::StringLit
                | TokenKind::ByteStringLit
                | TokenKind::RawStringLit
                | TokenKind::RawStringLitHash1
                | TokenKind::RawStringLitHash2
                | TokenKind::CharLit
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Ident
                | TokenKind::TypeIdent
                | TokenKind::SelfLower
                | TokenKind::SelfUpper
                | TokenKind::Default  // Special expression for default values
                | TokenKind::Handle   // Contextual keyword usable as identifier
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::LBracket
                | TokenKind::If
                | TokenKind::Match
                | TokenKind::Loop
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Or      // Closure with params |x|
                | TokenKind::OrOr    // Closure without params ||
                | TokenKind::Move    // Move closure
                | TokenKind::Not
                | TokenKind::Minus
                | TokenKind::Star    // Dereference
                | TokenKind::And     // Reference
                | TokenKind::AtUnsafe
                | TokenKind::Region
                | TokenKind::Perform
                | TokenKind::With
        )
    }
}

/// A token with its kind and source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn dummy(kind: TokenKind) -> Self {
        Self {
            kind,
            span: Span::dummy(),
        }
    }
}

/// The lexer for Blood source code.
#[derive(Clone)]
pub struct Lexer<'src> {
    inner: logos::Lexer<'src, TokenKind>,
    source: &'src str,
    /// Precomputed line index for O(log n) line/column lookup.
    line_index: crate::span::LineIndex,
    finished: bool,
}

impl<'src> Lexer<'src> {
    /// Create a new lexer for the given source.
    pub fn new(source: &'src str) -> Self {
        Self {
            inner: TokenKind::lexer(source),
            source,
            line_index: crate::span::LineIndex::new(source),
            finished: false,
        }
    }

    /// Get the source text for a span.
    pub fn slice(&self, span: &Span) -> &'src str {
        &self.source[span.start..span.end]
    }
}

impl<'src> Iterator for Lexer<'src> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        match self.inner.next() {
            Some(Ok(kind)) => {
                let logos_span = self.inner.span();
                let (line, col) = self.line_index.line_col(logos_span.start);
                let span = Span::new(logos_span.start, logos_span.end, line, col);
                Some(Token::new(kind, span))
            }
            Some(Err(())) => {
                let logos_span = self.inner.span();
                let (line, col) = self.line_index.line_col(logos_span.start);
                let span = Span::new(logos_span.start, logos_span.end, line, col);
                Some(Token::new(TokenKind::Error, span))
            }
            None => {
                self.finished = true;
                // Return EOF token once, then None
                let span = Span::new(self.source.len(), self.source.len(), 0, 0);
                Some(Token::new(TokenKind::Eof, span))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> Vec<TokenKind> {
        Lexer::new(source)
            .map(|t| t.kind)
            .filter(|k| *k != TokenKind::Eof)
            .collect()
    }

    #[test]
    fn test_keywords() {
        assert_eq!(lex("fn let if else match"), vec![
            TokenKind::Fn,
            TokenKind::Let,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::Match,
        ]);
    }

    #[test]
    fn test_identifiers() {
        assert_eq!(lex("foo Bar _baz"), vec![
            TokenKind::Ident,
            TokenKind::TypeIdent,
            TokenKind::Ident,
        ]);
    }

    #[test]
    fn test_literals() {
        assert_eq!(lex("42 3.14 0xFF 0b1010 0o77"), vec![
            TokenKind::IntLit,
            TokenKind::FloatLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
        ]);
    }

    #[test]
    fn test_strings() {
        assert_eq!(lex(r#""hello" "world\n""#), vec![
            TokenKind::StringLit,
            TokenKind::StringLit,
        ]);
    }

    #[test]
    fn test_chars() {
        assert_eq!(lex("'a' '\\n' '\\''"), vec![
            TokenKind::CharLit,
            TokenKind::CharLit,
            TokenKind::CharLit,
        ]);
    }

    #[test]
    fn test_operators() {
        assert_eq!(lex("+ - * / % == != < > <= >="), vec![
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
            TokenKind::EqEq,
            TokenKind::NotEq,
            TokenKind::Lt,
            TokenKind::Gt,
            TokenKind::LtEq,
            TokenKind::GtEq,
        ]);
    }

    #[test]
    fn test_punctuation() {
        assert_eq!(lex("( ) { } [ ] , ; : :: . .. ..= -> =>"), vec![
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::LBracket,
            TokenKind::RBracket,
            TokenKind::Comma,
            TokenKind::Semi,
            TokenKind::Colon,
            TokenKind::ColonColon,
            TokenKind::Dot,
            TokenKind::DotDot,
            TokenKind::DotDotEq,
            TokenKind::Arrow,
            TokenKind::FatArrow,
        ]);
    }

    #[test]
    fn test_lifetimes() {
        assert_eq!(lex("'a 'static '_"), vec![
            TokenKind::Lifetime,
            TokenKind::Lifetime,
            TokenKind::Lifetime,
        ]);
    }

    #[test]
    fn test_comments() {
        // Line comments should be skipped
        assert_eq!(lex("fn // comment\nlet"), vec![
            TokenKind::Fn,
            TokenKind::Let,
        ]);

        // Block comments should be skipped
        assert_eq!(lex("fn /* comment */ let"), vec![
            TokenKind::Fn,
            TokenKind::Let,
        ]);

        // Nested block comments
        assert_eq!(lex("fn /* outer /* inner */ outer */ let"), vec![
            TokenKind::Fn,
            TokenKind::Let,
        ]);
    }

    #[test]
    fn test_effect_keywords() {
        assert_eq!(lex("effect handler perform resume with handle"), vec![
            TokenKind::Effect,
            TokenKind::Handler,
            TokenKind::Perform,
            TokenKind::Resume,
            TokenKind::With,
            TokenKind::Handle,
        ]);
    }

    #[test]
    fn test_special_tokens() {
        assert_eq!(lex("@unsafe @heap @stack"), vec![
            TokenKind::AtUnsafe,
            TokenKind::AtHeap,
            TokenKind::AtStack,
        ]);
    }

    #[test]
    fn test_function_signature() {
        let tokens = lex("fn main() -> i32 / {IO} { 0 }");
        assert_eq!(tokens, vec![
            TokenKind::Fn,
            TokenKind::Ident,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::Arrow,
            TokenKind::Ident,
            TokenKind::Slash,
            TokenKind::LBrace,
            TokenKind::TypeIdent,
            TokenKind::RBrace,
            TokenKind::LBrace,
            TokenKind::IntLit,
            TokenKind::RBrace,
        ]);
    }

    #[test]
    fn test_span_positions() {
        let source = "fn main";
        let tokens: Vec<_> = Lexer::new(source).collect();

        assert_eq!(tokens[0].span.start, 0);
        assert_eq!(tokens[0].span.end, 2);
        assert_eq!(tokens[1].span.start, 3);
        assert_eq!(tokens[1].span.end, 7);
    }

    #[test]
    fn test_unclosed_block_comment() {
        // Unclosed block comment should produce UnclosedBlockComment token
        let tokens = lex("fn /* unclosed comment");
        assert_eq!(tokens, vec![
            TokenKind::Fn,
            TokenKind::UnclosedBlockComment,
        ]);

        // Unclosed nested block comment
        let tokens = lex("fn /* outer /* inner */");
        assert_eq!(tokens, vec![
            TokenKind::Fn,
            TokenKind::UnclosedBlockComment,
        ]);
    }

    #[test]
    fn test_integer_bases() {
        // Hex
        assert_eq!(lex("0xDEAD_BEEF"), vec![TokenKind::IntLit]);
        // Octal
        assert_eq!(lex("0o755"), vec![TokenKind::IntLit]);
        // Binary
        assert_eq!(lex("0b1010_1010"), vec![TokenKind::IntLit]);
        // Decimal with underscores
        assert_eq!(lex("1_000_000"), vec![TokenKind::IntLit]);
    }

    #[test]
    fn test_float_literals() {
        assert_eq!(lex("3.14"), vec![TokenKind::FloatLit]);
        assert_eq!(lex("2.5e10"), vec![TokenKind::FloatLit]);
        assert_eq!(lex("1.0e-5"), vec![TokenKind::FloatLit]);
        assert_eq!(lex("1_000.5"), vec![TokenKind::FloatLit]);
        // Float literals with type suffixes
        assert_eq!(lex("3.14f32"), vec![TokenKind::FloatLit]);
        assert_eq!(lex("2.718f64"), vec![TokenKind::FloatLit]);
        assert_eq!(lex("1.0e10f32"), vec![TokenKind::FloatLit]);
    }

    #[test]
    fn test_ownership_keywords() {
        assert_eq!(lex("linear affine"), vec![
            TokenKind::Linear,
            TokenKind::Affine,
        ]);
    }

    #[test]
    fn test_reserved_keywords() {
        assert_eq!(lex("abstract final virtual yield"), vec![
            TokenKind::Abstract,
            TokenKind::Final,
            TokenKind::Virtual,
            TokenKind::Yield,
        ]);
    }

    #[test]
    fn test_byte_string_literals() {
        assert_eq!(lex(r#"b"hello""#), vec![TokenKind::ByteStringLit]);
        assert_eq!(lex(r#"b"hello\n""#), vec![TokenKind::ByteStringLit]);
        assert_eq!(lex(r#"b"" b"x""#), vec![
            TokenKind::ByteStringLit,
            TokenKind::ByteStringLit,
        ]);
    }

    #[test]
    fn test_integer_type_suffixes() {
        // Basic integer with suffix
        assert_eq!(lex("42i32"), vec![TokenKind::IntLit]);
        assert_eq!(lex("0u64"), vec![TokenKind::IntLit]);
        assert_eq!(lex("255u8"), vec![TokenKind::IntLit]);

        // Hex with suffix
        assert_eq!(lex("0xFFi32"), vec![TokenKind::IntLit]);
        assert_eq!(lex("0xDEADu64"), vec![TokenKind::IntLit]);

        // Binary with suffix
        assert_eq!(lex("0b1010u8"), vec![TokenKind::IntLit]);

        // Octal with suffix
        assert_eq!(lex("0o777i32"), vec![TokenKind::IntLit]);

        // All suffix types
        assert_eq!(lex("1i8 2i16 3i32 4i64 5i128"), vec![
            TokenKind::IntLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
        ]);
        assert_eq!(lex("1u8 2u16 3u32 4u64 5u128"), vec![
            TokenKind::IntLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
            TokenKind::IntLit,
        ]);
        assert_eq!(lex("1isize 2usize"), vec![
            TokenKind::IntLit,
            TokenKind::IntLit,
        ]);

        // Mixed with underscores
        assert_eq!(lex("1_000_000i64"), vec![TokenKind::IntLit]);
    }

    #[test]
    fn test_doc_comments() {
        // Basic doc comment
        assert_eq!(lex("/// This is a doc comment"), vec![TokenKind::DocComment]);

        // Doc comment followed by code
        assert_eq!(lex("/// Doc comment\nfn foo"), vec![
            TokenKind::DocComment,
            TokenKind::Fn,
            TokenKind::Ident,
        ]);

        // Multiple doc comments
        assert_eq!(lex("/// First line\n/// Second line"), vec![
            TokenKind::DocComment,
            TokenKind::DocComment,
        ]);

        // Empty doc comment (just ///)
        assert_eq!(lex("///\nfn"), vec![
            TokenKind::DocComment,
            TokenKind::Fn,
        ]);

        // Doc comment with space
        assert_eq!(lex("/// "), vec![TokenKind::DocComment]);
    }

    #[test]
    fn test_line_comments_vs_doc_comments() {
        // Regular line comment should be skipped
        assert_eq!(lex("// regular comment\nfn"), vec![TokenKind::Fn]);

        // Four slashes is a line comment, not a doc comment
        assert_eq!(lex("//// not a doc comment\nfn"), vec![TokenKind::Fn]);

        // Five slashes is also a line comment
        assert_eq!(lex("///// also not a doc comment\nfn"), vec![TokenKind::Fn]);

        // Three slashes IS a doc comment
        assert_eq!(lex("/// this IS a doc comment\nfn"), vec![
            TokenKind::DocComment,
            TokenKind::Fn,
        ]);
    }
}
