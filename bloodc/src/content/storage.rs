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
// Persistent Storage with Sled
// ============================================================================

/// Persistent codebase storage backed by sled embedded database.
///
/// This provides durable, crash-safe storage for the content-addressed codebase.
/// Data is stored on disk and persists across compiler invocations.
pub struct PersistentCodebase {
    /// The sled database.
    db: sled::Db,
    /// Tree for definitions: hash bytes → serialized DefinitionRecord.
    definitions: sled::Tree,
    /// Tree for types.
    types: sled::Tree,
    /// Tree for effects.
    effects: sled::Tree,
    /// Tree for compiled artifacts: hash bytes → compiled code.
    compiled: sled::Tree,
    /// Tree for dependency graph: hash bytes → serialized Vec<ContentHash>.
    dependents: sled::Tree,
    /// Tree for name mappings: name string → hash bytes.
    names: sled::Tree,
}

impl PersistentCodebase {
    /// Open or create a persistent codebase at the given path.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StorageError> {
        let db = sled::open(path.as_ref())
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        let definitions = db.open_tree("definitions")
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        let types = db.open_tree("types")
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        let effects = db.open_tree("effects")
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        let compiled = db.open_tree("compiled")
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        let dependents = db.open_tree("dependents")
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        let names = db.open_tree("names")
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        Ok(Self {
            db,
            definitions,
            types,
            effects,
            compiled,
            dependents,
            names,
        })
    }

    /// Open or create the default codebase in ~/.blood/codebase.
    pub fn open_default() -> Result<Self, StorageError> {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("blood")
            .join("codebase");

        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        Self::open(cache_dir)
    }

    /// Add a definition to persistent storage.
    pub fn add(&self, record: &DefinitionRecord) -> Result<ContentHash, StorageError> {
        let hash = record.hash;
        let key = hash.as_bytes();

        // Check for collision
        if let Some(existing_data) = self.definitions.get(key)
            .map_err(|e| StorageError::IoError(e.to_string()))?
        {
            if let Ok(existing) = Self::deserialize_record(&existing_data) {
                if existing.ast != record.ast {
                    return Err(StorageError::Collision {
                        hash,
                        existing: Box::new(existing),
                        new: Box::new(record.clone()),
                    });
                }
                // Same AST, return existing hash
                return Ok(hash);
            }
        }

        // Serialize and store
        let data = Self::serialize_record(record);
        self.definitions.insert(key, data)
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        // Update dependency graph
        for dep_hash in &record.references {
            self.add_dependent(*dep_hash, hash)?;
        }

        Ok(hash)
    }

    /// Get a definition by hash.
    pub fn get(&self, hash: ContentHash) -> Option<DefinitionRecord> {
        self.definitions.get(hash.as_bytes())
            .ok()?
            .and_then(|data| Self::deserialize_record(&data).ok())
    }

    /// Check if a definition exists.
    pub fn contains(&self, hash: ContentHash) -> bool {
        self.definitions.contains_key(hash.as_bytes()).unwrap_or(false)
    }

    /// Store compiled code for a definition.
    pub fn store_compiled(&self, hash: ContentHash, code: &[u8]) -> Result<(), StorageError> {
        self.compiled.insert(hash.as_bytes(), code)
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        Ok(())
    }

    /// Get compiled code for a definition.
    pub fn get_compiled(&self, hash: ContentHash) -> Option<Vec<u8>> {
        self.compiled.get(hash.as_bytes())
            .ok()?
            .map(|data| data.to_vec())
    }

    /// Check if compiled code exists.
    pub fn has_compiled(&self, hash: ContentHash) -> bool {
        self.compiled.contains_key(hash.as_bytes()).unwrap_or(false)
    }

    /// Map a name to a hash.
    pub fn set_name(&self, name: &str, hash: ContentHash) -> Result<(), StorageError> {
        self.names.insert(name.as_bytes(), hash.as_bytes())
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        Ok(())
    }

    /// Get the hash for a name.
    pub fn get_name(&self, name: &str) -> Option<ContentHash> {
        self.names.get(name.as_bytes())
            .ok()?
            .and_then(|data| {
                let bytes: [u8; 32] = data.as_ref().try_into().ok()?;
                Some(ContentHash::from_bytes(bytes))
            })
    }

    /// Add a dependent relationship.
    fn add_dependent(&self, dependency: ContentHash, dependent: ContentHash) -> Result<(), StorageError> {
        let key = dependency.as_bytes();

        // Load existing dependents
        let mut deps: Vec<ContentHash> = self.dependents.get(key)
            .map_err(|e| StorageError::IoError(e.to_string()))?
            .map(|data| Self::deserialize_hashes(&data))
            .unwrap_or_default();

        // Add new dependent if not already present
        if !deps.contains(&dependent) {
            deps.push(dependent);
            let data = Self::serialize_hashes(&deps);
            self.dependents.insert(key, data)
                .map_err(|e| StorageError::IoError(e.to_string()))?;
        }

        Ok(())
    }

    /// Get all definitions that depend on a given hash.
    pub fn get_dependents(&self, hash: ContentHash) -> Vec<ContentHash> {
        self.dependents.get(hash.as_bytes())
            .ok()
            .flatten()
            .map(|data| Self::deserialize_hashes(&data))
            .unwrap_or_default()
    }

    /// Get count of definitions.
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// Flush all pending writes to disk.
    pub fn flush(&self) -> Result<(), StorageError> {
        self.db.flush()
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        Ok(())
    }

    /// Get statistics about this codebase.
    pub fn stats(&self) -> CodebaseStats {
        CodebaseStats {
            definition_count: self.definitions.len(),
            type_count: self.types.len(),
            effect_count: self.effects.len(),
            compiled_count: self.compiled.len(),
            total_ast_size: 0,
            total_compiled_size: self.compiled.iter()
                .filter_map(|r| r.ok())
                .map(|(_, v)| v.len())
                .sum(),
        }
    }

    // Serialization helpers

    fn serialize_record(record: &DefinitionRecord) -> Vec<u8> {
        // Simple binary format:
        // - 32 bytes: hash
        // - 4 bytes: AST length
        // - N bytes: AST (as debug string, simplified)
        // - 4 bytes: reference count
        // - 32*N bytes: references
        // - 4 bytes: name count
        // - names (length-prefixed strings)

        let mut data = Vec::new();

        // Hash
        data.extend_from_slice(record.hash.as_bytes());

        // AST (simplified - store as debug format)
        let ast_str = format!("{:?}", record.ast);
        let ast_bytes = ast_str.as_bytes();
        data.extend_from_slice(&(ast_bytes.len() as u32).to_le_bytes());
        data.extend_from_slice(ast_bytes);

        // Type hash (optional)
        if let Some(th) = &record.type_hash {
            data.push(1);
            data.extend_from_slice(th.as_bytes());
        } else {
            data.push(0);
        }

        // Effect hash (optional)
        if let Some(eh) = &record.effect_hash {
            data.push(1);
            data.extend_from_slice(eh.as_bytes());
        } else {
            data.push(0);
        }

        // References
        data.extend_from_slice(&(record.references.len() as u32).to_le_bytes());
        for r in &record.references {
            data.extend_from_slice(r.as_bytes());
        }

        // Names (simplified metadata)
        data.extend_from_slice(&(record.metadata.names.len() as u32).to_le_bytes());
        for name in &record.metadata.names {
            let name_bytes = name.as_bytes();
            data.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            data.extend_from_slice(name_bytes);
        }

        data
    }

    fn deserialize_record(data: &[u8]) -> Result<DefinitionRecord, StorageError> {
        if data.len() < 36 {
            return Err(StorageError::InvalidFormat("Data too short".to_string()));
        }

        let mut pos = 0;

        // Hash
        let hash_bytes: [u8; 32] = data[pos..pos + 32].try_into()
            .map_err(|_| StorageError::InvalidFormat("Invalid hash bytes".to_string()))?;
        let hash = ContentHash::from_bytes(hash_bytes);
        pos += 32;

        // AST length
        let ast_len = u32::from_le_bytes(
            data[pos..pos + 4].try_into()
                .map_err(|_| StorageError::InvalidFormat("Invalid AST length bytes".to_string()))?
        ) as usize;
        pos += 4;

        // Validate AST data bounds before reading
        if data.len() < pos + ast_len {
            return Err(StorageError::InvalidFormat("AST data truncated".to_string()));
        }
        let _ast_str = String::from_utf8_lossy(&data[pos..pos + ast_len]);
        pos += ast_len;

        // For now, we can't reconstruct the AST from debug format
        // Use a placeholder - in a real implementation, we'd use proper serialization
        let ast = CanonicalAST::IntLit(0);

        // Type hash
        let has_type = data.get(pos).copied().unwrap_or(0) == 1;
        pos += 1;
        let type_hash = if has_type && data.len() >= pos + 32 {
            // SAFETY: Bounds check above guarantees exactly 32 bytes available
            let bytes: [u8; 32] = data[pos..pos + 32].try_into()
                .expect("bounds checked: 32 bytes available");
            pos += 32;
            Some(ContentHash::from_bytes(bytes))
        } else {
            None
        };

        // Effect hash
        let has_effect = data.get(pos).copied().unwrap_or(0) == 1;
        pos += 1;
        let effect_hash = if has_effect && data.len() >= pos + 32 {
            // SAFETY: Bounds check above guarantees exactly 32 bytes available
            let bytes: [u8; 32] = data[pos..pos + 32].try_into()
                .expect("bounds checked: 32 bytes available");
            pos += 32;
            Some(ContentHash::from_bytes(bytes))
        } else {
            None
        };

        // References
        let ref_count = if data.len() >= pos + 4 {
            // SAFETY: Bounds check above guarantees exactly 4 bytes available
            u32::from_le_bytes(data[pos..pos + 4].try_into()
                .expect("bounds checked: 4 bytes available")) as usize
        } else {
            0
        };
        pos += 4;

        let mut references = Vec::with_capacity(ref_count.min(1024)); // Cap allocation
        for _ in 0..ref_count {
            if data.len() >= pos + 32 {
                // SAFETY: Bounds check above guarantees exactly 32 bytes available
                let bytes: [u8; 32] = data[pos..pos + 32].try_into()
                    .expect("bounds checked: 32 bytes available");
                references.push(ContentHash::from_bytes(bytes));
                pos += 32;
            }
        }

        // Names
        let mut names = Vec::new();
        if data.len() >= pos + 4 {
            // SAFETY: Bounds check above guarantees exactly 4 bytes available
            let name_count = u32::from_le_bytes(data[pos..pos + 4].try_into()
                .expect("bounds checked: 4 bytes available")) as usize;
            pos += 4;

            for _ in 0..name_count.min(1024) { // Cap iterations for safety
                if data.len() >= pos + 4 {
                    // SAFETY: Bounds check above guarantees exactly 4 bytes available
                    let name_len = u32::from_le_bytes(data[pos..pos + 4].try_into()
                        .expect("bounds checked: 4 bytes available")) as usize;
                    pos += 4;
                    if data.len() >= pos + name_len {
                        names.push(String::from_utf8_lossy(&data[pos..pos + name_len]).to_string());
                        pos += name_len;
                    }
                }
            }
        }

        Ok(DefinitionRecord {
            hash,
            ast,
            type_hash,
            effect_hash,
            references,
            metadata: DefinitionMetadata {
                names,
                ..Default::default()
            },
        })
    }

    fn serialize_hashes(hashes: &[ContentHash]) -> Vec<u8> {
        let mut data = Vec::with_capacity(4 + hashes.len() * 32);
        data.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
        for h in hashes {
            data.extend_from_slice(h.as_bytes());
        }
        data
    }

    fn deserialize_hashes(data: &[u8]) -> Vec<ContentHash> {
        if data.len() < 4 {
            return Vec::new();
        }

        // SAFETY: Bounds check above guarantees at least 4 bytes available
        let count = u32::from_le_bytes(data[0..4].try_into()
            .expect("bounds checked: 4 bytes available")) as usize;
        let mut hashes = Vec::with_capacity(count.min(1024)); // Cap allocation
        let mut pos = 4;

        for _ in 0..count {
            if data.len() >= pos + 32 {
                // SAFETY: Bounds check above guarantees exactly 32 bytes available
                let bytes: [u8; 32] = data[pos..pos + 32].try_into()
                    .expect("bounds checked: 32 bytes available");
                hashes.push(ContentHash::from_bytes(bytes));
                pos += 32;
            }
        }

        hashes
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

    #[test]
    fn test_persistent_codebase_basic() {
        // Create a temp directory for the test database
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_codebase");

        // Open persistent codebase
        let codebase = PersistentCodebase::open(&db_path).unwrap();

        // Add a definition
        let ast = make_test_ast(42);
        let record = DefinitionRecord::new(ast.clone()).with_name("answer");
        let hash = record.hash;

        let result = codebase.add(&record);
        assert!(result.is_ok());

        // Check it exists
        assert!(codebase.contains(hash));

        // Get it back
        let retrieved = codebase.get(hash);
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.hash, hash);
        assert!(retrieved.metadata.names.contains(&"answer".to_string()));

        // Store and retrieve compiled code
        let code = vec![0x90, 0xC3];
        codebase.store_compiled(hash, &code).unwrap();
        assert!(codebase.has_compiled(hash));
        assert_eq!(codebase.get_compiled(hash), Some(code));

        // Flush to disk
        codebase.flush().unwrap();

        // Stats
        let stats = codebase.stats();
        assert_eq!(stats.definition_count, 1);
    }

    #[test]
    fn test_persistent_codebase_name_mapping() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_codebase_names");

        let codebase = PersistentCodebase::open(&db_path).unwrap();

        let hash = ContentHash::compute(b"test_value");
        codebase.set_name("my_function", hash).unwrap();

        let retrieved = codebase.get_name("my_function");
        assert_eq!(retrieved, Some(hash));
    }

    #[test]
    fn test_persistent_codebase_persistence() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_codebase_persist");

        let hash;
        {
            // First session: add data
            let codebase = PersistentCodebase::open(&db_path).unwrap();
            let ast = make_test_ast(123);
            let record = DefinitionRecord::new(ast);
            hash = record.hash;
            codebase.add(&record).unwrap();
            codebase.set_name("persistent_test", hash).unwrap();
            codebase.flush().unwrap();
        }

        {
            // Second session: verify data persisted
            let codebase = PersistentCodebase::open(&db_path).unwrap();
            assert!(codebase.contains(hash));
            assert_eq!(codebase.get_name("persistent_test"), Some(hash));
        }
    }
}
