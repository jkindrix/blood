//! # Namespace Management
//!
//! Maps human-readable names to content hashes.
//!
//! In Blood, names are metadata—the true identity of code is its content hash.
//! Namespaces provide a human-friendly way to refer to definitions.
//!
//! ## Example
//!
//! ```text
//! namespace main {
//!     add       → #a7f2k9m3xp
//!     subtract  → #b3c1xp5jht
//!     multiply  → #c4d2yr6kiu
//!
//!     type Point  → #e5f3zs7lmn
//!     type Vector → #f6g4at8mop
//!
//!     effect IO    → #g7h5bu9npq
//!     effect State → #h8i6cv0oqr
//! }
//! ```

use std::collections::HashMap;

use super::hash::ContentHash;

/// The kind of a name binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    /// A function or value definition.
    Value,
    /// A type definition.
    Type,
    /// An effect definition.
    Effect,
    /// A type alias.
    TypeAlias,
    /// A module.
    Module,
}

/// A binding from a name to a content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameBinding {
    /// The content hash this name refers to.
    pub hash: ContentHash,
    /// The kind of binding.
    pub kind: BindingKind,
    /// Whether this binding is public.
    pub is_public: bool,
    /// Optional documentation.
    pub doc: Option<String>,
}

impl NameBinding {
    /// Create a new value binding.
    pub fn value(hash: ContentHash) -> Self {
        Self {
            hash,
            kind: BindingKind::Value,
            is_public: true,
            doc: None,
        }
    }

    /// Create a new type binding.
    pub fn ty(hash: ContentHash) -> Self {
        Self {
            hash,
            kind: BindingKind::Type,
            is_public: true,
            doc: None,
        }
    }

    /// Create a new effect binding.
    pub fn effect(hash: ContentHash) -> Self {
        Self {
            hash,
            kind: BindingKind::Effect,
            is_public: true,
            doc: None,
        }
    }

    /// Set visibility.
    pub fn with_visibility(mut self, is_public: bool) -> Self {
        self.is_public = is_public;
        self
    }

    /// Set documentation.
    pub fn with_doc(mut self, doc: String) -> Self {
        self.doc = Some(doc);
        self
    }
}

/// A namespace containing name-to-hash mappings.
#[derive(Debug, Clone, Default)]
pub struct Namespace {
    /// The name of this namespace.
    name: String,
    /// Name bindings in this namespace.
    bindings: HashMap<String, NameBinding>,
    /// Child namespaces.
    children: HashMap<String, Namespace>,
    /// Parent namespace name (for qualified names).
    parent: Option<String>,
}

