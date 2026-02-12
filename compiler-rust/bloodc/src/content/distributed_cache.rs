//! # Distributed Cache
//!
//! Remote cache layer for sharing compiled artifacts globally.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    DISTRIBUTED CACHE                         │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Request: #a7f2k9m3... (compiled object)                     │
//! │       ↓                                                      │
//! │  Check local cache: miss                                     │
//! │       ↓                                                      │
//! │  Check remote cache: hit!                                    │
//! │       ↓                                                      │
//! │  Download pre-compiled artifact                              │
//! │       ↓                                                      │
//! │  Store in local cache                                        │
//! │       ↓                                                      │
//! │  Return artifact                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use super::build_cache::{BuildCache, CacheError};
use super::hash::ContentHash;

/// Configuration for a remote cache endpoint.
#[derive(Debug, Clone)]
pub struct RemoteCacheConfig {
    /// Base URL for the cache server (e.g., "https://cache.blood-lang.org").
    pub url: String,
    /// Authentication token (if required).
    pub auth_token: Option<String>,
    /// Request timeout.
    pub timeout: Duration,
    /// Whether this cache is read-only (can't publish to it).
    pub read_only: bool,
    /// Priority (lower = tried first).
    pub priority: u32,
}

impl Default for RemoteCacheConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            auth_token: None,
            timeout: Duration::from_secs(30),
            read_only: true,
            priority: 100,
        }
    }
}

/// Statistics for remote cache operations.
#[derive(Debug, Clone, Default)]
pub struct RemoteCacheStats {
    /// Number of remote cache hits.
    pub remote_hits: usize,
    /// Number of remote cache misses.
    pub remote_misses: usize,
    /// Total bytes downloaded from remote.
    pub bytes_downloaded: u64,
    /// Total bytes uploaded to remote.
    pub bytes_uploaded: u64,
    /// Number of failed remote requests.
    pub failed_requests: usize,
}

/// Result of a cache fetch operation.
#[derive(Debug)]
pub enum FetchResult {
    /// Found in local cache.
    LocalHit(Vec<u8>),
    /// Found in remote cache (downloaded and stored locally).
    RemoteHit {
        data: Vec<u8>,
        source: String,
    },
    /// Not found anywhere.
    NotFound,
    /// Error during fetch.
    Error(CacheError),
}

/// A distributed cache client that combines local and remote caching.
pub struct DistributedCache {
    /// Local build cache.
    local: BuildCache,
    /// Remote cache endpoints, sorted by priority.
    remotes: Vec<RemoteCacheConfig>,
    /// In-flight request tracking to avoid duplicate downloads.
    in_flight: HashMap<ContentHash, ()>,
    /// Statistics.
    stats: RemoteCacheStats,
    /// Whether remote caching is enabled.
    enabled: bool,
}

impl DistributedCache {
    /// Create a new distributed cache with the given local cache.
    pub fn new(local: BuildCache) -> Self {
        Self {
            local,
            remotes: Vec::new(),
            in_flight: HashMap::new(),
            stats: RemoteCacheStats::default(),
            enabled: false,
        }
    }

    /// Create a distributed cache from environment configuration.
    ///
    /// Reads configuration from:
    /// - `BLOOD_CACHE_REMOTES`: Comma-separated list of remote URLs
    /// - `BLOOD_CACHE_TOKEN`: Authentication token for remotes
    pub fn from_env(local: BuildCache) -> Self {
        let mut cache = Self::new(local);

        if let Ok(remotes) = std::env::var("BLOOD_CACHE_REMOTES") {
            let auth_token = std::env::var("BLOOD_CACHE_TOKEN").ok();

            for (i, url) in remotes.split(',').enumerate() {
                let url = url.trim();
                if !url.is_empty() {
                    cache.add_remote(RemoteCacheConfig {
                        url: url.to_string(),
                        auth_token: auth_token.clone(),
                        priority: i as u32,
                        ..Default::default()
                    });
                }
            }
        }

        cache
    }

    /// Add a remote cache endpoint.
    pub fn add_remote(&mut self, config: RemoteCacheConfig) {
        self.remotes.push(config);
        // Sort by priority (lower first)
        self.remotes.sort_by_key(|c| c.priority);
        self.enabled = true;
    }

