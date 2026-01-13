//! # Build Cache
//!
//! Content-addressed build cache for incremental compilation.
//!
//! ## Cache Structure
//!
//! ```text
//! ~/.blood/cache/
//! ├── v1/                    # Version prefix for format changes
//! │   ├── objects/           # Compiled object code by hash prefix
//! │   │   ├── a7/
//! │   │   │   └── f2k9m3... # Object file
//! │   │   └── ...
//! │   ├── ir/                # LLVM IR by hash (for debugging)
//! │   └── deps.json          # Dependency graph
//! └── config.json            # Cache configuration
//! ```
//!
//! ## Incremental Compilation
//!
//! 1. Compute content hash for each HIR item
//! 2. Check cache for existing compiled artifact
//! 3. If cache hit: reuse compiled code
//! 4. If cache miss: compile and store
//! 5. Link all objects together

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write as IoWrite};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::hash::{ContentHash, ContentHasher};
use crate::hir;

/// Cache format version. Increment when changing cache structure.
pub const CACHE_VERSION: u32 = 1;

/// Build cache for content-addressed compilation artifacts.
#[derive(Debug)]
pub struct BuildCache {
    /// Root directory for cache storage.
    cache_dir: PathBuf,
    /// In-memory cache of hash → file path mappings.
    object_cache: HashMap<ContentHash, PathBuf>,
    /// Dependency graph: hash → set of dependency hashes.
    dependencies: HashMap<ContentHash, Vec<ContentHash>>,
    /// Whether to enable caching.
    enabled: bool,
}

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of cache hits.
    pub hits: usize,
    /// Number of cache misses.
    pub misses: usize,
    /// Total cached object size in bytes.
    pub cached_size: u64,
    /// Number of cached objects.
    pub cached_count: usize,
}

/// Errors that can occur during cache operations.
#[derive(Debug)]
pub enum CacheError {
    /// IO error.
    Io(io::Error),
    /// Cache is corrupted.
    Corrupted(String),
    /// Cache version mismatch.
    VersionMismatch { expected: u32, found: u32 },
    /// JSON serialization/deserialization error.
    Json(String),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cache IO error: {}", e),
            Self::Corrupted(msg) => write!(f, "cache corrupted: {}", msg),
            Self::VersionMismatch { expected, found } => {
                write!(f, "cache version mismatch: expected {}, found {}", expected, found)
            }
            Self::Json(msg) => write!(f, "cache JSON error: {}", msg),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<io::Error> for CacheError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for CacheError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}

/// Persistent cache index for dependency tracking.
///
/// This is serialized to `deps.json` in the cache directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheIndex {
    /// Cache format version for compatibility checking.
    version: u32,
    /// Dependency graph: hash → list of dependency hashes.
    dependencies: HashMap<ContentHash, Vec<ContentHash>>,
}

impl BuildCache {
    /// Create a new build cache.
    ///
    /// Uses `~/.blood/cache` by default, or `$BLOOD_CACHE` if set.
    pub fn new() -> Self {
        let cache_dir = Self::default_cache_dir();
        Self {
            cache_dir,
            object_cache: HashMap::new(),
            dependencies: HashMap::new(),
            enabled: true,
        }
    }

    /// Create a build cache with a custom directory.
    pub fn with_dir(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            object_cache: HashMap::new(),
            dependencies: HashMap::new(),
            enabled: true,
        }
    }

    /// Create a disabled cache (no-op for all operations).
    pub fn disabled() -> Self {
        Self {
            cache_dir: PathBuf::new(),
            object_cache: HashMap::new(),
            dependencies: HashMap::new(),
            enabled: false,
        }
    }

    /// Get the default cache directory.
    pub fn default_cache_dir() -> PathBuf {
        if let Ok(cache) = std::env::var("BLOOD_CACHE") {
            return PathBuf::from(cache);
        }

        if let Some(home) = dirs::home_dir() {
            return home.join(".blood").join("cache");
        }

        // Fallback to current directory
        PathBuf::from(".blood-cache")
    }

    /// Initialize the cache directory structure.
    pub fn init(&self) -> Result<(), CacheError> {
        if !self.enabled {
            return Ok(());
        }

        let version_dir = self.version_dir();
        fs::create_dir_all(version_dir.join("objects"))?;
        fs::create_dir_all(version_dir.join("ir"))?;
        Ok(())
    }

    /// Get the version-specific cache directory.
    fn version_dir(&self) -> PathBuf {
        self.cache_dir.join(format!("v{}", CACHE_VERSION))
    }

    /// Get the path for an object file by hash.
    fn object_path(&self, hash: &ContentHash) -> PathBuf {
        let hash_str = format!("{}", hash.full_display());
        let prefix = &hash_str[..2];
        self.version_dir()
            .join("objects")
            .join(prefix)
            .join(&hash_str[2..])
    }

    /// Get the path for an IR file by hash.
    fn ir_path(&self, hash: &ContentHash) -> PathBuf {
        let hash_str = format!("{}", hash.full_display());
        let prefix = &hash_str[..2];
        self.version_dir()
            .join("ir")
            .join(prefix)
            .join(format!("{}.ll", &hash_str[2..]))
    }

    /// Check if a compiled object exists for a hash.
    pub fn has_object(&self, hash: &ContentHash) -> bool {
        if !self.enabled {
            return false;
        }

        // Check in-memory cache first
        if self.object_cache.contains_key(hash) {
            return true;
        }

        // Check filesystem
        self.object_path(hash).exists()
    }

    /// Get a compiled object by hash.
    pub fn get_object(&self, hash: &ContentHash) -> Result<Option<Vec<u8>>, CacheError> {
        if !self.enabled {
            return Ok(None);
        }

        let path = self.object_path(hash);
        if !path.exists() {
            return Ok(None);
        }

        let mut file = fs::File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(Some(data))
    }

    /// Store a compiled object.
    pub fn store_object(&mut self, hash: ContentHash, data: &[u8]) -> Result<PathBuf, CacheError> {
        if !self.enabled {
            return Ok(PathBuf::new());
        }

        let path = self.object_path(&hash);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(&path)?;
        file.write_all(data)?;

        self.object_cache.insert(hash, path.clone());
        Ok(path)
    }

    /// Store LLVM IR for debugging.
    pub fn store_ir(&self, hash: &ContentHash, ir: &str) -> Result<PathBuf, CacheError> {
        if !self.enabled {
            return Ok(PathBuf::new());
        }

        let path = self.ir_path(hash);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&path, ir)?;
        Ok(path)
    }

    /// Get the path where an object file would be stored for a hash.
    /// Returns None if caching is disabled.
    pub fn get_object_path(&self, hash: &ContentHash) -> Option<PathBuf> {
        if self.enabled {
            Some(self.object_path(hash))
        } else {
            None
        }
    }

    /// Record dependencies for a hash.
    pub fn record_dependencies(&mut self, hash: ContentHash, deps: Vec<ContentHash>) {
        if self.enabled {
            self.dependencies.insert(hash, deps);
        }
    }

    /// Get dependencies for a hash.
    pub fn get_dependencies(&self, hash: &ContentHash) -> Option<&Vec<ContentHash>> {
        self.dependencies.get(hash)
    }

    /// Invalidate all dependents of a changed hash.
    pub fn invalidate(&mut self, changed_hash: &ContentHash) -> Result<Vec<ContentHash>, CacheError> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        let mut invalidated = Vec::new();
        let mut to_check = vec![*changed_hash];

        while let Some(hash) = to_check.pop() {
            // Find all hashes that depend on this one
            let dependents: Vec<_> = self.dependencies
                .iter()
                .filter(|(_, deps)| deps.contains(&hash))
                .map(|(h, _)| *h)
                .collect();

            for dep in dependents {
                if !invalidated.contains(&dep) {
                    invalidated.push(dep);
                    to_check.push(dep);

                    // Remove cached object
                    let path = self.object_path(&dep);
                    if path.exists() {
                        fs::remove_file(path)?;
                    }
                    self.object_cache.remove(&dep);
                }
            }
        }

        Ok(invalidated)
    }

    /// Get cache statistics.
    pub fn stats(&self) -> Result<CacheStats, CacheError> {
        if !self.enabled {
            return Ok(CacheStats::default());
        }

        let objects_dir = self.version_dir().join("objects");
        if !objects_dir.exists() {
            return Ok(CacheStats::default());
        }

        let mut stats = CacheStats::default();

        for entry in walkdir::WalkDir::new(&objects_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            stats.cached_count += 1;
            if let Ok(meta) = entry.metadata() {
                stats.cached_size += meta.len();
            }
        }

        Ok(stats)
    }

    /// Clear all cached objects.
    pub fn clear(&mut self) -> Result<(), CacheError> {
        if !self.enabled {
            return Ok(());
        }

        let version_dir = self.version_dir();
        if version_dir.exists() {
            fs::remove_dir_all(&version_dir)?;
        }
        self.object_cache.clear();
        self.dependencies.clear();
        self.init()
    }

    /// Get the path to the dependency index file.
    fn deps_path(&self) -> PathBuf {
        self.version_dir().join("deps.json")
    }

    /// Save the dependency index to disk.
    ///
    /// This persists the dependency graph so that subsequent builds
    /// can use it for incremental invalidation.
    pub fn save_index(&self) -> Result<(), CacheError> {
        if !self.enabled {
            return Ok(());
        }

        let index = CacheIndex {
            version: CACHE_VERSION,
            dependencies: self.dependencies.clone(),
        };

        let json = serde_json::to_string_pretty(&index)?;
        let path = self.deps_path();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&path, json)?;
        Ok(())
    }

    /// Load the dependency index from disk.
    ///
    /// Returns `Ok(true)` if the index was successfully loaded,
    /// `Ok(false)` if no index exists (fresh cache), or an error.
    pub fn load_index(&mut self) -> Result<bool, CacheError> {
        if !self.enabled {
            return Ok(false);
        }

        let path = self.deps_path();
        if !path.exists() {
            return Ok(false);
        }

        let json = fs::read_to_string(&path)?;
        let index: CacheIndex = serde_json::from_str(&json)?;

        // Verify version compatibility
        if index.version != CACHE_VERSION {
            return Err(CacheError::VersionMismatch {
                expected: CACHE_VERSION,
                found: index.version,
            });
        }

        self.dependencies = index.dependencies;
        Ok(true)
    }

    /// Initialize and load existing cache data.
    ///
    /// This creates the cache directory structure and loads any
    /// existing dependency index.
    pub fn init_and_load(&mut self) -> Result<bool, CacheError> {
        self.init()?;
        self.load_index()
    }
}

