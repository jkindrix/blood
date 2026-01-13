//! Project management for multi-file Blood projects.
//!
//! This module provides support for:
//! - `Blood.toml` manifest parsing
//! - Module file resolution
//! - Dependency graph construction
//! - Incremental compilation with file-level caching

mod manifest;
mod resolve;
mod graph;
mod file_cache;
mod compiler;

pub use manifest::{Manifest, Package, BinTarget, LibTarget, Edition, ManifestError};
pub use resolve::{ModuleResolver, ModuleTree, ModuleId, Module};
pub use graph::{DependencyGraph, GraphError};
pub use file_cache::{FileCache, FileCacheEntry, FileCacheStats, FileCacheError, FileStatus};
pub use compiler::{ProjectCompiler, ProjectCompilerBuilder, IncrementalAnalysis, IncrementalStats};

use std::path::{Path, PathBuf};

/// Discover the project root by searching upward for Blood.toml.
///
/// Returns `None` if no Blood.toml is found (single-file mode).
pub fn discover_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_file() {
        start.parent()?
    } else {
        start
    };

    loop {
        let manifest_path = current.join("Blood.toml");
        if manifest_path.exists() {
            return Some(current.to_path_buf());
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => return None,
        }
    }
}

/// Load a project manifest from a directory.
pub fn load_manifest(project_root: &Path) -> Result<Manifest, ManifestError> {
    let manifest_path = project_root.join("Blood.toml");
    Manifest::from_path(&manifest_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_discover_project_root_finds_manifest() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Create Blood.toml
        fs::write(root.join("Blood.toml"), "[package]\nname = \"test\"").unwrap();

        // Create nested directory
        let nested = root.join("src").join("deep");
        fs::create_dir_all(&nested).unwrap();

        // Starting from nested dir should find root
        let found = discover_project_root(&nested);
        assert_eq!(found, Some(root.to_path_buf()));
    }

    #[test]
    fn test_discover_project_root_none_when_missing() {
        let temp = TempDir::new().unwrap();
        let result = discover_project_root(temp.path());
        assert!(result.is_none());
    }
}
