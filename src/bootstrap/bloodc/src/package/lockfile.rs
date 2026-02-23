//! Lock file (Blood.lock) for recording resolved dependency versions.
//!
//! The lock file ensures reproducible builds by recording the exact versions
//! of all dependencies that were resolved. This file should be:
//! - Committed for applications (ensures reproducible builds)
//! - Not committed for libraries (allows flexibility for consumers)
//!
//! ## Format
//!
//! ```toml
//! # Blood.lock - auto-generated, do not edit manually
//! version = 1
//!
//! [[package]]
//! name = "my_package"
//! version = "0.1.0"
//! source = "registry+https://blood-lang.org/packages"
//! checksum = "sha256:abc123..."
//!
//! [[package]]
//! name = "json"
//! version = "1.2.3"
//! source = "registry+https://blood-lang.org/packages"
//! checksum = "sha256:def456..."
//! dependencies = ["unicode 1.0.0"]
//! ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::version::Version;

/// Serialize a Version as a string.
fn serialize_version<S>(version: &Version, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&version.to_string())
}

/// Deserialize a Version from a string.
fn deserialize_version<'de, D>(deserializer: D) -> Result<Version, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Version::parse(&s).map_err(serde::de::Error::custom)
}

/// The current lock file format version.
pub const LOCKFILE_VERSION: u32 = 1;

/// A lock file recording resolved dependency versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    /// Lock file format version.
    pub version: u32,
    /// Resolved packages.
    #[serde(rename = "package", default)]
    pub packages: Vec<LockedPackage>,
}

impl Lockfile {
    /// Create a new empty lockfile.
    pub fn new() -> Self {
        Lockfile {
            version: LOCKFILE_VERSION,
            packages: Vec::new(),
        }
    }

    /// Parse a lockfile from TOML content.
    #[allow(clippy::should_implement_trait)] // Uses custom LockfileError, not compatible with FromStr trait
    pub fn from_str(content: &str) -> Result<Self, LockfileError> {
        let lockfile: Lockfile = toml::from_str(content)?;
        lockfile.validate()?;
        Ok(lockfile)
    }

    /// Load a lockfile from a path.
    pub fn from_path(path: &Path) -> Result<Self, LockfileError> {
        let content = fs::read_to_string(path)?;
        Self::from_str(&content)
    }

    /// Serialize the lockfile to TOML.
    pub fn to_toml(&self) -> Result<String, LockfileError> {
        let mut output = String::new();
        output.push_str("# Blood.lock - auto-generated, do not edit manually\n\n");
        output.push_str(&format!("version = {}\n\n", self.version));

        for pkg in &self.packages {
            output.push_str("[[package]]\n");
            output.push_str(&format!("name = \"{}\"\n", pkg.name));
            output.push_str(&format!("version = \"{}\"\n", pkg.version));

            if let Some(ref source) = pkg.source {
                output.push_str(&format!("source = \"{}\"\n", source));
            }

            if let Some(ref checksum) = pkg.checksum {
                output.push_str(&format!("checksum = \"{}\"\n", checksum));
            }

            if let Some(ref content_hash) = pkg.content_hash {
                output.push_str(&format!("content-hash = \"{}\"\n", content_hash));
            }

            if !pkg.dependencies.is_empty() {
                let deps: Vec<String> = pkg
                    .dependencies
                    .iter()
                    .map(|d| format!("\"{}\"", d))
                    .collect();
                output.push_str(&format!("dependencies = [{}]\n", deps.join(", ")));
            }

            if !pkg.features.is_empty() {
                let features: Vec<String> =
                    pkg.features.iter().map(|f| format!("\"{}\"", f)).collect();
                output.push_str(&format!("features = [{}]\n", features.join(", ")));
            }

            output.push('\n');
        }

        Ok(output)
    }

    /// Save the lockfile to a path.
    pub fn save(&self, path: &Path) -> Result<(), LockfileError> {
        let content = self.to_toml()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Validate the lockfile.
    fn validate(&self) -> Result<(), LockfileError> {
        if self.version != LOCKFILE_VERSION {
            return Err(LockfileError::VersionMismatch {
                expected: LOCKFILE_VERSION,
                found: self.version,
            });
        }
        Ok(())
    }

    /// Add a package to the lockfile.
    pub fn add_package(&mut self, package: LockedPackage) {
        // Remove existing entry with same name and version
        self.packages
            .retain(|p| !(p.name == package.name && p.version == package.version));
        self.packages.push(package);
    }

    /// Get a package by name and version.
    pub fn get_package(&self, name: &str, version: &Version) -> Option<&LockedPackage> {
        self.packages
            .iter()
            .find(|p| p.name == name && &p.version == version)
    }

    /// Get all versions of a package.
    pub fn get_package_versions(&self, name: &str) -> Vec<&LockedPackage> {
        self.packages.iter().filter(|p| p.name == name).collect()
    }

    /// Build a lookup map by package name.
    pub fn by_name(&self) -> HashMap<&str, Vec<&LockedPackage>> {
        let mut map: HashMap<&str, Vec<&LockedPackage>> = HashMap::new();
        for pkg in &self.packages {
            map.entry(&pkg.name).or_default().push(pkg);
        }
        map
    }

    /// Sort packages for deterministic output.
    pub fn sort(&mut self) {
        self.packages.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.version.cmp(&b.version))
        });
    }
}

