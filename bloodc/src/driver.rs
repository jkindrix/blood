//! Multi-file compilation driver for Blood projects.
//!
//! This module orchestrates compilation of multi-file Blood projects by:
//! 1. Discovering all modules from the crate root
//! 2. Parsing each file into its own AST
//! 3. Building a unified type context across all modules
//! 4. Resolving imports across module boundaries
//! 5. Type checking in dependency order
//!
//! # Example
//!
//! ```ignore
//! use bloodc::driver::CompilationDriver;
//! use std::path::Path;
//!
//! let driver = CompilationDriver::new(Path::new("my_project"));
//! let result = driver.compile_project(Path::new("src/main.blood"));
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use string_interner::DefaultStringInterner;
use thiserror::Error;

use crate::ast;
use crate::diagnostics::Diagnostic;
use crate::hir;
use crate::parser::Parser;
use crate::project::{ModuleId, ModuleResolver, ModuleTree, DependencyGraph, Visibility};
use crate::stdlib_loader::StdlibLoader;
use crate::typeck::TypeContext;

/// Errors that can occur during compilation.
#[derive(Debug, Error)]
pub enum DriverError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error in {file}: {message}")]
    Parse { file: PathBuf, message: String },

    #[error("type error: {message}")]
    Type { message: String },

    #[error("module resolution error: {message}")]
    ModuleResolution { message: String },

    #[error("circular dependency: {cycle}")]
    CircularDependency { cycle: String },

    #[error("stdlib error: {message}")]
    Stdlib { message: String },
}

/// Information about a parsed module.
#[derive(Debug)]
pub struct ParsedModule {
    /// The module ID in the module tree.
    pub module_id: ModuleId,
    /// Path to the source file.
    pub file_path: PathBuf,
    /// The parsed AST.
    pub ast: ast::Program,
    /// The source code.
    pub source: String,
}

/// Multi-file compilation driver.
///
/// Orchestrates the compilation of a Blood project from source files
/// to a unified HIR crate.
#[derive(Debug)]
pub struct CompilationDriver {
    /// Project root directory.
    project_root: PathBuf,
    /// Standard library path (if provided).
    stdlib_path: Option<PathBuf>,
    /// Module tree for this crate.
    module_tree: ModuleTree,
    /// Module resolver for finding files.
    resolver: ModuleResolver,
    /// Parsed modules, indexed by ModuleId.
    parsed_modules: HashMap<ModuleId, ParsedModule>,
    /// Shared string interner for all modules.
    interner: DefaultStringInterner,
    /// Dependency graph for compilation order.
    dep_graph: DependencyGraph,
    /// Whether to include the standard library prelude.
    include_prelude: bool,
}

impl CompilationDriver {
    /// Create a new compilation driver for the given project root.
    pub fn new(project_root: &Path) -> Self {
        let project_root = project_root.to_path_buf();
        let resolver = ModuleResolver::new(project_root.clone());

        Self {
            project_root,
            stdlib_path: None,
            module_tree: ModuleTree::new(PathBuf::new(), "main"),
            resolver,
            parsed_modules: HashMap::new(),
            interner: DefaultStringInterner::new(),
            dep_graph: DependencyGraph::new(),
            include_prelude: true,
        }
    }

    /// Set the standard library path.
    pub fn with_stdlib(mut self, path: PathBuf) -> Self {
        self.stdlib_path = Some(path);
        self
    }

    /// Disable automatic prelude import.
    pub fn without_prelude(mut self) -> Self {
        self.include_prelude = false;
        self
    }

    /// Get the project root.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Get the module tree.
    pub fn module_tree(&self) -> &ModuleTree {
        &self.module_tree
    }

    /// Get all parsed modules.
    pub fn parsed_modules(&self) -> &HashMap<ModuleId, ParsedModule> {
        &self.parsed_modules
    }