    /// Check if remote caching is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled && !self.remotes.is_empty()
    }

    /// Fetch an object by hash, checking local then remote caches.
    pub fn fetch(&mut self, hash: &ContentHash) -> FetchResult {
        // Check local cache first
        match self.local.get_object(hash) {
            Ok(Some(data)) => return FetchResult::LocalHit(data),
            Ok(None) => {}
            Err(e) => return FetchResult::Error(e),
        }

        // If remote caching is not enabled, return miss
        if !self.is_enabled() {
            self.stats.remote_misses += 1;
            return FetchResult::NotFound;
        }

        // Check if already in flight (avoid duplicate downloads)
        if self.in_flight.contains_key(hash) {
            return FetchResult::NotFound;
        }

        // Clone remotes to avoid borrow issues
        let remotes = self.remotes.clone();

        // Try each remote endpoint
        for remote in &remotes {
            match self.fetch_from_remote(hash, remote) {
                Some(data) => {
                    self.stats.remote_hits += 1;
                    self.stats.bytes_downloaded += data.len() as u64;

                    // Store in local cache for future use
                    if let Err(e) = self.local.store_object(*hash, &data) {
                        // Log but don't fail - we have the data
                        eprintln!("Warning: failed to store in local cache: {}", e);
                    }

                    return FetchResult::RemoteHit {
                        data,
                        source: remote.url.clone(),
                    };
                }
                None => continue,
            }
        }

        self.stats.remote_misses += 1;
        FetchResult::NotFound
    }

    /// Fetch from a specific remote endpoint.
    fn fetch_from_remote(&mut self, hash: &ContentHash, config: &RemoteCacheConfig) -> Option<Vec<u8>> {
        // Mark as in-flight
        self.in_flight.insert(*hash, ());

        // Construct the URL for this hash
        let hash_str = format!("{}", hash.full_display());
        let url = format!("{}/objects/{}/{}", config.url, &hash_str[..2], &hash_str[2..]);

        // Attempt HTTP GET request
        let result = self.http_get(&url, config);

        // Remove from in-flight
        self.in_flight.remove(hash);

        match result {
            Ok(data) => Some(data),
            Err(_) => {
                self.stats.failed_requests += 1;
                None
            }
        }
    }

    /// Perform an HTTP GET request using ureq.
    fn http_get(&self, url: &str, config: &RemoteCacheConfig) -> Result<Vec<u8>, CacheError> {
        let mut request = ureq::get(url)
            .timeout(config.timeout);

        // Add auth header if configured
        if let Some(ref token) = config.auth_token {
            request = request.set("Authorization", &format!("Bearer {}", token));
        }

        let response = request.call().map_err(|e| {
            CacheError::Io(std::io::Error::other(
                format!("HTTP GET failed: {}", e),
            ))
        })?;

        // Check status code
        if response.status() == 404 {
            return Err(CacheError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Object not found in remote cache",
            )));
        }

        if response.status() >= 400 {
            return Err(CacheError::Io(std::io::Error::other(
                format!("HTTP error: status {}", response.status()),
            )));
        }

        // Read response body
        let mut data = Vec::new();
        response.into_reader().read_to_end(&mut data).map_err(|e| {
            CacheError::Io(std::io::Error::other(
                format!("Failed to read response body: {}", e),
            ))
        })?;

        Ok(data)
    }

    /// Publish an object to remote caches.
    ///
    /// Only publishes to remotes that are not read-only.
    pub fn publish(&mut self, hash: &ContentHash, data: &[u8]) -> Result<usize, CacheError> {
        if !self.is_enabled() {
            return Ok(0);
        }

        let mut published = 0;

        for remote in &self.remotes {
            if remote.read_only {
                continue;
            }

            if self.publish_to_remote(hash, data, remote).is_ok() {
                published += 1;
                self.stats.bytes_uploaded += data.len() as u64;
            }
        }

        Ok(published)
    }

    /// Publish to a specific remote endpoint.
    fn publish_to_remote(
        &self,
        hash: &ContentHash,
        data: &[u8],
        config: &RemoteCacheConfig,
    ) -> Result<(), CacheError> {
        let hash_str = format!("{}", hash.full_display());
        let url = format!("{}/objects/{}/{}", config.url, &hash_str[..2], &hash_str[2..]);

        self.http_put(&url, data, config)
    }

    /// Perform an HTTP PUT request using ureq.
    fn http_put(&self, url: &str, data: &[u8], config: &RemoteCacheConfig) -> Result<(), CacheError> {
        let mut request = ureq::put(url)
            .timeout(config.timeout)
            .set("Content-Type", "application/octet-stream");

        // Add auth header if configured
        if let Some(ref token) = config.auth_token {
            request = request.set("Authorization", &format!("Bearer {}", token));
        }

        let response = request.send_bytes(data).map_err(|e| {
            CacheError::Io(std::io::Error::other(
                format!("HTTP PUT failed: {}", e),
            ))
        })?;

        // Check status code
        if response.status() >= 400 {
            return Err(CacheError::Io(std::io::Error::other(
                format!("HTTP PUT error: status {}", response.status()),
            )));
        }

        Ok(())
    }

    /// Check if an object exists in any cache (local or remote).
    pub fn has_object(&self, hash: &ContentHash) -> bool {
        // Check local first
        if self.local.has_object(hash) {
            return true;
        }

        // For remote, we'd need to do a HEAD request
        // For now, return false and let fetch handle it
        false
    }

    /// Store an object in local cache and optionally publish to remotes.
    pub fn store(&mut self, hash: ContentHash, data: &[u8], publish: bool) -> Result<(), CacheError> {
        // Store locally
        self.local.store_object(hash, data)?;

        // Optionally publish to remotes
        if publish {
            self.publish(&hash, data)?;
        }

        Ok(())
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &RemoteCacheStats {
        &self.stats
    }

    /// Get mutable access to the local cache.
    pub fn local_mut(&mut self) -> &mut BuildCache {
        &mut self.local
    }

    /// Get immutable access to the local cache.
    pub fn local(&self) -> &BuildCache {
        &self.local
    }

    /// List configured remote endpoints.
    pub fn remotes(&self) -> &[RemoteCacheConfig] {
        &self.remotes
    }
}

