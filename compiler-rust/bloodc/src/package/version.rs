//! Semantic versioning for Blood packages.
//!
//! This module implements semver parsing and constraint matching following
//! the semantic versioning 2.0.0 specification.
//!
//! ## Version Format
//!
//! `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`
//!
//! ## Version Constraints
//!
//! | Syntax | Meaning |
//! |--------|---------|
//! | `1.2.3` | Exactly 1.2.3 |
//! | `^1.2.3` | Compatible: >=1.2.3, <2.0.0 |
//! | `~1.2.3` | Approximately: >=1.2.3, <1.3.0 |
//! | `>=1.0, <2.0` | Range constraint |
//! | `*` | Any version |

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A semantic version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    /// Major version (breaking changes)
    pub major: u64,
    /// Minor version (new features, backwards compatible)
    pub minor: u64,
    /// Patch version (bug fixes)
    pub patch: u64,
    /// Pre-release identifier (e.g., "alpha", "beta.1")
    pub pre: Option<String>,
    /// Build metadata (e.g., "build.123")
    pub build: Option<String>,
}

impl Version {
    /// Create a new version.
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version {
            major,
            minor,
            patch,
            pre: None,
            build: None,
        }
    }

    /// Parse a version string.
    pub fn parse(s: &str) -> Result<Self, ParseVersionError> {
        s.parse()
    }

    /// Check if this is a pre-release version.
    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some()
    }

    /// Get the base version (without pre-release or build metadata).
    pub fn base(&self) -> Version {
        Version {
            major: self.major,
            minor: self.minor,
            patch: self.patch,
            pre: None,
            build: None,
        }
    }
}

impl Default for Version {
    fn default() -> Self {
        Version::new(0, 0, 0)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare major, minor, patch first
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // Pre-release versions have lower precedence than normal versions
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,    // 1.0.0-alpha < 1.0.0
            (None, Some(_)) => Ordering::Greater, // 1.0.0 > 1.0.0-alpha
            (Some(a), Some(b)) => a.cmp(b),       // Compare pre-release identifiers
        }

        // Build metadata is ignored for version precedence
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(ref pre) = self.pre {
            write!(f, "-{}", pre)?;
        }
        if let Some(ref build) = self.build {
            write!(f, "+{}", build)?;
        }
        Ok(())
    }
}

impl FromStr for Version {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ParseVersionError::Empty);
        }

        // Split off build metadata first
        let (version_pre, build) = if let Some(idx) = s.find('+') {
            let (vp, b) = s.split_at(idx);
            (vp, Some(b[1..].to_string()))
        } else {
            (s, None)
        };

        // Split off pre-release
        let (version, pre) = if let Some(idx) = version_pre.find('-') {
            let (v, p) = version_pre.split_at(idx);
            (v, Some(p[1..].to_string()))
        } else {
            (version_pre, None)
        };

        // Parse version numbers
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() != 3 {
            return Err(ParseVersionError::InvalidFormat(s.to_string()));
        }

        let major = parts[0]
            .parse()
            .map_err(|_| ParseVersionError::InvalidNumber(parts[0].to_string()))?;
        let minor = parts[1]
            .parse()
            .map_err(|_| ParseVersionError::InvalidNumber(parts[1].to_string()))?;
        let patch = parts[2]
            .parse()
            .map_err(|_| ParseVersionError::InvalidNumber(parts[2].to_string()))?;

        Ok(Version {
            major,
            minor,
            patch,
            pre,
            build,
        })
    }
}

/// Errors that can occur parsing a version.
#[derive(Debug, Error)]
pub enum ParseVersionError {
    #[error("empty version string")]
    Empty,
    #[error("invalid version format: {0}")]
    InvalidFormat(String),
    #[error("invalid version number: {0}")]
    InvalidNumber(String),
}

/// A version requirement (constraint).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionReq {
    /// The constraints that make up this requirement.
    pub constraints: Vec<VersionConstraint>,
}

impl VersionReq {
    /// Create a requirement that matches any version.
    pub fn any() -> Self {
        VersionReq {
            constraints: vec![VersionConstraint::Any],
        }
    }

    /// Create a requirement that matches exactly one version.
    pub fn exact(version: Version) -> Self {
        VersionReq {
            constraints: vec![VersionConstraint::Comparator(Comparator {
                op: Op::Exact,
                version,
            })],
        }
    }

    /// Parse a version requirement string.
    pub fn parse(s: &str) -> Result<Self, ParseVersionReqError> {
        s.parse()
    }

    /// Check if a version matches this requirement.
    pub fn matches(&self, version: &Version) -> bool {
        self.constraints.iter().all(|c| c.matches(version))
    }
}

