//! Project-level incremental compiler.
//!
//! This module provides the `ProjectCompiler` which orchestrates
//! incremental compilation for multi-file Blood projects by integrating:
//!
//! - `FileCache`: Tracks source file changes
//! - `DependencyGraph`: Manages module dependencies
//! - `BuildCache`: Caches compiled artifacts
//!
//! ## Incremental Compilation Flow
//!
//! ```text
//! 1. Load file cache and build cache from disk
//! 2. Scan source files for changes
//! 3. Identify changed files and their dependencies
//! 4. Invalidate cached artifacts for changed definitions
//! 5. Parse/type-check only changed files
//! 6. Compile changed definitions
//! 7. Link with cached artifacts for unchanged definitions
//! 8. Save updated caches
//! ```

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::file_cache::{FileCache, FileCacheError, FileStatus};
use super::graph::DependencyGraph;
use super::resolve::ModuleId;
use crate::content::build_cache::BuildCache;
use crate::content::hash::ContentHash;
use crate::hir::DefId;

/// Result of an incremental compilation analysis.
#[derive(Debug, Clone)]
pub struct IncrementalAnalysis {
    /// Files that need to be recompiled.
    pub changed_files: Vec<PathBuf>,
    /// Files that have been deleted.
    pub deleted_files: Vec<PathBuf>,
    /// Files that are unchanged.
    pub unchanged_files: Vec<PathBuf>,
    /// Definitions that need to be recompiled.
    pub invalidated_defs: HashSet<DefId>,
    /// Modules that need to be recompiled.
    pub invalidated_modules: HashSet<ModuleId>,
    /// Content hashes that have been invalidated.
    pub invalidated_hashes: Vec<ContentHash>,
    /// Whether this is a full rebuild (no valid cache).
    pub is_full_rebuild: bool,
}

/// Statistics about an incremental compilation.
#[derive(Debug, Clone, Default)]
pub struct IncrementalStats {
    /// Number of files analyzed.
    pub files_analyzed: usize,
    /// Number of files recompiled.
    pub files_recompiled: usize,
    /// Number of files skipped (cache hit).
    pub files_cached: usize,
    /// Number of definitions invalidated.
    pub defs_invalidated: usize,
    /// Time spent on analysis (milliseconds).
    pub analysis_time_ms: u64,
    /// Time spent on compilation (milliseconds).
    pub compilation_time_ms: u64,
}

/// Project-level incremental compiler.
#[derive(Debug)]
pub struct ProjectCompiler {
    /// Project root directory.
    project_root: PathBuf,
    /// File-level cache.
    file_cache: FileCache,
    /// Dependency graph for modules.
    dep_graph: DependencyGraph,
    /// Whether incremental compilation is enabled.
    incremental_enabled: bool,
}

