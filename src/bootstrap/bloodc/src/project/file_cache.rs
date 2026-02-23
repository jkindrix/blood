//! File-level incremental compilation cache.
//!
//! This module tracks source files and their content hashes to enable
//! incremental compilation. When a file changes, only that file and its
//! dependents need to be recompiled.
//!
//! ## Cache Structure
//!
//! ```text
//! .blood/
//! └── file_cache.json    # File hash manifest
//! ```
//!
//! ## Integration with BuildCache
//!
//! The FileCache works alongside the BuildCache:
//! 1. FileCache tracks which source files have changed (by content hash)
//! 2. FileCache maps files to the definitions they contain
//! 3. When a file changes, all definitions from that file are invalidated
//! 4. BuildCache handles the actual compiled artifact caching

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::content::hash::ContentHash;
use crate::hir::DefId;

use super::resolve::ModuleId;

/// Version for the file cache format.
pub const FILE_CACHE_VERSION: u32 = 1;

/// File-level cache for incremental compilation.
#[derive(Debug)]
pub struct FileCache {
    /// Root directory for the project (where Blood.toml lives).
    project_root: PathBuf,
    /// Cached file entries.
    entries: HashMap<PathBuf, FileCacheEntry>,
    /// Mapping from file to the definitions it contains.
    file_to_defs: HashMap<PathBuf, HashSet<DefId>>,
    /// Mapping from file to its module ID.
    file_to_module: HashMap<PathBuf, ModuleId>,
    /// Whether caching is enabled.
    enabled: bool,
}

/// A cached entry for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCacheEntry {
    /// Content hash of the file.
    pub content_hash: ContentHash,
    /// Last modification time (for quick change detection).
    pub mtime: u64,
    /// File size in bytes.
    pub size: u64,
    /// Definition IDs contained in this file.
    #[serde(default)]
    pub definitions: Vec<u32>,
    /// Module ID for this file (if it's a module root).
    #[serde(default)]
    pub module_id: Option<u32>,
}

/// Persistent file cache index.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileCacheIndex {
    /// Cache format version.
    version: u32,
    /// File entries by relative path.
    entries: HashMap<String, FileCacheEntry>,
}

/// Result of checking if a file has changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// File has not changed since last cache.
    Unchanged,
    /// File has been modified.
    Modified,
    /// File is new (not in cache).
    New,
    /// File has been deleted.
    Deleted,
}

/// Errors that can occur during file cache operations.
#[derive(Debug)]
pub enum FileCacheError {
    /// IO error.
    Io(io::Error),
    /// JSON serialization/deserialization error.
    Json(String),
    /// Version mismatch.
    VersionMismatch { expected: u32, found: u32 },
}

impl std::fmt::Display for FileCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "file cache IO error: {}", e),
            Self::Json(msg) => write!(f, "file cache JSON error: {}", msg),
            Self::VersionMismatch { expected, found } => {
                write!(f, "file cache version mismatch: expected {}, found {}", expected, found)
            }
        }
    }
}

impl std::error::Error for FileCacheError {}

impl From<io::Error> for FileCacheError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for FileCacheError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}

