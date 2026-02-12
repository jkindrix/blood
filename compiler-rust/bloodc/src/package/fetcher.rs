//! Package fetching from various sources.
//!
//! This module handles downloading packages from:
//! - Registry (blood-lang.org/packages)
//! - Git repositories
//! - Local paths
//!
//! ## Fetching Process
//!
//! ```text
//! Source          Download        Verify          Extract
//!   │                │               │               │
//!   ├─ Registry ────>│ GET tarball ──┼─ checksum ───>│ untar
//!   │                │               │               │
//!   ├─ Git ─────────>│ clone/fetch ──┼─ ref check ──>│ checkout
//!   │                │               │               │
//!   └─ Path ────────>│ (no-op) ──────┼─ exists ─────>│ (symlink)
//! ```

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::cache::PackageCache;
use super::resolver::{PackageId, ResolvedSource};
use super::version::Version;

/// Package fetcher for downloading packages from various sources.
#[derive(Debug)]
#[allow(dead_code)] // Infrastructure for package registry
pub struct Fetcher {
    /// Package cache for storing downloaded packages.
    cache: PackageCache,
    /// Registry URL.
    registry_url: String,
}

impl Fetcher {
    /// Create a new fetcher with a package cache.
    pub fn new(cache: PackageCache) -> Self {
        Fetcher {
            cache,
            registry_url: "https://blood-lang.org/packages".to_string(),
        }
    }

    /// Create a fetcher with a custom registry URL.
    pub fn with_registry(cache: PackageCache, registry_url: String) -> Self {
        Fetcher { cache, registry_url }
    }

    /// Fetch a package from its source.
    pub fn fetch(
        &self,
        id: &PackageId,
        source: &ResolvedSource,
    ) -> Result<PathBuf, FetchError> {
        match source {
            ResolvedSource::Registry { url, checksum } => {
                self.fetch_from_registry(id, url, checksum.as_deref())
            }
            ResolvedSource::Git {
                url,
                reference,
                revision,
            } => self.fetch_from_git(id, url, reference.as_deref(), revision),
            ResolvedSource::Path { path } => self.fetch_from_path(id, path),
        }
    }

    /// Fetch a package from the registry.
    fn fetch_from_registry(
        &self,
        id: &PackageId,
        registry_url: &str,
        checksum: Option<&str>,
    ) -> Result<PathBuf, FetchError> {
        // Check if already cached
        if let Some(path) = self.cache.get_cached(&id.name, &id.version) {
            // Verify checksum if provided
            if let Some(expected) = checksum {
                self.verify_checksum(&path, expected)?;
            }
            return Ok(path);
        }

        // Download URL format: {registry}/api/v1/packages/{name}/{version}/download
        let download_url = format!(
            "{}/api/v1/packages/{}/{}/download",
            registry_url, id.name, id.version
        );

        // Download the tarball
        let response = ureq::get(&download_url)
            .call()
            .map_err(|e| FetchError::Network(format!("failed to download {}: {}", download_url, e)))?;

        if response.status() != 200 {
            return Err(FetchError::NotFound {
                name: id.name.clone(),
                version: id.version.clone(),
            });
        }

        // Read response body into memory
        let mut tarball_data = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut tarball_data)
            .map_err(|e| FetchError::Network(format!("failed to read response: {}", e)))?;

        // Verify checksum if provided (before extraction)
        if let Some(expected) = checksum {
            let (algo, hash) = expected.split_once(':').ok_or_else(|| FetchError::InvalidChecksum {
                expected: expected.to_string(),
                actual: "invalid format".to_string(),
            })?;

            if algo != "sha256" {
                return Err(FetchError::UnsupportedChecksum {
                    algorithm: algo.to_string(),
                });
            }

            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(&tarball_data);
            let actual = hex::encode(hasher.finalize());

            if actual != hash {
                return Err(FetchError::InvalidChecksum {
                    expected: hash.to_string(),
                    actual,
                });
            }
        }

        // Create cache directory for this package
        let cache_dir = self.cache.get_or_create_dir(&id.name, &id.version)?;

        // Extract the tarball
        self.extract_tarball(&tarball_data, &cache_dir)?;