impl Default for VersionReq {
    fn default() -> Self {
        VersionReq::any()
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.constraints.is_empty() {
            return write!(f, "*");
        }

        let parts: Vec<String> = self.constraints.iter().map(|c| c.to_string()).collect();
        write!(f, "{}", parts.join(", "))
    }
}

impl FromStr for VersionReq {
    type Err = ParseVersionReqError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() || s == "*" {
            return Ok(VersionReq::any());
        }

        // Handle comma-separated constraints
        let constraints: Result<Vec<_>, _> = s.split(',').map(|part| part.trim().parse()).collect();

        Ok(VersionReq {
            constraints: constraints?,
        })
    }
}

/// A single version constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionConstraint {
    /// Match any version.
    Any,
    /// A comparator constraint.
    Comparator(Comparator),
    /// Caret constraint: ^X.Y.Z means >=X.Y.Z, <(X+1).0.0 for X>0
    Caret(Version),
    /// Tilde constraint: ~X.Y.Z means >=X.Y.Z, <X.(Y+1).0
    Tilde(Version),
}

impl VersionConstraint {
    /// Check if a version matches this constraint.
    pub fn matches(&self, version: &Version) -> bool {
        match self {
            VersionConstraint::Any => true,
            VersionConstraint::Comparator(cmp) => cmp.matches(version),
            VersionConstraint::Caret(base) => {
                // ^X.Y.Z: Compatible versions
                // For X > 0: >=X.Y.Z, <(X+1).0.0
                // For X = 0, Y > 0: >=0.Y.Z, <0.(Y+1).0
                // For X = 0, Y = 0: >=0.0.Z, <0.0.(Z+1)
                if version < base {
                    return false;
                }

                if base.major > 0 {
                    version.major == base.major
                } else if base.minor > 0 {
                    version.major == 0 && version.minor == base.minor
                } else {
                    version.major == 0 && version.minor == 0 && version.patch == base.patch
                }
            }
            VersionConstraint::Tilde(base) => {
                // ~X.Y.Z: >=X.Y.Z, <X.(Y+1).0
                if version < base {
                    return false;
                }
                version.major == base.major && version.minor == base.minor
            }
        }
    }
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionConstraint::Any => write!(f, "*"),
            VersionConstraint::Comparator(cmp) => write!(f, "{}", cmp),
            VersionConstraint::Caret(v) => write!(f, "^{}", v),
            VersionConstraint::Tilde(v) => write!(f, "~{}", v),
        }
    }
}

impl FromStr for VersionConstraint {
    type Err = ParseVersionReqError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() || s == "*" {
            return Ok(VersionConstraint::Any);
        }

        // Check for operators
        if let Some(rest) = s.strip_prefix('^') {
            let version = rest.parse().map_err(ParseVersionReqError::Version)?;
            return Ok(VersionConstraint::Caret(version));
        }

        if let Some(rest) = s.strip_prefix('~') {
            let version = rest.parse().map_err(ParseVersionReqError::Version)?;
            return Ok(VersionConstraint::Tilde(version));
        }

        // Check for comparison operators
        for (prefix, op) in &[
            (">=", Op::GreaterEq),
            ("<=", Op::LessEq),
            (">", Op::Greater),
            ("<", Op::Less),
            ("=", Op::Exact),
        ] {
            if let Some(rest) = s.strip_prefix(prefix) {
                let version = rest.trim().parse().map_err(ParseVersionReqError::Version)?;
                return Ok(VersionConstraint::Comparator(Comparator {
                    op: *op,
                    version,
                }));
            }
        }

        // Bare version is treated as exact match
        let version = s.parse().map_err(ParseVersionReqError::Version)?;
        Ok(VersionConstraint::Comparator(Comparator {
            op: Op::Exact,
            version,
        }))
    }
}

/// A version comparator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparator {
    /// The comparison operator.
    pub op: Op,
    /// The version to compare against.
    pub version: Version,
}

impl Comparator {
    /// Check if a version matches this comparator.
    pub fn matches(&self, version: &Version) -> bool {
        match self.op {
            Op::Exact => version == &self.version,
            Op::Greater => version > &self.version,
            Op::GreaterEq => version >= &self.version,
            Op::Less => version < &self.version,
            Op::LessEq => version <= &self.version,
        }
    }
}

impl fmt::Display for Comparator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op_str = match self.op {
            Op::Exact => "=",
            Op::Greater => ">",
            Op::GreaterEq => ">=",
            Op::Less => "<",
            Op::LessEq => "<=",
        };
        write!(f, "{}{}", op_str, self.version)
    }
}

/// Comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Exact match.
    Exact,
    /// Greater than.
    Greater,
    /// Greater than or equal.
    GreaterEq,
    /// Less than.
    Less,
    /// Less than or equal.
    LessEq,
}

