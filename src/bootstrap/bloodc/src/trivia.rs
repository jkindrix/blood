//! Trivia preservation infrastructure for lossless syntax trees.
//!
//! Trivia represents tokens that don't affect the semantics of a program
//! but are important for formatting and source reconstruction:
//!
//! - Whitespace (spaces, tabs, newlines)
//! - Comments (line comments, block comments, doc comments)
//!
//! # Architecture
//!
//! In a lossless syntax tree (CST), each meaningful token can have
//! associated leading and trailing trivia:
//!
//! ```text
//! [leading trivia] TOKEN [trailing trivia]
//! ```
//!
//! For example:
//! ```text
//! "// comment\n  fn" -> Token{fn} with leading trivia [Comment, Whitespace]
//! ```
//!
//! # Usage
//!
//! This module provides the infrastructure for trivia handling. The actual
//! trivia collection happens in the lexer, and attachment happens during parsing.
//!
//! ```rust,ignore
//! use bloodc::trivia::{Trivia, TriviaKind, TriviaList};
//!
//! let trivia = Trivia::new(TriviaKind::Whitespace, 0..2);
//! let list = TriviaList::new(vec![trivia]);
//! ```

use crate::span::Span;

/// The kind of trivia token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriviaKind {
    /// Whitespace (spaces, tabs).
    Whitespace,
    /// Newline character(s).
    Newline,
    /// Line comment starting with `//`.
    LineComment,
    /// Block comment `/* ... */`.
    BlockComment,
    /// Doc comment starting with `///` or `//!`.
    DocComment,
}

impl TriviaKind {
    /// Returns true if this trivia kind is a comment.
    #[inline]
    pub const fn is_comment(self) -> bool {
        matches!(
            self,
            TriviaKind::LineComment | TriviaKind::BlockComment | TriviaKind::DocComment
        )
    }

    /// Returns true if this trivia kind is whitespace (including newlines).
    #[inline]
    pub const fn is_whitespace(self) -> bool {
        matches!(self, TriviaKind::Whitespace | TriviaKind::Newline)
    }

    /// Returns true if this trivia kind represents a documentation comment.
    #[inline]
    pub const fn is_doc_comment(self) -> bool {
        matches!(self, TriviaKind::DocComment)
    }
}

/// A single piece of trivia (whitespace or comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trivia {
    /// The kind of trivia.
    pub kind: TriviaKind,
    /// The span in the source code.
    pub span: Span,
}

impl Trivia {
    /// Create a new trivia element.
    pub fn new(kind: TriviaKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Get the length of this trivia in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.span.end - self.span.start
    }

    /// Returns true if this trivia has zero length.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.span.start == self.span.end
    }
}

/// A list of trivia elements.
///
/// Trivia lists are typically attached to tokens as either leading or
/// trailing trivia. Leading trivia appears before a token, trailing
/// trivia appears after.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TriviaList {
    /// The trivia elements.
    elements: Vec<Trivia>,
}

impl TriviaList {
    /// Create a new empty trivia list.
    pub const fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }

    /// Create a trivia list from a vector of trivia.
    pub fn from_vec(elements: Vec<Trivia>) -> Self {
        Self { elements }
    }

    /// Add trivia to the list.
    pub fn push(&mut self, trivia: Trivia) {
        self.elements.push(trivia);
    }

    /// Returns true if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Get the number of trivia elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Iterate over the trivia elements.
    pub fn iter(&self) -> impl Iterator<Item = &Trivia> {
        self.elements.iter()
    }

    /// Get all comments from this trivia list.
    pub fn comments(&self) -> impl Iterator<Item = &Trivia> {
        self.elements.iter().filter(|t| t.kind.is_comment())
    }

    /// Get doc comments from this trivia list.
    pub fn doc_comments(&self) -> impl Iterator<Item = &Trivia> {
        self.elements.iter().filter(|t| t.kind.is_doc_comment())
    }

    /// Calculate the total byte length of all trivia.
    pub fn total_len(&self) -> usize {
        self.elements.iter().map(|t| t.len()).sum()
    }

    /// Get the combined span of all trivia, or None if empty.
    pub fn span(&self) -> Option<Span> {
        if self.elements.is_empty() {
            return None;
        }
        let first = &self.elements[0];
        let last = &self.elements[self.elements.len() - 1];
        Some(first.span.merge(last.span))
    }

    /// Drain all elements from this list.
    pub fn drain(&mut self) -> impl Iterator<Item = Trivia> + '_ {
        self.elements.drain(..)
    }

    /// Take all elements, leaving the list empty.
    pub fn take(&mut self) -> Vec<Trivia> {
        std::mem::take(&mut self.elements)
    }
}

