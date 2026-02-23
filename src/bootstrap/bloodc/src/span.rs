//! Source span and location tracking.
//!
//! This module provides types for tracking source locations throughout
//! the compilation pipeline.

use serde::{Deserialize, Serialize};

/// A precomputed index of line start positions for O(log n) line/column lookup.
///
/// This avoids the O(n²) behavior of scanning from the start for each token.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offsets where each line starts. line_starts[0] = 0 (line 1 starts at byte 0).
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Build a line index from source code. O(n) one-time cost.
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0]; // Line 1 starts at byte 0
        for (offset, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push(offset + 1); // Next line starts after the newline
            }
        }
        Self { line_starts }
    }

    /// Look up line and column for a byte offset. O(log n) via binary search.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        // Binary search for the line containing this offset
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,      // Exact match - offset is at start of a line
            Err(idx) => idx - 1, // offset is between line_starts[idx-1] and line_starts[idx]
        };
        let line = (line_idx + 1) as u32; // 1-indexed
        let col = (offset - self.line_starts[line_idx] + 1) as u32; // 1-indexed
        (line, col)
    }
}

/// A span representing a contiguous region in source code.
///
/// Spans are byte offsets into the source text, along with cached
/// line/column information for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    /// Byte offset of the start (inclusive).
    pub start: usize,
    /// Byte offset of the end (exclusive).
    pub end: usize,
    /// 1-indexed line number of the start.
    pub start_line: u32,
    /// 1-indexed column number of the start.
    pub start_col: u32,
}

impl Span {
    /// Create a new span.
    pub fn new(start: usize, end: usize, start_line: u32, start_col: u32) -> Self {
        Self {
            start,
            end,
            start_line,
            start_col,
        }
    }

    /// Create a dummy span for synthesized code.
    pub fn dummy() -> Self {
        Self {
            start: 0,
            end: 0,
            start_line: 0,
            start_col: 0,
        }
    }

    /// The length of the span in bytes.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the span is empty.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Merge two spans into one that covers both.
    pub fn merge(self, other: Span) -> Span {
        let start = self.start.min(other.start);
        let end = self.end.max(other.end);
        let (start_line, start_col) = if self.start <= other.start {
            (self.start_line, self.start_col)
        } else {
            (other.start_line, other.start_col)
        };
        Span {
            start,
            end,
            start_line,
            start_col,
        }
    }

    /// Create a span from a logos span and source for line info.
    pub fn from_logos(span: logos::Span, source: &str) -> Self {
        let (line, col) = line_col(source, span.start);
        Self {
            start: span.start,
            end: span.end,
            start_line: line,
            start_col: col,
        }
    }
}

impl Default for Span {
    fn default() -> Self {
        Self::dummy()
    }
}

/// A value with an associated span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }
}

impl<T> std::ops::Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

/// Calculate line and column from byte offset.
/// Returns 1-indexed line and column numbers.
pub fn line_col(source: &str, offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 1u32;
    let mut current = 0;

    for ch in source.chars() {
        if current >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        current += ch.len_utf8();
    }

    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_col() {
        let source = "fn main() {\n    let x = 1;\n}";
        assert_eq!(line_col(source, 0), (1, 1)); // 'f'
        assert_eq!(line_col(source, 3), (1, 4)); // 'm'
        assert_eq!(line_col(source, 12), (2, 1)); // first char of line 2
        assert_eq!(line_col(source, 16), (2, 5)); // 'l'
    }

    #[test]
    fn test_span_merge() {
        let s1 = Span::new(0, 5, 1, 1);
        let s2 = Span::new(10, 15, 1, 11);
        let merged = s1.merge(s2);
        assert_eq!(merged.start, 0);
        assert_eq!(merged.end, 15);
    }
}
