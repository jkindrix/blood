//! # Content-Addressed Code
//!
//! Blood identifies all definitions by a cryptographic hash of their
//! canonicalized AST, not by name. This module implements the content-addressing
//! system as specified in [CONTENT_ADDRESSED.md](../../CONTENT_ADDRESSED.md).
//!
//! ## Overview
//!
//! ```text
//! Source Code → Canonicalized AST → BLAKE3-256 → Hash
//!                      │
//!                      ▼
//!             CODEBASE DATABASE
//!    Hash → { ast, metadata: { name } }
//! ```
//!
//! ## Benefits
//!
//! - **No Dependency Hell**: Multiple versions coexist by hash
//! - **Perfect Incremental Compilation**: Only recompile changed hashes
//! - **Safe Refactoring**: Renames never break code
//! - **Zero-Downtime Upgrades**: Hot-swap code by hash
//! - **Distributed Caching**: Share compiled artifacts globally
//!
//! ## Modules
//!
//! - [`hash`]: BLAKE3-256 hash types and computation
//! - [`canonical`]: AST canonicalization with de Bruijn indices
//! - [`storage`]: Codebase database operations
//! - [`namespace`]: Name-to-hash mappings
//! - [`vft`]: Virtual Function Table for runtime dispatch

pub mod build_cache;
pub mod canonical;
pub mod distributed_cache;
pub mod hash;
pub mod namespace;
pub mod storage;
pub mod vft;

pub use build_cache::{BuildCache, CacheStats, CacheError, hash_hir_item, extract_dependencies};
pub use canonical::{CanonicalAST, Canonicalizer, DeBruijnIndex};
pub use distributed_cache::{DistributedCache, RemoteCacheConfig, FetchResult};
pub use hash::{ContentHash, HashDisplay, FORMAT_VERSION};
pub use namespace::{Namespace, NameBinding};
pub use storage::{Codebase, DefinitionRecord, StorageError, PersistentCodebase};
pub use vft::{VFT, VFTEntry, CallingConvention};
