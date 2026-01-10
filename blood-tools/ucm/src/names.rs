//! Name Management
//!
//! Handles hierarchical names for definitions.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A hierarchical name for a definition.
///
/// Names are dot-separated paths like `std.io.File` or `my_project.utils.parse`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Name {
    segments: Vec<String>,
}

impl Name {
    /// Creates a new name from a string.
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        let segments = s.split('.').map(String::from).collect();
        Self { segments }
    }

    /// Creates a name from segments.
    pub fn from_segments(segments: Vec<String>) -> Self {
        Self { segments }
    }

    /// Returns the segments of this name.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Returns the last segment (the "local" name).
    pub fn local(&self) -> &str {
        self.segments.last().map(|s| s.as_str()).unwrap_or("")
    }

    /// Returns the parent namespace, if any.
    pub fn parent(&self) -> Option<Name> {
        if self.segments.len() > 1 {
            Some(Name::from_segments(
                self.segments[..self.segments.len() - 1].to_vec(),
            ))
        } else {
            None
        }
    }

    /// Returns a child name by appending a segment.
    pub fn child(&self, segment: impl Into<String>) -> Name {
        let mut segments = self.segments.clone();
        segments.push(segment.into());
        Name::from_segments(segments)
    }

    /// Checks if this name starts with the given prefix.
    pub fn starts_with(&self, prefix: &Name) -> bool {
        if prefix.segments.len() > self.segments.len() {
            return false;
        }
        self.segments[..prefix.segments.len()] == prefix.segments
    }

    /// Returns the name without the prefix, if it starts with the prefix.
    pub fn strip_prefix(&self, prefix: &Name) -> Option<Name> {
        if self.starts_with(prefix) {
            Some(Name::from_segments(
                self.segments[prefix.segments.len()..].to_vec(),
            ))
        } else {
            None
        }
    }

    /// Returns the depth (number of segments) of this name.
    pub fn depth(&self) -> usize {
        self.segments.len()
    }

    /// Returns true if this is a root-level name (single segment).
    pub fn is_root(&self) -> bool {
        self.segments.len() == 1
    }

    /// Converts to a path-like string (using `/` separator).
    pub fn to_path(&self) -> String {
        self.segments.join("/")
    }

    /// Creates from a path-like string.
    pub fn from_path(s: &str) -> Self {
        let segments = s.split('/').map(String::from).collect();
        Self { segments }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.segments.join("."))
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({})", self)
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Name::new(s)
    }
}

impl From<String> for Name {
    fn from(s: String) -> Self {
        Name::new(s)
    }
}

/// A namespace containing names and sub-namespaces.
#[derive(Debug, Clone, Default)]
pub struct Namespace {
    /// Direct children of this namespace
    children: std::collections::HashMap<String, NamespaceEntry>,
}

/// An entry in a namespace.
#[derive(Debug, Clone)]
pub enum NamespaceEntry {
    /// A definition (terminal)
    Definition(crate::Hash),
    /// A sub-namespace
    Namespace(Namespace),
    /// Both a definition and a sub-namespace (rare but possible)
    Both(crate::Hash, Namespace),
}

impl Namespace {
    /// Creates an empty namespace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a name to the namespace.
    pub fn add(&mut self, name: &Name, hash: crate::Hash) {
        self.add_segments(&name.segments, hash);
    }

    fn add_segments(&mut self, segments: &[String], hash: crate::Hash) {
        if segments.is_empty() {
            return;
        }

        let first = &segments[0];
        let rest = &segments[1..];

        if rest.is_empty() {
            // Terminal segment
            match self.children.get_mut(first) {
                Some(NamespaceEntry::Namespace(ns)) => {
                    self.children.insert(
                        first.clone(),
                        NamespaceEntry::Both(hash, ns.clone()),
                    );
                }
                _ => {
                    self.children
                        .insert(first.clone(), NamespaceEntry::Definition(hash));
                }
            }
        } else {
            // Non-terminal segment
            let entry = self
                .children
                .entry(first.clone())
                .or_insert_with(|| NamespaceEntry::Namespace(Namespace::new()));

            match entry {
                NamespaceEntry::Namespace(ns) => {
                    ns.add_segments(rest, hash);
                }
                NamespaceEntry::Definition(h) => {
                    let mut ns = Namespace::new();
                    ns.add_segments(rest, hash);
                    *entry = NamespaceEntry::Both(h.clone(), ns);
                }
                NamespaceEntry::Both(_, ns) => {
                    ns.add_segments(rest, hash);
                }
            }
        }
    }

