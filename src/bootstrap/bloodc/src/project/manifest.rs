//! Blood.toml manifest parsing and validation.
//!
//! The manifest defines a Blood project's metadata, dependencies, and targets.
//!
//! # Example Manifest
//!
//! ```toml
//! [package]
//! name = "my_project"
//! version = "0.1.0"
//! edition = "2024"
//! description = "An example Blood project"
//! authors = ["Alice <alice@example.com>"]
//!
//! [dependencies]
//! # future: external packages
//!
//! [[bin]]
//! name = "main"
//! path = "src/main.blood"
//!
//! [lib]
//! path = "src/lib.blood"
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur when parsing a manifest.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to read manifest file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("failed to parse manifest: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("missing required field: {field}")]
    MissingField { field: String },

    #[error("invalid edition: {edition}")]
    InvalidEdition { edition: String },

    #[error("duplicate target name: {name}")]
    DuplicateTarget { name: String },

    #[error("path not found: {path}")]
    PathNotFound { path: PathBuf },
}

/// The Blood language edition (determines language features).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Edition {
    /// The 2024 edition (initial release)
    #[default]
    #[serde(rename = "2024")]
    Edition2024,
}

impl Edition {
    /// Parse an edition from a string.
    #[allow(clippy::should_implement_trait)] // Uses custom ManifestError, not compatible with FromStr trait
    pub fn from_str(s: &str) -> Result<Self, ManifestError> {
        match s {
            "2024" => Ok(Edition::Edition2024),
            other => Err(ManifestError::InvalidEdition {
                edition: other.to_string(),
            }),
        }
    }
}

impl std::fmt::Display for Edition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Edition::Edition2024 => write!(f, "2024"),
        }
    }
}

/// Package metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Package name (required)
    pub name: String,

    /// Package version (semver, required)
    pub version: String,

    /// Language edition
    #[serde(default)]
    pub edition: Edition,

    /// Package description
    pub description: Option<String>,

    /// Package authors
    #[serde(default)]
    pub authors: Vec<String>,

    /// Package license
    pub license: Option<String>,

    /// Repository URL
    pub repository: Option<String>,

    /// Homepage URL
    pub homepage: Option<String>,

    /// Keywords for discovery
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Categories for classification
    #[serde(default)]
    pub categories: Vec<String>,

    /// Minimum Blood compiler version
    #[serde(rename = "blood-version")]
    pub blood_version: Option<String>,
}

/// A binary target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinTarget {
    /// Binary name (defaults to package name if omitted)
    pub name: Option<String>,

    /// Path to the binary source file
    pub path: PathBuf,

    /// Whether to use the default main entry point
    #[serde(default = "default_true")]
    pub main: bool,
}

fn default_true() -> bool {
    true
}

/// A library target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibTarget {
    /// Library name (defaults to package name if omitted)
    pub name: Option<String>,

    /// Path to the library source file
    pub path: PathBuf,

    /// Library type: "lib" (default), "staticlib", "cdylib"
    #[serde(rename = "crate-type", default)]
    pub crate_type: Vec<String>,
}

/// A test target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestTarget {
    /// Test name
    pub name: String,

    /// Path to the test source file
    pub path: PathBuf,
}

/// A benchmark target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchTarget {
    /// Benchmark name
    pub name: String,

    /// Path to the benchmark source file
    pub path: PathBuf,
}

/// An example target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExampleTarget {
    /// Example name
    pub name: String,

    /// Path to the example source file
    pub path: PathBuf,
}

/// Dependency specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    /// Simple version string: `foo = "1.0"`
    Version(String),

    /// Detailed specification
    Detailed(DetailedDependency),
}

/// Detailed dependency specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedDependency {
    /// Version requirement
    pub version: Option<String>,

    /// Git repository URL
    pub git: Option<String>,

    /// Git branch
    pub branch: Option<String>,

    /// Git tag
    pub tag: Option<String>,

    /// Git revision
    pub rev: Option<String>,

    /// Local path
    pub path: Option<PathBuf>,

    /// Optional features to enable
    #[serde(default)]
    pub features: Vec<String>,

    /// Whether this dependency is optional
    #[serde(default)]
    pub optional: bool,

    /// Package name (if different from key)
    pub package: Option<String>,
}