    /// Compile a project starting from an entry file.
    ///
    /// This is the main entry point for multi-file compilation:
    /// 1. Discovers all modules from the entry file
    /// 2. Parses all discovered modules
    /// 3. Builds dependency graph from imports
    /// 4. Type checks in topological order
    /// 5. Returns unified HIR crate
    pub fn compile_project(mut self, entry_file: &Path) -> Result<CompilationResult, Vec<DriverError>> {
        let mut errors = Vec::new();

        // Phase 1: Discover modules
        if let Err(e) = self.discover_modules(entry_file) {
            errors.push(e);
            return Err(errors);
        }

        // Phase 2: Parse all modules
        if let Err(parse_errors) = self.parse_all_modules() {
            errors.extend(parse_errors);
            return Err(errors);
        }

        // Phase 3: Build dependency graph
        if let Err(e) = self.build_dependency_graph() {
            errors.push(e);
            return Err(errors);
        }

        // Phase 4: Get compilation order
        let order = match self.dep_graph.topological_sort() {
            Ok(order) => order,
            Err(e) => {
                errors.push(DriverError::CircularDependency {
                    cycle: e.to_string(),
                });
                return Err(errors);
            }
        };

        // Phase 5: Type check in order
        let type_result = self.type_check_modules(&order);

        match type_result {
            Ok(result) => Ok(result),
            Err(type_errors) => {
                errors.extend(type_errors);
                Err(errors)
            }
        }
    }

    /// Discover all modules starting from the entry file.
    ///
    /// This walks the module tree by:
    /// 1. Parsing the entry file to find `mod` declarations
    /// 2. Resolving each `mod foo;` to a file path
    /// 3. Recursively processing each discovered module
    pub fn discover_modules(&mut self, entry_file: &Path) -> Result<(), DriverError> {
        // Normalize the entry file path
        let entry_file = if entry_file.is_absolute() {
            entry_file.to_path_buf()
        } else {
            self.project_root.join(entry_file)
        };

        // Determine crate name from entry file
        let crate_name = entry_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main")
            .to_string();

        // Initialize the module tree with the root module
        self.module_tree = ModuleTree::new(entry_file.clone(), &crate_name);

        // Queue of modules to process: (file_path, parent_module_id)
        let mut to_process: Vec<(PathBuf, ModuleId)> = vec![(entry_file, self.module_tree.root())];

        while let Some((file_path, parent_id)) = to_process.pop() {
            // Read and parse the file to find mod declarations
            let source = fs::read_to_string(&file_path)?;
            let mut parser = Parser::new(&source);

            let program = match parser.parse_program() {
                Ok(p) => p,
                Err(_errors) => {
                    // We'll report parse errors later; for now, skip this module
                    continue;
                }
            };

            // Take interner to resolve symbols
            let interner = parser.take_interner();

            // Look for mod declarations
            for decl in &program.declarations {
                if let ast::Declaration::Module(mod_decl) = decl {
                    // Get the module name from the interner
                    let mod_name = interner.resolve(mod_decl.name.node)
                        .unwrap_or("unknown")
                        .to_string();

                    // If it's an external module (mod foo;), resolve to a file
                    if mod_decl.body.is_none() {
                        let resolved_path = self.resolver.resolve_module(&file_path, &mod_name)
                            .map_err(|e| DriverError::ModuleResolution {
                                message: e.to_string(),
                            })?;

                        // Add to module tree
                        let child_id = self.module_tree.add_child(
                            parent_id,
                            mod_name,
                            resolved_path.clone(),
                            false, // not inline
                            visibility_to_project_visibility(&mod_decl.vis),
                        ).map_err(|e| DriverError::ModuleResolution {
                            message: e.to_string(),
                        })?;

                        // Queue for processing
                        to_process.push((resolved_path, child_id));
                    }
                    // Inline modules (mod foo { ... }) are handled during type checking
                }
            }
        }

        Ok(())
    }

