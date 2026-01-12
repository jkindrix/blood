//! Codebase Management
//!
//! The main interface for working with a Blood codebase.

use std::path::Path;

use crate::hash::{structural_hash, Hash};
use crate::names::Name;
use crate::storage::Storage;
use crate::{DefInfo, DefKind, DefRef, UcmError, UcmResult};

/// Statistics about a codebase.
#[derive(Debug, Clone, Default)]
pub struct CodebaseStats {
    pub terms: usize,
    pub types: usize,
    pub effects: usize,
    pub handlers: usize,
    pub tests: usize,
    pub names: usize,
}

/// A Blood codebase.
pub struct Codebase {
    storage: Storage,
    name: String,
}

impl Codebase {
    /// Creates a new codebase at the given path.
    pub fn create(path: impl AsRef<Path>, name: &str) -> UcmResult<Self> {
        let db_path = path.as_ref().join("codebase.db");

        // Create the directory if it doesn't exist
        std::fs::create_dir_all(&path)?;

        let storage = Storage::create(&db_path)?;
        storage.set_metadata("name", name)?;
        storage.set_metadata("version", "1")?;

        Ok(Self {
            storage,
            name: name.to_string(),
        })
    }

    /// Opens an existing codebase.
    pub fn open(path: impl AsRef<Path>) -> UcmResult<Self> {
        let db_path = path.as_ref().join("codebase.db");
        let storage = Storage::open(&db_path)?;
        let name = storage
            .get_metadata("name")?
            .unwrap_or_else(|| "unnamed".to_string());

        Ok(Self { storage, name })
    }

    /// Returns the codebase name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Adds a term (function/value) to the codebase.
    pub fn add_term(&mut self, source: &str) -> UcmResult<Hash> {
        self.add_definition(source, DefKind::Term)
    }

    /// Adds a type to the codebase.
    pub fn add_type(&mut self, source: &str) -> UcmResult<Hash> {
        self.add_definition(source, DefKind::Type)
    }

    /// Adds an effect to the codebase.
    pub fn add_effect(&mut self, source: &str) -> UcmResult<Hash> {
        self.add_definition(source, DefKind::Effect)
    }

    /// Adds a handler to the codebase.
    pub fn add_handler(&mut self, source: &str) -> UcmResult<Hash> {
        self.add_definition(source, DefKind::Handler)
    }

    /// Adds a test to the codebase.
    pub fn add_test(&mut self, source: &str) -> UcmResult<Hash> {
        self.add_definition(source, DefKind::Test)
    }

    /// Adds a definition of the given kind.
    fn add_definition(&mut self, source: &str, kind: DefKind) -> UcmResult<Hash> {
        let hash = structural_hash(source);
        self.storage.store_definition(&hash, kind, source)?;

        // TODO: Parse source and extract dependencies
        // For now, we don't track dependencies

        Ok(hash)
    }

    /// Adds a name for a hash.
    pub fn add_name(&mut self, name: Name, hash: Hash) -> UcmResult<()> {
        self.storage.add_name(&name, &hash)?;
        Ok(())
    }

    /// Removes a name.
    pub fn remove_name(&mut self, name: &Name) -> UcmResult<()> {
        self.storage.remove_name(name)?;
        Ok(())
    }

    /// Renames a definition.
    pub fn rename(&mut self, from: Name, to: Name) -> UcmResult<()> {
        let hash = self
            .storage
            .resolve_name(&from)?
            .ok_or_else(|| UcmError::NameNotFound(from.to_string()))?;

        self.storage.remove_name(&from)?;
        self.storage.add_name(&to, &hash)?;

        Ok(())
    }

    /// Finds a definition by reference.
    pub fn find(&self, def_ref: &DefRef) -> UcmResult<Option<DefInfo>> {
        let hash = match def_ref {
            DefRef::Name(name) => match self.storage.resolve_name(name)? {
                Some(h) => h,
                None => return Ok(None),
            },
            DefRef::Hash(h) => h.clone(),
        };

        let (kind, source) = match self.storage.get_definition(&hash)? {
            Some(def) => def,
            None => return Ok(None),
        };

        let names = self.storage.names_for_hash(&hash)?;
        let dependencies = self.storage.get_dependencies(&hash)?;
        let dependents = self.storage.get_dependents(&hash)?;

        Ok(Some(DefInfo {
            hash,
            kind,
            names,
            source,
            dependencies,
            dependents,
        }))
    }

    /// Resolves a name to a hash.
    pub fn resolve(&self, name: &Name) -> UcmResult<Option<Hash>> {
        Ok(self.storage.resolve_name(name)?)
    }

