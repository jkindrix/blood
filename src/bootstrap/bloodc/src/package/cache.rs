//! Package cache for storing downloaded packages.
//!
//! The package cache provides:
//! - Content-addressed storage for reproducibility
//! - Deduplication of shared dependencies
//! - Fast lookup of cached packages
//!
//! ## Cache Structure
//!
//! ```text
//! ~/.blood/
//! └── cache/
//!     ├── registry/
//!     │   └── blood-lang.org-packages/
//!     │       └── json/
//!     │           └── 1.2.3/
//!     │               └── ... (extracted package)
//!     ├── git/
//!     │   └── github.com-blood-lang-logger/
//!     │       └── abc123/
//!     │           └── ... (checked out repo)
//!     └── checkouts/
//!         └── sha256:abc123.../
//!             └── ... (content-addressed storage)
//! ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::version::Version;

/// Package cache for storing and retrieving downloaded packages.
#[derive(Debug)]
pub struct PackageCache {
    /// Root directory for the cache.
    root: PathBuf,
    /// In-memory index of cached packages.
    index: HashMap<CacheKey, CachedPackage>,
}

/// Key for looking up cached packages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    name: String,
    version: Version,
}

impl PackageCache {
    /// Create a new package cache at the specified root.
    pub fn new(root: PathBuf) -> Self {
        PackageCache {
            root,
            index: HashMap::new(),
        }
    }

    /// Create a package cache at the default location (~/.blood/cache).
    pub fn default_location() -> Result<Self, CacheError> {
        let home = dirs::home_dir().ok_or(CacheError::NoHomeDirectory)?;
        let root = home.join(".blood").join("cache");
        Ok(Self::new(root))
    }

    /// Get the cache root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Initialize the cache directory structure.
    pub fn init(&self) -> Result<(), CacheError> {
        fs::create_dir_all(self.registry_dir())?;
        fs::create_dir_all(self.git_dir())?;
        fs::create_dir_all(self.checkouts_dir())?;
        Ok(())
    }

    /// Get the registry cache directory.
    fn registry_dir(&self) -> PathBuf {
        self.root.join("registry")
    }

    /// Get the git cache directory.
    fn git_dir(&self) -> PathBuf {
        self.root.join("git")
    }

    /// Get the content-addressed checkouts directory.
    fn checkouts_dir(&self) -> PathBuf {
        self.root.join("checkouts")
    }

    /// Get the path for a registry package.
    fn registry_package_path(&self, registry: &str, name: &str, version: &Version) -> PathBuf {
        // Sanitize registry URL for use as directory name
        let registry_dir = sanitize_url(registry);
        self.registry_dir()
            .join(registry_dir)
            .join(name)
            .join(version.to_string())
    }

    /// Get the path for a git checkout.
    fn git_checkout_path(&self, url: &str, revision: &str) -> PathBuf {
        let repo_dir = sanitize_url(url);
        self.git_dir().join(repo_dir).join(revision)
    }

