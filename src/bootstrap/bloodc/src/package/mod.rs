//! Package management for Blood projects.
//!
//! This module provides the foundation for Blood's package management system:
//!
//! - Version constraint parsing and resolution
//! - Lock file (Blood.lock) generation and parsing
//! - Dependency resolution algorithm
//! - Package fetching from various sources
//! - Package caching with content addressing
//!
//! ## Overview
//!
//! ```text
//! Blood.toml          Blood.lock          Package Cache
//!     │                    │                    │
//!     v                    v                    v
//! ┌─────────┐        ┌─────────┐         ┌─────────┐
//! │ Manifest│   +    │Lockfile │   =     │ Resolver│
//! │ Parser  │        │ Parser  │         │         │
//! └────┬────┘        └────┬────┘         └────┬────┘
//!      │                  │                   │
//!      └──────────────────┴───────────────────┘
//!                         │
//!                         v
//!                  ┌─────────────┐
//!                  │   Fetcher   │
//!                  └──────┬──────┘
//!                         │
//!            ┌────────────┼────────────┐
//!            v            v            v
//!       Registry         Git         Path
//! ```

mod version;
mod lockfile;
mod resolver;
mod fetcher;
mod cache;

pub use version::{
    Version, VersionReq, VersionConstraint, Comparator, Op,
    ParseVersionError, ParseVersionReqError,
};
pub use lockfile::{Lockfile, LockedPackage, PackageSource, LockfileError};
pub use resolver::{Resolver, Resolution, ResolveError, PackageId};
pub use fetcher::{Fetcher, FetchError, FetchSource};
pub use cache::{PackageCache, CacheError, CachedPackage};

use std::path::Path;

/// Initialize package management for a project.
///
/// This is the main entry point for package operations. It:
/// 1. Reads Blood.toml manifest
/// 2. Reads Blood.lock if present
/// 3. Resolves dependencies
/// 4. Fetches missing packages
/// 5. Updates Blood.lock
pub fn resolve_dependencies(project_root: &Path) -> Result<Resolution, ResolveError> {
    let lock_path = project_root.join("Blood.lock");

    // Load manifest
    let manifest = crate::project::load_manifest(project_root)
        .map_err(|e| ResolveError::ManifestError(e.to_string()))?;

    // Try to load existing lockfile
    let existing_lock = if lock_path.exists() {
        Lockfile::from_path(&lock_path).ok()
    } else {
        None
    };

    // Create resolver and resolve
    let mut resolver = Resolver::new();
    let resolution = resolver.resolve(&manifest, existing_lock.as_ref())?;

    Ok(resolution)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_version_req_parsing() {
        let req = VersionReq::parse("^1.2.3").unwrap();
        assert!(req.matches(&Version::parse("1.2.3").unwrap()));
        assert!(req.matches(&Version::parse("1.9.0").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.0").unwrap()));
    }
}
