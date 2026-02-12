//! Module file resolution for Blood projects.
//!
//! This module handles finding and loading module files based on `mod` declarations.
//! For a declaration `mod foo;` in `src/main.blood`, it searches:
//! 1. `src/foo.blood`
//! 2. `src/foo/mod.blood`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use serde::{Deserialize, Serialize};

/// Unique identifier for a module in the module tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModuleId(u32);

impl ModuleId {
    /// Create a new module ID.
    pub fn new(id: u32) -> Self {
        ModuleId(id)
    }

    /// Get the raw ID value.
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// A module in the module tree.
#[derive(Debug, Clone)]
pub struct Module {
    /// The module's unique identifier.
    pub id: ModuleId,

    /// The module's simple name (e.g., "foo" for `mod foo;`).
    pub name: String,

    /// The full path from the crate root (e.g., "crate::foo::bar").
    pub path: ModulePath,

    /// The file path on disk.
    pub file_path: PathBuf,

    /// Parent module (None for the root).
    pub parent: Option<ModuleId>,

    /// Child modules declared in this module.
    pub children: Vec<ModuleId>,

    /// Whether this is an inline module (mod foo { ... }) vs external (mod foo;).
    pub is_inline: bool,

    /// Visibility of this module.
    pub visibility: Visibility,
}

/// Module path (e.g., "crate::foo::bar").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModulePath {
    /// Path segments.
    pub segments: Vec<String>,
}

impl ModulePath {
    /// Create a new module path from segments.
    pub fn new(segments: Vec<String>) -> Self {
        ModulePath { segments }
    }

    /// Create the crate root path.
    pub fn crate_root() -> Self {
        ModulePath {
            segments: vec!["crate".to_string()],
        }
    }

    /// Append a segment to the path.
    pub fn push(&mut self, segment: String) {
        self.segments.push(segment);
    }

    /// Create a child path.
    pub fn child(&self, name: &str) -> Self {
        let mut segments = self.segments.clone();
        segments.push(name.to_string());
        ModulePath { segments }
    }

    /// Get the parent path.
    pub fn parent(&self) -> Option<Self> {
        if self.segments.len() <= 1 {
            None
        } else {
            let mut segments = self.segments.clone();
            segments.pop();
            Some(ModulePath { segments })
        }
    }

    /// Check if this path starts with "crate".
    pub fn is_absolute(&self) -> bool {
        self.segments.first().is_some_and(|s| s == "crate")
    }

}

impl std::fmt::Display for ModulePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.segments.join("::"))
    }
}

/// Module visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Private to the parent module.
    #[default]
    Private,
    /// Public to all.
    Public,
    /// Public within the crate.
    PubCrate,
    /// Public to the parent module.
    PubSuper,
}

/// Errors that can occur during module resolution.
#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("module not found: `{name}` (searched: {searched:?})")]
    ModuleNotFound {
        name: String,
        searched: Vec<PathBuf>,
    },

    #[error("duplicate module: `{name}` already exists")]
    DuplicateModule { name: String },

    #[error("module path error: {message}")]
    PathError { message: String },

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

/// The module tree for a Blood project.
#[derive(Debug)]
pub struct ModuleTree {
    /// Root module ID.
    root: ModuleId,

    /// All modules by ID.
    modules: HashMap<ModuleId, Module>,

    /// Module lookup by path.
    by_path: HashMap<ModulePath, ModuleId>,

    /// Module lookup by file path.
    by_file: HashMap<PathBuf, ModuleId>,

    /// Next available module ID.
    next_id: u32,
}

impl ModuleTree {
    /// Create a new module tree with a root module.
    pub fn new(root_path: PathBuf, crate_name: &str) -> Self {
        let root_id = ModuleId::new(0);
        let root_module_path = ModulePath::crate_root();

        let root = Module {
            id: root_id,
            name: crate_name.to_string(),
            path: root_module_path.clone(),
            file_path: root_path.clone(),
            parent: None,
            children: Vec::new(),
            is_inline: false,
            visibility: Visibility::Public,
        };

        let mut modules = HashMap::new();
        let mut by_path = HashMap::new();
        let mut by_file = HashMap::new();

        modules.insert(root_id, root);
        by_path.insert(root_module_path, root_id);
        by_file.insert(root_path, root_id);

        ModuleTree {
            root: root_id,
            modules,
            by_path,
            by_file,
            next_id: 1,
        }
    }

    /// Get the root module ID.
    pub fn root(&self) -> ModuleId {
        self.root
    }

    /// Get a module by ID.
    pub fn get(&self, id: ModuleId) -> Option<&Module> {
        self.modules.get(&id)
    }

    /// Get a mutable reference to a module by ID.
    pub fn get_mut(&mut self, id: ModuleId) -> Option<&mut Module> {
        self.modules.get_mut(&id)
    }

    /// Look up a module by its path.
    pub fn by_path(&self, path: &ModulePath) -> Option<ModuleId> {
        self.by_path.get(path).copied()
    }