    /// Get a cached registry package path if it exists.
    pub fn get_cached(&self, name: &str, version: &Version) -> Option<PathBuf> {
        let default_registry = "blood-lang.org-packages";
        let path = self.registry_package_path(default_registry, name, version);

        if path.exists() && path.join("Blood.toml").exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Get a cached git checkout path if it exists.
    pub fn get_git_cached(&self, _name: &str, revision: &str) -> Option<PathBuf> {
        // Search for the package in git cache
        let git_dir = self.git_dir();
        if !git_dir.exists() {
            return None;
        }

        for entry in fs::read_dir(&git_dir).ok()? {
            let entry = entry.ok()?;
            let repo_dir = entry.path();

            if repo_dir.is_dir() {
                let checkout = repo_dir.join(revision);
                if checkout.exists() && checkout.join("Blood.toml").exists() {
                    return Some(checkout);
                }
            }
        }

        None
    }

    /// Store a registry package in the cache.
    pub fn store_registry(
        &mut self,
        registry: &str,
        name: &str,
        version: &Version,
        source: &Path,
    ) -> Result<CachedPackage, CacheError> {
        let dest = self.registry_package_path(registry, name, version);

        // Create parent directories
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy the package
        copy_dir_recursive(source, &dest)?;

        let cached = CachedPackage {
            name: name.to_string(),
            version: version.clone(),
            path: dest,
            source_type: CacheSourceType::Registry,
        };

        self.index.insert(
            CacheKey {
                name: name.to_string(),
                version: version.clone(),
            },
            cached.clone(),
        );

        Ok(cached)
    }

    /// Store a git checkout in the cache.
    pub fn store_git(
        &mut self,
        url: &str,
        revision: &str,
        name: &str,
        version: &Version,
        source: &Path,
    ) -> Result<CachedPackage, CacheError> {
        let dest = self.git_checkout_path(url, revision);

        // Create parent directories
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy the checkout
        copy_dir_recursive(source, &dest)?;

        let cached = CachedPackage {
            name: name.to_string(),
            version: version.clone(),
            path: dest,
            source_type: CacheSourceType::Git,
        };

        self.index.insert(
            CacheKey {
                name: name.to_string(),
                version: version.clone(),
            },
            cached.clone(),
        );

        Ok(cached)
    }

    /// Store a package by content hash.
    pub fn store_by_hash(
        &mut self,
        hash: &str,
        name: &str,
        version: &Version,
        source: &Path,
    ) -> Result<CachedPackage, CacheError> {
        let dest = self.checkouts_dir().join(hash);

        // Create parent directories
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy the package
        copy_dir_recursive(source, &dest)?;

        let cached = CachedPackage {
            name: name.to_string(),
            version: version.clone(),
            path: dest,
            source_type: CacheSourceType::ContentAddressed,
        };

        self.index.insert(
            CacheKey {
                name: name.to_string(),
                version: version.clone(),
            },
            cached.clone(),
        );

        Ok(cached)
    }

    /// Get a package by content hash.
    pub fn get_by_hash(&self, hash: &str) -> Option<PathBuf> {
        let path = self.checkouts_dir().join(hash);
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Remove a package from the cache.
    pub fn remove(&mut self, name: &str, version: &Version) -> Result<(), CacheError> {
        let key = CacheKey {
            name: name.to_string(),
            version: version.clone(),
        };

        if let Some(cached) = self.index.remove(&key) {
            if cached.path.exists() {
                fs::remove_dir_all(&cached.path)?;
            }
        }

        Ok(())
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) -> Result<(), CacheError> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        self.index.clear();
        Ok(())
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let mut stats = CacheStats::default();

        // Count registry packages
        if let Ok(entries) = fs::read_dir(self.registry_dir()) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    stats.registry_packages += count_packages(&entry.path());
                }
            }
        }

        // Count git checkouts
        if let Ok(entries) = fs::read_dir(self.git_dir()) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    stats.git_checkouts += count_dirs(&entry.path());
                }
            }
        }

        // Calculate total size
        stats.total_size = dir_size(&self.root);

        stats
    }

    /// List all cached packages.
    pub fn list(&self) -> Vec<&CachedPackage> {
        self.index.values().collect()
    }

    /// Get or create a directory for a registry package.
    pub fn get_or_create_dir(&self, name: &str, version: &Version) -> Result<PathBuf, CacheError> {
        let default_registry = "blood-lang.org-packages";
        let path = self.registry_package_path(default_registry, name, version);

        if !path.exists() {
            fs::create_dir_all(&path)?;
        }

        Ok(path)
    }

    /// Get or create a directory for a git checkout.
    pub fn get_or_create_git_dir(&self, name: &str, revision: &str) -> Result<PathBuf, CacheError> {
        let path = self.git_dir().join(sanitize_url(name)).join(revision);

        if !path.exists() {
            fs::create_dir_all(&path)?;
        }

        Ok(path)
    }

    /// Scan the cache directory and rebuild the index.
    pub fn rebuild_index(&mut self) -> Result<(), CacheError> {
        self.index.clear();

        // Scan registry directory
        let registry_dir = self.registry_dir();
        if registry_dir.exists() {
            for registry_entry in fs::read_dir(&registry_dir)?.flatten() {
                let registry_path = registry_entry.path();
                if !registry_path.is_dir() {
                    continue;
                }

                for pkg_entry in fs::read_dir(&registry_path)?.flatten() {
                    let pkg_path = pkg_entry.path();
                    if !pkg_path.is_dir() {
                        continue;
                    }

                    let name = pkg_path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    for version_entry in fs::read_dir(&pkg_path)?.flatten() {
                        let version_path = version_entry.path();
                        if !version_path.is_dir() {
                            continue;
                        }

                        let version_str = version_path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("");

                        if let Ok(version) = Version::parse(version_str) {
                            let cached = CachedPackage {
                                name: name.clone(),
                                version: version.clone(),
                                path: version_path,
                                source_type: CacheSourceType::Registry,
                            };

                            self.index.insert(
                                CacheKey {
                                    name: name.clone(),
                                    version,
                                },
                                cached,
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// A cached package.
#[derive(Debug, Clone)]
pub struct CachedPackage {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
    /// Path to the cached package.
    pub path: PathBuf,
    /// Source type.
    pub source_type: CacheSourceType,
}

/// Type of cache source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSourceType {
    /// From a package registry.
    Registry,
    /// From a git repository.
    Git,
    /// Content-addressed storage.
    ContentAddressed,
}

/// Cache statistics.
#[derive(Debug, Default)]
pub struct CacheStats {
    /// Number of cached registry packages.
    pub registry_packages: usize,
    /// Number of git checkouts.
    pub git_checkouts: usize,
    /// Total cache size in bytes.
    pub total_size: u64,
}

/// Errors that can occur with the package cache.
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("no home directory found")]
    NoHomeDirectory,
    #[error("invalid cache format")]
    InvalidFormat,
}

/// Sanitize a URL for use as a directory name.
fn sanitize_url(url: &str) -> String {
    url.replace("://", "-")
        .replace(['/', ':', '.'], "-")
        .trim_end_matches('-')
        .to_string()
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Count packages in a directory (directories with Blood.toml).
fn count_packages(dir: &Path) -> usize {
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.join("Blood.toml").exists() {
                    count += 1;
                } else {
                    count += count_packages(&path);
                }
            }
        }
    }

    count
}

/// Count directories.
fn count_dirs(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count())
        .unwrap_or(0)
}

/// Calculate directory size recursively.
fn dir_size(path: &Path) -> u64 {
    let mut size = 0;

    if path.is_file() {
        return fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_file() {
                size += fs::metadata(&entry_path).map(|m| m.len()).unwrap_or(0);
            } else if entry_path.is_dir() {
                size += dir_size(&entry_path);
            }
        }
    }

    size
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_package_cache_new() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().to_path_buf());
        assert_eq!(cache.root(), temp.path());
    }

    #[test]
    fn test_package_cache_init() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().to_path_buf());
        cache.init().unwrap();

        assert!(cache.registry_dir().exists());
        assert!(cache.git_dir().exists());
        assert!(cache.checkouts_dir().exists());
    }

    #[test]
    fn test_sanitize_url() {
        assert_eq!(
            sanitize_url("https://blood-lang.org/packages"),
            "https-blood-lang-org-packages"
        );
        assert_eq!(
            sanitize_url("https://github.com/user/repo"),
            "https-github-com-user-repo"
        );
    }

    #[test]
    fn test_store_and_get_registry() {
        let temp = TempDir::new().unwrap();
        let mut cache = PackageCache::new(temp.path().join("cache"));
        cache.init().unwrap();

        // Create a source package
        let src = temp.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(
            src.join("Blood.toml"),
            "[package]\nname = \"test\"\nversion = \"1.0.0\"",
        )
        .unwrap();

        let version = Version::parse("1.0.0").unwrap();
        // Use the default registry name so get_cached can find it
        let cached = cache.store_registry("blood-lang.org-packages", "test", &version, &src).unwrap();

        assert_eq!(cached.name, "test");
        assert_eq!(cached.version, version);
        assert!(cached.path.exists());

        // Should be able to retrieve it
        let retrieved = cache.get_cached("test", &version);
        assert!(retrieved.is_some());
    }

    #[test]
    fn test_get_cached_missing() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().to_path_buf());

        let version = Version::parse("1.0.0").unwrap();
        let result = cache.get_cached("nonexistent", &version);
        assert!(result.is_none());
    }

    #[test]
    fn test_cache_stats() {
        let temp = TempDir::new().unwrap();
        let mut cache = PackageCache::new(temp.path().to_path_buf());
        cache.init().unwrap();

        // Create a package
        let src = temp.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(
            src.join("Blood.toml"),
            "[package]\nname = \"test\"\nversion = \"1.0.0\"",
        )
        .unwrap();
        fs::write(src.join("lib.blood"), "// code").unwrap();

        let version = Version::parse("1.0.0").unwrap();
        cache.store_registry("test-registry", "test", &version, &src).unwrap();

        let stats = cache.stats();
        assert!(stats.total_size > 0);
    }

    #[test]
    fn test_cache_clear() {
        let temp = TempDir::new().unwrap();
        let mut cache = PackageCache::new(temp.path().join("cache"));
        cache.init().unwrap();

        // Store something
        let src = temp.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(
            src.join("Blood.toml"),
            "[package]\nname = \"test\"\nversion = \"1.0.0\"",
        )
        .unwrap();

        let version = Version::parse("1.0.0").unwrap();
        cache.store_registry("test-registry", "test", &version, &src).unwrap();

        // Clear the cache
        cache.clear().unwrap();

        assert!(!cache.root().exists());
        assert!(cache.index.is_empty());
    }

    #[test]
    fn test_copy_dir_recursive() {
        let temp = TempDir::new().unwrap();

        // Create source directory with nested structure
        let src = temp.path().join("src");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("file1.txt"), "content1").unwrap();
        fs::write(src.join("nested/file2.txt"), "content2").unwrap();

        // Copy to destination
        let dst = temp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        // Verify structure
        assert!(dst.join("file1.txt").exists());
        assert!(dst.join("nested/file2.txt").exists());
        assert_eq!(fs::read_to_string(dst.join("file1.txt")).unwrap(), "content1");
        assert_eq!(fs::read_to_string(dst.join("nested/file2.txt")).unwrap(), "content2");
    }
}