        Ok(cache_dir)
    }

    /// Extract a gzipped tarball to a directory.
    fn extract_tarball(&self, data: &[u8], dest: &Path) -> Result<(), FetchError> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let gz = GzDecoder::new(data);
        let mut archive = Archive::new(gz);

        archive
            .unpack(dest)
            .map_err(|e| FetchError::Io(io::Error::other(format!("failed to extract tarball: {}", e))))?;

        Ok(())
    }

    /// Fetch a package from a git repository.
    fn fetch_from_git(
        &self,
        id: &PackageId,
        url: &str,
        reference: Option<&str>,
        revision: &str,
    ) -> Result<PathBuf, FetchError> {
        use git2::{FetchOptions, RemoteCallbacks, Oid};

        // Check if already cached
        if let Some(path) = self.cache.get_git_cached(&id.name, revision) {
            return Ok(path);
        }

        // Create a cache directory for this git checkout
        let cache_dir = self.cache.get_or_create_git_dir(&id.name, revision)?;

        // Clone the repository
        let mut callbacks = RemoteCallbacks::new();
        callbacks.transfer_progress(|_progress| true);

        let mut fetch_options = FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);

        // Clone with specific options for shallow clone when possible
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);

        // Try to clone the repository
        let repo = builder
            .clone(url, &cache_dir)
            .map_err(|e| FetchError::Git(format!("failed to clone {}: {}", url, e)))?;

        // Parse the revision as an Oid (commit hash)
        let oid = Oid::from_str(revision)
            .map_err(|e| FetchError::Git(format!("invalid revision {}: {}", revision, e)))?;

        // Find the commit (to verify it exists)
        let _commit = repo
            .find_commit(oid)
            .map_err(|e| FetchError::Git(format!("revision {} not found: {}", revision, e)))?;

        // Checkout the commit
        let _ = repo.set_head_detached(oid);
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(|e| FetchError::Git(format!("failed to checkout {}: {}", revision, e)))?;

        // If a reference was provided and doesn't match, log a warning
        if let Some(ref_name) = reference {
            // This is informational - the revision takes precedence
            eprintln!("note: checked out revision {} (reference was {})", revision, ref_name);
        }

        Ok(cache_dir)
    }

    /// "Fetch" a package from a local path (just validate it exists).
    fn fetch_from_path(&self, id: &PackageId, path: &Path) -> Result<PathBuf, FetchError> {
        if !path.exists() {
            return Err(FetchError::PathNotFound {
                package: id.name.clone(),
                path: path.to_path_buf(),
            });
        }

        // Verify it's a valid Blood package (has Blood.toml)
        let manifest_path = path.join("Blood.toml");
        if !manifest_path.exists() {
            return Err(FetchError::InvalidPackage {
                package: id.name.clone(),
                reason: format!("no Blood.toml found at {}", path.display()),
            });
        }

        Ok(path.to_path_buf())
    }

    /// Verify a package checksum.
    fn verify_checksum(&self, path: &Path, expected: &str) -> Result<(), FetchError> {
        // Parse expected checksum format (sha256:...)
        let (algo, hash) = expected.split_once(':').ok_or_else(|| {
            FetchError::InvalidChecksum {
                expected: expected.to_string(),
                actual: "invalid format".to_string(),
            }
        })?;

        if algo != "sha256" {
            return Err(FetchError::UnsupportedChecksum {
                algorithm: algo.to_string(),
            });
        }

        // Compute actual checksum
        let actual = compute_dir_hash(path)?;

        if actual != hash {
            return Err(FetchError::InvalidChecksum {
                expected: hash.to_string(),
                actual,
            });
        }

        Ok(())
    }

    /// Get the package cache.
    pub fn cache(&self) -> &PackageCache {
        &self.cache
    }
}

/// Compute a hash of a directory's contents.
fn compute_dir_hash(path: &Path) -> Result<String, FetchError> {
    use sha2::{Sha256, Digest};

    let mut hasher = Sha256::new();

    // Get all files sorted by path for deterministic hashing
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(path, &mut files)?;
    files.sort();

    for file in files {
        // Include relative path in hash
        if let Ok(rel) = file.strip_prefix(path) {
            hasher.update(rel.to_string_lossy().as_bytes());
        }

        // Include file contents
        let contents = fs::read(&file).map_err(FetchError::Io)?;
        hasher.update(&contents);
    }

    let result = hasher.finalize();
    Ok(hex::encode(result))
}

/// Recursively collect all files in a directory.
fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), FetchError> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir).map_err(FetchError::Io)? {
        let entry = entry.map_err(FetchError::Io)?;
        let path = entry.path();

        if path.is_file() {
            files.push(path);
        } else if path.is_dir() {
            // Skip hidden directories and common ignores
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    collect_files(&path, files)?;
                }
            }
        }
    }

    Ok(())
}

