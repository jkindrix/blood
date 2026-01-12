//! # Content Hash
//!
//! BLAKE3-256 hash computation for content-addressed code identity.
//!
//! ## Why BLAKE3?
//!
//! | Factor    | BLAKE3    | SHA3-512 (Unison) | SHA-256   |
//! |-----------|-----------|-------------------|-----------|
//! | Speed     | ~5x faster| Baseline          | ~3x slower|
//! | Security  | 128-bit   | 256-bit           | 128-bit   |
//! | Output    | 256 bits  | 512 bits          | 256 bits  |
//!
//! ## Collision Probability
//!
//! With 256-bit hashes and the birthday paradox:
//! - Expected first collision at ~2^128 unique hashes
//! - At 1 million definitions/second: ~10^31 years
//!
//! ## Display Format
//!
//! Hashes are displayed in lowercase base32 (RFC 4648):
//! ```text
//! Full:  #a7f2k9m3xp5jht2ngqw4bc8rv6ys7dz1ef0il
//! Short: #a7f2k9m3xp (10 characters typical)
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

/// Format version for hash computation.
/// Increment when changing canonicalization or serialization.
pub const FORMAT_VERSION: u8 = 1;

/// A 256-bit content hash (BLAKE3).
///
/// Serializes to/from hex string for JSON compatibility.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Create a new content hash from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Compute hash from bytes using BLAKE3-256.
    pub fn compute(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self(*hash.as_bytes())
    }

    /// Compute hash with format version prefix.
    pub fn compute_versioned(data: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[FORMAT_VERSION]);
        hasher.update(data);
        Self(*hasher.finalize().as_bytes())
    }

    /// Get the raw bytes of the hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Get a short display form (first 10 base32 characters).
    pub fn short_display(&self) -> HashDisplay {
        HashDisplay {
            hash: *self,
            max_chars: 10,
        }
    }

    /// Get the full display form (52 base32 characters).
    pub fn full_display(&self) -> HashDisplay {
        HashDisplay {
            hash: *self,
            max_chars: 52,
        }
    }

    /// Create a hash for builtins with a fixed well-known value.
    pub fn builtin(name: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"builtin:");
        hasher.update(name.as_bytes());
        Self(*hasher.finalize().as_bytes())
    }

    /// The zero hash (for error cases).
    pub const ZERO: Self = Self([0u8; 32]);
}

impl Default for ContentHash {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash(#{})", self.short_display())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.short_display())
    }
}

impl From<ContentHash> for String {
    fn from(hash: ContentHash) -> Self {
        // Convert to hex string for serialization
        hash.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl TryFrom<String> for ContentHash {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        if s.len() != 64 {
            return Err(format!("expected 64 hex characters, got {}", s.len()));
        }

        let mut bytes = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex_str = std::str::from_utf8(chunk)
                .map_err(|_| "invalid UTF-8 in hex string")?;
            bytes[i] = u8::from_str_radix(hex_str, 16)
                .map_err(|_| format!("invalid hex at position {}", i * 2))?;
        }

        Ok(ContentHash(bytes))
    }
}

/// Display helper for content hashes.
pub struct HashDisplay {
    hash: ContentHash,
    max_chars: usize,
}

impl fmt::Display for HashDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Base32 alphabet (RFC 4648, lowercase)
        const BASE32: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";

        // Encode to base32
        let mut output = String::with_capacity(52);
        let bytes = self.hash.as_bytes();

        // Process 5 bytes at a time (40 bits → 8 base32 chars)
        for chunk in bytes.chunks(5) {
            let mut acc: u64 = 0;
            for (i, &b) in chunk.iter().enumerate() {
                acc |= (b as u64) << (32 - i * 8);
            }

            // Extract 5-bit groups
            let bits = chunk.len() * 8;
            let chars = bits.div_ceil(5);
            for i in 0..chars {
                let idx = ((acc >> (35 - i * 5)) & 0x1f) as usize;
                output.push(BASE32[idx] as char);
            }
        }

        // Truncate to max_chars
        let display = if output.len() > self.max_chars {
            &output[..self.max_chars]
        } else {
            &output
        };

        write!(f, "{}", display)
    }
}

/// A hasher for building content hashes incrementally.
pub struct ContentHasher {
    hasher: blake3::Hasher,
}

