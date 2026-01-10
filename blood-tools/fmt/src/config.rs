//! Formatter Configuration
//!
//! Defines configuration options for the Blood formatter.

use serde::{Deserialize, Serialize};

/// Configuration for the Blood formatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Maximum line width before wrapping.
    pub max_width: usize,

    /// Indentation width in spaces.
    pub indent_width: usize,

    /// Use tabs instead of spaces for indentation.
    pub use_tabs: bool,

    /// Style configuration for various constructs.
    pub style: StyleConfig,

    /// Effect annotation configuration.
    pub effects: EffectConfig,

    /// Import organization configuration.
    pub imports: ImportConfig,

    /// Comment configuration.
    pub comments: CommentConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_width: 100,
            indent_width: 4,
            use_tabs: false,
            style: StyleConfig::default(),
            effects: EffectConfig::default(),
            imports: ImportConfig::default(),
            comments: CommentConfig::default(),
        }
    }
}

/// Style configuration for code formatting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleConfig {
    /// Place opening braces on the same line (K&R style).
    pub brace_same_line: bool,

    /// Add trailing commas in multi-line lists.
    pub trailing_comma: TrailingComma,

    /// Space before opening parenthesis in function definitions.
    pub space_before_fn_paren: bool,

    /// Space after colon in type annotations.
    pub space_after_colon: bool,

    /// Space before colon in type annotations.
    pub space_before_colon: bool,

    /// Blank lines between top-level items.
    pub blank_lines_between_items: usize,

    /// Format match arms with braces.
    pub match_arm_braces: MatchArmBraces,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            brace_same_line: true,
            trailing_comma: TrailingComma::Always,
            space_before_fn_paren: false,
            space_after_colon: true,
            space_before_colon: false,
            blank_lines_between_items: 1,
            match_arm_braces: MatchArmBraces::WhenNeeded,
        }
    }
}

/// Trailing comma policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailingComma {
    /// Never add trailing commas.
    Never,
    /// Add trailing commas in multi-line contexts only.
    Multiline,
    /// Always add trailing commas.
    Always,
}

/// Match arm brace policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchArmBraces {
    /// Never use braces around match arms.
    Never,
    /// Use braces when the arm body is multi-line or contains statements.
    WhenNeeded,
    /// Always use braces around match arms.
    Always,
}

/// Effect annotation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectConfig {
    /// Align effect annotations vertically.
    pub align_effects: bool,

    /// Space before the effect slash.
    pub space_before_slash: bool,

    /// Space after the effect slash.
    pub space_after_slash: bool,

    /// Format for multiple effects.
    pub multi_effect_style: MultiEffectStyle,
}

impl Default for EffectConfig {
    fn default() -> Self {
        Self {
            align_effects: false,
            space_before_slash: true,
            space_after_slash: true,
            multi_effect_style: MultiEffectStyle::Braces,
        }
    }
}

/// Style for multiple effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MultiEffectStyle {
    /// Use braces: `/ {Effect1, Effect2}`
    Braces,
    /// Use plus signs: `/ Effect1 + Effect2`
    Plus,
}

/// Import organization configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportConfig {
    /// Sort imports alphabetically.
    pub sort_imports: bool,

    /// Group imports by category.
    pub group_imports: bool,

    /// Merge imports from the same module.
    pub merge_imports: bool,

    /// Order of import groups.
    pub group_order: Vec<ImportGroup>,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            sort_imports: true,
            group_imports: true,
            merge_imports: true,
            group_order: vec![
                ImportGroup::Std,
                ImportGroup::External,
                ImportGroup::Crate,
                ImportGroup::Super,
                ImportGroup::Self_,
            ],
        }
    }
}

/// Import group categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportGroup {
    /// Standard library imports (`std::`)
    Std,
    /// External crate imports
    External,
    /// Current crate imports (`crate::`)
    Crate,
    /// Parent module imports (`super::`)
    Super,
    /// Current module imports (`self::`)
    #[serde(rename = "self")]
    Self_,
}

/// Comment formatting configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommentConfig {
    /// Wrap doc comments at max_width.
    pub wrap_doc_comments: bool,

    /// Normalize comment markers (ensure space after `//`).
    pub normalize_comments: bool,

    /// Preserve blank lines in comments.
    pub preserve_blank_lines: bool,
}

impl Default for CommentConfig {
    fn default() -> Self {
        Self {
            wrap_doc_comments: true,
            normalize_comments: true,
            preserve_blank_lines: true,
        }
    }
}

impl Config {
    /// Creates a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the indentation string based on configuration.
    pub fn indent_str(&self) -> String {
        if self.use_tabs {
            "\t".to_string()
        } else {
            " ".repeat(self.indent_width)
        }
    }

    /// Returns indentation at the given level.
    pub fn indent_at(&self, level: usize) -> String {
        self.indent_str().repeat(level)
    }
}