    /// Parse all discovered modules.
    pub fn parse_all_modules(&mut self) -> Result<(), Vec<DriverError>> {
        let mut errors = Vec::new();

        // Get all modules in topological order (parents before children)
        let modules: Vec<_> = self.module_tree.topological_order();

        for module_id in modules {
            let module = match self.module_tree.get(module_id) {
                Some(m) => m,
                None => continue,
            };

            // Read the source file
            let source = match fs::read_to_string(&module.file_path) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(DriverError::Io(e));
                    continue;
                }
            };

            // Parse the file
            let mut parser = Parser::new(&source);
            let ast = match parser.parse_program() {
                Ok(program) => program,
                Err(parse_errors) => {
                    for err in parse_errors {
                        errors.push(DriverError::Parse {
                            file: module.file_path.clone(),
                            message: err.message,
                        });
                    }
                    continue;
                }
            };

            // Merge the interner from this parser
            // Note: We need to merge interners to share symbols across modules
            // For now, we store the source and re-parse during type checking
            // TODO: Implement interner merging for efficiency

            // Store the parsed module
            self.parsed_modules.insert(module_id, ParsedModule {
                module_id,
                file_path: module.file_path.clone(),
                ast,
                source,
            });
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Build dependency graph from import statements.
    fn build_dependency_graph(&mut self) -> Result<(), DriverError> {
        // Add all modules to the graph
        for &module_id in self.parsed_modules.keys() {
            self.dep_graph.add_module(module_id);
        }

        // TODO: Analyze use statements to determine dependencies
        // For now, we use the module tree structure (parents before children)
        // This is sufficient for initial implementation

        for module_id in self.parsed_modules.keys() {
            if let Some(module) = self.module_tree.get(*module_id) {
                // Child modules depend on their parent
                if let Some(parent_id) = module.parent {
                    self.dep_graph.add_dependency(*module_id, parent_id);
                }
            }
        }

        // Check for cycles
        if let Some(cycle) = self.dep_graph.find_cycle() {
            let cycle_str = cycle
                .iter()
                .filter_map(|id| self.module_tree.get(*id))
                .map(|m| m.name.clone())
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(DriverError::CircularDependency { cycle: cycle_str });
        }

        Ok(())
    }

    /// Type check all modules in the given order.
    fn type_check_modules(mut self, order: &[ModuleId]) -> Result<CompilationResult, Vec<DriverError>> {
        let mut errors = Vec::new();
        let mut diagnostics = Vec::new();

        // Get the root module for the type context
        let root_id = self.module_tree.root();
        let root_module = match self.parsed_modules.get(&root_id) {
            Some(m) => m,
            None => {
                errors.push(DriverError::ModuleResolution {
                    message: "root module not found".to_string(),
                });
                return Err(errors);
            }
        };

        // Create type context with the root module's source as the primary source
        // Take the interner out of self to satisfy the borrow checker
        let interner = std::mem::take(&mut self.interner);
        let root_source = root_module.source.clone();
        let root_path = root_module.file_path.clone();
        let mut ctx = TypeContext::new(&root_source, interner)
            .with_source_path(&root_path);

        // Load and register standard library modules if a stdlib path is provided
        if let Some(ref stdlib_path) = self.stdlib_path {
            let mut stdlib_loader = StdlibLoader::new(stdlib_path.clone());

            // Discover all stdlib modules
            if let Err(e) = stdlib_loader.discover() {
                errors.push(DriverError::Stdlib {
                    message: e.to_string(),
                });
                return Err(errors);
            }

            // Parse all discovered modules
            if let Err(parse_errors) = stdlib_loader.parse_all() {
                for e in parse_errors {
                    errors.push(DriverError::Stdlib {
                        message: e.to_string(),
                    });
                }
                return Err(errors);
            }

            // Register modules in the type context
            if let Err(register_errors) = stdlib_loader.register_in_context(&mut ctx) {
                for e in register_errors {
                    errors.push(DriverError::Stdlib {
                        message: e.to_string(),
                    });
                }
                return Err(errors);
            }
        }

        // Phase 1: Resolve declarations from all modules in topological order
        // This ensures parent modules are processed before children
        for &module_id in order {
            if let Some(parsed) = self.parsed_modules.get(&module_id) {
                // Get module info for this module
                let module_info = self.module_tree.get(module_id);
                let is_root = module_id == root_id;

                if is_root {
                    // Root module uses the main resolve_program
                    if let Err(type_errors) = ctx.resolve_program(&parsed.ast) {
                        diagnostics.extend(type_errors);
                    }
                } else {
                    // Non-root modules are registered as external modules
                    // and their declarations are collected
                    // Clone data needed to avoid borrow checker issues
                    let source_clone = parsed.source.clone();
                    let module_name = module_info.map(|i| i.name.clone()).unwrap_or_else(|| "mod".to_string());
                    let module_path = if let Some(info) = module_info {
                        build_module_path(&self.module_tree, module_id, info)
                    } else {
                        "unknown".to_string()
                    };
                    register_submodule(&mut ctx, &source_clone, &module_name, &module_path);
                }
            }
        }

        // Phase 2: Expand derives for all modules
        ctx.expand_derives();

        // Phase 3: Type check all function bodies
        if let Err(type_errors) = ctx.check_all_bodies() {
            diagnostics.extend(type_errors);
        }

        if !diagnostics.is_empty() {
            // Convert diagnostics to driver errors for now
            // TODO: Return diagnostics separately for better error reporting
            for diag in &diagnostics {
                errors.push(DriverError::Type {
                    message: diag.message.clone(),
                });
            }
            return Err(errors);
        }

        // Generate HIR
        let hir_crate = ctx.into_hir();

        Ok(CompilationResult {
            hir_crate,
            diagnostics,
            module_count: self.parsed_modules.len(),
        })
    }
}