impl Default for BuildCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute content hash for an HIR item.
///
/// This produces a hash based on the canonical form of the item,
/// suitable for incremental compilation cache keys.
pub fn hash_hir_item(item: &hir::Item, bodies: &HashMap<hir::BodyId, hir::Body>) -> ContentHash {
    let mut hasher = ContentHasher::new();

    // Hash item metadata
    hasher.update_str(&item.name);
    hash_item_kind(&item.kind, bodies, &mut hasher);

    hasher.finalize()
}

/// Hash an item kind.
fn hash_item_kind(kind: &hir::ItemKind, bodies: &HashMap<hir::BodyId, hir::Body>, hasher: &mut ContentHasher) {
    match kind {
        hir::ItemKind::Fn(fn_def) => {
            hasher.update_u8(0x01);
            hash_fn_sig(&fn_def.sig, hasher);
            hash_generics(&fn_def.generics, hasher);
            if let Some(body_id) = fn_def.body_id {
                if let Some(body) = bodies.get(&body_id) {
                    hash_body(body, hasher);
                }
            }
        }
        hir::ItemKind::Struct(struct_def) => {
            hasher.update_u8(0x02);
            hash_generics(&struct_def.generics, hasher);
            hash_struct_kind(&struct_def.kind, hasher);
        }
        hir::ItemKind::Enum(enum_def) => {
            hasher.update_u8(0x03);
            hash_generics(&enum_def.generics, hasher);
            hasher.update_u32(enum_def.variants.len() as u32);
            for variant in &enum_def.variants {
                hasher.update_str(&variant.name);
                hash_struct_kind(&variant.fields, hasher);
            }
        }
        hir::ItemKind::TypeAlias { generics, ty } => {
            hasher.update_u8(0x04);
            hash_generics(generics, hasher);
            hash_type(ty, hasher);
        }
        hir::ItemKind::Const { ty, body_id } => {
            hasher.update_u8(0x05);
            hash_type(ty, hasher);
            if let Some(body) = bodies.get(body_id) {
                hash_body(body, hasher);
            }
        }
        hir::ItemKind::Static { ty, mutable, body_id } => {
            hasher.update_u8(0x06);
            hash_type(ty, hasher);
            hasher.update_u8(if *mutable { 1 } else { 0 });
            if let Some(body) = bodies.get(body_id) {
                hash_body(body, hasher);
            }
        }
        hir::ItemKind::Trait { generics, items } => {
            hasher.update_u8(0x07);
            hash_generics(generics, hasher);
            hasher.update_u32(items.len() as u32);
            for item in items {
                hasher.update_str(&item.name);
            }
        }
        hir::ItemKind::Impl { generics, trait_ref, self_ty, items } => {
            hasher.update_u8(0x08);
            hash_generics(generics, hasher);
            if let Some(tr) = trait_ref {
                hasher.update_u8(1);
                hasher.update_u32(tr.def_id.index);
            } else {
                hasher.update_u8(0);
            }
            hash_type(self_ty, hasher);
            hasher.update_u32(items.len() as u32);
        }
        hir::ItemKind::Effect { generics, operations } => {
            hasher.update_u8(0x09);
            hash_generics(generics, hasher);
            hasher.update_u32(operations.len() as u32);
            for op in operations {
                hasher.update_str(&op.name);
                for input in &op.inputs {
                    hash_type(input, hasher);
                }
                hash_type(&op.output, hasher);
            }
        }
        hir::ItemKind::Handler { generics, kind, effect, state, operations, return_clause } => {
            hasher.update_u8(0x0A);
            hash_generics(generics, hasher);
            // Hash handler kind: 0 = Deep, 1 = Shallow
            hasher.update_u8(match kind {
                hir::HandlerKind::Deep => 0,
                hir::HandlerKind::Shallow => 1,
            });
            hash_type(effect, hasher);
            hasher.update_u32(state.len() as u32);
            hasher.update_u32(operations.len() as u32);
            hasher.update_u8(if return_clause.is_some() { 1 } else { 0 });
        }
        hir::ItemKind::ExternFn(extern_fn) => {
            hasher.update_u8(0x0B);
            hash_fn_sig(&extern_fn.sig, hasher);
            hasher.update_str(&extern_fn.abi);
            if let Some(ref link_name) = extern_fn.link_name {
                hasher.update_u8(1);
                hasher.update_str(link_name);
            } else {
                hasher.update_u8(0);
            }
            hasher.update_u8(if extern_fn.is_variadic { 1 } else { 0 });
        }
        hir::ItemKind::Bridge(bridge) => {
            hasher.update_u8(0x0C);
            hasher.update_str(&bridge.abi);
            hasher.update_u32(bridge.link_specs.len() as u32);
            for link_spec in &bridge.link_specs {
                hasher.update_str(&link_spec.name);
            }
            hasher.update_u32(bridge.extern_fns.len() as u32);
            for func in &bridge.extern_fns {
                hasher.update_str(&func.name);
                hash_fn_sig(&func.sig, hasher);
            }
            hasher.update_u32(bridge.opaque_types.len() as u32);
            hasher.update_u32(bridge.type_aliases.len() as u32);
            hasher.update_u32(bridge.structs.len() as u32);
            hasher.update_u32(bridge.enums.len() as u32);
            hasher.update_u32(bridge.unions.len() as u32);
            hasher.update_u32(bridge.consts.len() as u32);
            hasher.update_u32(bridge.callbacks.len() as u32);
        }
    }
}

