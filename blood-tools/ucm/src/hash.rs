//! Content Hashing
//!
//! Provides content-addressable hashing for Blood definitions.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A content hash identifying a definition.
///
/// Uses BLAKE3 for fast, secure hashing.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash([u8; 32]);

impl Hash {
    /// Creates a hash from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Computes the hash of the given content.
    pub fn of(content: &[u8]) -> Self {
        let hash = blake3::hash(content);
        Self(*hash.as_bytes())
    }

    /// Computes the hash of a string.
    pub fn of_str(s: &str) -> Self {
        Self::of(s.as_bytes())
    }

    /// Computes the hash of multiple parts.
    pub fn of_parts(parts: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        for part in parts {
            hasher.update(part);
        }
        Self(*hasher.finalize().as_bytes())
    }

    /// Returns the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the hash as a hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Returns a short version of the hash (first 8 chars).
    pub fn short(&self) -> String {
        self.to_hex()[..8].to_string()
    }

    /// Creates a hash from a hex string.
    pub fn from_hex(s: &str) -> Result<Self, HashParseError> {
        let bytes = hex::decode(s).map_err(|_| HashParseError::InvalidHex)?;
        if bytes.len() != 32 {
            return Err(HashParseError::InvalidLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", &self.to_hex()[..8])
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

impl FromStr for Hash {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix('#').unwrap_or(s);
        Self::from_hex(s)
    }
}

/// Error parsing a hash.
#[derive(Debug, Clone)]
pub enum HashParseError {
    InvalidHex,
    InvalidLength,
}

impl fmt::Display for HashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HashParseError::InvalidHex => write!(f, "invalid hex string"),
            HashParseError::InvalidLength => write!(f, "invalid hash length"),
        }
    }
}

impl std::error::Error for HashParseError {}

/// A hasher for incrementally building hashes.
pub struct Hasher {
    inner: blake3::Hasher,
}

impl Hasher {
    /// Creates a new hasher.
    pub fn new() -> Self {
        Self {
            inner: blake3::Hasher::new(),
        }
    }

    /// Updates the hasher with more data.
    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.inner.update(data);
        self
    }

    /// Updates the hasher with a string.
    pub fn update_str(&mut self, s: &str) -> &mut Self {
        self.update(s.as_bytes())
    }

    /// Finalizes and returns the hash.
    pub fn finalize(self) -> Hash {
        Hash(*self.inner.finalize().as_bytes())
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the structural hash of a Blood definition.
///
/// The structural hash is based on the AST structure, not the source text.
/// This means equivalent definitions with different formatting have the same hash.
pub fn structural_hash(source: &str) -> Hash {
    // TODO: Parse the source and hash the AST structure
    // For now, normalize whitespace and hash
    let normalized: String = source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Hash::of_str(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_creation() {
        let h1 = Hash::of_str("hello");
        let h2 = Hash::of_str("hello");
        let h3 = Hash::of_str("world");

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_hash_display() {
        let h = Hash::of_str("test");
        let s = h.to_string();
        assert!(s.starts_with('#'));
        assert_eq!(s.len(), 9); // # + 8 hex chars
    }

    #[test]
    fn test_hash_parse() {
        let h = Hash::of_str("test");
        let hex = h.to_hex();
        let parsed = Hash::from_hex(&hex).unwrap();
        assert_eq!(h, parsed);
    }

    #[test]
    fn test_structural_hash() {
        let h1 = structural_hash("fn foo() { 42 }");
        let h2 = structural_hash("fn  foo()  {  42  }");
        assert_eq!(h1, h2);
    }
}