/// Build profile settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    /// Optimization level: 0-3 or "s" for size
    #[serde(rename = "opt-level")]
    pub opt_level: Option<String>,

    /// Enable debug info
    pub debug: Option<bool>,

    /// Enable link-time optimization
    pub lto: Option<bool>,

    /// Panic strategy: "unwind" or "abort"
    pub panic: Option<String>,

    /// Code generation units
    #[serde(rename = "codegen-units")]
    pub codegen_units: Option<u32>,

    /// Strip symbols from binary
    pub strip: Option<String>,
}

/// Build configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildSection {
    /// Build output directory (default: "build")
    #[serde(default, rename = "build-dir")]
    pub build_dir: Option<String>,
}

/// The complete Blood.toml manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Package metadata (required)
    pub package: Package,

    /// Dependencies
    #[serde(default)]
    pub dependencies: HashMap<String, Dependency>,

    /// Development dependencies
    #[serde(default, rename = "dev-dependencies")]
    pub dev_dependencies: HashMap<String, Dependency>,

    /// Build dependencies
    #[serde(default, rename = "build-dependencies")]
    pub build_dependencies: HashMap<String, Dependency>,

    /// Binary targets
    #[serde(default, rename = "bin")]
    pub bin: Vec<BinTarget>,

    /// Library target
    pub lib: Option<LibTarget>,

    /// Test targets
    #[serde(default, rename = "test")]
    pub test: Vec<TestTarget>,

    /// Benchmark targets
    #[serde(default, rename = "bench")]
    pub bench: Vec<BenchTarget>,

    /// Example targets
    #[serde(default, rename = "example")]
    pub example: Vec<ExampleTarget>,

    /// Build profiles
    #[serde(default)]
    pub profile: ProfileSection,

    /// Build configuration
    #[serde(default)]
    pub build: BuildSection,

    /// Feature definitions
    #[serde(default)]
    pub features: HashMap<String, Vec<String>>,
}

/// Profile section containing release/dev/test profiles.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileSection {
    /// Release profile
    pub release: Option<Profile>,

    /// Development profile
    pub dev: Option<Profile>,

    /// Test profile
    pub test: Option<Profile>,

    /// Bench profile
    pub bench: Option<Profile>,
}

impl Manifest {
    /// Parse a manifest from a TOML string.
    #[allow(clippy::should_implement_trait)] // Uses custom ManifestError, not compatible with FromStr trait
    pub fn from_str(content: &str) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(content)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Load a manifest from a file path.
    pub fn from_path(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_str(&content)
    }

    /// Validate the manifest.
    fn validate(&self) -> Result<(), ManifestError> {
        // Package name is required
        if self.package.name.is_empty() {
            return Err(ManifestError::MissingField {
                field: "package.name".to_string(),
            });
        }

        // Version is required
        if self.package.version.is_empty() {
            return Err(ManifestError::MissingField {
                field: "package.version".to_string(),
            });
        }

        // Check for duplicate binary names
        let mut bin_names = std::collections::HashSet::new();
        for bin in &self.bin {
            let name = bin.name.as_ref().unwrap_or(&self.package.name);
            if !bin_names.insert(name.clone()) {
                return Err(ManifestError::DuplicateTarget {
                    name: name.clone(),
                });
            }
        }

        Ok(())
    }

    /// Get the effective library name.
    pub fn lib_name(&self) -> Option<&str> {
        self.lib.as_ref().map(|lib| {
            lib.name.as_deref().unwrap_or(&self.package.name)
        })
    }

    /// Get all binary targets with resolved names.
    pub fn resolved_bins(&self) -> Vec<(&str, &PathBuf)> {
        self.bin.iter().map(|bin| {
            let name = bin.name.as_deref().unwrap_or(&self.package.name);
            (name, &bin.path)
        }).collect()
    }

    /// Infer default targets if none are specified.
    ///
    /// If no targets are specified, looks for:
    /// - `src/main.blood` -> default binary
    /// - `src/lib.blood` -> default library
    pub fn with_defaults(mut self, project_root: &Path) -> Self {
        let src_main = project_root.join("src/main.blood");
        let src_lib = project_root.join("src/lib.blood");

        // If no bin targets specified and src/main.blood exists
        if self.bin.is_empty() && src_main.exists() {
            self.bin.push(BinTarget {
                name: None,
                path: PathBuf::from("src/main.blood"),
                main: true,
            });
        }

        // If no lib target specified and src/lib.blood exists
        if self.lib.is_none() && src_lib.exists() {
            self.lib = Some(LibTarget {
                name: None,
                path: PathBuf::from("src/lib.blood"),
                crate_type: vec!["lib".to_string()],
            });
        }

        self
    }

