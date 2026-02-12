//! Dependency resolution for Blood packages.
//!
//! This module implements a dependency resolver that:
//! - Parses dependency specifications from Blood.toml
//! - Resolves version constraints to concrete versions
//! - Detects and reports conflicts
//! - Produces a resolution plan for fetching packages
//!
//! ## Resolution Algorithm
//!
//! The resolver uses a SAT-solver inspired approach:
//! 1. Start with root package dependencies
//! 2. For each dependency, find compatible versions
//! 3. Recursively resolve transitive dependencies
//! 4. Backtrack on conflicts
//! 5. Prefer locked versions when available

use std::collections::{HashMap, HashSet};

use thiserror::Error;

use super::lockfile::{Lockfile, LockedPackage, PackageSource};
use super::version::{Version, VersionReq};
use crate::project::Manifest;

/// Unique identifier for a package in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageId {
    /// Package name.
    pub name: String,
    /// Resolved version.
    pub version: Version,
}

impl PackageId {
    /// Create a new package ID.
    pub fn new(name: String, version: Version) -> Self {
        PackageId { name, version }
    }
}

impl std::fmt::Display for PackageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} v{}", self.name, self.version)
    }
}

/// Result of dependency resolution.
#[derive(Debug, Clone)]
pub struct Resolution {
    /// All resolved packages.
    pub packages: Vec<ResolvedPackage>,
    /// Dependency graph (edges are "depends on" relationships).
    pub graph: HashMap<PackageId, Vec<PackageId>>,
    /// Packages that need to be fetched.
    pub to_fetch: Vec<PackageId>,
    /// Packages already available locally.
    pub available: Vec<PackageId>,
}

impl Resolution {
    /// Create a new empty resolution.
    pub fn new() -> Self {
        Resolution {
            packages: Vec::new(),
            graph: HashMap::new(),
            to_fetch: Vec::new(),
            available: Vec::new(),
        }
    }

    /// Get a resolved package by ID.
    pub fn get(&self, id: &PackageId) -> Option<&ResolvedPackage> {
        self.packages.iter().find(|p| &p.id == id)
    }

    /// Get all packages in topological order (dependencies before dependents).
    pub fn topological_order(&self) -> Vec<&ResolvedPackage> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut visiting = HashSet::new();

        for pkg in &self.packages {
            self.topo_visit(&pkg.id, &mut visited, &mut visiting, &mut result);
        }

        result
    }

    fn topo_visit<'a>(
        &'a self,
        id: &PackageId,
        visited: &mut HashSet<PackageId>,
        visiting: &mut HashSet<PackageId>,
        result: &mut Vec<&'a ResolvedPackage>,
    ) {
        if visited.contains(id) {
            return;
        }
        if visiting.contains(id) {
            // Cycle detected, but we should have caught this earlier
            return;
        }

        visiting.insert(id.clone());

        if let Some(deps) = self.graph.get(id) {
            for dep in deps {
                self.topo_visit(dep, visited, visiting, result);
            }
        }

        visiting.remove(id);
        visited.insert(id.clone());

        if let Some(pkg) = self.get(id) {
            result.push(pkg);
        }
    }

    /// Generate a lockfile from this resolution.
    pub fn to_lockfile(&self) -> Lockfile {
        let mut lockfile = Lockfile::new();

        for pkg in &self.packages {
            let mut locked = LockedPackage::new(pkg.id.name.clone(), pkg.id.version.clone());

            // Set source
            locked.source = match &pkg.source {
                ResolvedSource::Registry { url, checksum } => {
                    locked.checksum = checksum.clone();
                    Some(format!("registry+{}", url))
                }
                ResolvedSource::Git {
                    url,
                    reference,
                    revision,
                } => {
                    if let Some(reference) = reference {
                        Some(format!("git+{}?{}#{}", url, reference, revision))
                    } else {
                        Some(format!("git+{}#{}", url, revision))
                    }
                }
                ResolvedSource::Path { path } => Some(format!("path+{}", path.display())),
            };

            // Add dependencies
            if let Some(deps) = self.graph.get(&pkg.id) {
                for dep in deps {
                    locked.add_dependency(&dep.name, &dep.version);
                }
            }

            // Add features
            locked.features = pkg.features.clone();

            lockfile.add_package(locked);
        }

        lockfile.sort();
        lockfile
    }
}