/// Hash a function signature.
fn hash_fn_sig(sig: &hir::FnSig, hasher: &mut ContentHasher) {
    hasher.update_u32(sig.inputs.len() as u32);
    for input in &sig.inputs {
        hash_type(input, hasher);
    }
    hash_type(&sig.output, hasher);
    hasher.update_u8(if sig.is_const { 1 } else { 0 });
    hasher.update_u8(if sig.is_async { 1 } else { 0 });
    hasher.update_u8(if sig.is_unsafe { 1 } else { 0 });
}

/// Hash generics.
fn hash_generics(generics: &hir::Generics, hasher: &mut ContentHasher) {
    hasher.update_u32(generics.params.len() as u32);
    for param in &generics.params {
        hasher.update_str(&param.name);
    }
}

/// Hash a struct kind.
fn hash_struct_kind(kind: &hir::StructKind, hasher: &mut ContentHasher) {
    match kind {
        hir::StructKind::Record(fields) => {
            hasher.update_u8(0);
            hasher.update_u32(fields.len() as u32);
            for field in fields {
                if let Some(name) = &field.name {
                    hasher.update_str(name);
                }
                hash_type(&field.ty, hasher);
            }
        }
        hir::StructKind::Tuple(fields) => {
            hasher.update_u8(1);
            hasher.update_u32(fields.len() as u32);
            for field in fields {
                hash_type(&field.ty, hasher);
            }
        }
        hir::StructKind::Unit => {
            hasher.update_u8(2);
        }
    }
}

/// Hash a type.
fn hash_type(ty: &hir::Type, hasher: &mut ContentHasher) {
    match ty.kind() {
        hir::TypeKind::Primitive(p) => {
            hasher.update_u8(0x01);
            hash_primitive_ty(p, hasher);
        }
        hir::TypeKind::Adt { def_id, args } => {
            hasher.update_u8(0x02);
            hasher.update_u32(def_id.index);
            hasher.update_u32(args.len() as u32);
            for arg in args {
                hash_type(arg, hasher);
            }
        }
        hir::TypeKind::Tuple(elems) => {
            hasher.update_u8(0x03);
            hasher.update_u32(elems.len() as u32);
            for elem in elems {
                hash_type(elem, hasher);
            }
        }
        hir::TypeKind::Array { element, size } => {
            hasher.update_u8(0x04);
            hash_type(element, hasher);
            hasher.update_u64(*size);
        }
        hir::TypeKind::Slice { element } => {
            hasher.update_u8(0x05);
            hash_type(element, hasher);
        }
        hir::TypeKind::Ref { inner, mutable } => {
            hasher.update_u8(0x06);
            hasher.update_u8(if *mutable { 1 } else { 0 });
            hash_type(inner, hasher);
        }
        hir::TypeKind::Ptr { inner, mutable } => {
            hasher.update_u8(0x16);
            hasher.update_u8(if *mutable { 1 } else { 0 });
            hash_type(inner, hasher);
        }
        hir::TypeKind::Fn { params, ret } => {
            hasher.update_u8(0x07);
            hasher.update_u32(params.len() as u32);
            for param in params {
                hash_type(param, hasher);
            }
            hash_type(ret, hasher);
        }
        hir::TypeKind::Closure { def_id, params, ret } => {
            hasher.update_u8(0x17); // Distinct from Fn
            hasher.update_u32(def_id.index);
            hasher.update_u32(params.len() as u32);
            for param in params {
                hash_type(param, hasher);
            }
            hash_type(ret, hasher);
        }
        hir::TypeKind::Infer(id) => {
            hasher.update_u8(0x08);
            hasher.update_u32(id.0);
        }
        hir::TypeKind::Error => {
            hasher.update_u8(0xFF);
        }
        hir::TypeKind::Never => {
            hasher.update_u8(0xFE);
        }
        hir::TypeKind::Param(id) => {
            hasher.update_u8(0x09);
            hasher.update_u32(id.0);
        }
        hir::TypeKind::Range { element, inclusive } => {
            hasher.update_u8(0x0A); // Range type
            hasher.update_u8(if *inclusive { 1 } else { 0 });
            hash_type(element, hasher);
        }
        hir::TypeKind::DynTrait { trait_id, auto_traits } => {
            hasher.update_u8(0x0B); // DynTrait type
            hasher.update_u32(trait_id.index);
            hasher.update_u32(auto_traits.len() as u32);
            for auto_trait in auto_traits {
                hasher.update_u32(auto_trait.index);
            }
        }
        hir::TypeKind::Record { fields, row_var } => {
            hasher.update_u8(0x0C); // Record type
            hasher.update_u32(fields.len() as u32);
            for field in fields {
                // Hash field name (Symbol) - use its raw u32 representation
                let sym_raw: u32 = unsafe { std::mem::transmute(field.name) };
                hasher.update_u32(sym_raw);
                hash_type(&field.ty, hasher);
            }
            // Hash row variable if present
            hasher.update_u8(if row_var.is_some() { 1 } else { 0 });
            if let Some(rv) = row_var {
                hasher.update_u32(rv.0);
            }
        }
        hir::TypeKind::Forall { params, body } => {
            hasher.update_u8(0x0D); // Forall type
            hasher.update_u32(params.len() as u32);
            for param in params {
                hasher.update_u32(param.0);
            }
            hash_type(body, hasher);
        }
    }
}