/// Register a submodule in the type context.
///
/// This processes a non-root module and registers its declarations
/// so they can be imported from other modules.
fn register_submodule(
    ctx: &mut TypeContext<'_>,
    source: &str,
    module_name: &str,
    module_path: &str,
) {
    use crate::hir::DefKind;
    use crate::span::Span;

    // Create a DefId for this module
    let module_def_id = match ctx.resolver.define_item(
        module_name.to_string(),
        DefKind::Mod,
        Span::dummy(),
    ) {
        Ok(id) => id,
        Err(_) => return, // Module might already be defined
    };

    // Collect declarations from the module's AST
    let mut item_def_ids = Vec::new();

    // Re-parse to get a fresh interner for this module
    let mut parser = Parser::new(source);
    if let Ok(ast) = parser.parse_program() {
        let interner = parser.take_interner();

        for decl in &ast.declarations {
            if let Some(def_id) = create_item_def_id(ctx, &interner, decl) {
                item_def_ids.push(def_id);
            }
        }
    }

    // Register the module with its items
    ctx.register_external_module(
        module_path.to_string(),
        module_def_id,
        item_def_ids,
        Span::dummy(),
    );
}

/// Build the full module path for a module.
fn build_module_path(module_tree: &ModuleTree, _module_id: ModuleId, info: &crate::project::Module) -> String {
    let mut path_parts = vec![info.name.clone()];

    // Walk up the tree to build the full path
    let mut current_id = info.parent;
    while let Some(parent_id) = current_id {
        if let Some(parent) = module_tree.get(parent_id) {
            path_parts.push(parent.name.clone());
            current_id = parent.parent;
        } else {
            break;
        }
    }

    // Reverse to get root-first order
    path_parts.reverse();

    // Skip the crate root name (e.g., "main") and prefix with "crate"
    if path_parts.len() > 1 {
        path_parts[0] = "crate".to_string();
        path_parts.join(".")
    } else {
        // Single segment - this is the crate root
        "crate".to_string()
    }
}

