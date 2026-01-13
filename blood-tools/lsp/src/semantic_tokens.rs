//! Semantic Tokens Provider
//!
//! Provides semantic highlighting for Blood source files.
//!
//! This module integrates with bloodc's lexer to provide accurate semantic
//! tokens that match the compiler's tokenization.

use bloodc::{Lexer, TokenKind};
use tower_lsp::lsp_types::*;

use crate::document::Document;

/// Semantic token types for Blood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TokenType {
    Namespace = 0,
    Type = 1,
    Class = 2,
    Enum = 3,
    Interface = 4,
    Struct = 5,
    TypeParameter = 6,
    Parameter = 7,
    Variable = 8,
    Property = 9,
    EnumMember = 10,
    Event = 11,
    Function = 12,
    Method = 13,
    Macro = 14,
    Keyword = 15,
    Modifier = 16,
    Comment = 17,
    String = 18,
    Number = 19,
    Regexp = 20,
    Operator = 21,
    Decorator = 22,
    // Blood-specific
    Effect = 23,
    Handler = 24,
    Operation = 25,
    Lifetime = 26,
}

impl TokenType {
    /// Returns the LSP token type name.
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenType::Namespace => "namespace",
            TokenType::Type => "type",
            TokenType::Class => "class",
            TokenType::Enum => "enum",
            TokenType::Interface => "interface",
            TokenType::Struct => "struct",
            TokenType::TypeParameter => "typeParameter",
            TokenType::Parameter => "parameter",
            TokenType::Variable => "variable",
            TokenType::Property => "property",
            TokenType::EnumMember => "enumMember",
            TokenType::Event => "event",
            TokenType::Function => "function",
            TokenType::Method => "method",
            TokenType::Macro => "macro",
            TokenType::Keyword => "keyword",
            TokenType::Modifier => "modifier",
            TokenType::Comment => "comment",
            TokenType::String => "string",
            TokenType::Number => "number",
            TokenType::Regexp => "regexp",
            TokenType::Operator => "operator",
            TokenType::Decorator => "decorator",
            TokenType::Effect => "interface",  // Use interface styling for effects
            TokenType::Handler => "class",     // Use class styling for handlers
            TokenType::Operation => "method",  // Use method styling for operations
            TokenType::Lifetime => "label",    // Use label styling for lifetimes
        }
    }

    /// Returns all token types.
    pub fn all() -> Vec<SemanticTokenType> {
        vec![
            SemanticTokenType::NAMESPACE,
            SemanticTokenType::TYPE,
            SemanticTokenType::CLASS,
            SemanticTokenType::ENUM,
            SemanticTokenType::INTERFACE,
            SemanticTokenType::STRUCT,
            SemanticTokenType::TYPE_PARAMETER,
            SemanticTokenType::PARAMETER,
            SemanticTokenType::VARIABLE,
            SemanticTokenType::PROPERTY,
            SemanticTokenType::ENUM_MEMBER,
            SemanticTokenType::EVENT,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::METHOD,
            SemanticTokenType::MACRO,
            SemanticTokenType::KEYWORD,
            SemanticTokenType::MODIFIER,
            SemanticTokenType::COMMENT,
            SemanticTokenType::STRING,
            SemanticTokenType::NUMBER,
            SemanticTokenType::REGEXP,
            SemanticTokenType::OPERATOR,
            SemanticTokenType::DECORATOR,
        ]
    }
}

/// Semantic token modifiers for Blood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenModifier {
    Declaration = 0,
    Definition = 1,
    Readonly = 2,
    Static = 3,
    Deprecated = 4,
    Abstract = 5,
    Async = 6,
    Modification = 7,
    Documentation = 8,
    DefaultLibrary = 9,
    // Blood-specific
    Linear = 10,
    Pure = 11,
    Mutable = 12,
}

impl TokenModifier {
    /// Returns all token modifiers.
    pub fn all() -> Vec<SemanticTokenModifier> {
        vec![
            SemanticTokenModifier::DECLARATION,
            SemanticTokenModifier::DEFINITION,
            SemanticTokenModifier::READONLY,
            SemanticTokenModifier::STATIC,
            SemanticTokenModifier::DEPRECATED,
            SemanticTokenModifier::ABSTRACT,
            SemanticTokenModifier::ASYNC,
            SemanticTokenModifier::MODIFICATION,
            SemanticTokenModifier::DOCUMENTATION,
            SemanticTokenModifier::DEFAULT_LIBRARY,
        ]
    }

    /// Returns the bitmask for this modifier.
    pub fn bitmask(&self) -> u32 {
        1 << (*self as u32)
    }
}

/// Returns the semantic tokens legend.
pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TokenType::all(),
        token_modifiers: TokenModifier::all(),
    }
}

/// Provider for semantic tokens.
///
/// Uses bloodc's lexer for accurate tokenization matching the compiler.
pub struct SemanticTokensProvider {
    /// Blood type keywords (built-in types from stdlib).
    type_keywords: Vec<&'static str>,
}

impl SemanticTokensProvider {
    /// Creates a new semantic tokens provider.
    pub fn new() -> Self {
        Self {
            // Type keywords are used to distinguish standard library types
            type_keywords: vec![
                "i8", "i16", "i32", "i64", "i128", "isize",
                "u8", "u16", "u32", "u64", "u128", "usize",
                "f32", "f64", "bool", "char", "str",
                "Option", "Result", "Vec", "String", "Box",
            ],
        }
    }