    /// Look up a module by its file path.
    pub fn by_file(&self, path: &Path) -> Option<ModuleId> {
        self.by_file.get(path).copied()
    }

    /// Iterate over all modules.
    pub fn iter(&self) -> impl Iterator<Item = &Module> {
        self.modules.values()
    }

    /// Add a child module to a parent.
    pub fn add_child(
        &mut self,
        parent_id: ModuleId,
        name: String,
        file_path: PathBuf,
        is_inline: bool,
        visibility: Visibility,
    ) -> Result<ModuleId, ResolveError> {
        let parent = self.modules.get(&parent_id).ok_or_else(|| {
            ResolveError::PathError {
                message: format!("parent module not found: {:?}", parent_id),
            }
        })?;

        let child_path = parent.path.child(&name);

        // Check for duplicates
        if self.by_path.contains_key(&child_path) {
            return Err(ResolveError::DuplicateModule { name });
        }

        let child_id = ModuleId::new(self.next_id);
        self.next_id += 1;

        let child = Module {
            id: child_id,
            name,
            path: child_path.clone(),
            file_path: file_path.clone(),
            parent: Some(parent_id),
            children: Vec::new(),
            is_inline,
            visibility,
        };

        // Add to parent's children
        if let Some(parent) = self.modules.get_mut(&parent_id) {
            parent.children.push(child_id);
        }

        self.by_path.insert(child_path, child_id);
        self.by_file.insert(file_path, child_id);
        self.modules.insert(child_id, child);

        Ok(child_id)
    }

    /// Get all modules in topological order (parents before children).
    pub fn topological_order(&self) -> Vec<ModuleId> {
        let mut result = Vec::with_capacity(self.modules.len());
        let mut visited = std::collections::HashSet::new();

        fn visit(
            tree: &ModuleTree,
            id: ModuleId,
            visited: &mut std::collections::HashSet<ModuleId>,
            result: &mut Vec<ModuleId>,
        ) {
            if visited.contains(&id) {
                return;
            }
            visited.insert(id);

            // Visit parent first if it exists
            if let Some(module) = tree.modules.get(&id) {
                if let Some(parent) = module.parent {
                    visit(tree, parent, visited, result);
                }
            }

            result.push(id);
        }

        for &id in self.modules.keys() {
            visit(self, id, &mut visited, &mut result);
        }

        result
    }
}

/// Module resolver for finding module files.
#[derive(Debug)]
pub struct ModuleResolver {
    /// Project root directory.
    project_root: PathBuf,

    /// Source root directory (usually project_root/src).
    source_root: PathBuf,
}

impl ModuleResolver {
    /// Create a new module resolver.
    pub fn new(project_root: PathBuf) -> Self {
        let source_root = project_root.join("src");
        ModuleResolver {
            project_root,
            source_root,
        }
    }

    /// Create a resolver with a custom source root.
    pub fn with_source_root(project_root: PathBuf, source_root: PathBuf) -> Self {
        ModuleResolver {
            project_root,
            source_root,
        }
    }

    /// Get the project root.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Get the source root.
    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    /// Resolve a module declaration to a file path.
    ///
    /// For `mod foo;` in `src/main.blood`, searches:
    /// 1. `src/foo.blood`
    /// 2. `src/foo/mod.blood`
    pub fn resolve_module(
        &self,
        parent_file: &Path,
        module_name: &str,
    ) -> Result<PathBuf, ResolveError> {
        let parent_dir = parent_file.parent().unwrap_or(Path::new("."));

        // Search paths
        let file_path = parent_dir.join(format!("{}.blood", module_name));
        let dir_path = parent_dir.join(module_name).join("mod.blood");

        let searched = vec![file_path.clone(), dir_path.clone()];

        if file_path.exists() {
            Ok(file_path)
        } else if dir_path.exists() {
            Ok(dir_path)
        } else {
            Err(ResolveError::ModuleNotFound {
                name: module_name.to_string(),
                searched,
            })
        }
    }