impl Default for Resolution {
    fn default() -> Self {
        Self::new()
    }
}

/// A resolved package with its source and features.
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    /// Package identifier.
    pub id: PackageId,
    /// Where to fetch the package from.
    pub source: ResolvedSource,
    /// Enabled features.
    pub features: Vec<String>,
}

/// Resolved source for a package.
#[derive(Debug, Clone)]
pub enum ResolvedSource {
    /// Registry source.
    Registry {
        /// Registry URL.
        url: String,
        /// Package checksum.
        checksum: Option<String>,
    },
    /// Git repository source.
    Git {
        /// Repository URL.
        url: String,
        /// Reference (branch, tag).
        reference: Option<String>,
        /// Commit revision.
        revision: String,
    },
    /// Local path source.
    Path {
        /// Path to the package.
        path: std::path::PathBuf,
    },
}

/// The dependency resolver.
#[derive(Debug)]
pub struct Resolver {
    /// Available package versions (would be populated from registry index).
    available_versions: HashMap<String, Vec<PackageMetadata>>,
    /// Resolution in progress.
    resolution: Resolution,
    /// Packages currently being resolved (cycle detection).
    resolving: HashSet<String>,
}

/// Metadata about an available package version.
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
    /// Dependencies.
    pub dependencies: Vec<DependencySpec>,
    /// Available features.
    pub features: HashMap<String, Vec<String>>,
    /// Is this version yanked?
    pub yanked: bool,
}

/// A dependency specification.
#[derive(Debug, Clone)]
pub struct DependencySpec {
    /// Dependency name.
    pub name: String,
    /// Version requirement.
    pub req: VersionReq,
    /// Is this optional?
    pub optional: bool,
    /// Required features.
    pub features: Vec<String>,
}

impl Resolver {
    /// Create a new resolver.
    pub fn new() -> Self {
        Resolver {
            available_versions: HashMap::new(),
            resolution: Resolution::new(),
            resolving: HashSet::new(),
        }
    }

    /// Register available versions for a package (from registry index).
    pub fn register_versions(&mut self, name: &str, versions: Vec<PackageMetadata>) {
        self.available_versions.insert(name.to_string(), versions);
    }

    /// Resolve dependencies from a manifest.
    pub fn resolve(
        &mut self,
        manifest: &Manifest,
        existing_lock: Option<&Lockfile>,
    ) -> Result<Resolution, ResolveError> {
        self.resolution = Resolution::new();
        self.resolving.clear();

        // Start with the root package
        let root_id = PackageId::new(
            manifest.package.name.clone(),
            Version::parse(&manifest.package.version)
                .map_err(|e| ResolveError::InvalidVersion(e.to_string()))?,
        );

        // Resolve each dependency
        for (name, dep) in &manifest.dependencies {
            let req = self.parse_dependency_req(dep)?;
            self.resolve_dependency(name, &req, &root_id, existing_lock)?;
        }

        Ok(self.resolution.clone())
    }

    /// Parse a dependency specification from the manifest.
    fn parse_dependency_req(
        &self,
        dep: &crate::project::Dependency,
    ) -> Result<DependencyReq, ResolveError> {
        match dep {
            crate::project::Dependency::Version(v) => {
                let req = VersionReq::parse(v)
                    .map_err(|e| ResolveError::InvalidVersion(e.to_string()))?;
                Ok(DependencyReq {
                    version: Some(req),
                    git: None,
                    path: None,
                    features: Vec::new(),
                    optional: false,
                })
            }
            crate::project::Dependency::Detailed(d) => {
                let version = if let Some(v) = &d.version {
                    Some(
                        VersionReq::parse(v)
                            .map_err(|e| ResolveError::InvalidVersion(e.to_string()))?,
                    )
                } else {
                    None
                };

                Ok(DependencyReq {
                    version,
                    git: d.git.clone(),
                    path: d.path.clone(),
                    features: d.features.clone(),
                    optional: d.optional,
                })
            }
        }
    }

    /// Resolve a single dependency.
    fn resolve_dependency(
        &mut self,
        name: &str,
        req: &DependencyReq,
        parent: &PackageId,
        existing_lock: Option<&Lockfile>,
    ) -> Result<PackageId, ResolveError> {
        // Check for cycles
        if self.resolving.contains(name) {
            return Err(ResolveError::CyclicDependency {
                package: name.to_string(),
            });
        }
        self.resolving.insert(name.to_string());

        let result = self.do_resolve_dependency(name, req, parent, existing_lock);

        self.resolving.remove(name);
        result
    }