    /// Provides semantic tokens for a document.
    ///
    /// Uses bloodc's lexer for accurate tokenization that matches the compiler.
    pub fn provide(&self, doc: &Document) -> SemanticTokens {
        let mut tokens: Vec<SemanticToken> = Vec::new();
        let text = doc.text();

        let mut prev_line = 0u32;
        let mut prev_char = 0u32;

        // Use bloodc's lexer for accurate tokenization
        let lexer = Lexer::new(&text);

        for token in lexer {
            // Skip EOF and whitespace-only spans
            if matches!(token.kind, TokenKind::Eof) {
                continue;
            }

            // Map bloodc TokenKind to LSP semantic token type
            let (token_type, modifiers) = self.map_token_kind(&token.kind, &text[token.span.start..token.span.end]);

            // Skip tokens we don't want to highlight (operators, punctuation, etc.)
            if token_type.is_none() {
                continue;
            }

            let token_type = token_type.unwrap();
            let length = (token.span.end - token.span.start) as u32;

            self.add_token(
                &mut tokens,
                token.span.start_line as u32,
                token.span.start_col as u32,
                length,
                token_type,
                modifiers,
                &mut prev_line,
                &mut prev_char,
            );
        }

        SemanticTokens {
            result_id: None,
            data: tokens,
        }
    }

    /// Maps a bloodc TokenKind to an LSP semantic token type.
    ///
    /// Returns None for tokens that shouldn't be highlighted (punctuation, operators).
    fn map_token_kind(&self, kind: &TokenKind, text: &str) -> (Option<u32>, u32) {
        match kind {
            // Keywords
            TokenKind::As | TokenKind::Break | TokenKind::Const | TokenKind::Continue
            | TokenKind::Else | TokenKind::Enum | TokenKind::False | TokenKind::Fn
            | TokenKind::For | TokenKind::If | TokenKind::Impl | TokenKind::In
            | TokenKind::Let | TokenKind::Loop | TokenKind::Match | TokenKind::Mod
            | TokenKind::Module | TokenKind::Move | TokenKind::Mut | TokenKind::Pub
            | TokenKind::Ref | TokenKind::Return | TokenKind::SelfLower | TokenKind::SelfUpper
            | TokenKind::Static | TokenKind::Struct | TokenKind::Super | TokenKind::Trait
            | TokenKind::True | TokenKind::Type | TokenKind::Use | TokenKind::Where
            | TokenKind::While | TokenKind::Yield | TokenKind::Async | TokenKind::Await
            | TokenKind::Dyn | TokenKind::Crate | TokenKind::Bridge | TokenKind::Extern => {
                (Some(TokenType::Keyword as u32), 0)
            }

            // Effect-related keywords
            TokenKind::Effect | TokenKind::Handler | TokenKind::Perform
            | TokenKind::Resume | TokenKind::With | TokenKind::Handle
            | TokenKind::Deep | TokenKind::Shallow | TokenKind::Pure => {
                let modifier = if matches!(kind, TokenKind::Pure) {
                    TokenModifier::Pure.bitmask()
                } else {
                    0
                };
                (Some(TokenType::Keyword as u32), modifier)
            }

            // Numeric literals
            TokenKind::IntLit | TokenKind::FloatLit => {
                (Some(TokenType::Number as u32), 0)
            }

            // String and character literals
            TokenKind::StringLit | TokenKind::CharLit => {
                (Some(TokenType::String as u32), 0)
            }

            // Comments (if lexer preserves them - currently skipped)
            TokenKind::LineComment | TokenKind::BlockComment => {
                (Some(TokenType::Comment as u32), 0)
            }

            // Identifiers
            TokenKind::Ident => {
                // Check if it's a known type keyword
                if self.type_keywords.contains(&text) {
                    (Some(TokenType::Type as u32), TokenModifier::DefaultLibrary.bitmask())
                } else {
                    (Some(TokenType::Variable as u32), 0)
                }
            }

            // Type identifiers (uppercase)
            TokenKind::TypeIdent => {
                if self.type_keywords.contains(&text) {
                    (Some(TokenType::Type as u32), TokenModifier::DefaultLibrary.bitmask())
                } else {
                    (Some(TokenType::Type as u32), 0)
                }
            }

            // Lifetime identifiers
            TokenKind::Lifetime => {
                (Some(TokenType::Lifetime as u32), 0)
            }

            // Attributes/decorators
            TokenKind::AtUnsafe | TokenKind::AtHeap | TokenKind::AtStack => {
                (Some(TokenType::Decorator as u32), 0)
            }

            // Operators and punctuation - don't highlight
            _ => (None, 0),
        }
    }

    /// Adds a token with delta encoding.
    #[allow(clippy::too_many_arguments)]
    fn add_token(
        &self,
        tokens: &mut Vec<SemanticToken>,
        line: u32,
        char: u32,
        length: u32,
        token_type: u32,
        token_modifiers: u32,
        prev_line: &mut u32,
        prev_char: &mut u32,
    ) {
        let delta_line = line - *prev_line;
        let delta_start = if delta_line == 0 {
            char - *prev_char
        } else {
            char
        };

        tokens.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: token_modifiers,
        });

        *prev_line = line;
        *prev_char = char;
    }
}

impl Default for SemanticTokensProvider {
    fn default() -> Self {
        Self::new()
    }
}