    /// Resolve an absolute module path to a file.
    ///
    /// For `crate::foo::bar`, searches:
    /// 1. `src/foo/bar.blood`
    /// 2. `src/foo/bar/mod.blood`
    pub fn resolve_absolute_path(&self, path: &ModulePath) -> Result<PathBuf, ResolveError> {
        if path.segments.is_empty() {
            return Err(ResolveError::PathError {
                message: "empty module path".to_string(),
            });
        }

        // Skip "crate" prefix
        let segments: &[String] = if path.segments[0] == "crate" {
            &path.segments[1..]
        } else {
            &path.segments
        };

        if segments.is_empty() {
            // This is the crate root
            let lib_path = self.source_root.join("lib.blood");
            let main_path = self.source_root.join("main.blood");

            if lib_path.exists() {
                return Ok(lib_path);
            } else if main_path.exists() {
                return Ok(main_path);
            } else {
                return Err(ResolveError::ModuleNotFound {
                    name: "crate root".to_string(),
                    searched: vec![lib_path, main_path],
                });
            }
        }

        // Build the path
        let mut dir_path = self.source_root.clone();
        for segment in &segments[..segments.len().saturating_sub(1)] {
            dir_path = dir_path.join(segment);
        }

        let last = &segments[segments.len() - 1];
        let file_path = dir_path.join(format!("{}.blood", last));
        let mod_path = dir_path.join(last).join("mod.blood");

        let searched = vec![file_path.clone(), mod_path.clone()];

        if file_path.exists() {
            Ok(file_path)
        } else if mod_path.exists() {
            Ok(mod_path)
        } else {
            Err(ResolveError::ModuleNotFound {
                name: path.to_string(),
                searched,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_module_path_operations() {
        let path = ModulePath::crate_root();
        assert_eq!(path.to_string(), "crate");
        assert!(path.is_absolute());

        let child = path.child("foo");
        assert_eq!(child.to_string(), "crate::foo");

        let grandchild = child.child("bar");
        assert_eq!(grandchild.to_string(), "crate::foo::bar");

        let parent = grandchild.parent().unwrap();
        assert_eq!(parent.to_string(), "crate::foo");
    }

    #[test]
    fn test_module_tree_creation() {
        let tree = ModuleTree::new(PathBuf::from("/project/src/lib.blood"), "mylib");

        assert_eq!(tree.modules.len(), 1);
        let root = tree.get(tree.root()).unwrap();
        assert_eq!(root.name, "mylib");
        assert_eq!(root.path.to_string(), "crate");
    }

    #[test]
    fn test_add_child_module() {
        let mut tree = ModuleTree::new(PathBuf::from("/project/src/lib.blood"), "mylib");

        let child_id = tree.add_child(
            tree.root(),
            "utils".to_string(),
            PathBuf::from("/project/src/utils.blood"),
            false,
            Visibility::Public,
        ).unwrap();

        let child = tree.get(child_id).unwrap();
        assert_eq!(child.name, "utils");
        assert_eq!(child.path.to_string(), "crate::utils");

        // Root should have the child
        let root = tree.get(tree.root()).unwrap();
        assert!(root.children.contains(&child_id));
    }

    #[test]
    fn test_duplicate_module_error() {
        let mut tree = ModuleTree::new(PathBuf::from("/project/src/lib.blood"), "mylib");

        tree.add_child(
            tree.root(),
            "foo".to_string(),
            PathBuf::from("/project/src/foo.blood"),
            false,
            Visibility::Public,
        ).unwrap();

        let result = tree.add_child(
            tree.root(),
            "foo".to_string(),
            PathBuf::from("/project/src/foo2.blood"),
            false,
            Visibility::Public,
        );

        assert!(matches!(result, Err(ResolveError::DuplicateModule { .. })));
    }

    #[test]
    fn test_resolve_module_file() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        fs::create_dir(&src).unwrap();

        // Create src/foo.blood
        fs::write(src.join("foo.blood"), "// foo").unwrap();

        // Create src/bar/mod.blood
        let bar_dir = src.join("bar");
        fs::create_dir(&bar_dir).unwrap();
        fs::write(bar_dir.join("mod.blood"), "// bar").unwrap();

        let resolver = ModuleResolver::new(temp.path().to_path_buf());

        // Resolve from src/lib.blood
        let lib_path = src.join("lib.blood");

        // foo.blood should be found
        let foo_path = resolver.resolve_module(&lib_path, "foo").unwrap();
        assert_eq!(foo_path, src.join("foo.blood"));

        // bar/mod.blood should be found
        let bar_path = resolver.resolve_module(&lib_path, "bar").unwrap();
        assert_eq!(bar_path, bar_dir.join("mod.blood"));

        // missing should error
        let missing = resolver.resolve_module(&lib_path, "missing");
        assert!(matches!(missing, Err(ResolveError::ModuleNotFound { .. })));
    }

    #[test]
    fn test_topological_order() {
        let mut tree = ModuleTree::new(PathBuf::from("/project/src/lib.blood"), "mylib");

        let utils_id = tree.add_child(
            tree.root(),
            "utils".to_string(),
            PathBuf::from("/project/src/utils.blood"),
            false,
            Visibility::Public,
        ).unwrap();

        let helpers_id = tree.add_child(
            utils_id,
            "helpers".to_string(),
            PathBuf::from("/project/src/utils/helpers.blood"),
            false,
            Visibility::Public,
        ).unwrap();

        let order = tree.topological_order();

        // Root should come before utils, utils before helpers
        let root_pos = order.iter().position(|&id| id == tree.root()).unwrap();
        let utils_pos = order.iter().position(|&id| id == utils_id).unwrap();
        let helpers_pos = order.iter().position(|&id| id == helpers_id).unwrap();

        assert!(root_pos < utils_pos);
        assert!(utils_pos < helpers_pos);
    }
}