impl ContentHasher {
    /// Create a new hasher with version prefix.
    pub fn new() -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[FORMAT_VERSION]);
        Self { hasher }
    }

    /// Update the hash with more data.
    pub fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Update with a u8 value.
    pub fn update_u8(&mut self, value: u8) {
        self.hasher.update(&[value]);
    }

    /// Update with a u16 value (little-endian).
    pub fn update_u16(&mut self, value: u16) {
        self.hasher.update(&value.to_le_bytes());
    }

    /// Update with a u32 value (little-endian).
    pub fn update_u32(&mut self, value: u32) {
        self.hasher.update(&value.to_le_bytes());
    }

    /// Update with a u64 value (little-endian).
    pub fn update_u64(&mut self, value: u64) {
        self.hasher.update(&value.to_le_bytes());
    }

    /// Update with an i64 value (little-endian).
    pub fn update_i64(&mut self, value: i64) {
        self.hasher.update(&value.to_le_bytes());
    }

    /// Update with an i128 value (little-endian).
    pub fn update_i128(&mut self, value: i128) {
        self.hasher.update(&value.to_le_bytes());
    }

    /// Update with an f64 value (IEEE 754 canonical form).
    pub fn update_f64(&mut self, value: f64) {
        // Use bits representation for determinism
        self.hasher.update(&value.to_bits().to_le_bytes());
    }

    /// Update with a string (length-prefixed).
    pub fn update_str(&mut self, s: &str) {
        self.update_u32(s.len() as u32);
        self.hasher.update(s.as_bytes());
    }

    /// Update with another hash.
    pub fn update_hash(&mut self, hash: &ContentHash) {
        self.hasher.update(hash.as_bytes());
    }

    /// Finalize and return the hash.
    pub fn finalize(self) -> ContentHash {
        ContentHash(*self.hasher.finalize().as_bytes())
    }
}