/// Errors that can occur parsing a version requirement.
#[derive(Debug, Error)]
pub enum ParseVersionReqError {
    #[error("version parse error: {0}")]
    Version(#[from] ParseVersionError),
    #[error("invalid constraint format: {0}")]
    InvalidFormat(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert!(v.pre.is_none());
        assert!(v.build.is_none());
    }

    #[test]
    fn test_version_parse_prerelease() {
        let v = Version::parse("1.0.0-alpha").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 0);
        assert_eq!(v.pre, Some("alpha".to_string()));
    }

    #[test]
    fn test_version_parse_build() {
        let v = Version::parse("1.0.0+build.123").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.build, Some("build.123".to_string()));
    }

    #[test]
    fn test_version_parse_full() {
        let v = Version::parse("1.2.3-beta.1+build.456").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.pre, Some("beta.1".to_string()));
        assert_eq!(v.build, Some("build.456".to_string()));
    }

    #[test]
    fn test_version_ordering() {
        let v1 = Version::parse("1.0.0").unwrap();
        let v2 = Version::parse("1.0.1").unwrap();
        let v3 = Version::parse("1.1.0").unwrap();
        let v4 = Version::parse("2.0.0").unwrap();

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v3 < v4);
    }

    #[test]
    fn test_version_prerelease_ordering() {
        let v1 = Version::parse("1.0.0-alpha").unwrap();
        let v2 = Version::parse("1.0.0-beta").unwrap();
        let v3 = Version::parse("1.0.0").unwrap();

        assert!(v1 < v2);
        assert!(v2 < v3);
    }

    #[test]
    fn test_version_display() {
        let v = Version::parse("1.2.3-beta+build").unwrap();
        assert_eq!(v.to_string(), "1.2.3-beta+build");
    }

    #[test]
    fn test_version_req_any() {
        let req = VersionReq::any();
        assert!(req.matches(&Version::parse("0.0.1").unwrap()));
        assert!(req.matches(&Version::parse("1.0.0").unwrap()));
        assert!(req.matches(&Version::parse("99.99.99").unwrap()));
    }

    #[test]
    fn test_version_req_exact() {
        let req = VersionReq::parse("1.2.3").unwrap();
        assert!(req.matches(&Version::parse("1.2.3").unwrap()));
        assert!(!req.matches(&Version::parse("1.2.4").unwrap()));
    }

    #[test]
    fn test_version_req_caret() {
        let req = VersionReq::parse("^1.2.3").unwrap();
        assert!(req.matches(&Version::parse("1.2.3").unwrap()));
        assert!(req.matches(&Version::parse("1.2.4").unwrap()));
        assert!(req.matches(&Version::parse("1.9.0").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.0").unwrap()));
        assert!(!req.matches(&Version::parse("1.2.2").unwrap()));
    }

    #[test]
    fn test_version_req_caret_zero() {
        // ^0.2.3 should be >=0.2.3, <0.3.0
        let req = VersionReq::parse("^0.2.3").unwrap();
        assert!(req.matches(&Version::parse("0.2.3").unwrap()));
        assert!(req.matches(&Version::parse("0.2.9").unwrap()));
        assert!(!req.matches(&Version::parse("0.3.0").unwrap()));
        assert!(!req.matches(&Version::parse("0.2.2").unwrap()));
    }

    #[test]
    fn test_version_req_tilde() {
        let req = VersionReq::parse("~1.2.3").unwrap();
        assert!(req.matches(&Version::parse("1.2.3").unwrap()));
        assert!(req.matches(&Version::parse("1.2.9").unwrap()));
        assert!(!req.matches(&Version::parse("1.3.0").unwrap()));
        assert!(!req.matches(&Version::parse("1.2.2").unwrap()));
    }

    #[test]
    fn test_version_req_greater() {
        let req = VersionReq::parse(">1.0.0").unwrap();
        assert!(!req.matches(&Version::parse("1.0.0").unwrap()));
        assert!(req.matches(&Version::parse("1.0.1").unwrap()));
        assert!(req.matches(&Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn test_version_req_range() {
        let req = VersionReq::parse(">=1.0.0, <2.0.0").unwrap();
        assert!(req.matches(&Version::parse("1.0.0").unwrap()));
        assert!(req.matches(&Version::parse("1.9.9").unwrap()));
        assert!(!req.matches(&Version::parse("0.9.9").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn test_version_req_display() {
        let req = VersionReq::parse("^1.2.3").unwrap();
        assert_eq!(req.to_string(), "^1.2.3");

        let req = VersionReq::parse(">=1.0.0, <2.0.0").unwrap();
        assert!(req.to_string().contains(">=1.0.0"));
        assert!(req.to_string().contains("<2.0.0"));
    }
}