    fn do_resolve_dependency(
        &mut self,
        name: &str,
        req: &DependencyReq,
        parent: &PackageId,
        existing_lock: Option<&Lockfile>,
    ) -> Result<PackageId, ResolveError> {
        // Handle path dependency
        if let Some(path) = &req.path {
            let version = Version::new(0, 0, 0); // Path deps use local version
            let id = PackageId::new(name.to_string(), version.clone());

            // Check if already resolved
            if self.resolution.get(&id).is_some() {
                self.add_edge(parent, &id);
                return Ok(id);
            }

            let resolved = ResolvedPackage {
                id: id.clone(),
                source: ResolvedSource::Path { path: path.clone() },
                features: req.features.clone(),
            };

            self.resolution.packages.push(resolved);
            self.resolution.available.push(id.clone());
            self.add_edge(parent, &id);

            return Ok(id);
        }

        // Handle git dependency
        if let Some(git_url) = &req.git {
            let version = Version::new(0, 0, 0); // Git deps use commit version
            let id = PackageId::new(name.to_string(), version.clone());

            if self.resolution.get(&id).is_some() {
                self.add_edge(parent, &id);
                return Ok(id);
            }

            let resolved = ResolvedPackage {
                id: id.clone(),
                source: ResolvedSource::Git {
                    url: git_url.clone(),
                    reference: None, // Would be populated from git info
                    revision: "HEAD".to_string(),
                },
                features: req.features.clone(),
            };

            self.resolution.packages.push(resolved);
            self.resolution.to_fetch.push(id.clone());
            self.add_edge(parent, &id);

            return Ok(id);
        }

        // Handle registry dependency
        let version_req = req.version.as_ref().ok_or_else(|| {
            ResolveError::InvalidDependency {
                name: name.to_string(),
                reason: "no version, git, or path specified".to_string(),
            }
        })?;

        // Try to use locked version first
        if let Some(lockfile) = existing_lock {
            let locked_versions = lockfile.get_package_versions(name);
            for locked in locked_versions {
                if version_req.matches(&locked.version) {
                    let id = PackageId::new(name.to_string(), locked.version.clone());

                    if self.resolution.get(&id).is_some() {
                        self.add_edge(parent, &id);
                        return Ok(id);
                    }

                    let source = locked.parse_source().map(|s| match s {
                        PackageSource::Registry(url) => ResolvedSource::Registry {
                            url,
                            checksum: locked.checksum.clone(),
                        },
                        PackageSource::Git {
                            url,
                            reference,
                            revision,
                        } => ResolvedSource::Git {
                            url,
                            reference,
                            revision,
                        },
                        PackageSource::Path(path) => ResolvedSource::Path { path },
                    });

                    let resolved = ResolvedPackage {
                        id: id.clone(),
                        source: source.unwrap_or(ResolvedSource::Registry {
                            url: PackageSource::default_registry(),
                            checksum: locked.checksum.clone(),
                        }),
                        features: req.features.clone(),
                    };

                    self.resolution.packages.push(resolved);
                    self.resolution.available.push(id.clone());
                    self.add_edge(parent, &id);

                    return Ok(id);
                }
            }
        }

        // Find available versions
        let best_match = self.available_versions.get(name).and_then(|versions| {
            // Find best matching version (newest that satisfies constraint)
            let mut candidates: Vec<_> = versions
                .iter()
                .filter(|v| !v.yanked && version_req.matches(&v.version))
                .collect();

            candidates.sort_by(|a, b| b.version.cmp(&a.version)); // Newest first
            candidates.first().map(|best| (*best).clone())
        });

        if let Some(best) = best_match {
            let id = PackageId::new(name.to_string(), best.version.clone());

            if self.resolution.get(&id).is_some() {
                self.add_edge(parent, &id);
                return Ok(id);
            }

            let resolved = ResolvedPackage {
                id: id.clone(),
                source: ResolvedSource::Registry {
                    url: PackageSource::default_registry(),
                    checksum: None,
                },
                features: req.features.clone(),
            };

            self.resolution.packages.push(resolved);
            self.resolution.to_fetch.push(id.clone());
            self.add_edge(parent, &id);

            // Clone dependencies to avoid borrow issues
            let dependencies = best.dependencies.clone();

            // Recursively resolve dependencies of this package
            for dep in &dependencies {
                if dep.optional && !req.features.contains(&dep.name) {
                    continue;
                }

                let dep_req = DependencyReq {
                    version: Some(dep.req.clone()),
                    git: None,
                    path: None,
                    features: dep.features.clone(),
                    optional: dep.optional,
                };

                self.resolve_dependency(&dep.name, &dep_req, &id, existing_lock)?;
            }

            return Ok(id);
        }

        // No version found - return an error
        // The package is not in the lockfile and not in registered available versions
        Err(ResolveError::NoMatchingVersion {
            package: name.to_string(),
            requirement: version_req.to_string(),
        })
    }