    /// Looks up a name in the namespace.
    pub fn lookup(&self, name: &Name) -> Option<crate::Hash> {
        self.lookup_segments(&name.segments)
    }

    fn lookup_segments(&self, segments: &[String]) -> Option<crate::Hash> {
        if segments.is_empty() {
            return None;
        }

        let first = &segments[0];
        let rest = &segments[1..];

        match self.children.get(first)? {
            NamespaceEntry::Definition(h) if rest.is_empty() => Some(h.clone()),
            NamespaceEntry::Namespace(ns) if !rest.is_empty() => ns.lookup_segments(rest),
            NamespaceEntry::Both(h, _) if rest.is_empty() => Some(h.clone()),
            NamespaceEntry::Both(_, ns) if !rest.is_empty() => ns.lookup_segments(rest),
            _ => None,
        }
    }

    /// Returns all names in this namespace with the given prefix.
    pub fn list_with_prefix(&self, prefix: &Name) -> Vec<(Name, crate::Hash)> {
        let mut results = Vec::new();
        self.collect_names(&prefix.segments, prefix.clone(), &mut results);
        results
    }

    fn collect_names(
        &self,
        prefix: &[String],
        current: Name,
        results: &mut Vec<(Name, crate::Hash)>,
    ) {
        if prefix.is_empty() {
            // Collect all names under this namespace
            for (segment, entry) in &self.children {
                let name = current.child(segment);
                match entry {
                    NamespaceEntry::Definition(h) => {
                        results.push((name, h.clone()));
                    }
                    NamespaceEntry::Namespace(ns) => {
                        ns.collect_names(&[], name, results);
                    }
                    NamespaceEntry::Both(h, ns) => {
                        results.push((name.clone(), h.clone()));
                        ns.collect_names(&[], name, results);
                    }
                }
            }
        } else {
            // Navigate to the prefix first
            let first = &prefix[0];
            let rest = &prefix[1..];

            if let Some(entry) = self.children.get(first) {
                let name = current.child(first);
                match entry {
                    NamespaceEntry::Namespace(ns) => {
                        ns.collect_names(rest, name, results);
                    }
                    NamespaceEntry::Both(_, ns) => {
                        ns.collect_names(rest, name, results);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_creation() {
        let name = Name::new("std.io.File");
        assert_eq!(name.segments(), &["std", "io", "File"]);
        assert_eq!(name.local(), "File");
    }

    #[test]
    fn test_name_parent() {
        let name = Name::new("std.io.File");
        let parent = name.parent().unwrap();
        assert_eq!(parent.to_string(), "std.io");
    }

    #[test]
    fn test_name_child() {
        let name = Name::new("std.io");
        let child = name.child("File");
        assert_eq!(child.to_string(), "std.io.File");
    }

    #[test]
    fn test_namespace() {
        use crate::Hash;

        let mut ns = Namespace::new();
        let h1 = Hash::of_str("file");
        let h2 = Hash::of_str("reader");

        ns.add(&Name::new("std.io.File"), h1.clone());
        ns.add(&Name::new("std.io.Reader"), h2.clone());

        assert_eq!(ns.lookup(&Name::new("std.io.File")), Some(h1));
        assert_eq!(ns.lookup(&Name::new("std.io.Reader")), Some(h2));
        assert_eq!(ns.lookup(&Name::new("std.io.Writer")), None);
    }
}