impl IntoIterator for TriviaList {
    type Item = Trivia;
    type IntoIter = std::vec::IntoIter<Trivia>;

    fn into_iter(self) -> Self::IntoIter {
        self.elements.into_iter()
    }
}

impl<'a> IntoIterator for &'a TriviaList {
    type Item = &'a Trivia;
    type IntoIter = std::slice::Iter<'a, Trivia>;

    fn into_iter(self) -> Self::IntoIter {
        self.elements.iter()
    }
}

impl FromIterator<Trivia> for TriviaList {
    fn from_iter<I: IntoIterator<Item = Trivia>>(iter: I) -> Self {
        Self {
            elements: iter.into_iter().collect(),
        }
    }
}

impl Extend<Trivia> for TriviaList {
    fn extend<I: IntoIterator<Item = Trivia>>(&mut self, iter: I) {
        self.elements.extend(iter);
    }
}

/// A token with attached trivia.
///
/// This is used when building a lossless syntax tree where we need to
/// preserve all whitespace and comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriviaToken<T> {
    /// Leading trivia (before the token).
    pub leading: TriviaList,
    /// The actual token.
    pub token: T,
    /// Trailing trivia (after the token).
    pub trailing: TriviaList,
}

impl<T> TriviaToken<T> {
    /// Create a new token with empty trivia.
    pub fn new(token: T) -> Self {
        Self {
            leading: TriviaList::new(),
            token,
            trailing: TriviaList::new(),
        }
    }

    /// Create a token with leading trivia.
    pub fn with_leading(token: T, leading: TriviaList) -> Self {
        Self {
            leading,
            token,
            trailing: TriviaList::new(),
        }
    }

    /// Create a token with trailing trivia.
    pub fn with_trailing(token: T, trailing: TriviaList) -> Self {
        Self {
            leading: TriviaList::new(),
            token,
            trailing,
        }
    }

    /// Create a token with both leading and trailing trivia.
    pub fn with_trivia(token: T, leading: TriviaList, trailing: TriviaList) -> Self {
        Self {
            leading,
            token,
            trailing,
        }
    }

    /// Get a reference to the inner token.
    pub fn inner(&self) -> &T {
        &self.token
    }

    /// Get a mutable reference to the inner token.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.token
    }

    /// Extract the inner token, discarding trivia.
    pub fn into_inner(self) -> T {
        self.token
    }

    /// Check if there's any leading trivia.
    pub fn has_leading_trivia(&self) -> bool {
        !self.leading.is_empty()
    }

    /// Check if there's any trailing trivia.
    pub fn has_trailing_trivia(&self) -> bool {
        !self.trailing.is_empty()
    }

    /// Get leading doc comments.
    pub fn leading_doc_comments(&self) -> impl Iterator<Item = &Trivia> {
        self.leading.doc_comments()
    }
}

/// Attachment strategy for trivia.
///
/// Determines whether trivia is attached as leading or trailing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriviaAttachment {
    /// Trivia attaches to the following token (leading).
    #[default]
    Leading,
    /// Trivia attaches to the preceding token (trailing).
    Trailing,
    /// Newlines attach to preceding, other trivia to following.
    NewlineTrailing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trivia_kind_classification() {
        assert!(TriviaKind::LineComment.is_comment());
        assert!(TriviaKind::BlockComment.is_comment());
        assert!(TriviaKind::DocComment.is_comment());
        assert!(!TriviaKind::Whitespace.is_comment());

        assert!(TriviaKind::Whitespace.is_whitespace());
        assert!(TriviaKind::Newline.is_whitespace());
        assert!(!TriviaKind::LineComment.is_whitespace());

        assert!(TriviaKind::DocComment.is_doc_comment());
        assert!(!TriviaKind::LineComment.is_doc_comment());
    }

    #[test]
    fn test_trivia_list() {
        let mut list = TriviaList::new();
        assert!(list.is_empty());

        list.push(Trivia::new(TriviaKind::Whitespace, Span::new(0, 2, 1, 1)));
        list.push(Trivia::new(TriviaKind::LineComment, Span::new(2, 10, 1, 3)));

        assert_eq!(list.len(), 2);
        assert_eq!(list.comments().count(), 1);
    }

    #[test]
    fn test_trivia_token() {
        let token = TriviaToken::new("fn");
        assert!(!token.has_leading_trivia());
        assert!(!token.has_trailing_trivia());
        assert_eq!(*token.inner(), "fn");

        let leading = TriviaList::from_vec(vec![
            Trivia::new(TriviaKind::Whitespace, Span::new(0, 2, 1, 1)),
        ]);
        let token = TriviaToken::with_leading("fn", leading);
        assert!(token.has_leading_trivia());
    }
}