/// Hash a primitive type.
fn hash_primitive_ty(prim: &hir::PrimitiveTy, hasher: &mut ContentHasher) {
    match prim {
        hir::PrimitiveTy::Bool => hasher.update_u8(0x01),
        hir::PrimitiveTy::Char => hasher.update_u8(0x02),
        hir::PrimitiveTy::Int(int_ty) => {
            hasher.update_u8(0x10);
            hasher.update_u8(match int_ty {
                hir::IntTy::I8 => 0,
                hir::IntTy::I16 => 1,
                hir::IntTy::I32 => 2,
                hir::IntTy::I64 => 3,
                hir::IntTy::I128 => 4,
                hir::IntTy::Isize => 5,
            });
        }
        hir::PrimitiveTy::Uint(uint_ty) => {
            hasher.update_u8(0x20);
            hasher.update_u8(match uint_ty {
                hir::UintTy::U8 => 0,
                hir::UintTy::U16 => 1,
                hir::UintTy::U32 => 2,
                hir::UintTy::U64 => 3,
                hir::UintTy::U128 => 4,
                hir::UintTy::Usize => 5,
            });
        }
        hir::PrimitiveTy::Float(float_ty) => {
            hasher.update_u8(0x30);
            hasher.update_u8(match float_ty {
                hir::FloatTy::F32 => 0,
                hir::FloatTy::F64 => 1,
            });
        }
        hir::PrimitiveTy::Str => hasher.update_u8(0x40),
        hir::PrimitiveTy::String => hasher.update_u8(0x41),
        hir::PrimitiveTy::Unit => hasher.update_u8(0x42),
    }
}

/// Hash a function body.
fn hash_body(body: &hir::Body, hasher: &mut ContentHasher) {
    // Hash locals
    hasher.update_u32(body.locals.len() as u32);
    for local in &body.locals {
        if let Some(name) = &local.name {
            hasher.update_str(name);
        }
        hash_type(&local.ty, hasher);
        hasher.update_u8(if local.mutable { 1 } else { 0 });
    }

    // Hash the root expression
    hash_expr(&body.expr, hasher);
}

