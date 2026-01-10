//! # Codebase Storage
//!
//! Content-addressed storage for definitions.
//!
//! ## Storage Layout
//!
//! ```text
//! ~/.blood/codebases/
//! ├── default/
//! │   ├── terms/           # Definition storage
//! │   │   ├── a7/
//! │   │   │   └── f2k9m3...
//! │   │   └── ...
//! │   ├── types/           # Type definitions
//! │   ├── effects/         # Effect definitions
//! │   ├── names/           # Name → Hash mappings
//! │   ├── metadata/        # Comments, docs, locations
//! │   └── compiled/        # Compiled artifacts
//! └── projects/
//!     └── myproject/
//!         └── namespace.idx
//! ```
//!
//! ## Binary Format
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ Magic: "BLOD" (0x424C4F44)                               │
//! │ Format version                                           │
//! │ Hash (32 bytes)                                          │
//! │ AST size (4 bytes)                                       │
//! │ Canonical AST                                            │
//! │ Type size (4 bytes)                                      │
//! │ Type signature                                           │
//! │ References count (4 bytes)                               │
//! │ Reference hashes (32 * N bytes)                          │
//! │ Optional metadata                                        │
//! └──────────────────────────────────────────────────────────┘
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::canonical::CanonicalAST;
use super::hash::ContentHash;
use super::namespace::NameRegistry;

/// Magic bytes for definition files: "BLOD".
pub const MAGIC: [u8; 4] = [0x42, 0x4C, 0x4F, 0x44];

/// Current storage format version.
pub const STORAGE_VERSION: u8 = 1;