/// Create a DefId for a declaration.
fn create_item_def_id(
    ctx: &mut TypeContext<'_>,
    interner: &DefaultStringInterner,
    decl: &ast::Declaration,
) -> Option<hir::DefId> {
    use crate::hir::DefKind;
    use crate::span::Span;

    let resolve_symbol = |sym: ast::Symbol| -> String {
        interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
    };

    let (name, kind) = match decl {
        ast::Declaration::Function(f) => {
            let name = resolve_symbol(f.name.node);
            (name, DefKind::Fn)
        }
        ast::Declaration::Struct(s) => {
            let name = resolve_symbol(s.name.node);
            (name, DefKind::Struct)
        }
        ast::Declaration::Enum(e) => {
            let name = resolve_symbol(e.name.node);
            (name, DefKind::Enum)
        }
        ast::Declaration::Trait(t) => {
            let name = resolve_symbol(t.name.node);
            (name, DefKind::Trait)
        }
        ast::Declaration::Type(t) => {
            let name = resolve_symbol(t.name.node);
            (name, DefKind::TypeAlias)
        }
        ast::Declaration::Effect(e) => {
            let name = resolve_symbol(e.name.node);
            (name, DefKind::Effect)
        }
        ast::Declaration::Const(c) => {
            let name = resolve_symbol(c.name.node);
            (name, DefKind::Const)
        }
        ast::Declaration::Static(s) => {
            let name = resolve_symbol(s.name.node);
            (name, DefKind::Static)
        }
        // These don't create named items at the module level
        ast::Declaration::Handler(_) => return None,
        ast::Declaration::Impl(_) => return None,
        ast::Declaration::Bridge(_) => return None,
        ast::Declaration::Module(_) => return None,
        ast::Declaration::Macro(_) => return None,
        ast::Declaration::MacroInvocation(_) => return None,
        ast::Declaration::Use(_) => return None,
    };

    ctx.resolver.define_item(name, kind, Span::dummy()).ok()
}

/// Result of a successful compilation.
#[derive(Debug)]
pub struct CompilationResult {
    /// The unified HIR crate.
    pub hir_crate: hir::Crate,
    /// Any warnings or informational diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Number of modules compiled.
    pub module_count: usize,
}

/// Convert AST visibility to project visibility.
fn visibility_to_project_visibility(vis: &ast::Visibility) -> Visibility {
    match vis {
        ast::Visibility::Private => Visibility::Private,
        ast::Visibility::Public => Visibility::Public,
        ast::Visibility::PublicCrate => Visibility::PubCrate,
        ast::Visibility::PublicSuper => Visibility::PubSuper,
        ast::Visibility::PublicSelf => Visibility::Private, // pub(self) = private
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();

        // Create project structure
        let src = temp.path().join("src");
        fs::create_dir(&src).unwrap();

        // Create main.blood
        fs::write(
            src.join("main.blood"),
            r#"
mod foo;

fn main() {
    foo::greet();
}
"#,
        ).unwrap();

        // Create foo.blood
        fs::write(
            src.join("foo.blood"),
            r#"
pub fn greet() {
    println!("Hello!");
}
"#,
        ).unwrap();

        temp
    }

    #[test]
    fn test_module_discovery() {
        let project = create_test_project();
        let mut driver = CompilationDriver::new(project.path());

        let result = driver.discover_modules(&project.path().join("src/main.blood"));
        assert!(result.is_ok());

        // Should have discovered 2 modules: main and foo
        let modules: Vec<_> = driver.module_tree().iter().collect();
        assert_eq!(modules.len(), 2);

        // Check module names
        let names: Vec<_> = modules.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"foo"));
    }

    #[test]
    fn test_parse_all_modules() {
        let project = create_test_project();
        let mut driver = CompilationDriver::new(project.path());

        driver.discover_modules(&project.path().join("src/main.blood")).unwrap();
        let result = driver.parse_all_modules();
        assert!(result.is_ok());

        // Should have parsed 2 modules
        assert_eq!(driver.parsed_modules().len(), 2);
    }
}