/// Hash an expression.
fn hash_expr(expr: &hir::Expr, hasher: &mut ContentHasher) {
    match &expr.kind {
        hir::ExprKind::Literal(lit) => {
            hasher.update_u8(0x01);
            hash_literal(lit, hasher);
        }
        hir::ExprKind::Local(local_id) => {
            hasher.update_u8(0x02);
            hasher.update_u32(local_id.index);
        }
        hir::ExprKind::Def(def_id) => {
            hasher.update_u8(0x03);
            hasher.update_u32(def_id.index);
        }
        hir::ExprKind::Call { callee, args } => {
            hasher.update_u8(0x04);
            hash_expr(callee, hasher);
            hasher.update_u32(args.len() as u32);
            for arg in args {
                hash_expr(arg, hasher);
            }
        }
        hir::ExprKind::MethodCall { receiver, method, args } => {
            hasher.update_u8(0x05);
            hash_expr(receiver, hasher);
            hasher.update_u32(method.index);
            hasher.update_u32(args.len() as u32);
            for arg in args {
                hash_expr(arg, hasher);
            }
        }
        hir::ExprKind::Binary { op, left, right } => {
            hasher.update_u8(0x06);
            hasher.update_u8(*op as u8);
            hash_expr(left, hasher);
            hash_expr(right, hasher);
        }
        hir::ExprKind::Unary { op, operand } => {
            hasher.update_u8(0x07);
            hasher.update_u8(*op as u8);
            hash_expr(operand, hasher);
        }
        hir::ExprKind::Block { stmts, expr } => {
            hasher.update_u8(0x08);
            hasher.update_u32(stmts.len() as u32);
            for stmt in stmts {
                hash_stmt(stmt, hasher);
            }
            if let Some(e) = expr {
                hasher.update_u8(1);
                hash_expr(e, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::ExprKind::If { condition, then_branch, else_branch } => {
            hasher.update_u8(0x09);
            hash_expr(condition, hasher);
            hash_expr(then_branch, hasher);
            if let Some(else_e) = else_branch {
                hasher.update_u8(1);
                hash_expr(else_e, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::ExprKind::Loop { body, .. } => {
            hasher.update_u8(0x0A);
            hash_expr(body, hasher);
        }
        hir::ExprKind::While { condition, body, .. } => {
            hasher.update_u8(0x0B);
            hash_expr(condition, hasher);
            hash_expr(body, hasher);
        }
        hir::ExprKind::Match { scrutinee, arms } => {
            hasher.update_u8(0x0C);
            hash_expr(scrutinee, hasher);
            hasher.update_u32(arms.len() as u32);
            for arm in arms {
                hash_pattern(&arm.pattern, hasher);
                if let Some(guard) = &arm.guard {
                    hasher.update_u8(1);
                    hash_expr(guard, hasher);
                } else {
                    hasher.update_u8(0);
                }
                hash_expr(&arm.body, hasher);
            }
        }
        hir::ExprKind::Return(value) => {
            hasher.update_u8(0x0D);
            if let Some(v) = value {
                hasher.update_u8(1);
                hash_expr(v, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::ExprKind::Break { value, .. } => {
            hasher.update_u8(0x0E);
            if let Some(v) = value {
                hasher.update_u8(1);
                hash_expr(v, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::ExprKind::Continue { .. } => {
            hasher.update_u8(0x0F);
        }
        hir::ExprKind::Tuple(elems) => {
            hasher.update_u8(0x10);
            hasher.update_u32(elems.len() as u32);
            for elem in elems {
                hash_expr(elem, hasher);
            }
        }
        hir::ExprKind::Array(elems) => {
            hasher.update_u8(0x11);
            hasher.update_u32(elems.len() as u32);
            for elem in elems {
                hash_expr(elem, hasher);
            }
        }
        hir::ExprKind::Repeat { value, count } => {
            hasher.update_u8(0x12);
            hash_expr(value, hasher);
            hasher.update_u64(*count);
        }
        hir::ExprKind::Index { base, index } => {
            hasher.update_u8(0x13);
            hash_expr(base, hasher);
            hash_expr(index, hasher);
        }
        hir::ExprKind::Field { base, field_idx } => {
            hasher.update_u8(0x14);
            hash_expr(base, hasher);
            hasher.update_u32(*field_idx);
        }
        hir::ExprKind::Struct { def_id, fields, base } => {
            hasher.update_u8(0x15);
            hasher.update_u32(def_id.index);
            hasher.update_u32(fields.len() as u32);
            for field in fields {
                hasher.update_u32(field.field_idx);
                hash_expr(&field.value, hasher);
            }
            if let Some(b) = base {
                hasher.update_u8(1);
                hash_expr(b, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::ExprKind::Variant { def_id, variant_idx, fields } => {
            hasher.update_u8(0x16);
            hasher.update_u32(def_id.index);
            hasher.update_u32(*variant_idx);
            hasher.update_u32(fields.len() as u32);
            for field in fields {
                hash_expr(field, hasher);
            }
        }
        hir::ExprKind::Assign { target, value } => {
            hasher.update_u8(0x17);
            hash_expr(target, hasher);
            hash_expr(value, hasher);
        }
        hir::ExprKind::Cast { expr, target_ty } => {
            hasher.update_u8(0x18);
            hash_expr(expr, hasher);
            hash_type(target_ty, hasher);
        }
        hir::ExprKind::Closure { body_id, captures } => {
            hasher.update_u8(0x19);
            hasher.update_u32(body_id.0);
            hasher.update_u32(captures.len() as u32);
        }
        hir::ExprKind::Borrow { expr, mutable } => {
            hasher.update_u8(0x1A);
            hash_expr(expr, hasher);
            hasher.update_u8(if *mutable { 1 } else { 0 });
        }
        hir::ExprKind::Deref(inner) => {
            hasher.update_u8(0x1B);
            hash_expr(inner, hasher);
        }
        hir::ExprKind::AddrOf { expr, mutable } => {
            hasher.update_u8(0x1C);
            hash_expr(expr, hasher);
            hasher.update_u8(if *mutable { 1 } else { 0 });
        }
        hir::ExprKind::Let { pattern, init } => {
            hasher.update_u8(0x1D);
            hash_pattern(pattern, hasher);
            hash_expr(init, hasher);
        }
        hir::ExprKind::Unsafe(inner) => {
            hasher.update_u8(0x1E);
            hash_expr(inner, hasher);
        }
        hir::ExprKind::Perform { effect_id, op_index, args } => {
            hasher.update_u8(0x1F);
            hasher.update_u32(effect_id.index);
            hasher.update_u32(*op_index);
            hasher.update_u32(args.len() as u32);
            for arg in args {
                hash_expr(arg, hasher);
            }
        }
        hir::ExprKind::Resume { value } => {
            hasher.update_u8(0x20);
            if let Some(v) = value {
                hasher.update_u8(1);
                hash_expr(v, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::ExprKind::Handle { body, handler_id, handler_instance } => {
            hasher.update_u8(0x21);
            hash_expr(body, hasher);
            hasher.update_u32(handler_id.index);
            hash_expr(handler_instance, hasher);
        }
        hir::ExprKind::Range { start, end, inclusive } => {
            hasher.update_u8(0x22);
            hasher.update_u8(if *inclusive { 1 } else { 0 });
            if let Some(s) = start {
                hasher.update_u8(1);
                hash_expr(s, hasher);
            } else {
                hasher.update_u8(0);
            }
            if let Some(e) = end {
                hasher.update_u8(1);
                hash_expr(e, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::ExprKind::Error => {
            hasher.update_u8(0xFF);
        }
    }
}

/// Hash a literal value.
fn hash_literal(lit: &hir::LiteralValue, hasher: &mut ContentHasher) {
    match lit {
        hir::LiteralValue::Int(n) => {
            hasher.update_u8(0x01);
            hasher.update_i128(*n);
        }
        hir::LiteralValue::Uint(n) => {
            hasher.update_u8(0x02);
            // Hash as bytes since we don't have update_u128
            hasher.update(&n.to_le_bytes());
        }
        hir::LiteralValue::Float(f) => {
            hasher.update_u8(0x03);
            hasher.update_f64(*f);
        }
        hir::LiteralValue::Bool(b) => {
            hasher.update_u8(0x04);
            hasher.update_u8(if *b { 1 } else { 0 });
        }
        hir::LiteralValue::Char(c) => {
            hasher.update_u8(0x05);
            hasher.update_u32(*c as u32);
        }
        hir::LiteralValue::String(s) => {
            hasher.update_u8(0x06);
            hasher.update_str(s);
        }
    }
}

/// Hash a statement.
fn hash_stmt(stmt: &hir::Stmt, hasher: &mut ContentHasher) {
    match stmt {
        hir::Stmt::Let { local_id, init } => {
            hasher.update_u8(0x01);
            hasher.update_u32(local_id.index);
            if let Some(init) = init {
                hasher.update_u8(1);
                hash_expr(init, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::Stmt::Expr(expr) => {
            hasher.update_u8(0x02);
            hash_expr(expr, hasher);
        }
        hir::Stmt::Item(def_id) => {
            hasher.update_u8(0x03);
            hasher.update_u32(def_id.index);
        }
    }
}

/// Hash a pattern.
fn hash_pattern(pattern: &hir::Pattern, hasher: &mut ContentHasher) {
    match &pattern.kind {
        hir::PatternKind::Wildcard => {
            hasher.update_u8(0x01);
        }
        hir::PatternKind::Binding { local_id, mutable, subpattern } => {
            hasher.update_u8(0x02);
            hasher.update_u32(local_id.index);
            hasher.update_u8(if *mutable { 1 } else { 0 });
            if let Some(sub) = subpattern {
                hasher.update_u8(1);
                hash_pattern(sub, hasher);
            } else {
                hasher.update_u8(0);
            }
        }
        hir::PatternKind::Literal(lit) => {
            hasher.update_u8(0x03);
            hash_literal(lit, hasher);
        }
        hir::PatternKind::Tuple(pats) => {
            hasher.update_u8(0x04);
            hasher.update_u32(pats.len() as u32);
            for pat in pats {
                hash_pattern(pat, hasher);
            }
        }
        hir::PatternKind::Struct { def_id, fields } => {
            hasher.update_u8(0x05);
            hasher.update_u32(def_id.index);
            hasher.update_u32(fields.len() as u32);
            for field in fields {
                hasher.update_u32(field.field_idx);
                hash_pattern(&field.pattern, hasher);
            }
        }
        hir::PatternKind::Variant { def_id, variant_idx, fields } => {
            hasher.update_u8(0x06);
            hasher.update_u32(def_id.index);
            hasher.update_u32(*variant_idx);
            hasher.update_u32(fields.len() as u32);
            for field in fields {
                hash_pattern(field, hasher);
            }
        }
        hir::PatternKind::Or(pats) => {
            hasher.update_u8(0x07);
            hasher.update_u32(pats.len() as u32);
            for pat in pats {
                hash_pattern(pat, hasher);
            }
        }
        hir::PatternKind::Slice { prefix, slice, suffix } => {
            hasher.update_u8(0x08);
            hasher.update_u32(prefix.len() as u32);
            for pat in prefix {
                hash_pattern(pat, hasher);
            }
            if let Some(s) = slice {
                hasher.update_u8(1);
                hash_pattern(s, hasher);
            } else {
                hasher.update_u8(0);
            }
            hasher.update_u32(suffix.len() as u32);
            for pat in suffix {
                hash_pattern(pat, hasher);
            }
        }
        hir::PatternKind::Ref { mutable, inner } => {
            hasher.update_u8(0x09);
            hasher.update_u8(if *mutable { 1 } else { 0 });
            hash_pattern(inner, hasher);
        }
        hir::PatternKind::Range { start, end, inclusive } => {
            hasher.update_u8(0x0A);
            if let Some(s) = start {
                hasher.update_u8(1);
                hash_pattern(s, hasher);
            } else {
                hasher.update_u8(0);
            }
            if let Some(e) = end {
                hasher.update_u8(1);
                hash_pattern(e, hasher);
            } else {
                hasher.update_u8(0);
            }
            hasher.update_u8(if *inclusive { 1 } else { 0 });
        }
    }
}

// ============================================================================
// Dependency Extraction
// ============================================================================

use std::collections::HashSet;
use crate::hir::DefId;

/// Extract all DefId dependencies from an HIR item.
///
/// This walks the item's AST to find all referenced definitions,
/// which is used for incremental compilation invalidation.
pub fn extract_dependencies(
    item: &hir::Item,
    bodies: &HashMap<hir::BodyId, hir::Body>,
) -> HashSet<DefId> {
    let mut deps = HashSet::new();
    extract_item_deps(&item.kind, bodies, &mut deps);
    deps
}

/// Extract dependencies from an item kind.
fn extract_item_deps(
    kind: &hir::ItemKind,
    bodies: &HashMap<hir::BodyId, hir::Body>,
    deps: &mut HashSet<DefId>,
) {
    match kind {
        hir::ItemKind::Fn(fn_def) => {
            extract_type_deps(&fn_def.sig.output, deps);
            for input in &fn_def.sig.inputs {
                extract_type_deps(input, deps);
            }
            if let Some(body_id) = fn_def.body_id {
                if let Some(body) = bodies.get(&body_id) {
                    extract_body_deps(body, deps);
                }
            }
        }
        hir::ItemKind::Struct(struct_def) => {
            extract_struct_kind_deps(&struct_def.kind, deps);
        }
        hir::ItemKind::Enum(enum_def) => {
            for variant in &enum_def.variants {
                extract_struct_kind_deps(&variant.fields, deps);
            }
        }
        hir::ItemKind::TypeAlias { ty, .. } => {
            extract_type_deps(ty, deps);
        }
        hir::ItemKind::Const { ty, body_id } => {
            extract_type_deps(ty, deps);
            if let Some(body) = bodies.get(body_id) {
                extract_body_deps(body, deps);
            }
        }
        hir::ItemKind::Static { ty, body_id, .. } => {
            extract_type_deps(ty, deps);
            if let Some(body) = bodies.get(body_id) {
                extract_body_deps(body, deps);
            }
        }
        hir::ItemKind::Trait { items, .. } => {
            for item in items {
                if let hir::TraitItemKind::Fn(sig, _body_id) = &item.kind {
                    extract_type_deps(&sig.output, deps);
                    for input in &sig.inputs {
                        extract_type_deps(input, deps);
                    }
                }
            }
        }
        hir::ItemKind::Impl { trait_ref, self_ty, items, .. } => {
            if let Some(tr) = trait_ref {
                deps.insert(tr.def_id);
            }
            extract_type_deps(self_ty, deps);
            for impl_item in items {
                if let hir::ImplItemKind::Fn(sig, body_id) = &impl_item.kind {
                    extract_type_deps(&sig.output, deps);
                    for input in &sig.inputs {
                        extract_type_deps(input, deps);
                    }
                    if let Some(body) = bodies.get(body_id) {
                        extract_body_deps(body, deps);
                    }
                }
            }
        }
        hir::ItemKind::Effect { operations, .. } => {
            for op in operations {
                for input in &op.inputs {
                    extract_type_deps(input, deps);
                }
                extract_type_deps(&op.output, deps);
            }
        }
        hir::ItemKind::Handler { effect, operations, .. } => {
            extract_type_deps(effect, deps);
            for op in operations {
                // body_id is BodyId, not Option<BodyId>
                if let Some(body) = bodies.get(&op.body_id) {
                    extract_body_deps(body, deps);
                }
            }
        }
        hir::ItemKind::ExternFn(extern_fn) => {
            // External functions only depend on their signature types
            extract_type_deps(&extern_fn.sig.output, deps);
            for input in &extern_fn.sig.inputs {
                extract_type_deps(input, deps);
            }
        }
        hir::ItemKind::Bridge(bridge) => {
            // Bridge depends on all types used in its declarations
            for func in &bridge.extern_fns {
                extract_type_deps(&func.sig.output, deps);
                for input in &func.sig.inputs {
                    extract_type_deps(input, deps);
                }
            }
            for ta in &bridge.type_aliases {
                extract_type_deps(&ta.ty, deps);
            }
            for s in &bridge.structs {
                for field in &s.fields {
                    extract_type_deps(&field.ty, deps);
                }
            }
            for e in &bridge.enums {
                extract_type_deps(&e.repr, deps);
            }
            for u in &bridge.unions {
                for field in &u.fields {
                    extract_type_deps(&field.ty, deps);
                }
            }
            for c in &bridge.consts {
                extract_type_deps(&c.ty, deps);
            }
            for cb in &bridge.callbacks {
                extract_type_deps(&cb.return_type, deps);
                for param in &cb.params {
                    extract_type_deps(param, deps);
                }
            }
        }
    }
}

/// Extract dependencies from a struct kind.
fn extract_struct_kind_deps(kind: &hir::StructKind, deps: &mut HashSet<DefId>) {
    match kind {
        hir::StructKind::Record(fields) | hir::StructKind::Tuple(fields) => {
            for field in fields {
                extract_type_deps(&field.ty, deps);
            }
        }
        hir::StructKind::Unit => {}
    }
}

/// Extract dependencies from a type.
fn extract_type_deps(ty: &hir::Type, deps: &mut HashSet<DefId>) {
    match ty.kind() {
        hir::TypeKind::Adt { def_id, args } => {
            deps.insert(*def_id);
            for arg in args {
                extract_type_deps(arg, deps);
            }
        }
        hir::TypeKind::Tuple(elems) => {
            for elem in elems {
                extract_type_deps(elem, deps);
            }
        }
        hir::TypeKind::Array { element, .. } => {
            extract_type_deps(element, deps);
        }
        hir::TypeKind::Slice { element } => {
            extract_type_deps(element, deps);
        }
        hir::TypeKind::Ref { inner, .. } | hir::TypeKind::Ptr { inner, .. } => {
            extract_type_deps(inner, deps);
        }
        hir::TypeKind::Fn { params, ret } => {
            for param in params {
                extract_type_deps(param, deps);
            }
            extract_type_deps(ret, deps);
        }
        hir::TypeKind::Closure { def_id, params, ret } => {
            deps.insert(*def_id);
            for param in params {
                extract_type_deps(param, deps);
            }
            extract_type_deps(ret, deps);
        }
        hir::TypeKind::DynTrait { trait_id, auto_traits } => {
            deps.insert(*trait_id);
            for auto_trait in auto_traits {
                deps.insert(*auto_trait);
            }
        }
        hir::TypeKind::Range { element, .. } => {
            extract_type_deps(element, deps);
        }
        hir::TypeKind::Record { fields, .. } => {
            for field in fields {
                extract_type_deps(&field.ty, deps);
            }
        }
        hir::TypeKind::Forall { body, .. } => {
            extract_type_deps(body, deps);
        }
        // Primitive types, inference variables, type parameters, never, and error types
        // don't contain dependencies on other definitions
        hir::TypeKind::Primitive(_)
        | hir::TypeKind::Infer(_)
        | hir::TypeKind::Param(_)
        | hir::TypeKind::Never
        | hir::TypeKind::Error => {}
    }
}

/// Extract dependencies from a function body.
fn extract_body_deps(body: &hir::Body, deps: &mut HashSet<DefId>) {
    extract_expr_deps(&body.expr, deps);
}

/// Extract dependencies from an expression.
fn extract_expr_deps(expr: &hir::Expr, deps: &mut HashSet<DefId>) {
    // Extract type dependencies from expression type
    extract_type_deps(&expr.ty, deps);

    match &expr.kind {
        // Definition reference
        hir::ExprKind::Def(def_id) => {
            deps.insert(*def_id);
        }
        // Local variables don't add dependencies
        hir::ExprKind::Local(_) => {}
        hir::ExprKind::Call { callee, args } => {
            extract_expr_deps(callee, deps);
            for arg in args {
                extract_expr_deps(arg, deps);
            }
        }
        hir::ExprKind::MethodCall { receiver, method, args } => {
            deps.insert(*method);
            extract_expr_deps(receiver, deps);
            for arg in args {
                extract_expr_deps(arg, deps);
            }
        }
        hir::ExprKind::Struct { def_id, fields, base } => {
            deps.insert(*def_id);
            for field in fields {
                extract_expr_deps(&field.value, deps);
            }
            if let Some(b) = base {
                extract_expr_deps(b, deps);
            }
        }
        hir::ExprKind::Variant { def_id, fields, .. } => {
            deps.insert(*def_id);
            for field in fields {
                extract_expr_deps(field, deps);
            }
        }
        hir::ExprKind::Binary { left, right, .. } => {
            extract_expr_deps(left, deps);
            extract_expr_deps(right, deps);
        }
        hir::ExprKind::Unary { operand, .. } => {
            extract_expr_deps(operand, deps);
        }
        hir::ExprKind::Block { stmts, expr } => {
            for stmt in stmts {
                extract_stmt_deps(stmt, deps);
            }
            if let Some(e) = expr {
                extract_expr_deps(e, deps);
            }
        }
        hir::ExprKind::If { condition, then_branch, else_branch } => {
            extract_expr_deps(condition, deps);
            extract_expr_deps(then_branch, deps);
            if let Some(e) = else_branch {
                extract_expr_deps(e, deps);
            }
        }
        hir::ExprKind::Loop { body, .. } => {
            extract_expr_deps(body, deps);
        }
        hir::ExprKind::While { condition, body, .. } => {
            extract_expr_deps(condition, deps);
            extract_expr_deps(body, deps);
        }
        hir::ExprKind::Match { scrutinee, arms } => {
            extract_expr_deps(scrutinee, deps);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    extract_expr_deps(guard, deps);
                }
                extract_expr_deps(&arm.body, deps);
            }
        }
        hir::ExprKind::Return(value) => {
            if let Some(v) = value {
                extract_expr_deps(v, deps);
            }
        }
        hir::ExprKind::Break { value, .. } => {
            if let Some(v) = value {
                extract_expr_deps(v, deps);
            }
        }
        hir::ExprKind::Tuple(elems) | hir::ExprKind::Array(elems) => {
            for elem in elems {
                extract_expr_deps(elem, deps);
            }
        }
        hir::ExprKind::Repeat { value, .. } => {
            extract_expr_deps(value, deps);
        }
        hir::ExprKind::Index { base, index } => {
            extract_expr_deps(base, deps);
            extract_expr_deps(index, deps);
        }
        hir::ExprKind::Field { base, .. } => {
            extract_expr_deps(base, deps);
        }
        hir::ExprKind::Cast { expr, target_ty } => {
            extract_expr_deps(expr, deps);
            extract_type_deps(target_ty, deps);
        }
        hir::ExprKind::Assign { target, value } => {
            extract_expr_deps(target, deps);
            extract_expr_deps(value, deps);
        }
        hir::ExprKind::Deref(inner) | hir::ExprKind::Borrow { expr: inner, .. }
        | hir::ExprKind::AddrOf { expr: inner, .. } => {
            extract_expr_deps(inner, deps);
        }
        hir::ExprKind::Closure { body_id, .. } => {
            // Closure body dependencies would be tracked separately
            // when the closure body is analyzed
            let _ = body_id;
        }
        hir::ExprKind::Perform { effect_id, args, .. } => {
            deps.insert(*effect_id);
            for arg in args {
                extract_expr_deps(arg, deps);
            }
        }
        hir::ExprKind::Resume { value } => {
            if let Some(v) = value {
                extract_expr_deps(v, deps);
            }
        }
        hir::ExprKind::Handle { body, handler_id, handler_instance } => {
            deps.insert(*handler_id);
            extract_expr_deps(body, deps);
            // handler_instance is Box<Expr>, not Option
            extract_expr_deps(handler_instance, deps);
        }
        hir::ExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                extract_expr_deps(s, deps);
            }
            if let Some(e) = end {
                extract_expr_deps(e, deps);
            }
        }
        hir::ExprKind::Let { init, .. } => {
            extract_expr_deps(init, deps);
        }
        hir::ExprKind::Unsafe(inner) => {
            extract_expr_deps(inner, deps);
        }
        // Leaf expressions that don't contain dependencies
        // (Local is handled above at line 1330)
        hir::ExprKind::Literal(_)
        | hir::ExprKind::Continue { .. }
        | hir::ExprKind::Error => {}
    }
}

/// Extract dependencies from a statement.
fn extract_stmt_deps(stmt: &hir::Stmt, deps: &mut HashSet<DefId>) {
    match stmt {
        hir::Stmt::Let { init, .. } => {
            if let Some(e) = init {
                extract_expr_deps(e, deps);
            }
        }
        hir::Stmt::Expr(e) => {
            extract_expr_deps(e, deps);
        }
        hir::Stmt::Item(def_id) => {
            deps.insert(*def_id);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_build_cache_init() {
        let temp_dir = TempDir::new().unwrap();
        let cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        assert!(temp_dir.path().join("v1/objects").exists());
        assert!(temp_dir.path().join("v1/ir").exists());
    }

    #[test]
    fn test_build_cache_store_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let hash = ContentHash::compute(b"test function");
        let data = vec![0x90, 0x90, 0xC3]; // NOP NOP RET

        cache.store_object(hash, &data).unwrap();

        assert!(cache.has_object(&hash));
        let retrieved = cache.get_object(&hash).unwrap().unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_build_cache_disabled() {
        let cache = BuildCache::disabled();
        let hash = ContentHash::compute(b"test");

        assert!(!cache.has_object(&hash));
        assert!(cache.get_object(&hash).unwrap().is_none());
    }

    #[test]
    fn test_build_cache_dependencies() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let hash_a = ContentHash::compute(b"a");
        let hash_b = ContentHash::compute(b"b");
        let hash_c = ContentHash::compute(b"c");

        // C depends on B, B depends on A
        cache.record_dependencies(hash_b, vec![hash_a]);
        cache.record_dependencies(hash_c, vec![hash_b]);

        // Store objects
        cache.store_object(hash_a, b"a").unwrap();
        cache.store_object(hash_b, b"b").unwrap();
        cache.store_object(hash_c, b"c").unwrap();

        // Invalidate A
        let invalidated = cache.invalidate(&hash_a).unwrap();

        // Both B and C should be invalidated (transitive)
        assert!(invalidated.contains(&hash_b));
        assert!(invalidated.contains(&hash_c));
    }

    #[test]
    fn test_build_cache_clear() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let hash = ContentHash::compute(b"test");
        cache.store_object(hash, b"data").unwrap();
        assert!(cache.has_object(&hash));

        cache.clear().unwrap();
        assert!(!cache.has_object(&hash));
    }

    #[test]
    fn test_build_cache_save_and_load_index() {
        let temp_dir = TempDir::new().unwrap();

        let hash_a = ContentHash::compute(b"a");
        let hash_b = ContentHash::compute(b"b");
        let hash_c = ContentHash::compute(b"c");

        // Create cache, add dependencies, save index
        {
            let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
            cache.init().unwrap();

            cache.record_dependencies(hash_b, vec![hash_a]);
            cache.record_dependencies(hash_c, vec![hash_b, hash_a]);

            cache.save_index().unwrap();
        }

        // Create new cache instance and load index
        {
            let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
            let loaded = cache.load_index().unwrap();
            assert!(loaded);

            // Verify dependencies were restored
            let deps_b = cache.get_dependencies(&hash_b).unwrap();
            assert_eq!(deps_b.len(), 1);
            assert!(deps_b.contains(&hash_a));

            let deps_c = cache.get_dependencies(&hash_c).unwrap();
            assert_eq!(deps_c.len(), 2);
            assert!(deps_c.contains(&hash_a));
            assert!(deps_c.contains(&hash_b));
        }
    }

    #[test]
    fn test_build_cache_load_nonexistent_index() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        // Should return Ok(false) for missing index
        let loaded = cache.load_index().unwrap();
        assert!(!loaded);
    }

    #[test]
    fn test_build_cache_init_and_load() {
        let temp_dir = TempDir::new().unwrap();

        let hash_a = ContentHash::compute(b"a");
        let hash_b = ContentHash::compute(b"b");

        // First build: create and save
        {
            let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
            cache.init().unwrap();
            cache.record_dependencies(hash_b, vec![hash_a]);
            cache.save_index().unwrap();
        }

        // Second build: init_and_load should restore dependencies
        {
            let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
            let loaded = cache.init_and_load().unwrap();
            assert!(loaded);

            let deps = cache.get_dependencies(&hash_b).unwrap();
            assert!(deps.contains(&hash_a));
        }
    }

    #[test]
    fn test_content_hash_serialization_roundtrip() {
        let original = ContentHash::compute(b"test serialization");

        // Serialize to string
        let serialized: String = original.into();
        assert_eq!(serialized.len(), 64); // 32 bytes * 2 hex chars

        // Deserialize back
        let deserialized = ContentHash::try_from(serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_incremental_compilation_simulation() {
        // Simulates an incremental compilation scenario:
        // 1. First build: compile all items, cache them
        // 2. Second build: detect cached items, skip recompilation

        let temp_dir = TempDir::new().unwrap();

        // Define some "source code" that produces deterministic hashes
        let fn_add_source = b"fn add(x, y) { x + y }";
        let fn_mul_source = b"fn mul(x, y) { x * y }";
        let fn_compute_source = b"fn compute(a, b) { add(a, mul(a, b)) }";

        // Compiled "object code" (simulated)
        let fn_add_obj = b"add_compiled_object_code";
        let fn_mul_obj = b"mul_compiled_object_code";
        let fn_compute_obj = b"compute_compiled_object_code";

        // First build: everything is new
        let (hash_add, hash_mul, hash_compute) = {
            let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
            cache.init().unwrap();

            let hash_add = ContentHash::compute(fn_add_source);
            let hash_mul = ContentHash::compute(fn_mul_source);
            let hash_compute = ContentHash::compute(fn_compute_source);

            // Nothing should be cached yet
            assert!(!cache.has_object(&hash_add));
            assert!(!cache.has_object(&hash_mul));
            assert!(!cache.has_object(&hash_compute));

            // "Compile" and store
            cache.store_object(hash_add, fn_add_obj).unwrap();
            cache.store_object(hash_mul, fn_mul_obj).unwrap();
            cache.store_object(hash_compute, fn_compute_obj).unwrap();

            // Record dependencies: compute depends on add and mul
            cache.record_dependencies(hash_compute, vec![hash_add, hash_mul]);

            // Save index for next build
            cache.save_index().unwrap();

            (hash_add, hash_mul, hash_compute)
        };

        // Second build: everything should be cached
        {
            let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
            cache.init_and_load().unwrap();

            // All items should be cached
            assert!(cache.has_object(&hash_add));
            assert!(cache.has_object(&hash_mul));
            assert!(cache.has_object(&hash_compute));

            // Verify cached content is correct
            let cached_add = cache.get_object(&hash_add).unwrap().unwrap();
            assert_eq!(&cached_add[..], fn_add_obj);

            // Dependencies should be restored
            let deps = cache.get_dependencies(&hash_compute).unwrap();
            assert!(deps.contains(&hash_add));
            assert!(deps.contains(&hash_mul));
        }

        // Third build: change `add` function
        let new_fn_add_source = b"fn add(x, y) { x + y + 0 }"; // Changed!
        let new_hash_add = ContentHash::compute(new_fn_add_source);
        assert_ne!(hash_add, new_hash_add); // Hash should be different

        {
            let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
            cache.init_and_load().unwrap();

            // Old add is cached, new add is not
            assert!(cache.has_object(&hash_add));
            assert!(!cache.has_object(&new_hash_add));

            // Invalidate old add - this should invalidate compute too
            let invalidated = cache.invalidate(&hash_add).unwrap();
            assert!(invalidated.contains(&hash_compute), "compute should be invalidated");

            // mul is not affected
            assert!(cache.has_object(&hash_mul));
        }
    }

    #[test]
    fn test_ir_storage() {
        let temp_dir = TempDir::new().unwrap();
        let cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        let hash = ContentHash::compute(b"some function");
        let ir = "; ModuleID = 'test'\ndefine i32 @main() { ret i32 0 }";

        let path = cache.store_ir(&hash, ir).unwrap();
        assert!(path.exists());

        let stored_ir = std::fs::read_to_string(&path).unwrap();
        assert_eq!(stored_ir, ir);
    }

    #[test]
    fn test_get_object_path() {
        let temp_dir = TempDir::new().unwrap();
        let cache = BuildCache::with_dir(temp_dir.path().to_path_buf());

        let hash = ContentHash::compute(b"test");

        // Enabled cache should return a path
        let path = cache.get_object_path(&hash);
        assert!(path.is_some());

        // Path should contain the hash
        let path_str = path.unwrap().to_string_lossy().to_string();
        let hash_str = format!("{}", hash.full_display());
        assert!(path_str.contains(&hash_str[2..]), "path should contain hash suffix");

        // Disabled cache should return None
        let disabled = BuildCache::disabled();
        assert!(disabled.get_object_path(&hash).is_none());
    }

    #[test]
    fn test_cache_stats() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = BuildCache::with_dir(temp_dir.path().to_path_buf());
        cache.init().unwrap();

        // Initially empty
        let stats = cache.stats().unwrap();
        assert_eq!(stats.cached_count, 0);
        assert_eq!(stats.cached_size, 0);

        // Add some objects
        let hash1 = ContentHash::compute(b"obj1");
        let hash2 = ContentHash::compute(b"obj2");
        let data1 = vec![0u8; 100];
        let data2 = vec![0u8; 200];

        cache.store_object(hash1, &data1).unwrap();
        cache.store_object(hash2, &data2).unwrap();

        let stats = cache.stats().unwrap();
        assert_eq!(stats.cached_count, 2);
        assert_eq!(stats.cached_size, 300);
    }
}
