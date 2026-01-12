//! Semantic Tokens Provider
//!
//! Provides semantic highlighting for Blood source files.

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
pub struct SemanticTokensProvider {
    /// Blood keywords.
    keywords: Vec<&'static str>,
    /// Blood effect keywords.
    effect_keywords: Vec<&'static str>,
    /// Blood type keywords.
    type_keywords: Vec<&'static str>,
}

impl SemanticTokensProvider {
    /// Creates a new semantic tokens provider.
    pub fn new() -> Self {
        Self {
            keywords: vec![
                "fn", "let", "mut", "if", "else", "match", "loop", "while", "for", "in",
                "break", "continue", "return", "struct", "enum", "trait", "impl", "type",
                "where", "pub", "mod", "use", "as", "self", "Self", "super", "crate",
                "const", "static", "async", "await", "move", "ref", "true", "false",
            ],
            effect_keywords: vec![
                "effect", "handler", "perform", "resume", "handle", "with", "deep", "shallow",
                "pure", "linear",
            ],
            type_keywords: vec![
                "i8", "i16", "i32", "i64", "i128", "isize",
                "u8", "u16", "u32", "u64", "u128", "usize",
                "f32", "f64", "bool", "char", "str",
                "Option", "Result", "Vec", "String", "Box",
            ],
        }
    }

    /// Provides semantic tokens for a document.
    pub fn provide(&self, doc: &Document) -> SemanticTokens {
        let mut tokens: Vec<SemanticToken> = Vec::new();
        let text = doc.text();

        let mut prev_line = 0u32;
        let mut prev_char = 0u32;

        // Simple lexical tokenization
        // TODO: Integrate with bloodc parser for accurate semantic tokens
        for (line_idx, line) in text.lines().enumerate() {
            let line_num = line_idx as u32;
            let mut char_idx = 0u32;
            let chars: Vec<char> = line.chars().collect();

            while (char_idx as usize) < chars.len() {
                let remaining = &line[char_idx as usize..];

                // Skip whitespace
                if chars[char_idx as usize].is_whitespace() {
                    char_idx += 1;
                    continue;
                }

                // Comments
                if remaining.starts_with("//") {
                    let length = remaining.len() as u32;
                    self.add_token(
                        &mut tokens,
                        line_num,
                        char_idx,
                        length,
                        TokenType::Comment as u32,
                        0,
                        &mut prev_line,
                        &mut prev_char,
                    );
                    break; // Rest of line is comment
                }

                // String literals
                if chars[char_idx as usize] == '"' {
                    let start = char_idx;
                    char_idx += 1;
                    while (char_idx as usize) < chars.len() {
                        if chars[char_idx as usize] == '"'
                            && (char_idx == 0 || chars[(char_idx - 1) as usize] != '\\')
                        {
                            char_idx += 1;
                            break;
                        }
                        char_idx += 1;
                    }
                    self.add_token(
                        &mut tokens,
                        line_num,
                        start,
                        char_idx - start,
                        TokenType::String as u32,
                        0,
                        &mut prev_line,
                        &mut prev_char,
                    );
                    continue;
                }

                // Character literals
                if chars[char_idx as usize] == '\'' {
                    let start = char_idx;
                    char_idx += 1;
                    while (char_idx as usize) < chars.len() && chars[char_idx as usize] != '\'' {
                        char_idx += 1;
                    }
                    if (char_idx as usize) < chars.len() {
                        char_idx += 1;
                    }
                    self.add_token(
                        &mut tokens,
                        line_num,
                        start,
                        char_idx - start,
                        TokenType::String as u32,
                        0,
                        &mut prev_line,
                        &mut prev_char,
                    );
                    continue;
                }

                // Numbers
                if chars[char_idx as usize].is_ascii_digit() {
                    let start = char_idx;
                    while (char_idx as usize) < chars.len()
                        && (chars[char_idx as usize].is_ascii_alphanumeric()
                            || chars[char_idx as usize] == '_'
                            || chars[char_idx as usize] == '.')
                    {
                        char_idx += 1;
                    }
                    self.add_token(
                        &mut tokens,
                        line_num,
                        start,
                        char_idx - start,
                        TokenType::Number as u32,
                        0,
                        &mut prev_line,
                        &mut prev_char,
                    );
                    continue;
                }

                // Identifiers and keywords
                if chars[char_idx as usize].is_alphabetic() || chars[char_idx as usize] == '_' {
                    let start = char_idx;
                    while (char_idx as usize) < chars.len()
                        && (chars[char_idx as usize].is_alphanumeric()
                            || chars[char_idx as usize] == '_')
                    {
                        char_idx += 1;
                    }
                    let word = &line[start as usize..char_idx as usize];
                    let length = char_idx - start;

                    let (token_type, modifiers) = self.classify_word(word);
                    self.add_token(
                        &mut tokens,
                        line_num,
                        start,
                        length,
                        token_type,
                        modifiers,
                        &mut prev_line,
                        &mut prev_char,
                    );
                    continue;
                }

                // Operators and punctuation
                char_idx += 1;
            }
        }

        SemanticTokens {
            result_id: None,
            data: tokens,
        }
    }

    /// Classifies a word as a token type.
    fn classify_word(&self, word: &str) -> (u32, u32) {
        // Keywords
        if self.keywords.contains(&word) {
            return (TokenType::Keyword as u32, 0);
        }

        // Effect keywords
        if self.effect_keywords.contains(&word) {
            let modifier = if word == "pure" {
                TokenModifier::Pure.bitmask()
            } else {
                0
            };
            return (TokenType::Keyword as u32, modifier);
        }

        // Type keywords
        if self.type_keywords.contains(&word) {
            return (
                TokenType::Type as u32,
                TokenModifier::DefaultLibrary.bitmask(),
            );
        }

        // Uppercase = type
        if word.chars().next().is_some_and(|c| c.is_uppercase()) {
            return (TokenType::Type as u32, 0);
        }

        // Default to variable
        (TokenType::Variable as u32, 0)
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