    /// Create a minimal manifest for a new project.
    pub fn new(name: &str, version: &str) -> Self {
        Manifest {
            package: Package {
                name: name.to_string(),
                version: version.to_string(),
                edition: Edition::Edition2024,
                description: None,
                authors: Vec::new(),
                license: None,
                repository: None,
                homepage: None,
                keywords: Vec::new(),
                categories: Vec::new(),
                blood_version: None,
            },
            dependencies: HashMap::new(),
            dev_dependencies: HashMap::new(),
            build_dependencies: HashMap::new(),
            bin: Vec::new(),
            lib: None,
            test: Vec::new(),
            bench: Vec::new(),
            example: Vec::new(),
            profile: ProfileSection::default(),
            build: BuildSection::default(),
            features: HashMap::new(),
        }
    }

    /// Serialize the manifest to TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_manifest() {
        let content = r#"
            [package]
            name = "test_project"
            version = "0.1.0"
        "#;

        let manifest = Manifest::from_str(content).unwrap();
        assert_eq!(manifest.package.name, "test_project");
        assert_eq!(manifest.package.version, "0.1.0");
        assert_eq!(manifest.package.edition, Edition::Edition2024);
    }

    #[test]
    fn test_parse_full_manifest() {
        let content = r#"
            [package]
            name = "my_project"
            version = "1.0.0"
            edition = "2024"
            description = "A test project"
            authors = ["Alice <alice@example.com>"]
            license = "MIT"

            [dependencies]
            foo = "1.0"
            bar = { version = "2.0", features = ["serde"] }

            [[bin]]
            name = "main"
            path = "src/main.blood"

            [lib]
            path = "src/lib.blood"

            [profile.release]
            opt-level = "3"
            lto = true
        "#;

        let manifest = Manifest::from_str(content).unwrap();
        assert_eq!(manifest.package.name, "my_project");
        assert_eq!(manifest.package.description, Some("A test project".to_string()));
        assert_eq!(manifest.bin.len(), 1);
        assert!(manifest.lib.is_some());
        assert_eq!(manifest.dependencies.len(), 2);
    }

    #[test]
    fn test_missing_name_error() {
        let content = r#"
            [package]
            version = "0.1.0"
        "#;

        // toml parsing will fail because name is missing and not optional
        let result = Manifest::from_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_bin_error() {
        let content = r#"
            [package]
            name = "test"
            version = "0.1.0"

            [[bin]]
            name = "foo"
            path = "src/foo.blood"

            [[bin]]
            name = "foo"
            path = "src/bar.blood"
        "#;

        let result = Manifest::from_str(content);
        assert!(matches!(result, Err(ManifestError::DuplicateTarget { .. })));
    }

    #[test]
    fn test_edition_parsing() {
        assert_eq!(Edition::from_str("2024").unwrap(), Edition::Edition2024);
        assert!(Edition::from_str("2025").is_err());
    }

    #[test]
    fn test_new_manifest() {
        let manifest = Manifest::new("my_project", "0.1.0");
        assert_eq!(manifest.package.name, "my_project");
        assert_eq!(manifest.package.version, "0.1.0");
    }

    #[test]
    fn test_resolved_bins() {
        let content = r#"
            [package]
            name = "test"
            version = "0.1.0"

            [[bin]]
            path = "src/main.blood"

            [[bin]]
            name = "other"
            path = "src/other.blood"
        "#;

        let manifest = Manifest::from_str(content).unwrap();
        let bins = manifest.resolved_bins();

        // First bin uses package name, second has explicit name
        assert_eq!(bins.len(), 2);
        assert_eq!(bins[0].0, "test");
        assert_eq!(bins[1].0, "other");
    }

    #[test]
    fn test_build_section_defaults() {
        let content = r#"
            [package]
            name = "test"
            version = "0.1.0"
        "#;

        let manifest = Manifest::from_str(content).unwrap();
        assert!(manifest.build.build_dir.is_none());
    }

    #[test]
    fn test_build_section_custom() {
        let content = r#"
            [package]
            name = "test"
            version = "0.1.0"

            [build]
            build-dir = "out"
        "#;

        let manifest = Manifest::from_str(content).unwrap();
        assert_eq!(manifest.build.build_dir, Some("out".to_string()));
    }
}