/// Errors that can occur during storage operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// Definition not found.
    NotFound(ContentHash),
    /// Hash collision detected.
    Collision {
        hash: ContentHash,
        existing: Box<DefinitionRecord>,
        new: Box<DefinitionRecord>,
    },
    /// Invalid format.
    InvalidFormat(String),
    /// IO error.
    IoError(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(hash) => write!(f, "Definition not found: {}", hash),
            Self::Collision { hash, .. } => write!(f, "Hash collision detected: {}", hash),
            Self::InvalidFormat(msg) => write!(f, "Invalid format: {}", msg),
            Self::IoError(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

impl std::error::Error for StorageError {}

/// Metadata associated with a definition (not part of hash).
#[derive(Debug, Clone, Default)]
pub struct DefinitionMetadata {
    /// Human-readable names for this definition.
    pub names: Vec<String>,
    /// Documentation string.
    pub documentation: Option<String>,
    /// Source file location.
    pub source_location: Option<SourceLocation>,
    /// Creation timestamp.
    pub created_at: Option<u64>,
    /// Author information.
    pub author: Option<String>,
}

/// Source file location.
#[derive(Debug, Clone)]
pub struct SourceLocation {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

/// A definition record stored in the codebase.
#[derive(Debug, Clone)]
pub struct DefinitionRecord {
    /// The content hash (identity).
    pub hash: ContentHash,
    /// The canonical AST.
    pub ast: CanonicalAST,
    /// Type signature hash.
    pub type_hash: Option<ContentHash>,
    /// Effect signature hash.
    pub effect_hash: Option<ContentHash>,
    /// Hashes of definitions this depends on.
    pub references: Vec<ContentHash>,
    /// Optional metadata (not part of hash).
    pub metadata: DefinitionMetadata,
}

impl DefinitionRecord {
    /// Create a new definition record.
    pub fn new(ast: CanonicalAST) -> Self {
        let hash = ast.compute_hash();
        Self {
            hash,
            ast,
            type_hash: None,
            effect_hash: None,
            references: Vec::new(),
            metadata: DefinitionMetadata::default(),
        }
    }

    /// Add a name to the metadata.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.metadata.names.push(name.into());
        self
    }

    /// Add documentation.
    pub fn with_doc(mut self, doc: impl Into<String>) -> Self {
        self.metadata.documentation = Some(doc.into());
        self
    }

    /// Add a dependency reference.
    pub fn with_reference(mut self, hash: ContentHash) -> Self {
        self.references.push(hash);
        self
    }

    /// Set the type signature hash.
    pub fn with_type(mut self, type_hash: ContentHash) -> Self {
        self.type_hash = Some(type_hash);
        self
    }

    /// Set the effect signature hash.
    pub fn with_effect(mut self, effect_hash: ContentHash) -> Self {
        self.effect_hash = Some(effect_hash);
        self
    }
}

impl PartialEq for DefinitionRecord {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl Eq for DefinitionRecord {}

/// In-memory codebase storage.
///
/// For simplicity, this implementation stores everything in memory.
/// A production implementation would use persistent storage (e.g., sled).
#[derive(Debug, Clone, Default)]
pub struct Codebase {
    /// Definition storage: hash → record.
    definitions: HashMap<ContentHash, DefinitionRecord>,
    /// Type definitions: hash → type record.
    types: HashMap<ContentHash, DefinitionRecord>,
    /// Effect definitions: hash → effect record.
    effects: HashMap<ContentHash, DefinitionRecord>,
    /// Name registry.
    names: NameRegistry,
    /// Compiled artifacts: hash → compiled code.
    compiled: HashMap<ContentHash, Vec<u8>>,
    /// Dependency graph: hash → set of dependents.
    dependents: HashMap<ContentHash, HashSet<ContentHash>>,
}

impl Codebase {
    /// Create a new empty codebase.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a definition to the codebase.
    pub fn add(&mut self, record: DefinitionRecord) -> Result<ContentHash, StorageError> {
        let hash = record.hash;

        // Check for collision
        if let Some(existing) = self.definitions.get(&hash) {
            if existing.ast != record.ast {
                return Err(StorageError::Collision {
                    hash,
                    existing: Box::new(existing.clone()),
                    new: Box::new(record),
                });
            }
            // Same AST, just return existing hash
            return Ok(hash);
        }

        // Update dependency graph
        for dep_hash in &record.references {
            self.dependents
                .entry(*dep_hash)
                .or_default()
                .insert(hash);
        }

        self.definitions.insert(hash, record);
        Ok(hash)
    }

    /// Get a definition by hash.
    pub fn get(&self, hash: ContentHash) -> Option<&DefinitionRecord> {
        self.definitions.get(&hash)
    }

    /// Check if a definition exists.
    pub fn contains(&self, hash: ContentHash) -> bool {
        self.definitions.contains_key(&hash)
    }

    /// Get all definitions that depend on a given hash.
    pub fn get_dependents(&self, hash: ContentHash) -> Vec<ContentHash> {
        self.dependents
            .get(&hash)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get the transitive closure of dependents.
    pub fn get_transitive_dependents(&self, hash: ContentHash) -> HashSet<ContentHash> {
        let mut visited = HashSet::new();
        let mut stack = vec![hash];

        while let Some(current) = stack.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            if let Some(deps) = self.dependents.get(&current) {
                stack.extend(deps.iter());
            }
        }

        visited.remove(&hash);
        visited
    }

    /// Get the name registry.
    pub fn names(&self) -> &NameRegistry {
        &self.names
    }

    /// Get the name registry mutably.
    pub fn names_mut(&mut self) -> &mut NameRegistry {
        &mut self.names
    }

    /// Store compiled code for a definition.
    pub fn store_compiled(&mut self, hash: ContentHash, code: Vec<u8>) {
        self.compiled.insert(hash, code);
    }

    /// Get compiled code for a definition.
    pub fn get_compiled(&self, hash: ContentHash) -> Option<&[u8]> {
        self.compiled.get(&hash).map(|v| v.as_slice())
    }

    /// Check if compiled code exists.
    pub fn has_compiled(&self, hash: ContentHash) -> bool {
        self.compiled.contains_key(&hash)
    }

    /// Add a type definition.
    pub fn add_type(&mut self, record: DefinitionRecord) -> Result<ContentHash, StorageError> {
        let hash = record.hash;
        self.types.insert(hash, record);
        Ok(hash)
    }

    /// Get a type definition.
    pub fn get_type(&self, hash: ContentHash) -> Option<&DefinitionRecord> {
        self.types.get(&hash)
    }

    /// Add an effect definition.
    pub fn add_effect(&mut self, record: DefinitionRecord) -> Result<ContentHash, StorageError> {
        let hash = record.hash;
        self.effects.insert(hash, record);
        Ok(hash)
    }

    /// Get an effect definition.
    pub fn get_effect(&self, hash: ContentHash) -> Option<&DefinitionRecord> {
        self.effects.get(&hash)
    }

    /// Get all definition hashes.
    pub fn all_hashes(&self) -> impl Iterator<Item = ContentHash> + '_ {
        self.definitions.keys().copied()
    }

    /// Get count of definitions.
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// Garbage collect unreferenced definitions.
    pub fn collect_garbage(&mut self, roots: &HashSet<ContentHash>) -> usize {
        let reachable = self.transitive_closure(roots);

        let to_remove: Vec<_> = self
            .definitions
            .keys()
            .filter(|h| !reachable.contains(h))
            .copied()
            .collect();

        let count = to_remove.len();

        for hash in to_remove {
            self.definitions.remove(&hash);
            self.compiled.remove(&hash);
            self.dependents.remove(&hash);
        }

        count
    }

    /// Compute the transitive closure of a set of roots.
    fn transitive_closure(&self, roots: &HashSet<ContentHash>) -> HashSet<ContentHash> {
        let mut visited = HashSet::new();
        let mut stack: Vec<_> = roots.iter().copied().collect();

        while let Some(hash) = stack.pop() {
            if visited.contains(&hash) {
                continue;
            }
            visited.insert(hash);

            if let Some(record) = self.definitions.get(&hash) {
                stack.extend(&record.references);
            }
        }

        visited
    }
}

/// Statistics about a codebase.
#[derive(Debug, Clone, Default)]
pub struct CodebaseStats {
    pub definition_count: usize,
    pub type_count: usize,
    pub effect_count: usize,
    pub compiled_count: usize,
    pub total_ast_size: usize,
    pub total_compiled_size: usize,
}

impl Codebase {
    /// Get statistics about this codebase.
    pub fn stats(&self) -> CodebaseStats {
        CodebaseStats {
            definition_count: self.definitions.len(),
            type_count: self.types.len(),
            effect_count: self.effects.len(),
            compiled_count: self.compiled.len(),
            total_ast_size: 0, // Would need AST size tracking
            total_compiled_size: self.compiled.values().map(|v| v.len()).sum(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_ast(value: i128) -> CanonicalAST {
        CanonicalAST::IntLit(value)
    }

    #[test]
    fn test_codebase_add_and_get() {
        let mut codebase = Codebase::new();
        let ast = make_test_ast(42);
        let record = DefinitionRecord::new(ast);
        let hash = record.hash;

        let result = codebase.add(record);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), hash);

        let retrieved = codebase.get(hash);
        assert!(retrieved.is_some());
    }

    #[test]
    fn test_codebase_duplicate_same_ast() {
        let mut codebase = Codebase::new();
        let ast1 = make_test_ast(42);
        let ast2 = make_test_ast(42);

        let record1 = DefinitionRecord::new(ast1);
        let record2 = DefinitionRecord::new(ast2);

        let hash1 = codebase.add(record1).unwrap();
        let hash2 = codebase.add(record2).unwrap();

        // Same AST should produce same hash, no collision
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_definition_record_builder() {
        let ast = make_test_ast(42);
        let dep_hash = ContentHash::compute(b"dependency");

        let record = DefinitionRecord::new(ast)
            .with_name("answer")
            .with_doc("The answer to everything")
            .with_reference(dep_hash);

        assert!(record.metadata.names.contains(&"answer".to_string()));
        assert_eq!(
            record.metadata.documentation.as_deref(),
            Some("The answer to everything")
        );
        assert!(record.references.contains(&dep_hash));
    }

    #[test]
    fn test_codebase_dependents() {
        let mut codebase = Codebase::new();

        // Create definition A
        let ast_a = make_test_ast(1);
        let record_a = DefinitionRecord::new(ast_a);
        let hash_a = record_a.hash;
        codebase.add(record_a).unwrap();

        // Create definition B that depends on A
        let ast_b = make_test_ast(2);
        let record_b = DefinitionRecord::new(ast_b).with_reference(hash_a);
        let hash_b = record_b.hash;
        codebase.add(record_b).unwrap();

        // A should have B as a dependent
        let dependents = codebase.get_dependents(hash_a);
        assert!(dependents.contains(&hash_b));
    }

    #[test]
    fn test_codebase_garbage_collection() {
        let mut codebase = Codebase::new();

        // Add some definitions
        let ast1 = make_test_ast(1);
        let ast2 = make_test_ast(2);
        let ast3 = make_test_ast(3);

        let hash1 = codebase.add(DefinitionRecord::new(ast1)).unwrap();
        let hash2 = codebase.add(DefinitionRecord::new(ast2)).unwrap();
        let _hash3 = codebase.add(DefinitionRecord::new(ast3)).unwrap();

        // Only keep hash1 and hash2 as roots
        let mut roots = HashSet::new();
        roots.insert(hash1);
        roots.insert(hash2);

        let collected = codebase.collect_garbage(&roots);

        // hash3 should be collected
        assert_eq!(collected, 1);
        assert_eq!(codebase.len(), 2);
    }

    #[test]
    fn test_codebase_compiled_storage() {
        let mut codebase = Codebase::new();
        let hash = ContentHash::compute(b"test");
        let code = vec![0x90, 0x90, 0xC3]; // NOP NOP RET

        codebase.store_compiled(hash, code.clone());

        assert!(codebase.has_compiled(hash));
        assert_eq!(codebase.get_compiled(hash), Some(code.as_slice()));
    }

    #[test]
    fn test_codebase_stats() {
        let mut codebase = Codebase::new();
        let ast = make_test_ast(42);
        codebase.add(DefinitionRecord::new(ast)).unwrap();

        let stats = codebase.stats();
        assert_eq!(stats.definition_count, 1);
    }

    #[test]
    fn test_storage_error_display() {
        let hash = ContentHash::compute(b"test");
        let err = StorageError::NotFound(hash);
        let display = format!("{}", err);
        assert!(display.contains("not found"));
    }
}
