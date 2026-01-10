//! Blood Code Formatter
//!
//! A formatter for Blood source files that enforces consistent code style.
//!
//! # Features
//!
//! - Consistent indentation (4 spaces by default)
//! - Proper spacing around operators
//! - Effect annotation alignment
//! - Handler block formatting
//! - Comment preservation
//! - Configurable line width
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_fmt::{format_source, Config};
//!
//! let source = "fn main(){let x=1+2}";
//! let config = Config::default();
//! let formatted = format_source(source, &config)?;
//! assert_eq!(formatted, "fn main() {\n    let x = 1 + 2\n}\n");
//! ```

pub mod config;
pub mod formatter;
pub mod printer;
pub mod tokens;

pub use config::Config;
pub use formatter::Formatter;

use thiserror::Error;

/// Errors that can occur during formatting.
#[derive(Debug, Error)]
pub enum FormatError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Result type for formatting operations.
pub type FormatResult<T> = Result<T, FormatError>;

/// Formats Blood source code using default configuration.
pub fn format_source(source: &str) -> FormatResult<String> {
    format_source_with_config(source, &Config::default())
}

/// Formats Blood source code using the provided configuration.
pub fn format_source_with_config(source: &str, config: &Config) -> FormatResult<String> {
    let formatter = Formatter::new(config.clone());
    formatter.format(source)
}

/// Checks if source code is already formatted.
pub fn check_formatted(source: &str) -> FormatResult<bool> {
    check_formatted_with_config(source, &Config::default())
}

/// Checks if source code is already formatted with the given config.
pub fn check_formatted_with_config(source: &str, config: &Config) -> FormatResult<bool> {
    let formatted = format_source_with_config(source, config)?;
    Ok(source == formatted)
}

/// Computes the diff between original and formatted source.
pub fn format_diff(source: &str) -> FormatResult<Option<String>> {
    format_diff_with_config(source, &Config::default())
}

/// Computes the diff between original and formatted source with config.
pub fn format_diff_with_config(source: &str, config: &Config) -> FormatResult<Option<String>> {
    let formatted = format_source_with_config(source, config)?;

    if source == formatted {
        return Ok(None);
    }

    // Simple line-by-line diff
    let mut diff = String::new();
    let original_lines: Vec<&str> = source.lines().collect();
    let formatted_lines: Vec<&str> = formatted.lines().collect();

    let max_lines = original_lines.len().max(formatted_lines.len());

    for i in 0..max_lines {
        let orig = original_lines.get(i).copied().unwrap_or("");
        let fmt = formatted_lines.get(i).copied().unwrap_or("");

        if orig != fmt {
            if !orig.is_empty() {
                diff.push_str(&format!("-{}: {}\n", i + 1, orig));
            }
            if !fmt.is_empty() {
                diff.push_str(&format!("+{}: {}\n", i + 1, fmt));
            }
        }
    }

    Ok(Some(diff))
}