impl ProjectCompiler {
    /// Create a new project compiler.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            project_root: project_root.clone(),
            file_cache: FileCache::new(project_root),
            dep_graph: DependencyGraph::new(),
            incremental_enabled: true,
        }
    }

    /// Create a project compiler with incremental compilation disabled.
    pub fn without_incremental(project_root: PathBuf) -> Self {
        Self {
            project_root,
            file_cache: FileCache::disabled(),
            dep_graph: DependencyGraph::new(),
            incremental_enabled: false,
        }
    }

    /// Get the project root.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Get a mutable reference to the file cache.
    pub fn file_cache_mut(&mut self) -> &mut FileCache {
        &mut self.file_cache
    }

    /// Get a reference to the file cache.
    pub fn file_cache(&self) -> &FileCache {
        &self.file_cache
    }

    /// Get a mutable reference to the dependency graph.
    pub fn dep_graph_mut(&mut self) -> &mut DependencyGraph {
        &mut self.dep_graph
    }

    /// Get a reference to the dependency graph.
    pub fn dep_graph(&self) -> &DependencyGraph {
        &self.dep_graph
    }

    /// Get the path to the dependency graph cache file.
    fn dep_graph_path(&self) -> PathBuf {
        self.project_root.join(".blood").join("dep_graph.json")
    }

    /// Initialize the compiler caches.
    ///
    /// Returns `true` if existing cache data was loaded.
    pub fn init(&mut self) -> Result<bool, FileCacheError> {
        if !self.incremental_enabled {
            return Ok(false);
        }

        let file_cache_loaded = self.file_cache.init_and_load()?;

        // Load dependency graph if file cache was loaded
        if file_cache_loaded {
            self.dep_graph = DependencyGraph::load_or_new(&self.dep_graph_path());
        }

        Ok(file_cache_loaded)
    }

    /// Save the compiler caches.
    pub fn save(&self) -> Result<(), FileCacheError> {
        if !self.incremental_enabled {
            return Ok(());
        }

        self.file_cache.save()?;

        // Save dependency graph
        let dep_graph_path = self.dep_graph_path();
        if let Some(parent) = dep_graph_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.dep_graph.save(&dep_graph_path)
            .map_err(|e| FileCacheError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;

        Ok(())
    }

    /// Analyze a set of source files for incremental compilation.
    ///
    /// This determines which files have changed and what needs to be recompiled.
    pub fn analyze(&self, source_files: &[PathBuf]) -> IncrementalAnalysis {
        if !self.incremental_enabled || self.file_cache.is_empty() {
            // No cache or disabled - full rebuild
            return IncrementalAnalysis {
                changed_files: source_files.to_vec(),
                deleted_files: Vec::new(),
                unchanged_files: Vec::new(),
                invalidated_defs: HashSet::new(),
                invalidated_modules: HashSet::new(),
                invalidated_hashes: Vec::new(),
                is_full_rebuild: true,
            };
        }

        let mut changed_files = Vec::new();
        let mut deleted_files = Vec::new();
        let mut unchanged_files = Vec::new();

        // Check each source file
        for path in source_files {
            match self.file_cache.check_file(path) {
                FileStatus::Unchanged => unchanged_files.push(path.clone()),
                FileStatus::Modified | FileStatus::New => changed_files.push(path.clone()),
                FileStatus::Deleted => deleted_files.push(path.clone()),
            }
        }

        // Check for files in cache that are no longer in source list
        for cached_path in self.file_cache.cached_files() {
            if !source_files.contains(cached_path) && !deleted_files.contains(cached_path) {
                deleted_files.push(cached_path.clone());
            }
        }

        // Get invalidated definitions from changed/deleted files
        let mut invalidated_defs = self.file_cache.get_invalidated_definitions(&changed_files);
        invalidated_defs.extend(self.file_cache.get_invalidated_definitions(&deleted_files));

        // Get invalidated modules
        let mut invalidated_modules = self.file_cache.get_invalidated_modules(&changed_files);
        invalidated_modules.extend(self.file_cache.get_invalidated_modules(&deleted_files));

        // Use dependency graph to find transitive invalidations
        let all_invalidated_modules: HashSet<_> = self.dep_graph
            .invalidation_set(&invalidated_modules.iter().copied().collect::<Vec<_>>());

        // Add files for transitively invalidated modules
        for path in self.file_cache.cached_files() {
            if let Some(module_id) = self.file_cache.get_module(path) {
                if all_invalidated_modules.contains(&module_id) && !changed_files.contains(path) {
                    changed_files.push(path.clone());
                    if let Some(defs) = self.file_cache.get_definitions(path) {
                        invalidated_defs.extend(defs.iter().copied());
                    }
                }
            }
        }

        IncrementalAnalysis {
            changed_files,
            deleted_files,
            unchanged_files,
            invalidated_defs,
            invalidated_modules: all_invalidated_modules,
            invalidated_hashes: Vec::new(), // Will be populated during compilation
            is_full_rebuild: false,
        }
    }

    /// Propagate invalidation through the build cache.
    ///
    /// This takes content hashes that have been invalidated and cascades
    /// the invalidation through dependencies.
    pub fn propagate_invalidation(
        &self,
        build_cache: &mut BuildCache,
        changed_hashes: &[ContentHash],
    ) -> Vec<ContentHash> {
        let mut all_invalidated = Vec::new();

        for hash in changed_hashes {
            if let Ok(invalidated) = build_cache.invalidate(hash) {
                all_invalidated.extend(invalidated);
            }
        }

        all_invalidated
    }

    /// Update the file cache after successful compilation.
    pub fn update_file(&mut self, path: &Path) -> Result<(), FileCacheError> {
        self.file_cache.update_file(path)
    }

    /// Register definitions produced by a file.
    pub fn register_definitions(&mut self, path: &Path, defs: HashSet<DefId>) {
        self.file_cache.register_definitions(path, defs);
    }

    /// Register a module for a file.
    pub fn register_module(&mut self, path: &Path, module_id: ModuleId) {
        self.file_cache.register_module(path, module_id);
    }

    /// Add a module dependency.
    pub fn add_dependency(&mut self, from: ModuleId, to: ModuleId) {
        self.dep_graph.add_dependency(from, to);
    }

    /// Clear all caches (force full rebuild next time).
    pub fn clear_caches(&mut self) {
        self.file_cache.clear();
        self.dep_graph = DependencyGraph::new();
    }

    /// Get the compilation order for modules.
    ///
    /// Returns modules in topological order (dependencies before dependents).
    pub fn compilation_order(&self) -> Result<Vec<ModuleId>, super::graph::GraphError> {
        self.dep_graph.topological_sort()
    }

    /// Check if incremental compilation is possible.
    pub fn can_compile_incrementally(&self) -> bool {
        self.incremental_enabled && !self.file_cache.is_empty()
    }
}