impl Default for Lockfile {
    fn default() -> Self {
        Self::new()
    }
}

/// A locked package with its exact resolved version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedPackage {
    /// Package name.
    pub name: String,
    /// Resolved version.
    #[serde(serialize_with = "serialize_version", deserialize_with = "deserialize_version")]
    pub version: Version,
    /// Package source (registry, git, path).
    pub source: Option<String>,
    /// Content checksum (SHA-256 of tarball).
    pub checksum: Option<String>,
    /// Blood content hash for reproducibility.
    #[serde(rename = "content-hash")]
    pub content_hash: Option<String>,
    /// Resolved dependencies (as "name version" strings).
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Enabled features.
    #[serde(default)]
    pub features: Vec<String>,
}

impl LockedPackage {
    /// Create a new locked package.
    pub fn new(name: String, version: Version) -> Self {
        LockedPackage {
            name,
            version,
            source: None,
            checksum: None,
            content_hash: None,
            dependencies: Vec::new(),
            features: Vec::new(),
        }
    }

    /// Create a locked package from a registry source.
    pub fn from_registry(
        name: String,
        version: Version,
        registry: &str,
        checksum: Option<String>,
    ) -> Self {
        LockedPackage {
            name,
            version,
            source: Some(format!("registry+{}", registry)),
            checksum,
            content_hash: None,
            dependencies: Vec::new(),
            features: Vec::new(),
        }
    }

    /// Create a locked package from a git source.
    pub fn from_git(
        name: String,
        version: Version,
        url: &str,
        reference: Option<&str>,
        revision: &str,
    ) -> Self {
        let source = if let Some(reference) = reference {
            format!("git+{}?{}#{}", url, reference, revision)
        } else {
            format!("git+{}#{}", url, revision)
        };

        LockedPackage {
            name,
            version,
            source: Some(source),
            checksum: None,
            content_hash: None,
            dependencies: Vec::new(),
            features: Vec::new(),
        }
    }

    /// Create a locked package from a local path.
    pub fn from_path(name: String, version: Version, path: &Path) -> Self {
        LockedPackage {
            name,
            version,
            source: Some(format!("path+{}", path.display())),
            checksum: None,
            content_hash: None,
            dependencies: Vec::new(),
            features: Vec::new(),
        }
    }

    /// Add a dependency.
    pub fn add_dependency(&mut self, name: &str, version: &Version) {
        self.dependencies.push(format!("{} {}", name, version));
    }

    /// Parse the source to get the package source type.
    pub fn parse_source(&self) -> Option<PackageSource> {
        let source = self.source.as_ref()?;

        if let Some(registry) = source.strip_prefix("registry+") {
            return Some(PackageSource::Registry(registry.to_string()));
        }

        if let Some(git_part) = source.strip_prefix("git+") {
            // Parse git+URL?ref#revision or git+URL#revision
            if let Some(hash_idx) = git_part.find('#') {
                let (url_ref, revision) = git_part.split_at(hash_idx);
                let revision = &revision[1..]; // Skip the '#'

                let (url, reference) = if let Some(q_idx) = url_ref.find('?') {
                    let (u, r) = url_ref.split_at(q_idx);
                    (u.to_string(), Some(r[1..].to_string()))
                } else {
                    (url_ref.to_string(), None)
                };

                return Some(PackageSource::Git {
                    url,
                    reference,
                    revision: revision.to_string(),
                });
            }
        }

        if let Some(path) = source.strip_prefix("path+") {
            return Some(PackageSource::Path(path.into()));
        }

        None
    }
}

/// Package source type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSource {
    /// Registry source (e.g., "https://blood-lang.org/packages").
    Registry(String),
    /// Git repository source.
    Git {
        /// Repository URL.
        url: String,
        /// Reference (branch, tag, etc.).
        reference: Option<String>,
        /// Commit revision.
        revision: String,
    },
    /// Local path.
    Path(std::path::PathBuf),
}

impl PackageSource {
    /// Get the default registry URL.
    pub fn default_registry() -> String {
        "https://blood-lang.org/packages".to_string()
    }
}