impl Namespace {
    /// Create a new root namespace.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bindings: HashMap::new(),
            children: HashMap::new(),
            parent: None,
        }
    }

    /// Create a child namespace.
    pub fn child(name: impl Into<String>, parent: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bindings: HashMap::new(),
            children: HashMap::new(),
            parent: Some(parent.into()),
        }
    }

    /// Get the name of this namespace.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the fully qualified name of this namespace.
    pub fn qualified_name(&self) -> String {
        match &self.parent {
            Some(parent) => format!("{}.{}", parent, self.name),
            None => self.name.clone(),
        }
    }

    /// Bind a name to a hash.
    pub fn bind(&mut self, name: impl Into<String>, binding: NameBinding) {
        self.bindings.insert(name.into(), binding);
    }

    /// Look up a name in this namespace.
    pub fn lookup(&self, name: &str) -> Option<&NameBinding> {
        self.bindings.get(name)
    }

    /// Look up a name by kind.
    pub fn lookup_kind(&self, name: &str, kind: BindingKind) -> Option<&NameBinding> {
        self.bindings.get(name).filter(|b| b.kind == kind)
    }

    /// Look up a value.
    pub fn lookup_value(&self, name: &str) -> Option<ContentHash> {
        self.lookup_kind(name, BindingKind::Value).map(|b| b.hash)
    }

    /// Look up a type.
    pub fn lookup_type(&self, name: &str) -> Option<ContentHash> {
        self.lookup_kind(name, BindingKind::Type).map(|b| b.hash)
    }

    /// Look up an effect.
    pub fn lookup_effect(&self, name: &str) -> Option<ContentHash> {
        self.lookup_kind(name, BindingKind::Effect).map(|b| b.hash)
    }

    /// Get or create a child namespace.
    pub fn get_or_create_child(&mut self, name: &str) -> &mut Namespace {
        let qualified = self.qualified_name();
        self.children
            .entry(name.to_string())
            .or_insert_with(|| Namespace::child(name, qualified))
    }

    /// Get a child namespace.
    pub fn get_child(&self, name: &str) -> Option<&Namespace> {
        self.children.get(name)
    }

    /// Get a child namespace mutably.
    pub fn get_child_mut(&mut self, name: &str) -> Option<&mut Namespace> {
        self.children.get_mut(name)
    }

    /// Iterate over all bindings.
    pub fn bindings(&self) -> impl Iterator<Item = (&str, &NameBinding)> {
        self.bindings.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over all public bindings.
    pub fn public_bindings(&self) -> impl Iterator<Item = (&str, &NameBinding)> {
        self.bindings
            .iter()
            .filter(|(_, v)| v.is_public)
            .map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over child namespaces.
    pub fn children(&self) -> impl Iterator<Item = (&str, &Namespace)> {
        self.children.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Remove a binding.
    pub fn unbind(&mut self, name: &str) -> Option<NameBinding> {
        self.bindings.remove(name)
    }

    /// Rename a binding (doesn't change the hash, only the name).
    pub fn rename(&mut self, old_name: &str, new_name: impl Into<String>) -> bool {
        if let Some(binding) = self.bindings.remove(old_name) {
            self.bindings.insert(new_name.into(), binding);
            true
        } else {
            false
        }
    }

    /// Find all names that map to a given hash.
    pub fn find_names(&self, hash: ContentHash) -> Vec<&str> {
        self.bindings
            .iter()
            .filter(|(_, b)| b.hash == hash)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Get the number of bindings in this namespace.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Check if the namespace is empty.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Merge another namespace into this one.
    /// Overwrites existing bindings with the same name.
    pub fn merge(&mut self, other: Namespace) {
        self.bindings.extend(other.bindings);
        for (name, child) in other.children {
            self.children.insert(name, child);
        }
    }

    /// Look up a qualified name (e.g., "std.io.println").
    pub fn lookup_qualified(&self, path: &str) -> Option<&NameBinding> {
        let parts: Vec<&str> = path.split('.').collect();
        self.lookup_path(&parts)
    }

    /// Look up a path of names.
    fn lookup_path(&self, path: &[&str]) -> Option<&NameBinding> {
        match path {
            [] => None,
            [name] => self.lookup(name),
            [child, rest @ ..] => self.children.get(*child)?.lookup_path(rest),
        }
    }
}

/// A collection of namespaces forming a codebase's name registry.
#[derive(Debug, Clone, Default)]
pub struct NameRegistry {
    /// Root namespaces.
    roots: HashMap<String, Namespace>,
    /// The current/default namespace.
    current: String,
}

impl NameRegistry {
    /// Create a new name registry with a default namespace.
    pub fn new() -> Self {
        let mut roots = HashMap::new();
        roots.insert("main".into(), Namespace::new("main"));
        Self {
            roots,
            current: "main".into(),
        }
    }

    /// Get or create a root namespace.
    pub fn get_or_create(&mut self, name: &str) -> &mut Namespace {
        self.roots
            .entry(name.to_string())
            .or_insert_with(|| Namespace::new(name))
    }

    /// Get a root namespace.
    pub fn get(&self, name: &str) -> Option<&Namespace> {
        self.roots.get(name)
    }

    /// Get the current namespace mutably.
    pub fn current_mut(&mut self) -> &mut Namespace {
        self.get_or_create(&self.current.clone())
    }

    /// Get the current namespace.
    pub fn current(&self) -> Option<&Namespace> {
        self.get(&self.current)
    }

    /// Set the current namespace.
    pub fn set_current(&mut self, name: impl Into<String>) {
        self.current = name.into();
    }

    /// Look up a name in the current namespace.
    pub fn lookup(&self, name: &str) -> Option<&NameBinding> {
        self.current()?.lookup(name)
    }

    /// Look up a qualified name across all namespaces.
    pub fn lookup_qualified(&self, path: &str) -> Option<&NameBinding> {
        let parts: Vec<&str> = path.split('.').collect();
        match parts.as_slice() {
            [] => None,
            [_single] => self.lookup(path),
            [root, rest @ ..] => {
                let ns = self.roots.get(*root)?;
                ns.lookup_path(rest)
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_bind_lookup() {
        let mut ns = Namespace::new("test");
        let hash = ContentHash::compute(b"add");

        ns.bind("add", NameBinding::value(hash));

        let binding = ns.lookup("add").unwrap();
        assert_eq!(binding.hash, hash);
        assert_eq!(binding.kind, BindingKind::Value);
    }

    #[test]
    fn test_namespace_lookup_by_kind() {
        let mut ns = Namespace::new("test");
        let hash = ContentHash::compute(b"Int");

        ns.bind("Int", NameBinding::ty(hash));

        assert!(ns.lookup_kind("Int", BindingKind::Type).is_some());
        assert!(ns.lookup_kind("Int", BindingKind::Value).is_none());
    }

    #[test]
    fn test_namespace_rename() {
        let mut ns = Namespace::new("test");
        let hash = ContentHash::compute(b"add");

        ns.bind("add", NameBinding::value(hash));
        assert!(ns.rename("add", "addition"));

        assert!(ns.lookup("add").is_none());
        assert_eq!(ns.lookup("addition").unwrap().hash, hash);
    }

    #[test]
    fn test_namespace_find_names() {
        let mut ns = Namespace::new("test");
        let hash = ContentHash::compute(b"add");

        ns.bind("add", NameBinding::value(hash));
        ns.bind("plus", NameBinding::value(hash));

        let names = ns.find_names(hash);
        assert!(names.contains(&"add"));
        assert!(names.contains(&"plus"));
    }

    #[test]
    fn test_child_namespace() {
        let mut ns = Namespace::new("std");
        {
            let io = ns.get_or_create_child("io");
            io.bind("println", NameBinding::value(ContentHash::compute(b"println")));
        }

        let io = ns.get_child("io").unwrap();
        assert!(io.lookup("println").is_some());
    }

    #[test]
    fn test_qualified_lookup() {
        let mut ns = Namespace::new("std");
        {
            let io = ns.get_or_create_child("io");
            io.bind("println", NameBinding::value(ContentHash::compute(b"println")));
        }

        assert!(ns.lookup_qualified("io.println").is_some());
    }

    #[test]
    fn test_name_registry() {
        let mut registry = NameRegistry::new();
        let hash = ContentHash::compute(b"main");

        registry.current_mut().bind("main", NameBinding::value(hash));

        assert!(registry.lookup("main").is_some());
    }

    #[test]
    fn test_binding_with_doc() {
        let binding = NameBinding::value(ContentHash::ZERO)
            .with_doc("Adds two numbers".into());

        assert_eq!(binding.doc.as_deref(), Some("Adds two numbers"));
    }

    #[test]
    fn test_public_bindings() {
        let mut ns = Namespace::new("test");
        ns.bind("public_fn", NameBinding::value(ContentHash::ZERO));
        ns.bind(
            "private_fn",
            NameBinding::value(ContentHash::ZERO).with_visibility(false),
        );

        let public: Vec<_> = ns.public_bindings().collect();
        assert_eq!(public.len(), 1);
        assert_eq!(public[0].0, "public_fn");
    }
}