/// Source to fetch a package from.
#[derive(Debug, Clone)]
pub enum FetchSource {
    /// Fetch from the default registry.
    Registry,
    /// Fetch from a custom registry.
    CustomRegistry(String),
    /// Fetch from a git repository.
    Git {
        /// Repository URL.
        url: String,
        /// Branch name.
        branch: Option<String>,
        /// Tag name.
        tag: Option<String>,
        /// Commit revision.
        rev: Option<String>,
    },
    /// Use a local path.
    Path(PathBuf),
}

/// Errors that can occur during package fetching.
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("path not found for package '{package}': {path}")]
    PathNotFound { package: String, path: PathBuf },

    #[error("invalid package '{package}': {reason}")]
    InvalidPackage { package: String, reason: String },

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    InvalidChecksum { expected: String, actual: String },

    #[error("unsupported checksum algorithm: {algorithm}")]
    UnsupportedChecksum { algorithm: String },

    #[error("network error: {0}")]
    Network(String),

    #[error("git error: {0}")]
    Git(String),

    #[error("not yet implemented: {0}")]
    NotImplemented(String),

    #[error("package not found: {name} v{version}")]
    NotFound { name: String, version: Version },

    #[error("cache error: {0}")]
    Cache(#[from] super::cache::CacheError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_fetcher_new() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().to_path_buf());
        let fetcher = Fetcher::new(cache);
        assert_eq!(fetcher.registry_url, "https://blood-lang.org/packages");
    }

    #[test]
    fn test_fetch_from_path_exists() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().join("cache"));
        let fetcher = Fetcher::new(cache);

        // Create a valid package directory
        let pkg_dir = temp.path().join("my-package");
        fs::create_dir(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("Blood.toml"),
            "[package]\nname = \"test\"\nversion = \"1.0.0\"",
        )
        .unwrap();

        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());
        let result = fetcher.fetch_from_path(&id, &pkg_dir);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), pkg_dir);
    }

    #[test]
    fn test_fetch_from_path_missing() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().join("cache"));
        let fetcher = Fetcher::new(cache);

        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());
        let result = fetcher.fetch_from_path(&id, &temp.path().join("nonexistent"));

        assert!(matches!(result, Err(FetchError::PathNotFound { .. })));
    }

    #[test]
    fn test_fetch_from_path_no_manifest() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().join("cache"));
        let fetcher = Fetcher::new(cache);

        // Create directory without Blood.toml
        let pkg_dir = temp.path().join("my-package");
        fs::create_dir(&pkg_dir).unwrap();

        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());
        let result = fetcher.fetch_from_path(&id, &pkg_dir);

        assert!(matches!(result, Err(FetchError::InvalidPackage { .. })));
    }

    #[test]
    fn test_fetch_from_registry_network_error() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().to_path_buf());
        let fetcher = Fetcher::new(cache);

        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());
        // This should fail with a network error since example.com doesn't have this endpoint
        let result = fetcher.fetch_from_registry(&id, "https://example.com", None);

        // Should get a network error (either connection refused or HTTP error)
        assert!(matches!(result, Err(FetchError::Network(_)) | Err(FetchError::NotFound { .. })));
    }

    #[test]
    fn test_fetch_from_git_invalid_revision() {
        let temp = TempDir::new().unwrap();
        let cache = PackageCache::new(temp.path().to_path_buf());
        let fetcher = Fetcher::new(cache);

        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());
        // "abc123" is not a valid 40-character SHA, so this should fail
        let result = fetcher.fetch_from_git(&id, "https://github.com/test/test", None, "abc123");

        // Should get a git error (invalid URL, clone failed, or invalid revision format)
        assert!(matches!(result, Err(FetchError::Git(_))));
    }

    #[test]
    fn test_compute_dir_hash() {
        let temp = TempDir::new().unwrap();

        // Create some files
        fs::write(temp.path().join("a.txt"), "hello").unwrap();
        fs::write(temp.path().join("b.txt"), "world").unwrap();

        let hash1 = compute_dir_hash(temp.path()).unwrap();

        // Hash should be deterministic
        let hash2 = compute_dir_hash(temp.path()).unwrap();
        assert_eq!(hash1, hash2);

        // Modifying content should change hash
        fs::write(temp.path().join("a.txt"), "changed").unwrap();
        let hash3 = compute_dir_hash(temp.path()).unwrap();
        assert_ne!(hash1, hash3);
    }
}
