//! Storage Backend
//!
//! SQLite-based storage for the codebase.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

use crate::hash::Hash;
use crate::names::Name;
use crate::DefKind;

/// Storage errors.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("{0}")]
    Other(String),
}

/// Storage result type.
pub type StorageResult<T> = Result<T, StorageError>;

/// SQLite-based storage backend.
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Creates a new storage at the given path.
    pub fn create(path: impl AsRef<Path>) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Opens existing storage at the given path.
    pub fn open(path: impl AsRef<Path>) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }

    /// Initializes the database schema.
    fn init_schema(&self) -> StorageResult<()> {
        self.conn.execute_batch(
            r#"
            -- Definitions table: stores code by hash
            CREATE TABLE IF NOT EXISTS definitions (
                hash BLOB PRIMARY KEY,
                kind TEXT NOT NULL,
                source TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );

            -- Names table: maps names to hashes
            CREATE TABLE IF NOT EXISTS names (
                name TEXT PRIMARY KEY,
                hash BLOB NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (hash) REFERENCES definitions(hash)
            );

            -- Dependencies table: tracks what each definition references
            CREATE TABLE IF NOT EXISTS dependencies (
                from_hash BLOB NOT NULL,
                to_hash BLOB NOT NULL,
                PRIMARY KEY (from_hash, to_hash),
                FOREIGN KEY (from_hash) REFERENCES definitions(hash),
                FOREIGN KEY (to_hash) REFERENCES definitions(hash)
            );

            -- History table: tracks name changes
            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                hash BLOB NOT NULL,
                action TEXT NOT NULL,
                timestamp INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );

            -- Codebase metadata
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- Indexes
            CREATE INDEX IF NOT EXISTS idx_names_hash ON names(hash);
            CREATE INDEX IF NOT EXISTS idx_deps_to ON dependencies(to_hash);
            CREATE INDEX IF NOT EXISTS idx_history_name ON history(name);
            "#,
        )?;
        Ok(())
    }

    /// Sets a metadata value.
    pub fn set_metadata(&self, key: &str, value: &str) -> StorageResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Gets a metadata value.
    pub fn get_metadata(&self, key: &str) -> StorageResult<Option<String>> {
        let result = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result)
    }

    /// Stores a definition.
    pub fn store_definition(
        &self,
        hash: &Hash,
        kind: DefKind,
        source: &str,
    ) -> StorageResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO definitions (hash, kind, source) VALUES (?1, ?2, ?3)",
            params![hash.as_bytes().as_slice(), kind.as_str(), source],
        )?;
        Ok(())
    }

    /// Retrieves a definition by hash.
    pub fn get_definition(&self, hash: &Hash) -> StorageResult<Option<(DefKind, String)>> {
        let result = self
            .conn
            .query_row(
                "SELECT kind, source FROM definitions WHERE hash = ?1",
                params![hash.as_bytes().as_slice()],
                |row| {
                    let kind_str: String = row.get(0)?;
                    let source: String = row.get(1)?;
                    let kind = match kind_str.as_str() {
                        "term" => DefKind::Term,
                        "type" => DefKind::Type,
                        "effect" => DefKind::Effect,
                        "handler" => DefKind::Handler,
                        "test" => DefKind::Test,
                        "doc" => DefKind::Doc,
                        other => return Err(rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(StorageError::Other(format!("invalid DefKind in database: {}", other))),
                        )),
                    };
                    Ok((kind, source))
                },
            )
            .optional()?;
        Ok(result)
    }

    /// Adds a name mapping.
    pub fn add_name(&self, name: &Name, hash: &Hash) -> StorageResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO names (name, hash) VALUES (?1, ?2)",
            params![name.to_string(), hash.as_bytes().as_slice()],
        )?;

        // Record in history
        self.conn.execute(
            "INSERT INTO history (name, hash, action) VALUES (?1, ?2, 'add')",
            params![name.to_string(), hash.as_bytes().as_slice()],
        )?;

        Ok(())
    }

    /// Removes a name mapping.
    pub fn remove_name(&self, name: &Name) -> StorageResult<()> {
        // Get the hash first for history
        if let Some(hash) = self.resolve_name(name)? {
            self.conn.execute(
                "INSERT INTO history (name, hash, action) VALUES (?1, ?2, 'remove')",
                params![name.to_string(), hash.as_bytes().as_slice()],
            )?;
        }

        self.conn.execute(
            "DELETE FROM names WHERE name = ?1",
            params![name.to_string()],
        )?;
        Ok(())
    }

    /// Resolves a name to a hash.
    pub fn resolve_name(&self, name: &Name) -> StorageResult<Option<Hash>> {
        let result = self
            .conn
            .query_row(
                "SELECT hash FROM names WHERE name = ?1",
                params![name.to_string()],
                |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    Ok(Hash::from_bytes(arr))
                },
            )
            .optional()?;
        Ok(result)
    }

    /// Lists all names with optional prefix filter.
    pub fn list_names(&self, prefix: Option<&str>) -> StorageResult<Vec<(Name, Hash)>> {
        let mut stmt = if prefix.is_some() {
            self.conn.prepare(
                "SELECT name, hash FROM names WHERE name LIKE ?1 ORDER BY name",
            )?
        } else {
            self.conn.prepare("SELECT name, hash FROM names ORDER BY name")?
        };

        let mut results = Vec::new();

        // Define the row mapper function
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<(Name, Hash)> {
            let name_str: String = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok((Name::new(name_str), Hash::from_bytes(arr)))
        };

        // Query and collect results
        if let Some(prefix) = prefix {
            let pattern = format!("{}%", prefix);
            let rows = stmt.query_map(params![pattern], map_row)?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let rows = stmt.query_map([], map_row)?;
            for row in rows {
                results.push(row?);
            }
        }

        Ok(results)
    }

    /// Gets all names for a hash.
    pub fn names_for_hash(&self, hash: &Hash) -> StorageResult<Vec<Name>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM names WHERE hash = ?1 ORDER BY name")?;

        let mut results = Vec::new();
        let rows = stmt.query_map(params![hash.as_bytes().as_slice()], |row| {
            let name_str: String = row.get(0)?;
            Ok(Name::new(name_str))
        })?;

        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Adds a dependency.
    pub fn add_dependency(&self, from: &Hash, to: &Hash) -> StorageResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO dependencies (from_hash, to_hash) VALUES (?1, ?2)",
            params![from.as_bytes().as_slice(), to.as_bytes().as_slice()],
        )?;
        Ok(())
    }

    /// Gets dependencies of a hash.
    pub fn get_dependencies(&self, hash: &Hash) -> StorageResult<Vec<Hash>> {
        let mut stmt = self
            .conn
            .prepare("SELECT to_hash FROM dependencies WHERE from_hash = ?1")?;

        let mut results = Vec::new();
        let rows = stmt.query_map(params![hash.as_bytes().as_slice()], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Hash::from_bytes(arr))
        })?;

        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Gets dependents of a hash (reverse dependencies).
    pub fn get_dependents(&self, hash: &Hash) -> StorageResult<Vec<Hash>> {
        let mut stmt = self
            .conn
            .prepare("SELECT from_hash FROM dependencies WHERE to_hash = ?1")?;

        let mut results = Vec::new();
        let rows = stmt.query_map(params![hash.as_bytes().as_slice()], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Hash::from_bytes(arr))
        })?;

        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Gets the history of a name.
    pub fn get_history(&self, name: &Name) -> StorageResult<Vec<(Hash, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, timestamp FROM history WHERE name = ?1 ORDER BY timestamp DESC",
        )?;

        let mut results = Vec::new();
        let rows = stmt.query_map(params![name.to_string()], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let timestamp: i64 = row.get(1)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok((Hash::from_bytes(arr), timestamp))
        })?;

        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Counts definitions by kind.
    pub fn count_by_kind(&self, kind: DefKind) -> StorageResult<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM definitions WHERE kind = ?1",
            params![kind.as_str()],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Counts total names.
    pub fn count_names(&self) -> StorageResult<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM names", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Finds definitions by hash prefix.
    ///
    /// The prefix is a hex string (without the '#' prefix). Returns all hashes
    /// that start with the given prefix.
    pub fn find_by_hash_prefix(&self, prefix: &str) -> StorageResult<Vec<Hash>> {
        // Validate the prefix is valid hex
        if !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(Vec::new());
        }

        // Query all hashes and filter by prefix
        // Note: For a production system with many entries, we would use a more
        // efficient index. For now, we scan all hashes.
        let mut stmt = self.conn.prepare("SELECT hash FROM definitions")?;

        let mut results = Vec::new();
        let prefix_lower = prefix.to_lowercase();

        let rows = stmt.query_map([], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Hash::from_bytes(arr))
        })?;

        for row in rows {
            let hash = row?;
            let hash_hex = hash.to_hex();
            if hash_hex.starts_with(&prefix_lower) {
                results.push(hash);
            }
        }

        Ok(results)
    }

    /// Checks if a hash exists in the definitions table.
    pub fn has_definition(&self, hash: &Hash) -> StorageResult<bool> {
        let result: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM definitions WHERE hash = ?1",
            params![hash.as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        Ok(result > 0)
    }

    /// Lists all definitions with their kind.
    pub fn list_definitions(&self) -> StorageResult<Vec<(Hash, DefKind)>> {
        let mut stmt = self.conn.prepare("SELECT hash, kind FROM definitions ORDER BY created_at DESC")?;

        let mut results = Vec::new();
        let rows = stmt.query_map([], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let kind_str: String = row.get(1)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            let kind = match kind_str.as_str() {
                "term" => DefKind::Term,
                "type" => DefKind::Type,
                "effect" => DefKind::Effect,
                "handler" => DefKind::Handler,
                "test" => DefKind::Test,
                "doc" => DefKind::Doc,
                other => return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(StorageError::Other(format!("invalid DefKind in database: {}", other))),
                )),
            };
            Ok((Hash::from_bytes(arr), kind))
        })?;

        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Finds unreferenced definitions (garbage).
    ///
    /// Returns definitions that:
    /// - Have no names pointing to them
    /// - Are not dependencies of any other definition
    ///
    /// These are candidates for garbage collection.
    pub fn find_unreferenced(&self) -> StorageResult<Vec<Hash>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT d.hash FROM definitions d
            WHERE NOT EXISTS (SELECT 1 FROM names n WHERE n.hash = d.hash)
              AND NOT EXISTS (SELECT 1 FROM dependencies dep WHERE dep.to_hash = d.hash)
            "#
        )?;

        let mut results = Vec::new();
        let rows = stmt.query_map([], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Hash::from_bytes(arr))
        })?;

        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Deletes a definition by hash.
    ///
    /// This also removes any dependencies from this definition.
    /// Use with caution - only for garbage collection of unreferenced definitions.
    pub fn delete_definition(&self, hash: &Hash) -> StorageResult<()> {
        // First remove any dependencies from this definition
        self.conn.execute(
            "DELETE FROM dependencies WHERE from_hash = ?1",
            params![hash.as_bytes().as_slice()],
        )?;

        // Then remove the definition itself
        self.conn.execute(
            "DELETE FROM definitions WHERE hash = ?1",
            params![hash.as_bytes().as_slice()],
        )?;

        Ok(())
    }

    /// Garbage collects unreferenced definitions.
    ///
    /// Returns the number of definitions removed.
    pub fn garbage_collect(&self) -> StorageResult<usize> {
        let unreferenced = self.find_unreferenced()?;
        let count = unreferenced.len();

        for hash in unreferenced {
            self.delete_definition(&hash)?;
        }

        Ok(count)
    }
}
