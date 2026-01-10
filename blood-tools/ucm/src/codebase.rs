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
}