    /// Lists all names with optional prefix filter.
    pub fn list_names(&self, prefix: Option<&str>) -> UcmResult<Vec<(Name, Hash)>> {
        Ok(self.storage.list_names(prefix)?)
    }

    /// Gets the history of a definition.
    pub fn history(&self, def_ref: &DefRef) -> UcmResult<Vec<(Hash, i64)>> {
        let name = match def_ref {
            DefRef::Name(n) => n.clone(),
            DefRef::Hash(h) => {
                let names = self.storage.names_for_hash(h)?;
                names.into_iter().next().ok_or_else(|| {
                    UcmError::HashNotFound(h.to_string())
                })?
            }
        };

        Ok(self.storage.get_history(&name)?)
    }

    /// Gets dependencies of a definition.
    pub fn dependencies(&self, def_ref: &DefRef) -> UcmResult<Vec<(Hash, Vec<String>)>> {
        let hash = self.resolve_ref(def_ref)?;
        let deps = self.storage.get_dependencies(&hash)?;

        let mut results = Vec::new();
        for dep_hash in deps {
            let names = self
                .storage
                .names_for_hash(&dep_hash)?
                .into_iter()
                .map(|n| n.to_string())
                .collect();
            results.push((dep_hash, names));
        }

        Ok(results)
    }

    /// Gets dependents of a definition (reverse dependencies).
    pub fn dependents(&self, def_ref: &DefRef) -> UcmResult<Vec<(Hash, Vec<String>)>> {
        let hash = self.resolve_ref(def_ref)?;
        let deps = self.storage.get_dependents(&hash)?;

        let mut results = Vec::new();
        for dep_hash in deps {
            let names = self
                .storage
                .names_for_hash(&dep_hash)?
                .into_iter()
                .map(|n| n.to_string())
                .collect();
            results.push((dep_hash, names));
        }

        Ok(results)
    }

    /// Lists all tests with optional filter.
    pub fn list_tests(&self, filter: Option<&str>) -> UcmResult<Vec<(Name, Hash)>> {
        let all_names = self.storage.list_names(filter)?;

        let mut tests = Vec::new();
        for (name, hash) in all_names {
            if let Some((kind, _)) = self.storage.get_definition(&hash)? {
                if kind == DefKind::Test {
                    tests.push((name, hash));
                }
            }
        }

        Ok(tests)
    }

    /// Returns codebase statistics.
    pub fn stats(&self) -> UcmResult<CodebaseStats> {
        Ok(CodebaseStats {
            terms: self.storage.count_by_kind(DefKind::Term)?,
            types: self.storage.count_by_kind(DefKind::Type)?,
            effects: self.storage.count_by_kind(DefKind::Effect)?,
            handlers: self.storage.count_by_kind(DefKind::Handler)?,
            tests: self.storage.count_by_kind(DefKind::Test)?,
            names: self.storage.count_names()?,
        })
    }

    /// Resolves a DefRef to a Hash.
    fn resolve_ref(&self, def_ref: &DefRef) -> UcmResult<Hash> {
        match def_ref {
            DefRef::Name(name) => self
                .storage
                .resolve_name(name)?
                .ok_or_else(|| UcmError::NameNotFound(name.to_string())),
            DefRef::Hash(h) => Ok(h.clone()),
        }
    }

    /// Finds definitions by hash prefix.
    ///
    /// Given a short hash prefix (e.g., "a7f2"), returns all definitions
    /// whose full hash starts with that prefix. This allows users to reference
    /// definitions without typing the full 64-character hash.
    ///
    /// # Returns
    ///
    /// - `Ok(vec![info])` if exactly one match is found
    /// - `Ok(vec![info1, info2, ...])` if multiple matches are found
    /// - `Ok(vec![])` if no matches are found
    pub fn find_by_hash_prefix(&self, prefix: &str) -> UcmResult<Vec<DefInfo>> {
        let hashes = self.storage.find_by_hash_prefix(prefix)?;

        let mut results = Vec::new();
        for hash in hashes {
            if let Some((kind, source)) = self.storage.get_definition(&hash)? {
                let names = self.storage.names_for_hash(&hash)?;
                let dependencies = self.storage.get_dependencies(&hash)?;
                let dependents = self.storage.get_dependents(&hash)?;

                results.push(DefInfo {
                    hash,
                    kind,
                    names,
                    source,
                    dependencies,
                    dependents,
                });
            }
        }

        Ok(results)
    }