    fn add_edge(&mut self, from: &PackageId, to: &PackageId) {
        self.resolution
            .graph
            .entry(from.clone())
            .or_default()
            .push(to.clone());
    }
}

impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal representation of a dependency requirement.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Infrastructure for package resolution
struct DependencyReq {
    version: Option<VersionReq>,
    git: Option<String>,
    path: Option<std::path::PathBuf>,
    features: Vec<String>,
    optional: bool,
}

/// Errors that can occur during resolution.
#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("manifest error: {0}")]
    ManifestError(String),
    #[error("invalid version: {0}")]
    InvalidVersion(String),
    #[error("invalid dependency '{name}': {reason}")]
    InvalidDependency { name: String, reason: String },
    #[error("cyclic dependency detected: {package}")]
    CyclicDependency { package: String },
    #[error("no matching version for {package}: {requirement}")]
    NoMatchingVersion { package: String, requirement: String },
    #[error("version conflict for {package}: {existing} vs {requested}")]
    VersionConflict {
        package: String,
        existing: String,
        requested: String,
    },
    #[error("fetch error: {0}")]
    FetchError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_id() {
        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());
        assert_eq!(id.name, "test");
        assert_eq!(id.version, Version::parse("1.0.0").unwrap());
        assert_eq!(id.to_string(), "test v1.0.0");
    }

    #[test]
    fn test_resolver_new() {
        let resolver = Resolver::new();
        assert!(resolver.available_versions.is_empty());
    }

    #[test]
    fn test_resolver_register_versions() {
        let mut resolver = Resolver::new();

        let versions = vec![PackageMetadata {
            name: "test".to_string(),
            version: Version::parse("1.0.0").unwrap(),
            dependencies: Vec::new(),
            features: HashMap::new(),
            yanked: false,
        }];

        resolver.register_versions("test", versions);
        assert!(resolver.available_versions.contains_key("test"));
    }

    #[test]
    fn test_resolution_topological_order() {
        let mut resolution = Resolution::new();

        // Package A depends on B
        let a = PackageId::new("a".to_string(), Version::parse("1.0.0").unwrap());
        let b = PackageId::new("b".to_string(), Version::parse("1.0.0").unwrap());

        resolution.packages.push(ResolvedPackage {
            id: a.clone(),
            source: ResolvedSource::Registry {
                url: "test".to_string(),
                checksum: None,
            },
            features: Vec::new(),
        });

        resolution.packages.push(ResolvedPackage {
            id: b.clone(),
            source: ResolvedSource::Registry {
                url: "test".to_string(),
                checksum: None,
            },
            features: Vec::new(),
        });

        resolution.graph.insert(a.clone(), vec![b.clone()]);

        let order = resolution.topological_order();
        assert_eq!(order.len(), 2);

        // B should come before A (dependencies first)
        let b_pos = order.iter().position(|p| p.id.name == "b").unwrap();
        let a_pos = order.iter().position(|p| p.id.name == "a").unwrap();
        assert!(b_pos < a_pos);
    }

    #[test]
    fn test_resolution_to_lockfile() {
        let mut resolution = Resolution::new();

        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());

        resolution.packages.push(ResolvedPackage {
            id: id.clone(),
            source: ResolvedSource::Registry {
                url: "https://blood-lang.org/packages".to_string(),
                checksum: Some("sha256:abc".to_string()),
            },
            features: vec!["feature1".to_string()],
        });

        let lockfile = resolution.to_lockfile();
        assert_eq!(lockfile.packages.len(), 1);
        assert_eq!(lockfile.packages[0].name, "test");
        assert_eq!(
            lockfile.packages[0].checksum,
            Some("sha256:abc".to_string())
        );
    }
}