/// A shared distributed cache that can be used across threads.
pub type SharedDistributedCache = Arc<std::sync::Mutex<DistributedCache>>;

/// Create a new shared distributed cache.
pub fn shared_cache(local: BuildCache) -> SharedDistributedCache {
    Arc::new(std::sync::Mutex::new(DistributedCache::from_env(local)))
}

// ============================================================================
// Cache Server Protocol
// ============================================================================

/// Request to check if an object exists.
#[derive(Debug, Clone)]
pub struct HeadRequest {
    pub hash: ContentHash,
}

/// Request to get an object.
#[derive(Debug, Clone)]
pub struct GetRequest {
    pub hash: ContentHash,
}

/// Request to put an object.
#[derive(Debug, Clone)]
pub struct PutRequest {
    pub hash: ContentHash,
    pub data: Vec<u8>,
}

/// Response from cache server.
#[derive(Debug, Clone)]
pub enum CacheResponse {
    /// Object found.
    Found { data: Vec<u8> },
    /// Object not found.
    NotFound,
    /// Object exists (for HEAD requests).
    Exists,
    /// Request was successful (for PUT).
    Ok,
    /// Error occurred.
    Error { message: String },
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_cache() -> (TempDir, DistributedCache) {
        let temp_dir = TempDir::new().unwrap();
        let local = BuildCache::with_dir(temp_dir.path().to_path_buf());
        let cache = DistributedCache::new(local);
        (temp_dir, cache)
    }

    #[test]
    fn test_distributed_cache_local_hit() {
        let (_temp, mut cache) = create_test_cache();
        cache.local_mut().init().unwrap();

        let hash = ContentHash::compute(b"test object");
        let data = b"compiled artifact data";

        // Store in local cache
        cache.local_mut().store_object(hash, data).unwrap();

        // Fetch should hit local
        match cache.fetch(&hash) {
            FetchResult::LocalHit(fetched) => {
                assert_eq!(&fetched[..], data);
            }
            _ => panic!("Expected LocalHit"),
        }
    }

    #[test]
    fn test_distributed_cache_not_found() {
        let (_temp, mut cache) = create_test_cache();
        cache.local_mut().init().unwrap();

        let hash = ContentHash::compute(b"nonexistent");

        // Fetch should miss (no remotes configured)
        match cache.fetch(&hash) {
            FetchResult::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn test_distributed_cache_add_remote() {
        let (_temp, mut cache) = create_test_cache();

        assert!(!cache.is_enabled());

        cache.add_remote(RemoteCacheConfig {
            url: "https://cache.example.com".to_string(),
            priority: 1,
            ..Default::default()
        });

        assert!(cache.is_enabled());
        assert_eq!(cache.remotes().len(), 1);
    }

    #[test]
    fn test_distributed_cache_stats() {
        let (_temp, cache) = create_test_cache();

        let stats = cache.stats();
        assert_eq!(stats.remote_hits, 0);
        assert_eq!(stats.remote_misses, 0);
        assert_eq!(stats.bytes_downloaded, 0);
    }

    #[test]
    fn test_distributed_cache_store() {
        let (_temp, mut cache) = create_test_cache();
        cache.local_mut().init().unwrap();

        let hash = ContentHash::compute(b"new object");
        let data = b"object data";

        // Store without publishing
        cache.store(hash, data, false).unwrap();

        // Should be in local cache
        assert!(cache.local().has_object(&hash));
    }

    #[test]
    fn test_remote_config_priority() {
        let (_temp, mut cache) = create_test_cache();

        // Add in reverse priority order
        cache.add_remote(RemoteCacheConfig {
            url: "https://third.example.com".to_string(),
            priority: 3,
            ..Default::default()
        });
        cache.add_remote(RemoteCacheConfig {
            url: "https://first.example.com".to_string(),
            priority: 1,
            ..Default::default()
        });
        cache.add_remote(RemoteCacheConfig {
            url: "https://second.example.com".to_string(),
            priority: 2,
            ..Default::default()
        });

        // Should be sorted by priority
        assert_eq!(cache.remotes()[0].url, "https://first.example.com");
        assert_eq!(cache.remotes()[1].url, "https://second.example.com");
        assert_eq!(cache.remotes()[2].url, "https://third.example.com");
    }
}