impl FileCache {
    /// Create a new file cache for a project.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            entries: HashMap::new(),
            file_to_defs: HashMap::new(),
            file_to_module: HashMap::new(),
            enabled: true,
        }
    }

    /// Create a disabled file cache (no-op for all operations).
    pub fn disabled() -> Self {
        Self {
            project_root: PathBuf::new(),
            entries: HashMap::new(),
            file_to_defs: HashMap::new(),
            file_to_module: HashMap::new(),
            enabled: false,
        }
    }

    /// Get the cache file path.
    fn cache_path(&self) -> PathBuf {
        self.project_root.join(".blood").join("file_cache.json")
    }

    /// Initialize the cache directory.
    pub fn init(&self) -> Result<(), FileCacheError> {
        if !self.enabled {
            return Ok(());
        }

        let cache_dir = self.project_root.join(".blood");
        fs::create_dir_all(cache_dir)?;
        Ok(())
    }

    /// Load the file cache from disk.
    pub fn load(&mut self) -> Result<bool, FileCacheError> {
        if !self.enabled {
            return Ok(false);
        }

        let cache_path = self.cache_path();
        if !cache_path.exists() {
            return Ok(false);
        }

        let json = fs::read_to_string(&cache_path)?;
        let index: FileCacheIndex = serde_json::from_str(&json)?;

        if index.version != FILE_CACHE_VERSION {
            return Err(FileCacheError::VersionMismatch {
                expected: FILE_CACHE_VERSION,
                found: index.version,
            });
        }

        // Convert relative paths back to absolute
        self.entries.clear();
        self.file_to_defs.clear();
        self.file_to_module.clear();

        for (rel_path, entry) in index.entries {
            let abs_path = self.project_root.join(&rel_path);

            // Restore file_to_defs mapping
            let def_ids: HashSet<DefId> = entry.definitions
                .iter()
                .map(|&idx| DefId::new(idx))
                .collect();
            if !def_ids.is_empty() {
                self.file_to_defs.insert(abs_path.clone(), def_ids);
            }

            // Restore file_to_module mapping
            if let Some(module_idx) = entry.module_id {
                self.file_to_module.insert(abs_path.clone(), ModuleId::new(module_idx));
            }

            self.entries.insert(abs_path, entry);
        }

        Ok(true)
    }

    /// Save the file cache to disk.
    pub fn save(&self) -> Result<(), FileCacheError> {
        if !self.enabled {
            return Ok(());
        }

        // Convert absolute paths to relative for storage
        let mut entries = HashMap::new();
        for (abs_path, entry) in &self.entries {
            if let Ok(rel_path) = abs_path.strip_prefix(&self.project_root) {
                let rel_str = rel_path.to_string_lossy().to_string();

                // Include definitions and module_id from our tracking maps
                let mut entry = entry.clone();
                if let Some(defs) = self.file_to_defs.get(abs_path) {
                    entry.definitions = defs.iter().map(|d| d.index).collect();
                }
                if let Some(module) = self.file_to_module.get(abs_path) {
                    entry.module_id = Some(module.raw());
                }

                entries.insert(rel_str, entry);
            }
        }

        let index = FileCacheIndex {
            version: FILE_CACHE_VERSION,
            entries,
        };

        let json = serde_json::to_string_pretty(&index)?;
        let cache_path = self.cache_path();

        // Ensure parent directory exists
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(cache_path, json)?;
        Ok(())
    }

    /// Initialize and load existing cache data.
    pub fn init_and_load(&mut self) -> Result<bool, FileCacheError> {
        self.init()?;
        self.load()
    }

    /// Compute the content hash of a file.
    pub fn compute_file_hash(path: &Path) -> io::Result<ContentHash> {
        let content = fs::read(path)?;
        Ok(ContentHash::compute(&content))
    }

    /// Get file metadata (mtime, size).
    fn get_file_metadata(path: &Path) -> io::Result<(u64, u64)> {
        let metadata = fs::metadata(path)?;
        let mtime = metadata.modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size = metadata.len();
        Ok((mtime, size))
    }

    /// Check if a file has changed since the last cache.
    ///
    /// Uses mtime and size for quick checks, falls back to content hash
    /// if metadata has changed.
    pub fn check_file(&self, path: &Path) -> FileStatus {
        if !self.enabled {
            return FileStatus::New;
        }

        let cached = match self.entries.get(path) {
            Some(entry) => entry,
            None => return FileStatus::New,
        };

        // Check if file still exists
        let (mtime, size) = match Self::get_file_metadata(path) {
            Ok(meta) => meta,
            Err(_) => return FileStatus::Deleted,
        };

        // Quick check: if mtime and size are same, file is unchanged
        if cached.mtime == mtime && cached.size == size {
            return FileStatus::Unchanged;
        }

        // Mtime or size changed - compute content hash to verify
        match Self::compute_file_hash(path) {
            Ok(hash) => {
                if hash == cached.content_hash {
                    FileStatus::Unchanged
                } else {
                    FileStatus::Modified
                }
            }
            Err(_) => FileStatus::Deleted,
        }
    }

    /// Update the cache entry for a file.
    pub fn update_file(&mut self, path: &Path) -> Result<(), FileCacheError> {
        if !self.enabled {
            return Ok(());
        }

        let (mtime, size) = Self::get_file_metadata(path)?;
        let content_hash = Self::compute_file_hash(path)?;

        let entry = FileCacheEntry {
            content_hash,
            mtime,
            size,
            definitions: Vec::new(),
            module_id: None,
        };

        self.entries.insert(path.to_path_buf(), entry);
        Ok(())
    }

    /// Get the cached entry for a file.
    pub fn get_entry(&self, path: &Path) -> Option<&FileCacheEntry> {
        self.entries.get(path)
    }

    /// Get the content hash for a file.
    pub fn get_content_hash(&self, path: &Path) -> Option<ContentHash> {
        self.entries.get(path).map(|e| e.content_hash)
    }

    /// Register definitions for a file.
    ///
    /// This maps a source file to the DefIds it produces during compilation.
    pub fn register_definitions(&mut self, path: &Path, defs: HashSet<DefId>) {
        if self.enabled {
            self.file_to_defs.insert(path.to_path_buf(), defs);
        }
    }

    /// Register a module ID for a file.
    pub fn register_module(&mut self, path: &Path, module_id: ModuleId) {
        if self.enabled {
            self.file_to_module.insert(path.to_path_buf(), module_id);
        }
    }

    /// Get all definitions from a file.
    pub fn get_definitions(&self, path: &Path) -> Option<&HashSet<DefId>> {
        self.file_to_defs.get(path)
    }

    /// Get the module ID for a file.
    pub fn get_module(&self, path: &Path) -> Option<ModuleId> {
        self.file_to_module.get(path).copied()
    }

    /// Find all files that have changed since the last cache.
    ///
    /// Returns (changed_files, deleted_files).
    pub fn find_changed_files(&self, files: &[PathBuf]) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut changed = Vec::new();
        let mut deleted = Vec::new();

        for path in files {
            match self.check_file(path) {
                FileStatus::Unchanged => {}
                FileStatus::Modified | FileStatus::New => {
                    changed.push(path.clone());
                }
                FileStatus::Deleted => {
                    deleted.push(path.clone());
                }
            }
        }

        // Also check for cached files that no longer exist
        for cached_path in self.entries.keys() {
            if !files.contains(cached_path) && !deleted.contains(cached_path) {
                deleted.push(cached_path.clone());
            }
        }

        (changed, deleted)
    }

    /// Get all definitions that need recompilation due to changed files.
    ///
    /// This returns the DefIds from all changed files.
    pub fn get_invalidated_definitions(&self, changed_files: &[PathBuf]) -> HashSet<DefId> {
        let mut invalidated = HashSet::new();

        for path in changed_files {
            if let Some(defs) = self.file_to_defs.get(path) {
                invalidated.extend(defs.iter().copied());
            }
        }

        invalidated
    }

    /// Get all modules that need recompilation due to changed files.
    pub fn get_invalidated_modules(&self, changed_files: &[PathBuf]) -> HashSet<ModuleId> {
        let mut invalidated = HashSet::new();

        for path in changed_files {
            if let Some(module_id) = self.file_to_module.get(path) {
                invalidated.insert(*module_id);
            }
        }

        invalidated
    }

    /// Remove a file from the cache.
    pub fn remove_file(&mut self, path: &Path) {
        self.entries.remove(path);
        self.file_to_defs.remove(path);
        self.file_to_module.remove(path);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.file_to_defs.clear();
        self.file_to_module.clear();
    }

    /// Get the number of cached files.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get all cached file paths.
    pub fn cached_files(&self) -> impl Iterator<Item = &PathBuf> {
        self.entries.keys()
    }
}