    /// Resolves a hash prefix to a single hash.
    ///
    /// Returns `Ok(Some(hash))` if exactly one definition matches the prefix.
    /// Returns `Err(UcmError::AmbiguousHash)` if multiple definitions match.
    /// Returns `Ok(None)` if no definitions match.
    pub fn resolve_hash_prefix(&self, prefix: &str) -> UcmResult<Option<Hash>> {
        let hashes = self.storage.find_by_hash_prefix(prefix)?;

        match hashes.len() {
            0 => Ok(None),
            1 => Ok(Some(hashes.into_iter().next().unwrap())),
            n => Err(UcmError::AmbiguousHash {
                prefix: prefix.to_string(),
                count: n,
            }),
        }
    }

    /// Lists all definitions (without requiring names).
    pub fn list_definitions(&self) -> UcmResult<Vec<(Hash, DefKind)>> {
        Ok(self.storage.list_definitions()?)
    }

    /// Checks if a definition exists by hash.
    pub fn has_definition(&self, hash: &Hash) -> UcmResult<bool> {
        Ok(self.storage.has_definition(hash)?)
    }

    /// Finds unreferenced definitions that can be garbage collected.
    pub fn find_garbage(&self) -> UcmResult<Vec<Hash>> {
        Ok(self.storage.find_unreferenced()?)
    }

    /// Garbage collects unreferenced definitions.
    ///
    /// Returns the number of definitions removed.
    pub fn garbage_collect(&self) -> UcmResult<usize> {
        Ok(self.storage.garbage_collect()?)
    }

    /// Compares two definitions and returns a structural diff.
    ///
    /// Returns a list of differences between the two definitions.
    pub fn diff(&self, hash1: &Hash, hash2: &Hash) -> UcmResult<DiffResult> {
        let def1 = self.storage.get_definition(hash1)?
            .ok_or_else(|| UcmError::HashNotFound(hash1.to_string()))?;
        let def2 = self.storage.get_definition(hash2)?
            .ok_or_else(|| UcmError::HashNotFound(hash2.to_string()))?;

        let (kind1, source1) = def1;
        let (kind2, source2) = def2;

        let mut differences = Vec::new();

        // Compare kinds
        if kind1 != kind2 {
            differences.push(Difference::KindChange {
                old: kind1,
                new: kind2,
            });
        }

        // Compare source using line-by-line diff
        let lines1: Vec<&str> = source1.lines().collect();
        let lines2: Vec<&str> = source2.lines().collect();

        let mut i = 0;
        let mut j = 0;

        while i < lines1.len() || j < lines2.len() {
            if i >= lines1.len() {
                // Additions at end
                differences.push(Difference::LineAdded {
                    line_num: j + 1,
                    content: lines2[j].to_string(),
                });
                j += 1;
            } else if j >= lines2.len() {
                // Deletions at end
                differences.push(Difference::LineRemoved {
                    line_num: i + 1,
                    content: lines1[i].to_string(),
                });
                i += 1;
            } else if lines1[i] == lines2[j] {
                // Same line
                i += 1;
                j += 1;
            } else {
                // Different - check if it's a modification or add/remove
                // Simple heuristic: look ahead for matches
                let mut found_in_new = false;
                for (k, line) in lines2[j..].iter().enumerate().take(5) {
                    if lines1[i] == *line {
                        // Old line found later in new, so lines before were added
                        for l in j..(j + k) {
                            differences.push(Difference::LineAdded {
                                line_num: l + 1,
                                content: lines2[l].to_string(),
                            });
                        }
                        j += k;
                        found_in_new = true;
                        break;
                    }
                }

                if !found_in_new {
                    let mut found_in_old = false;
                    for (k, line) in lines1[i..].iter().enumerate().take(5) {
                        if lines2[j] == *line {
                            // New line found later in old, so lines before were removed
                            for l in i..(i + k) {
                                differences.push(Difference::LineRemoved {
                                    line_num: l + 1,
                                    content: lines1[l].to_string(),
                                });
                            }
                            i += k;
                            found_in_old = true;
                            break;
                        }
                    }

                    if !found_in_old {
                        // Simple modification
                        differences.push(Difference::LineChanged {
                            line_num: i + 1,
                            old: lines1[i].to_string(),
                            new: lines2[j].to_string(),
                        });
                        i += 1;
                        j += 1;
                    }
                }
            }
        }

        Ok(DiffResult {
            hash1: hash1.clone(),
            hash2: hash2.clone(),
            differences,
        })
    }
}

/// Result of comparing two definitions.
#[derive(Debug, Clone)]
pub struct DiffResult {
    /// First hash.
    pub hash1: Hash,
    /// Second hash.
    pub hash2: Hash,
    /// List of differences.
    pub differences: Vec<Difference>,
}