/// Errors that can occur with lock files.
#[derive(Debug, Error)]
pub enum LockfileError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("lock file version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },
    #[error("serialization error: {0}")]
    Serialization(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lockfile_new() {
        let lockfile = Lockfile::new();
        assert_eq!(lockfile.version, LOCKFILE_VERSION);
        assert!(lockfile.packages.is_empty());
    }

    #[test]
    fn test_lockfile_parse() {
        let content = r#"
            version = 1

            [[package]]
            name = "test"
            version = "1.0.0"
            source = "registry+https://blood-lang.org/packages"
            checksum = "sha256:abc123"
            dependencies = ["dep 0.1.0"]
        "#;

        let lockfile = Lockfile::from_str(content).unwrap();
        assert_eq!(lockfile.packages.len(), 1);

        let pkg = &lockfile.packages[0];
        assert_eq!(pkg.name, "test");
        assert_eq!(pkg.version, Version::parse("1.0.0").unwrap());
        assert_eq!(
            pkg.source,
            Some("registry+https://blood-lang.org/packages".to_string())
        );
        assert_eq!(pkg.dependencies, vec!["dep 0.1.0"]);
    }

    #[test]
    fn test_lockfile_roundtrip() {
        let mut lockfile = Lockfile::new();

        let mut pkg = LockedPackage::new("test".to_string(), Version::parse("1.0.0").unwrap());
        pkg.source = Some("registry+https://blood-lang.org/packages".to_string());
        pkg.checksum = Some("sha256:abc123".to_string());
        pkg.add_dependency("dep", &Version::parse("0.1.0").unwrap());
        pkg.features = vec!["feature1".to_string()];

        lockfile.add_package(pkg);

        let toml = lockfile.to_toml().unwrap();
        let parsed = Lockfile::from_str(&toml).unwrap();

        assert_eq!(parsed.packages.len(), 1);
        assert_eq!(parsed.packages[0].name, "test");
    }

    #[test]
    fn test_locked_package_from_registry() {
        let pkg = LockedPackage::from_registry(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://blood-lang.org/packages",
            Some("sha256:abc".to_string()),
        );

        assert_eq!(
            pkg.source,
            Some("registry+https://blood-lang.org/packages".to_string())
        );

        let source = pkg.parse_source().unwrap();
        assert!(matches!(source, PackageSource::Registry(_)));
    }

    #[test]
    fn test_locked_package_from_git() {
        let pkg = LockedPackage::from_git(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://github.com/test/test",
            Some("tag=v1.0.0"),
            "abc123",
        );

        let source = pkg.parse_source().unwrap();
        match source {
            PackageSource::Git {
                url,
                reference,
                revision,
            } => {
                assert_eq!(url, "https://github.com/test/test");
                assert_eq!(reference, Some("tag=v1.0.0".to_string()));
                assert_eq!(revision, "abc123");
            }
            _ => panic!("Expected Git source"),
        }
    }

    #[test]
    fn test_locked_package_from_path() {
        let pkg = LockedPackage::from_path(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            Path::new("/local/path"),
        );

        let source = pkg.parse_source().unwrap();
        match source {
            PackageSource::Path(path) => {
                assert_eq!(path, Path::new("/local/path"));
            }
            _ => panic!("Expected Path source"),
        }
    }

    #[test]
    fn test_lockfile_add_package() {
        let mut lockfile = Lockfile::new();

        let pkg1 = LockedPackage::new("test".to_string(), Version::parse("1.0.0").unwrap());
        lockfile.add_package(pkg1);
        assert_eq!(lockfile.packages.len(), 1);

        // Adding same name/version should replace
        let mut pkg2 = LockedPackage::new("test".to_string(), Version::parse("1.0.0").unwrap());
        pkg2.checksum = Some("new".to_string());
        lockfile.add_package(pkg2);
        assert_eq!(lockfile.packages.len(), 1);
        assert_eq!(
            lockfile.packages[0].checksum,
            Some("new".to_string())
        );
    }

    #[test]
    fn test_lockfile_get_package() {
        let mut lockfile = Lockfile::new();

        let pkg = LockedPackage::new("test".to_string(), Version::parse("1.0.0").unwrap());
        lockfile.add_package(pkg);

        let found = lockfile.get_package("test", &Version::parse("1.0.0").unwrap());
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "test");

        let not_found = lockfile.get_package("other", &Version::parse("1.0.0").unwrap());
        assert!(not_found.is_none());
    }

    #[test]
    fn test_lockfile_sort() {
        let mut lockfile = Lockfile::new();

        lockfile.add_package(LockedPackage::new(
            "zebra".to_string(),
            Version::parse("1.0.0").unwrap(),
        ));
        lockfile.add_package(LockedPackage::new(
            "alpha".to_string(),
            Version::parse("1.0.0").unwrap(),
        ));
        lockfile.add_package(LockedPackage::new(
            "alpha".to_string(),
            Version::parse("0.5.0").unwrap(),
        ));

        lockfile.sort();

        assert_eq!(lockfile.packages[0].name, "alpha");
        assert_eq!(lockfile.packages[0].version, Version::parse("0.5.0").unwrap());
        assert_eq!(lockfile.packages[1].name, "alpha");
        assert_eq!(lockfile.packages[1].version, Version::parse("1.0.0").unwrap());
        assert_eq!(lockfile.packages[2].name, "zebra");
    }
}