/// Statistics about the file cache.
#[derive(Debug, Clone, Default)]
pub struct FileCacheStats {
    /// Total number of cached files.
    pub total_files: usize,
    /// Number of unchanged files.
    pub unchanged: usize,
    /// Number of modified files.
    pub modified: usize,
    /// Number of new files.
    pub new_files: usize,
    /// Number of deleted files.
    pub deleted: usize,
    /// Total cached size in bytes.
    pub total_size: u64,
}

impl FileCache {
    /// Compute statistics for a set of source files.
    pub fn compute_stats(&self, files: &[PathBuf]) -> FileCacheStats {
        let mut stats = FileCacheStats::default();

        for path in files {
            match self.check_file(path) {
                FileStatus::Unchanged => stats.unchanged += 1,
                FileStatus::Modified => stats.modified += 1,
                FileStatus::New => stats.new_files += 1,
                FileStatus::Deleted => stats.deleted += 1,
            }
        }

        stats.total_files = files.len();
        stats.total_size = self.entries.values().map(|e| e.size).sum();

        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_file_cache_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let cache = FileCache::new(temp_dir.path().to_path_buf());

        let test_file = temp_dir.path().join("test.blood");
        fs::write(&test_file, "fn main() {}").unwrap();

        // New file should be detected as New
        assert_eq!(cache.check_file(&test_file), FileStatus::New);
    }

    #[test]
    fn test_file_cache_unchanged_file() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FileCache::new(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let test_file = temp_dir.path().join("test.blood");
        fs::write(&test_file, "fn main() {}").unwrap();

        // Update cache
        cache.update_file(&test_file).unwrap();

        // File should be unchanged
        assert_eq!(cache.check_file(&test_file), FileStatus::Unchanged);
    }

    #[test]
    fn test_file_cache_modified_file() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FileCache::new(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let test_file = temp_dir.path().join("test.blood");
        fs::write(&test_file, "fn main() {}").unwrap();

        // Update cache
        cache.update_file(&test_file).unwrap();

        // Modify file
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&test_file, "fn main() { 42 }").unwrap();

