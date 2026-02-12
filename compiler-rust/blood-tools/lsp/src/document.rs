//! Document Management
//!
//! Represents open text documents and handles incremental updates.

use ropey::Rope;
use tower_lsp::lsp_types::*;

/// An open text document in the editor.
#[derive(Debug, Clone)]
pub struct Document {
    /// The document URI.
    uri: Url,
    /// Document version (increments on each change).
    version: i32,
    /// The document content as a rope for efficient editing.
    content: Rope,
    /// Cached line offsets for position conversion.
    line_offsets: Vec<usize>,
}

impl Document {
    /// Creates a new document from initial text.
    pub fn new(uri: Url, version: i32, text: String) -> Self {
        let content = Rope::from_str(&text);
        let line_offsets = Self::compute_line_offsets(&content);

        Self {
            uri,
            version,
            content,
            line_offsets,
        }
    }

    /// Returns the document URI.
    pub fn uri(&self) -> &Url {
        &self.uri
    }

    /// Returns the document version.
    pub fn version(&self) -> i32 {
        self.version
    }

    /// Returns the full document text.
    pub fn text(&self) -> String {
        self.content.to_string()
    }

    /// Returns a slice of the document text.
    pub fn slice(&self, start: usize, end: usize) -> String {
        self.content.slice(start..end).to_string()
    }

    /// Returns the number of lines in the document.
    pub fn line_count(&self) -> usize {
        self.content.len_lines()
    }

    /// Returns the text of a specific line.
    pub fn line(&self, line_idx: usize) -> Option<String> {
        if line_idx < self.content.len_lines() {
            Some(self.content.line(line_idx).to_string())
        } else {
            None
        }
    }

    /// Applies a text change to the document.
    pub fn apply_change(&mut self, version: i32, change: TextDocumentContentChangeEvent) {
        self.version = version;

        match change.range {
            Some(range) => {
                // Incremental change
                let start_offset = self.position_to_offset(range.start);
                let end_offset = self.position_to_offset(range.end);

                if let (Some(start), Some(end)) = (start_offset, end_offset) {
                    self.content.remove(start..end);
                    self.content.insert(start, &change.text);
                }
            }
            None => {
                // Full document replacement
                self.content = Rope::from_str(&change.text);
            }
        }

        // Recompute line offsets after change
        self.line_offsets = Self::compute_line_offsets(&self.content);
    }

    /// Converts an LSP position to a byte offset.
    pub fn position_to_offset(&self, position: Position) -> Option<usize> {
        let line_idx = position.line as usize;
        let char_idx = position.character as usize;

        if line_idx >= self.content.len_lines() {
            return None;
        }

        let line_start = self.content.line_to_char(line_idx);
        let line = self.content.line(line_idx);
        let line_len = line.len_chars();

        // Clamp character index to line length
        let char_idx = char_idx.min(line_len);
        let char_offset = line_start + char_idx;

        Some(self.content.char_to_byte(char_offset))
    }

    /// Converts a byte offset to an LSP position.
    pub fn offset_to_position(&self, offset: usize) -> Position {
        let char_idx = self.content.byte_to_char(offset.min(self.content.len_bytes()));
        let line_idx = self.content.char_to_line(char_idx);
        let line_start = self.content.line_to_char(line_idx);
        let character = char_idx - line_start;

        Position {
            line: line_idx as u32,
            character: character as u32,
        }
    }

    /// Returns the word at the given position.
    pub fn word_at_position(&self, position: Position) -> Option<WordInfo> {
        let offset = self.position_to_offset(position)?;
        let char_idx = self.content.byte_to_char(offset);

        // Find word boundaries
        let mut start = char_idx;
        let mut end = char_idx;

        // Scan backwards for word start
        while start > 0 {
            let prev_char = self.content.char(start - 1);
            if !is_word_char(prev_char) {
                break;
            }
            start -= 1;
        }

        // Scan forwards for word end
        while end < self.content.len_chars() {
            let next_char = self.content.char(end);
            if !is_word_char(next_char) {
                break;
            }
            end += 1;
        }

        if start == end {
            return None;
        }

        let word: String = self.content.slice(start..end).chars().collect();
        let start_pos = self.offset_to_position(self.content.char_to_byte(start));
        let end_pos = self.offset_to_position(self.content.char_to_byte(end));

        Some(WordInfo {
            text: word,
            range: Range {
                start: start_pos,
                end: end_pos,
            },
        })
    }

    /// Returns the identifier path at the given position (e.g., "foo.bar.baz").
    pub fn identifier_path_at_position(&self, position: Position) -> Option<IdentifierPath> {
        let offset = self.position_to_offset(position)?;
        let char_idx = self.content.byte_to_char(offset);

        // Find the full path including dots and double colons
        let mut start = char_idx;
        let mut end = char_idx;

        // Scan backwards
        while start > 0 {
            let prev_char = self.content.char(start - 1);
            if is_word_char(prev_char) || prev_char == '.' || prev_char == ':' {
                start -= 1;
            } else {
                break;
            }
        }

        // Scan forwards
        while end < self.content.len_chars() {
            let next_char = self.content.char(end);
            if is_word_char(next_char) || next_char == '.' || next_char == ':' {
                end += 1;
            } else {
                break;
            }
        }

        if start == end {
            return None;
        }

        let full_path: String = self.content.slice(start..end).chars().collect();

        // Split into segments
        let segments: Vec<String> = full_path
            .split(['.', ':'])
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        if segments.is_empty() {
            return None;
        }

        Some(IdentifierPath {
            full_path,
            segments,
        })
    }

    /// Computes line offsets for efficient position conversion.
    fn compute_line_offsets(rope: &Rope) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(rope.len_lines());
        let mut current = 0;

        for line in rope.lines() {
            offsets.push(current);
            current += line.len_bytes();
        }

        offsets
    }
}

/// Information about a word in the document.
#[derive(Debug, Clone)]
pub struct WordInfo {
    /// The word text.
    pub text: String,
    /// The range of the word in the document.
    pub range: Range,
}

/// A path of identifiers (e.g., "std::io::File").
#[derive(Debug, Clone)]
pub struct IdentifierPath {
    /// The full path as a string.
    pub full_path: String,
    /// Individual path segments.
    pub segments: Vec<String>,
}

/// Checks if a character is part of a Blood identifier.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_creation() {
        let uri = Url::parse("file:///test.blood").unwrap();
        let doc = Document::new(uri.clone(), 1, "fn main() {\n    42\n}".to_string());

        assert_eq!(doc.version(), 1);
        assert_eq!(doc.line_count(), 3);
    }

    #[test]
    fn test_position_conversion() {
        let uri = Url::parse("file:///test.blood").unwrap();
        let doc = Document::new(uri, 1, "fn main() {\n    42\n}".to_string());

        let pos = Position {
            line: 1,
            character: 4,
        };
        let offset = doc.position_to_offset(pos).unwrap();
        let back = doc.offset_to_position(offset);

        assert_eq!(pos, back);
    }

    #[test]
    fn test_word_at_position() {
        let uri = Url::parse("file:///test.blood").unwrap();
        let doc = Document::new(uri, 1, "let foo = 42".to_string());

        let word = doc
            .word_at_position(Position {
                line: 0,
                character: 5,
            })
            .unwrap();

        assert_eq!(word.text, "foo");
    }
}