/// Builder for `ProjectCompiler`.
pub struct ProjectCompilerBuilder {
    project_root: PathBuf,
    incremental: bool,
}

impl ProjectCompilerBuilder {
    /// Create a new builder.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            incremental: true,
        }
    }

    /// Enable or disable incremental compilation.
    pub fn incremental(mut self, enabled: bool) -> Self {
        self.incremental = enabled;
        self
    }

    /// Build the project compiler.
    pub fn build(self) -> ProjectCompiler {
        if self.incremental {
            ProjectCompiler::new(self.project_root)
        } else {
            ProjectCompiler::without_incremental(self.project_root)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_project_compiler_init() {
        let temp_dir = TempDir::new().unwrap();
        let mut compiler = ProjectCompiler::new(temp_dir.path().to_path_buf());

        let loaded = compiler.init().unwrap();
        assert!(!loaded); // No previous cache

        // Should be able to save
        compiler.save().unwrap();
    }

    #[test]
    fn test_project_compiler_analyze_no_cache() {
        let temp_dir = TempDir::new().unwrap();
        let compiler = ProjectCompiler::new(temp_dir.path().to_path_buf());

        let files = vec![
            temp_dir.path().join("a.blood"),
            temp_dir.path().join("b.blood"),
        ];

        let analysis = compiler.analyze(&files);

        // No cache means full rebuild
        assert!(analysis.is_full_rebuild);
        assert_eq!(analysis.changed_files.len(), 2);
    }

    #[test]
    fn test_project_compiler_analyze_with_cache() {
        let temp_dir = TempDir::new().unwrap();

        let file1 = temp_dir.path().join("unchanged.blood");
        let file2 = temp_dir.path().join("modified.blood");
        let file3 = temp_dir.path().join("new.blood");

        fs::write(&file1, "fn f1() {}").unwrap();
        fs::write(&file2, "fn f2() {}").unwrap();

        // First run: cache files
        {
            let mut compiler = ProjectCompiler::new(temp_dir.path().to_path_buf());
            compiler.init().unwrap();

            compiler.update_file(&file1).unwrap();
            compiler.update_file(&file2).unwrap();

            // Register definitions
            let mut defs1 = HashSet::new();
            defs1.insert(DefId::new(0));
            compiler.register_definitions(&file1, defs1);

            let mut defs2 = HashSet::new();
            defs2.insert(DefId::new(1));
            compiler.register_definitions(&file2, defs2);

            compiler.save().unwrap();
        }

        // Modify file2 and create file3
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file2, "fn f2_modified() {}").unwrap();
        fs::write(&file3, "fn f3() {}").unwrap();

        // Second run: analyze changes
        {
            let mut compiler = ProjectCompiler::new(temp_dir.path().to_path_buf());
            compiler.init().unwrap();

            let analysis = compiler.analyze(&[file1.clone(), file2.clone(), file3.clone()]);

            assert!(!analysis.is_full_rebuild);
            assert_eq!(analysis.unchanged_files.len(), 1);
            assert!(analysis.unchanged_files.contains(&file1));
            assert!(analysis.changed_files.contains(&file2));
            assert!(analysis.changed_files.contains(&file3));

            // Definition from file2 should be invalidated
            assert!(analysis.invalidated_defs.contains(&DefId::new(1)));
        }
    }

    #[test]
    fn test_project_compiler_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let compiler = ProjectCompiler::without_incremental(temp_dir.path().to_path_buf());

        let files = vec![temp_dir.path().join("a.blood")];
        let analysis = compiler.analyze(&files);

        // Should always be full rebuild
        assert!(analysis.is_full_rebuild);
    }

    #[test]
    fn test_project_compiler_dependency_propagation() {
        let temp_dir = TempDir::new().unwrap();

        let file_a = temp_dir.path().join("a.blood");
        let file_b = temp_dir.path().join("b.blood");
        let file_c = temp_dir.path().join("c.blood");

        fs::write(&file_a, "fn a() {}").unwrap();
        fs::write(&file_b, "fn b() { a() }").unwrap();
        fs::write(&file_c, "fn c() { b() }").unwrap();

        // First run: cache files and set up dependencies
        {
            let mut compiler = ProjectCompiler::new(temp_dir.path().to_path_buf());
            compiler.init().unwrap();

            compiler.update_file(&file_a).unwrap();
            compiler.update_file(&file_b).unwrap();
            compiler.update_file(&file_c).unwrap();

            // Register modules
            let mod_a = ModuleId::new(0);
            let mod_b = ModuleId::new(1);
            let mod_c = ModuleId::new(2);

            compiler.register_module(&file_a, mod_a);
            compiler.register_module(&file_b, mod_b);
            compiler.register_module(&file_c, mod_c);

            // b depends on a, c depends on b
            compiler.add_dependency(mod_b, mod_a);
            compiler.add_dependency(mod_c, mod_b);

            compiler.save().unwrap();
        }

        // Modify file_a
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file_a, "fn a_modified() {}").unwrap();

        // Second run: analyze changes
        {
            let mut compiler = ProjectCompiler::new(temp_dir.path().to_path_buf());
            compiler.init().unwrap();

            let analysis = compiler.analyze(&[file_a.clone(), file_b.clone(), file_c.clone()]);

            // All three modules should be invalidated due to transitive dependencies
            assert!(analysis.invalidated_modules.contains(&ModuleId::new(0)));
            assert!(analysis.invalidated_modules.contains(&ModuleId::new(1)));
            assert!(analysis.invalidated_modules.contains(&ModuleId::new(2)));
        }
    }

    #[test]
    fn test_project_compiler_compilation_order() {
        let temp_dir = TempDir::new().unwrap();
        let mut compiler = ProjectCompiler::new(temp_dir.path().to_path_buf());

        let mod_a = ModuleId::new(0);
        let mod_b = ModuleId::new(1);
        let mod_c = ModuleId::new(2);

        // c depends on b, b depends on a
        compiler.dep_graph_mut().add_module(mod_a);
        compiler.add_dependency(mod_b, mod_a);
        compiler.add_dependency(mod_c, mod_b);

        let order = compiler.compilation_order().unwrap();

        // a should come before b, b should come before c
        let pos_a = order.iter().position(|&m| m == mod_a).unwrap();
        let pos_b = order.iter().position(|&m| m == mod_b).unwrap();
        let pos_c = order.iter().position(|&m| m == mod_c).unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_project_compiler_builder() {
        let temp_dir = TempDir::new().unwrap();

        let compiler = ProjectCompilerBuilder::new(temp_dir.path().to_path_buf())
            .incremental(true)
            .build();

        assert!(compiler.can_compile_incrementally() || compiler.file_cache.is_empty());

        let compiler = ProjectCompilerBuilder::new(temp_dir.path().to_path_buf())
            .incremental(false)
            .build();

        assert!(!compiler.can_compile_incrementally());
    }
}