impl DiffResult {
    /// Returns true if the definitions are identical.
    pub fn is_identical(&self) -> bool {
        self.differences.is_empty()
    }
}

/// A single difference between two definitions.
#[derive(Debug, Clone)]
pub enum Difference {
    /// The definition kind changed.
    KindChange { old: DefKind, new: DefKind },
    /// A line was added.
    LineAdded { line_num: usize, content: String },
    /// A line was removed.
    LineRemoved { line_num: usize, content: String },
    /// A line was changed.
    LineChanged { line_num: usize, old: String, new: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        // Create
        {
            let codebase = Codebase::create(&path, "test").unwrap();
            assert_eq!(codebase.name(), "test");
        }

        // Open
        {
            let codebase = Codebase::open(&path).unwrap();
            assert_eq!(codebase.name(), "test");
        }
    }

    #[test]
    fn test_add_and_find() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        let source = "fn double(x: i32) -> i32 { x * 2 }";
        let hash = codebase.add_term(source).unwrap();
        codebase.add_name(Name::new("math.double"), hash.clone()).unwrap();

        let info = codebase.find(&DefRef::name("math.double")).unwrap().unwrap();
        assert_eq!(info.hash, hash);
        assert_eq!(info.kind, DefKind::Term);
    }

    #[test]
    fn test_rename() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        let source = "fn foo() {}";
        let hash = codebase.add_term(source).unwrap();
        codebase.add_name(Name::new("old.name"), hash.clone()).unwrap();

        codebase.rename(Name::new("old.name"), Name::new("new.name")).unwrap();

        assert!(codebase.resolve(&Name::new("old.name")).unwrap().is_none());
        assert_eq!(codebase.resolve(&Name::new("new.name")).unwrap(), Some(hash));
    }

    #[test]
    fn test_find_by_hash_prefix() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        // Add a definition
        let source = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let hash = codebase.add_term(source).unwrap();
        codebase.add_name(Name::new("math.add"), hash.clone()).unwrap();

        // Get the first 4 characters of the hash
        let full_hash = hash.to_hex();
        let prefix = &full_hash[..4];

        // Find by prefix
        let results = codebase.find_by_hash_prefix(prefix).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hash, hash);
        assert_eq!(results[0].source, source);
    }

    #[test]
    fn test_resolve_hash_prefix_unique() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        let source = "fn sub(a: i32, b: i32) -> i32 { a - b }";
        let hash = codebase.add_term(source).unwrap();

        let prefix = &hash.to_hex()[..8];
        let resolved = codebase.resolve_hash_prefix(prefix).unwrap();
        assert_eq!(resolved, Some(hash));
    }

    #[test]
    fn test_resolve_hash_prefix_not_found() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let codebase = Codebase::create(&path, "test").unwrap();

        // This prefix should not exist
        let resolved = codebase.resolve_hash_prefix("ffffffff").unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn test_list_definitions() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        let source1 = "fn one() -> i32 { 1 }";
        let source2 = "fn two() -> i32 { 2 }";
        let hash1 = codebase.add_term(source1).unwrap();
        let hash2 = codebase.add_term(source2).unwrap();

        let defs = codebase.list_definitions().unwrap();
        assert_eq!(defs.len(), 2);

        let hashes: Vec<_> = defs.iter().map(|(h, _)| h.clone()).collect();
        assert!(hashes.contains(&hash1));
        assert!(hashes.contains(&hash2));
    }

    #[test]
    fn test_has_definition() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        let source = "fn exists() {}";
        let hash = codebase.add_term(source).unwrap();

        assert!(codebase.has_definition(&hash).unwrap());

        // Non-existent hash
        let fake_hash = crate::hash::Hash::of_str("nonexistent");
        assert!(!codebase.has_definition(&fake_hash).unwrap());
    }

    #[test]
    fn test_find_by_hash_direct() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        let source = "fn direct() -> bool { true }";
        let hash = codebase.add_term(source).unwrap();

        // Find by full hash (no name assigned)
        let info = codebase.find(&DefRef::Hash(hash.clone())).unwrap().unwrap();
        assert_eq!(info.hash, hash);
        assert_eq!(info.source, source);
        assert!(info.names.is_empty()); // No name assigned
    }

    #[test]
    fn test_content_hash_determinism() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-codebase");

        let mut codebase = Codebase::create(&path, "test").unwrap();

        // Adding the same source twice should produce the same hash
        let source = "fn deterministic() -> i32 { 42 }";
        let hash1 = codebase.add_term(source).unwrap();
        let hash2 = codebase.add_term(source).unwrap();

        assert_eq!(hash1, hash2, "Same source should produce same hash");
    }
}