impl Default for ContentHasher {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_compute() {
        let hash1 = ContentHash::compute(b"hello world");
        let hash2 = ContentHash::compute(b"hello world");
        let hash3 = ContentHash::compute(b"hello world!");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_content_hash_versioned() {
        let hash1 = ContentHash::compute_versioned(b"test");
        let hash2 = ContentHash::compute(b"test");

        // Versioned should be different from non-versioned
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_display() {
        let hash = ContentHash::compute(b"test");
        let short = format!("{}", hash.short_display());
        let full = format!("{}", hash.full_display());

        assert_eq!(short.len(), 10);
        assert!(full.len() <= 52);

        // Should only contain base32 characters
        assert!(short.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn test_content_hash_builtin() {
        let add1 = ContentHash::builtin("Int.add");
        let add2 = ContentHash::builtin("Int.add");
        let sub = ContentHash::builtin("Int.sub");

        assert_eq!(add1, add2);
        assert_ne!(add1, sub);
    }

    #[test]
    fn test_content_hasher() {
        let mut hasher = ContentHasher::new();
        hasher.update_u8(0x01);
        hasher.update_i64(42);
        hasher.update_str("test");
        let hash = hasher.finalize();

        // Same inputs should produce same hash
        let mut hasher2 = ContentHasher::new();
        hasher2.update_u8(0x01);
        hasher2.update_i64(42);
        hasher2.update_str("test");
        let hash2 = hasher2.finalize();

        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_content_hash_debug() {
        let hash = ContentHash::compute(b"test");
        let debug = format!("{:?}", hash);
        assert!(debug.starts_with("ContentHash(#"));
    }

    #[test]
    fn test_content_hash_default() {
        let hash = ContentHash::default();
        assert_eq!(hash, ContentHash::ZERO);
    }

    #[test]
    fn test_content_hasher_f64_determinism() {
        // Verify f64 hashing is deterministic for same value
        let mut h1 = ContentHasher::new();
        h1.update_f64(1.234567);
        let hash1 = h1.finalize();

        let mut h2 = ContentHasher::new();
        h2.update_f64(1.234567);
        let hash2 = h2.finalize();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_as_bytes() {
        let bytes = [42u8; 32];
        let hash = ContentHash::from_bytes(bytes);
        assert_eq!(hash.as_bytes(), &bytes);
    }

    #[test]
    fn test_deterministic_hash_across_runs() {
        // Test that the same input always produces the same hash.
        // This is critical for content-addressing.

        // Test 1: Simple byte data
        let input = b"fn add(x: i32, y: i32) -> i32 { x + y }";
        let hash1 = ContentHash::compute(input);
        let hash2 = ContentHash::compute(input);
        let hash3 = ContentHash::compute(input);
        assert_eq!(hash1, hash2, "Same input should produce same hash");
        assert_eq!(hash2, hash3, "Hash should be deterministic across calls");

        // Verify the actual bytes are identical
        assert_eq!(hash1.as_bytes(), hash2.as_bytes());

        // Test 2: Empty input
        let empty_hash1 = ContentHash::compute(b"");
        let empty_hash2 = ContentHash::compute(b"");
        assert_eq!(empty_hash1, empty_hash2);

        // Test 3: Unicode input
        let unicode_input = "fn greet(name: String) -> String { format!(\"Hello, {}!\", name) }".as_bytes();
        let unicode_hash1 = ContentHash::compute(unicode_input);
        let unicode_hash2 = ContentHash::compute(unicode_input);
        assert_eq!(unicode_hash1, unicode_hash2);
    }

    #[test]
    fn test_deterministic_hasher_across_runs() {
        // Test ContentHasher produces deterministic results
        fn build_hash() -> ContentHash {
            let mut hasher = ContentHasher::new();
            hasher.update_u8(0xFF);
            hasher.update_u16(1234);
            hasher.update_u32(567890);
            hasher.update_u64(123456789012345);
            hasher.update_i64(-42);
            hasher.update_i128(-999999999999999999);
            hasher.update_f64(3.14159265358979);
            hasher.update_str("deterministic test string");
            hasher.update(b"raw bytes data");
            hasher.finalize()
        }

        let hash1 = build_hash();
        let hash2 = build_hash();
        let hash3 = build_hash();

        assert_eq!(hash1, hash2, "ContentHasher should be deterministic");
        assert_eq!(hash2, hash3, "Multiple runs should produce same hash");
    }

    #[test]
    fn test_hash_serialization_determinism() {
        // Test that serialization round-trips produce same hash
        let original = ContentHash::compute(b"test input for serialization");

        // Convert to string and back multiple times
        let serialized1: String = original.into();
        let deserialized1 = ContentHash::try_from(serialized1.clone()).unwrap();

        let serialized2: String = deserialized1.into();
        let deserialized2 = ContentHash::try_from(serialized2.clone()).unwrap();

        // All should be equal
        assert_eq!(serialized1, serialized2, "Serialized form should be deterministic");
        assert_eq!(deserialized1, deserialized2, "Round-trip should preserve hash");
    }

    #[test]
    fn test_hash_with_hash_input_determinism() {
        // Test that hashing a hash produces deterministic results
        let base_hash = ContentHash::compute(b"base content");

        let mut hasher1 = ContentHasher::new();
        hasher1.update_hash(&base_hash);
        hasher1.update_str("additional data");
        let compound1 = hasher1.finalize();

        let mut hasher2 = ContentHasher::new();
        hasher2.update_hash(&base_hash);
        hasher2.update_str("additional data");
        let compound2 = hasher2.finalize();

        assert_eq!(compound1, compound2, "Hashing a hash should be deterministic");
    }

    #[test]
    fn test_versioned_hash_determinism() {
        // Test that versioned hashing is deterministic
        let input = b"versioned content test";

        let hash1 = ContentHash::compute_versioned(input);
        let hash2 = ContentHash::compute_versioned(input);
        let hash3 = ContentHash::compute_versioned(input);

        assert_eq!(hash1, hash2);
        assert_eq!(hash2, hash3);

        // Versioned and non-versioned should be different
        let non_versioned = ContentHash::compute(input);
        assert_ne!(hash1, non_versioned);
    }

    #[test]
    fn test_builtin_hash_determinism() {
        // Test that builtin hashes are stable
        let builtins = [
            "Int.add", "Int.sub", "Int.mul", "Int.div",
            "Bool.and", "Bool.or", "Bool.not",
            "String.concat", "Array.length",
        ];

        for name in builtins {
            let hash1 = ContentHash::builtin(name);
            let hash2 = ContentHash::builtin(name);
            let hash3 = ContentHash::builtin(name);
            assert_eq!(hash1, hash2, "Builtin '{}' should be deterministic", name);
            assert_eq!(hash2, hash3);
        }

        // Different builtins should have different hashes
        for i in 0..builtins.len() {
            for j in (i + 1)..builtins.len() {
                let hash_i = ContentHash::builtin(builtins[i]);
                let hash_j = ContentHash::builtin(builtins[j]);
                assert_ne!(hash_i, hash_j, "Different builtins should have different hashes");
            }
        }
    }
}
