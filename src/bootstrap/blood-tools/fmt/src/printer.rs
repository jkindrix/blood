//! Pretty Printer
//!
//! Handles output formatting with indentation management.

use crate::config::Config;

/// A pretty printer that manages indentation and output.
pub struct Printer<'a> {
    config: &'a Config,
    output: String,
    indent_level: usize,
    at_line_start: bool,
    last_char: Option<char>,
}

impl<'a> Printer<'a> {
    /// Creates a new printer with the given configuration.
    pub fn new(config: &'a Config) -> Self {
        Self {
            config,
            output: String::new(),
            indent_level: 0,
            at_line_start: true,
            last_char: None,
        }
    }

    /// Writes text to the output.
    pub fn write(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        // Add indentation if at line start
        if self.at_line_start && !text.starts_with('\n') {
            self.write_indent();
            self.at_line_start = false;
        }

        self.output.push_str(text);
        self.last_char = text.chars().last();
    }

    /// Writes a newline.
    pub fn newline(&mut self) {
        // Don't add multiple blank lines
        if !self.output.ends_with("\n\n") {
            self.output.push('\n');
            self.at_line_start = true;
            self.last_char = Some('\n');
        }
    }

    /// Writes a newline only if not already at line start.
    pub fn newline_if_needed(&mut self) {
        if !self.at_line_start {
            self.newline();
        }
    }

    /// Writes indentation at the current level.
    fn write_indent(&mut self) {
        self.output.push_str(&self.config.indent_at(self.indent_level));
    }

    /// Increases the indentation level.
    pub fn increase_indent(&mut self) {
        self.indent_level += 1;
    }

    /// Decreases the indentation level.
    pub fn decrease_indent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    /// Returns the current indentation level.
    pub fn indent_level(&self) -> usize {
        self.indent_level
    }

    /// Writes a space if the last character is not already a space.
    pub fn space(&mut self) {
        if self.last_char != Some(' ') && self.last_char != Some('\n') {
            self.write(" ");
        }
    }

    /// Writes a blank line (two newlines).
    pub fn blank_line(&mut self) {
        if !self.output.ends_with("\n\n") {
            if self.output.ends_with('\n') {
                self.output.push('\n');
            } else {
                self.output.push_str("\n\n");
            }
            self.at_line_start = true;
            self.last_char = Some('\n');
        }
    }

    /// Returns whether we're at the start of a line.
    pub fn at_line_start(&self) -> bool {
        self.at_line_start
    }

    /// Returns the last character written.
    pub fn last_char(&self) -> Option<char> {
        self.last_char
    }

    /// Returns the current output length.
    pub fn len(&self) -> usize {
        self.output.len()
    }

    /// Returns whether the output is empty.
    pub fn is_empty(&self) -> bool {
        self.output.is_empty()
    }

    /// Finishes printing and returns the output.
    pub fn finish(mut self) -> String {
        // Ensure file ends with a newline
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }

        // Remove trailing blank lines (keep one newline)
        while self.output.ends_with("\n\n") {
            self.output.pop();
        }

        self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_printing() {
        let config = Config::default();
        let mut printer = Printer::new(&config);

        printer.write("fn main()");
        printer.write(" {");
        printer.increase_indent();
        printer.newline();
        printer.write("42");
        printer.decrease_indent();
        printer.newline();
        printer.write("}");

        let output = printer.finish();
        assert!(output.contains("fn main()"));
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn test_indentation() {
        let config = Config::default();
        let mut printer = Printer::new(&config);

        printer.increase_indent();
        printer.newline();
        printer.write("indented");

        let output = printer.finish();
        assert!(output.contains("    indented")); // 4 spaces
    }
}