        // File should be modified
        assert_eq!(cache.check_file(&test_file), FileStatus::Modified);
    }

    #[test]
    fn test_file_cache_deleted_file() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FileCache::new(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let test_file = temp_dir.path().join("test.blood");
        fs::write(&test_file, "fn main() {}").unwrap();

        // Update cache
        cache.update_file(&test_file).unwrap();

        // Delete file
        fs::remove_file(&test_file).unwrap();

        // File should be deleted
        assert_eq!(cache.check_file(&test_file), FileStatus::Deleted);
    }

    #[test]
    fn test_file_cache_save_and_load() {
        let temp_dir = TempDir::new().unwrap();

        let test_file = temp_dir.path().join("test.blood");
        fs::write(&test_file, "fn main() {}").unwrap();

        // Create cache and update
        {
            let mut cache = FileCache::new(temp_dir.path().to_path_buf());
            cache.init().unwrap();
            cache.update_file(&test_file).unwrap();

            // Register some definitions
            let mut defs = HashSet::new();
            defs.insert(DefId::new(0));
            defs.insert(DefId::new(1));
            cache.register_definitions(&test_file, defs);

            cache.save().unwrap();
        }

        // Load in new cache instance
        {
            let mut cache = FileCache::new(temp_dir.path().to_path_buf());
            let loaded = cache.load().unwrap();
            assert!(loaded);

            // File should be unchanged
            assert_eq!(cache.check_file(&test_file), FileStatus::Unchanged);

            // Definitions should be restored
            let defs = cache.get_definitions(&test_file).unwrap();
            assert!(defs.contains(&DefId::new(0)));
            assert!(defs.contains(&DefId::new(1)));
        }
    }

    #[test]
    fn test_file_cache_find_changed_files() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FileCache::new(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let file1 = temp_dir.path().join("file1.blood");
        let file2 = temp_dir.path().join("file2.blood");
        let file3 = temp_dir.path().join("file3.blood");

        fs::write(&file1, "fn f1() {}").unwrap();
        fs::write(&file2, "fn f2() {}").unwrap();
        fs::write(&file3, "fn f3() {}").unwrap();

        // Cache all files
        cache.update_file(&file1).unwrap();
        cache.update_file(&file2).unwrap();
        cache.update_file(&file3).unwrap();

        // Modify file2
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file2, "fn f2_modified() {}").unwrap();

        // Delete file3
        fs::remove_file(&file3).unwrap();

        // Check changes
        let (changed, deleted) = cache.find_changed_files(&[
            file1.clone(),
            file2.clone(),
            file3.clone(),
        ]);

        assert!(changed.contains(&file2));
        assert!(deleted.contains(&file3));
        assert!(!changed.contains(&file1));
    }

    #[test]
    fn test_file_cache_invalidated_definitions() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FileCache::new(temp_dir.path().to_path_buf());

        let file1 = temp_dir.path().join("file1.blood");
        let file2 = temp_dir.path().join("file2.blood");

        // Register definitions for files
        let mut defs1 = HashSet::new();
        defs1.insert(DefId::new(0));
        defs1.insert(DefId::new(1));
        cache.register_definitions(&file1, defs1);

        let mut defs2 = HashSet::new();
        defs2.insert(DefId::new(2));
        defs2.insert(DefId::new(3));
        cache.register_definitions(&file2, defs2);

        // Get invalidated definitions for file1 change
        let invalidated = cache.get_invalidated_definitions(&[file1]);

        assert!(invalidated.contains(&DefId::new(0)));
        assert!(invalidated.contains(&DefId::new(1)));
        assert!(!invalidated.contains(&DefId::new(2)));
        assert!(!invalidated.contains(&DefId::new(3)));
    }

    #[test]
    fn test_file_cache_disabled() {
        let cache = FileCache::disabled();
        let path = PathBuf::from("/nonexistent/file.blood");

        // Disabled cache should always return New status
        assert_eq!(cache.check_file(&path), FileStatus::New);
    }

    #[test]
    fn test_file_cache_compute_stats() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FileCache::new(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let file1 = temp_dir.path().join("unchanged.blood");
        let file2 = temp_dir.path().join("modified.blood");
        let new_file = temp_dir.path().join("new.blood");

        fs::write(&file1, "fn f1() {}").unwrap();
        fs::write(&file2, "fn f2() {}").unwrap();

        // Cache existing files
        cache.update_file(&file1).unwrap();
        cache.update_file(&file2).unwrap();

        // Modify file2
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file2, "fn f2_modified() {}").unwrap();

        // Create new file
        fs::write(&new_file, "fn new() {}").unwrap();

        let stats = cache.compute_stats(&[file1, file2, new_file]);

        assert_eq!(stats.total_files, 3);
        assert_eq!(stats.unchanged, 1);
        assert_eq!(stats.modified, 1);
        assert_eq!(stats.new_files, 1);
    }
}
