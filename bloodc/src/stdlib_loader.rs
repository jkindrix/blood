//! Standard library loader for the Blood compiler.
//!
//! This module handles loading and parsing the Blood standard library,
//! creating a module hierarchy that can be resolved during import resolution.
//!
//! # Module Path Mapping
//!
//! File paths under the stdlib root are mapped to module paths:
//! - `std/compiler/lexer.blood` → module `std.compiler.lexer`
//! - `std/compiler/ast/node.blood` → module `std.compiler.ast.node`
//! - `std/compiler/ast/mod.blood` → module `std.compiler.ast`
//!
//! This allows imports like `use std.compiler.lexer::Token` to work.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use rayon::prelude::*;
use string_interner::DefaultStringInterner;

use crate::ast;
use crate::hir::{DefId, DefKind, Type, TypeKind, PrimitiveTy, IntTy, UintTy, TyVarId};
use crate::parser::Parser;
use crate::span::Span;
use crate::typeck::TypeContext;
use crate::typeck::context::{StructInfo, FieldInfo, EnumInfo, VariantInfo, ImplBlockInfo, ImplMethodInfo, TypeAliasInfo};

/// Information about a loaded stdlib module.
#[derive(Debug)]
pub struct LoadedModule {
    /// The module path (e.g., "std.compiler.lexer")
    pub path: String,
    /// The file path
    pub file_path: PathBuf,
    /// The parsed AST
    pub ast: ast::Program,
    /// The source code
    pub source: String,
    /// String interner from the parser used to parse this module
    pub interner: DefaultStringInterner,
    /// DefId assigned to this module
    pub def_id: Option<DefId>,
    /// DefIds of items in this module
    pub items: Vec<DefId>,
}

/// Loader for the Blood standard library.
///
/// Discovers, parses, and registers stdlib modules so they can be
/// resolved during import resolution.
pub struct StdlibLoader {
    /// Root path of the standard library
    stdlib_root: PathBuf,
    /// Loaded modules, indexed by module path
    modules: HashMap<String, LoadedModule>,
    /// Module hierarchy: parent -> children
    children: HashMap<String, Vec<String>>,
    /// File path to exclude from loading (the file being checked)
    excluded_path: Option<PathBuf>,
}

impl StdlibLoader {
    /// Create a new stdlib loader.
    pub fn new(stdlib_root: PathBuf) -> Self {
        Self {
            stdlib_root,
            modules: HashMap::new(),
            children: HashMap::new(),
            excluded_path: None,
        }
    }

    /// Set a file path to exclude from stdlib loading.
    /// This should be the file being directly compiled/checked.
    pub fn exclude_file(&mut self, path: PathBuf) {
        self.excluded_path = Some(path.canonicalize().unwrap_or(path));
    }

    /// Discover all modules in the stdlib.
    ///
    /// This walks the stdlib directory tree and creates LoadedModule
    /// entries for each .blood file found.
    pub fn discover(&mut self) -> Result<(), StdlibError> {
        // Create the root "std" module first so imports like `use std.core.Option` can resolve
        if !self.modules.contains_key("std") {
            self.modules.insert("std".to_string(), LoadedModule {
                path: "std".to_string(),
                file_path: self.stdlib_root.clone(),
                ast: ast::Program {
                    module: None,
                    imports: Vec::new(),
                    declarations: Vec::new(),
                    span: Span::dummy(),
                },
                source: String::new(),
                interner: DefaultStringInterner::new(),
                def_id: None,
                items: Vec::new(),
            });
        }
        self.discover_directory(&self.stdlib_root.clone(), "std")
    }

    fn discover_directory(&mut self, dir: &Path, module_prefix: &str) -> Result<(), StdlibError> {
        if !dir.exists() {
            return Err(StdlibError::PathNotFound(dir.to_path_buf()));
        }

        let entries = fs::read_dir(dir)
            .map_err(|e| StdlibError::IoError(e.to_string()))?;

        for entry in entries {
            let entry = entry.map_err(|e| StdlibError::IoError(e.to_string()))?;
            let path = entry.path();

            if path.is_dir() {
                // Recurse into subdirectory
                let dir_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                // Skip hidden directories, build artifacts, and test directories
                if dir_name.starts_with('.') || dir_name.ends_with("_objs") || dir_name == "tests" {
                    continue;
                }

                // Skip non-module directories at the top level (src, bootstrap-std, etc.)
                // Only descend into the "std" directory when at the root
                if module_prefix == "std" && dir_name != "std" && dir_name != "compiler"
                    && dir_name != "collections" && dir_name != "io" && dir_name != "net"
                    && dir_name != "ops" && dir_name != "effects" && dir_name != "traits"
                    && dir_name != "prelude" && dir_name != "sync" && dir_name != "iter"
                    && dir_name != "fmt" && dir_name != "mem" && dir_name != "ptr"
                    && dir_name != "alloc" && dir_name != "core" {
                    // Check if this is a known non-module directory
                    if dir_name == "src" || dir_name == "bootstrap-std" {
                        continue;
                    }
                }

                // If the directory is named "std" and prefix is "std", recurse without doubling
                let child_prefix = if dir_name == "std" && module_prefix == "std" {
                    "std".to_string()
                } else {
                    format!("{}.{}", module_prefix, dir_name)
                };
                self.discover_directory(&path, &child_prefix)?;

                // Create a virtual module for this directory if it doesn't have a mod.blood
                // This allows intermediate path resolution (std.compiler when only
                // std.compiler.lexer has actual content)
                if !self.modules.contains_key(&child_prefix) {
                    self.modules.insert(child_prefix.clone(), LoadedModule {
                        path: child_prefix.clone(),
                        file_path: path.clone(),
                        ast: ast::Program {
                            module: None,
                            imports: Vec::new(),
                            declarations: Vec::new(),
                            span: Span::dummy(),
                        },
                        source: String::new(),
                        interner: DefaultStringInterner::new(),
                        def_id: None,
                        items: Vec::new(),
                    });
                }

                // Record parent-child relationship
                self.children
                    .entry(module_prefix.to_string())
                    .or_insert_with(Vec::new)
                    .push(child_prefix);

            } else if path.extension().map_or(false, |ext| ext == "blood") {
                // Skip the file being directly compiled/checked
                if let Some(ref excluded) = self.excluded_path {
                    if let Ok(canonical) = path.canonicalize() {
                        if canonical == *excluded {
                            continue;
                        }
                    }
                }

                // Load .blood file
                let file_name = path.file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                // Determine module path:
                // - mod.blood -> use parent module path
                // - foo.blood -> parent.foo
                let module_path = if file_name == "mod" || file_name == "lib" {
                    module_prefix.to_string()
                } else {
                    format!("{}.{}", module_prefix, file_name)
                };

                // Skip if already loaded (mod.blood takes precedence)
                if self.modules.contains_key(&module_path) {
                    continue;
                }

                // Read source
                let source = fs::read_to_string(&path)
                    .map_err(|e| StdlibError::IoError(format!("{}: {}", path.display(), e)))?;

                self.modules.insert(module_path.clone(), LoadedModule {
                    path: module_path.clone(),
                    file_path: path,
                    ast: ast::Program {
                        module: None,
                        imports: Vec::new(),
                        declarations: Vec::new(),
                        span: Span::dummy(),
                    },
                    source,
                    interner: DefaultStringInterner::new(),
                    def_id: None,
                    items: Vec::new(),
                });

                // Record as child of parent module
                if module_path != module_prefix {
                    self.children
                        .entry(module_prefix.to_string())
                        .or_insert_with(Vec::new)
                        .push(module_path);
                }
            }
        }

        Ok(())
    }

    /// Parse all discovered modules.
    ///
    /// Uses batched parallel parsing to balance speed and memory usage.
    /// Parsing 188 modules in parallel would consume excessive memory,
    /// so we process in batches of ~20 modules at a time.
    pub fn parse_all(&mut self) -> Result<(), Vec<StdlibError>> {
        let start = Instant::now();

        // Take all modules out of the HashMap for processing
        let mut modules: Vec<(String, LoadedModule)> = self.modules.drain().collect();
        let module_count = modules.len();

        // Thread-safe error collection
        let errors: Mutex<Vec<StdlibError>> = Mutex::new(Vec::new());

        // Process in batches to limit memory usage
        // Each batch is parsed in parallel, but batches are sequential
        // Small batch size to avoid OOM on large codebases
        const BATCH_SIZE: usize = 10;

        for chunk in modules.chunks_mut(BATCH_SIZE) {
            // Parse this batch in parallel
            chunk.par_iter_mut().for_each(|(_module_path, module)| {
                // Skip empty source (virtual modules for directories)
                if module.source.is_empty() {
                    return;
                }

                let mut parser = Parser::new(&module.source);
                match parser.parse_program() {
                    Ok(ast) => {
                        module.ast = ast;
                        module.interner = parser.take_interner();
                    }
                    Err(parse_errors) => {
                        // Collect errors thread-safely
                        if let Ok(mut errs) = errors.lock() {
                            for err in parse_errors {
                                errs.push(StdlibError::ParseError {
                                    file: module.file_path.clone(),
                                    message: err.message,
                                });
                            }
                        }
                        // Still keep the module (with empty AST) so we don't lose track
                        module.interner = parser.take_interner();
                    }
                }
            });
        }

        // Put all modules back
        for (path, module) in modules {
            self.modules.insert(path, module);
        }

        // Extract errors
        let errors = errors.into_inner().unwrap_or_else(|e| e.into_inner());

        let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
        if verbose {
            eprintln!("  Parsed {} modules in {:.2}s (batched parallel, batch_size={})",
                      module_count, start.elapsed().as_secs_f64(), BATCH_SIZE);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Register all modules in a TypeContext.
    ///
    /// This creates DefIds for each module and collects their declarations,
    /// allowing imports like `use std.compiler.lexer::Token` to resolve.
    pub fn register_in_context<'a>(&mut self, ctx: &mut TypeContext<'a>) -> Result<(), Vec<StdlibError>> {
        let start = Instant::now();
        let errors: Vec<StdlibError> = Vec::new();
        let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();

        // First, create DefIds for all modules
        if verbose {
            eprintln!("  Creating module DefIds...");
        }
        self.create_module_def_ids(ctx)?;
        if verbose {
            eprintln!("    DefIds created in {:.2}s", start.elapsed().as_secs_f64());
        }

        // Then, collect declarations for each module
        let decl_start = Instant::now();
        let mut decl_count = 0;
        let mut skipped_no_defid = 0;
        for (module_path, module) in &mut self.modules {
            let def_id = match module.def_id {
                Some(id) => id,
                None => {
                    if verbose {
                        eprintln!("    Skipping '{}': no DefId assigned", module_path);
                    }
                    skipped_no_defid += 1;
                    continue;
                }
            };

            // Collect declarations from this module's AST
            // We need to run the collection phase for each module
            // For now, we'll extract just the top-level item names
            // Note: Modules with no declarations (like mod.blood files that just re-export)
            // still need to be registered so child modules can be added later.

            let ast_decl_count = module.ast.declarations.len();
            let mut module_decl_count = 0;
            for decl in &module.ast.declarations {
                if let Some(item_def_id) = create_item_def_id(ctx, &module.interner, decl, verbose) {
                    module.items.push(item_def_id);
                    // Register struct/enum type info for type system integration
                    register_type_info(ctx, &module.interner, item_def_id, decl);
                    // Register function signatures for callable functions
                    register_fn_sig(ctx, &module.interner, item_def_id, decl);
                    decl_count += 1;
                    module_decl_count += 1;
                }
            }
            if verbose {
                eprintln!("    Module '{}': {} AST decls, {} items created",
                          module_path, ast_decl_count, module_decl_count);
            }

            // Register the module with its items (even if empty - child modules will be added later)
            ctx.register_external_module(
                module_path.clone(),
                def_id,
                module.items.clone(),
                Span::dummy(),
            );
        }
        if verbose {
            eprintln!("    Collected {} declarations in {:.2}s", decl_count, decl_start.elapsed().as_secs_f64());
            eprintln!("    Skipped: {} (no DefId)", skipped_no_defid);
        }

        // Build module hierarchy (parent modules contain child modules)
        if verbose {
            eprintln!("  Building module hierarchy...");
        }
        let hierarchy_start = Instant::now();
        self.register_module_hierarchy(ctx)?;
        if verbose {
            eprintln!("    Hierarchy built in {:.2}s", hierarchy_start.elapsed().as_secs_f64());
        }

        // Process pub use re-exports (after all modules are registered)
        if verbose {
            eprintln!("  Processing re-exports...");
        }
        let reexport_start = Instant::now();
        self.process_reexports(ctx)?;
        if verbose {
            eprintln!("    Re-exports processed in {:.2}s", reexport_start.elapsed().as_secs_f64());
        }

        // Process impl blocks (after all types and re-exports are registered)
        if verbose {
            eprintln!("  Processing impl blocks...");
        }
        let impl_start = Instant::now();
        self.process_impl_blocks(ctx)?;
        if verbose {
            eprintln!("    Impl blocks processed in {:.2}s", impl_start.elapsed().as_secs_f64());
            eprintln!("  Total registration time: {:.2}s", start.elapsed().as_secs_f64());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Create DefIds for all modules.
    fn create_module_def_ids<'a>(&mut self, ctx: &mut TypeContext<'a>) -> Result<(), Vec<StdlibError>> {
        // Sort modules by path depth so parents are created before children
        let mut paths: Vec<String> = self.modules.keys().cloned().collect();
        paths.sort_by_key(|p| p.matches('.').count());

        let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
        if verbose {
            eprintln!("    Processing {} modules for DefId creation", paths.len());
        }

        for module_path in paths {
            if let Some(module) = self.modules.get_mut(&module_path) {
                // Extract just the last segment of the path for the item name
                // e.g., "std.compiler.lexer" -> "lexer"
                let simple_name = module_path
                    .rsplit('.')
                    .next()
                    .unwrap_or(&module_path)
                    .to_string();

                // Create a DefId for this module WITHOUT adding to root scope.
                // Stdlib modules should only be accessible via their full paths (e.g., std.io.effects),
                // not by their simple names in root scope. This prevents collisions when multiple
                // modules share the same simple name (e.g., std.compiler.effects and std.io.effects).
                let def_id = ctx.resolver.define_namespaced_item(
                    simple_name.clone(),
                    DefKind::Mod,
                    Span::dummy(),
                );
                if verbose {
                    eprintln!("      ✓ Created DefId({}) for module '{}' (name: '{}')",
                              def_id.index(), module_path, simple_name);
                }

                module.def_id = Some(def_id);
            }
        }

        Ok(())
    }


    /// Register module hierarchy so parent modules can find child modules.
    fn register_module_hierarchy<'a>(&self, ctx: &mut TypeContext<'a>) -> Result<(), Vec<StdlibError>> {
        // For each parent module, add its children to the module's items list
        for (parent_path, child_paths) in &self.children {
            if let Some(parent_module) = self.modules.get(parent_path) {
                if let Some(parent_def_id) = parent_module.def_id {
                    // Get child DefIds
                    let child_def_ids: Vec<DefId> = child_paths
                        .iter()
                        .filter_map(|child_path| {
                            self.modules.get(child_path)
                                .and_then(|m| m.def_id)
                        })
                        .collect();

                    // Add children to parent's items
                    if let Some(module_info) = ctx.module_defs.get_mut(&parent_def_id) {
                        module_info.items.extend(child_def_ids);
                    }
                }
            }
        }

        Ok(())
    }

    /// Build a dependency graph for glob re-exports.
    ///
    /// Returns a map from module path to the list of module paths it depends on
    /// (i.e., modules it glob-reexports from with `pub use X::*`).
    fn build_reexport_graph(&self) -> HashMap<String, Vec<String>> {
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();

        for (module_path, module) in &self.modules {
            let mut module_deps = Vec::new();

            for import in &module.ast.imports {
                // Only glob imports with public visibility create dependencies
                if let ast::Import::Glob { visibility, path, .. } = import {
                    if *visibility == ast::Visibility::Public {
                        // Build the full path string from the import path
                        let dep_path: String = path.segments
                            .iter()
                            .filter_map(|seg| module.interner.resolve(seg.node))
                            .collect::<Vec<_>>()
                            .join(".");

                        // Only add if this is a known module
                        if self.modules.contains_key(&dep_path) {
                            module_deps.push(dep_path);
                        }
                    }
                }
            }

            deps.insert(module_path.clone(), module_deps);
        }

        deps
    }

    /// Sort modules in dependency order for re-export processing.
    ///
    /// Returns modules ordered so that dependencies are processed before dependents.
    /// Returns Err with cycle path if circular dependencies exist.
    fn topological_sort_modules(
        &self,
        deps: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<String>, StdlibError> {
        // Build in-degree map (how many modules each module depends on)
        let mut in_degree: HashMap<&String, usize> = HashMap::new();

        // Initialize all modules with in-degree 0
        for path in deps.keys() {
            in_degree.insert(path, 0);
        }

        // Count dependencies for each module
        for (path, path_deps) in deps {
            in_degree.insert(path, path_deps.len());
        }

        // Build reverse map: module -> modules that depend on it (dependents)
        let mut dependents: HashMap<&String, Vec<&String>> = HashMap::new();
        for path in deps.keys() {
            dependents.insert(path, Vec::new());
        }
        for (dependent, dep_list) in deps {
            for dep in dep_list {
                if let Some(dep_dependents) = dependents.get_mut(dep) {
                    dep_dependents.push(dependent);
                }
            }
        }

        // Start with modules that have no dependencies (in-degree 0)
        let mut queue: VecDeque<&String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&path, _)| path)
            .collect();

        let mut sorted = Vec::new();

        while let Some(path) = queue.pop_front() {
            sorted.push(path.clone());

            // Decrease in-degree for all modules that depend on this one
            if let Some(path_dependents) = dependents.get(path) {
                for dependent in path_dependents {
                    if let Some(deg) = in_degree.get_mut(*dependent) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(*dependent);
                        }
                    }
                }
            }
        }

        // If not all modules processed, there's a cycle
        if sorted.len() < deps.len() {
            let cycle = self.find_reexport_cycle(deps);
            return Err(StdlibError::CyclicReexport { cycle });
        }

        Ok(sorted)
    }

    /// Find a cycle in the re-export dependency graph using DFS.
    fn find_reexport_cycle(&self, deps: &HashMap<String, Vec<String>>) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for start in deps.keys() {
            if let Some(cycle) = self.dfs_find_cycle(start, deps, &mut visited, &mut rec_stack, &mut path) {
                return cycle;
            }
        }

        // Fallback if we couldn't find the exact cycle
        vec!["unknown cycle".to_string()]
    }

    /// DFS helper for cycle detection.
    fn dfs_find_cycle(
        &self,
        node: &String,
        deps: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if rec_stack.contains(node) {
            // Found a cycle - extract from path
            let cycle_start = path.iter().position(|n| n == node).unwrap_or(0);
            let mut cycle: Vec<_> = path[cycle_start..].to_vec();
            cycle.push(node.clone());
            return Some(cycle);
        }

        if visited.contains(node) {
            return None;
        }

        visited.insert(node.clone());
        rec_stack.insert(node.clone());
        path.push(node.clone());

        if let Some(node_deps) = deps.get(node) {
            for dep in node_deps {
                if let Some(cycle) = self.dfs_find_cycle(dep, deps, visited, rec_stack, path) {
                    return Some(cycle);
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
        None
    }

    /// Process impl blocks from all stdlib modules.
    ///
    /// This creates DefIds for methods and registers ImplBlockInfo so that
    /// method calls on stdlib types can be resolved.
    fn process_impl_blocks<'a>(&self, ctx: &mut TypeContext<'a>) -> Result<(), Vec<StdlibError>> {
        let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
        let mut impl_count = 0;
        let mut method_count = 0;
        // Counter for generating unique TyVarIds for stdlib generics
        // Starting at 100000 to avoid conflicts with other synthetic TyVarIds
        let mut next_tyvar_id: u32 = 100000;

        for (module_path, module) in &self.modules {
            let module_def_id = match module.def_id {
                Some(id) => id,
                None => continue,
            };

            // Helper to resolve symbols
            let resolve_symbol = |sym: ast::Symbol| -> String {
                module.interner.resolve(sym)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            };

            for decl in &module.ast.declarations {
                if let ast::Declaration::Impl(impl_block) = decl {
                    // Extract impl-level type parameters and create TyVarIds for them
                    let mut impl_type_params: HashMap<String, TyVarId> = HashMap::new();
                    let mut impl_generics: Vec<TyVarId> = Vec::new();

                    if let Some(type_params) = &impl_block.type_params {
                        for param in &type_params.params {
                            if let ast::GenericParam::Type(type_param) = param {
                                let param_name = resolve_symbol(type_param.name.node);
                                let tyvar_id = TyVarId::new(next_tyvar_id);
                                next_tyvar_id += 1;
                                impl_type_params.insert(param_name, tyvar_id);
                                impl_generics.push(tyvar_id);
                            }
                        }
                    }

                    // Resolve the self type to a Type, using impl-level type params
                    // IMPORTANT: Pass the current module's DefId so single-segment type names
                    // (like `Token`) resolve to types defined in THIS module first, rather than
                    // finding a type with the same name in a different module.
                    let self_ty = self.resolve_type_with_generics_in_module(ctx, &module.interner, &impl_block.self_ty, &impl_type_params, Some(module_def_id));

                    // Skip if we couldn't resolve the self type
                    if matches!(self_ty.kind(), TypeKind::Error) {
                        if verbose {
                            // Extract self type name for debugging
                            let self_type_name = match &impl_block.self_ty.kind {
                                ast::TypeKind::Path(path) if !path.segments.is_empty() => {
                                    module.interner.resolve(path.segments.last().unwrap().name.node)
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| "?".to_string())
                                }
                                _ => "?".to_string(),
                            };
                            eprintln!("    Skipping impl block for '{}' in '{}': couldn't resolve self type", self_type_name, module_path);
                        }
                        continue;
                    }

                    // Resolve trait ref if present (trait_ty is the trait type in `impl Trait for Type`)
                    let trait_ref = impl_block.trait_ty.as_ref().and_then(|trait_ty| {
                        if let ast::TypeKind::Path(path) = &trait_ty.kind {
                            self.resolve_type_path_to_def_id(ctx, &module.interner, path)
                        } else {
                            None
                        }
                    });

                    // Check for duplicate impl block BEFORE processing methods
                    // (same self_ty and trait_ref combination already exists)
                    let is_duplicate = ctx.impl_blocks.iter().any(|existing| {
                        existing.self_ty == self_ty && existing.trait_ref == trait_ref
                    });

                    if is_duplicate {
                        if verbose {
                            let trait_name = trait_ref
                                .and_then(|id| ctx.trait_defs.get(&id))
                                .map(|t| t.name.clone())
                                .unwrap_or_else(|| "inherent".to_string());
                            eprintln!("    Skipping duplicate impl {} for {:?} in '{}'",
                                      trait_name, self_ty, module_path);
                        }
                        continue;
                    }

                    // Process methods from impl items
                    let mut methods = Vec::new();
                    for item in &impl_block.items {
                        let method = match item {
                            ast::ImplItem::Function(f) => f,
                            _ => continue, // Skip type aliases and consts for now
                        };
                        let method_name = resolve_symbol(method.name.node);

                        // Create a DefId for this method
                        let method_def_id = ctx.resolver.define_namespaced_item(
                            method_name.clone(),
                            DefKind::Fn,
                            method.span,
                        );

                        // Check if method has a self parameter (first param is `self`, `&self`, etc.)
                        let is_static = method.params.first()
                            .map(|p| {
                                // Check if it's a self pattern by looking at the pattern type
                                // A self parameter typically has pattern Ident { name: "self", ... }
                                match &p.pattern.kind {
                                    ast::PatternKind::Ident { name, .. } => {
                                        let name_str = module.interner.resolve(name.node)
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        name_str != "self"
                                    }
                                    ast::PatternKind::Ref { inner, .. } => {
                                        // &self or &mut self
                                        match &inner.kind {
                                            ast::PatternKind::Ident { name, .. } => {
                                                let name_str = module.interner.resolve(name.node)
                                                    .map(|s| s.to_string())
                                                    .unwrap_or_default();
                                                name_str != "self"
                                            }
                                            _ => true
                                        }
                                    }
                                    _ => true
                                }
                            })
                            .unwrap_or(true);

                        // Extract method-level type parameters
                        // Method type params are merged with impl type params for type resolution
                        let mut method_type_params = impl_type_params.clone();
                        let mut method_generics: Vec<TyVarId> = impl_generics.clone();

                        if let Some(type_params) = &method.type_params {
                            for param in &type_params.params {
                                if let ast::GenericParam::Type(type_param) = param {
                                    let param_name = resolve_symbol(type_param.name.node);
                                    // Only add if not already present from impl level
                                    if !method_type_params.contains_key(&param_name) {
                                        let tyvar_id = TyVarId::new(next_tyvar_id);
                                        next_tyvar_id += 1;
                                        method_type_params.insert(param_name, tyvar_id);
                                        method_generics.push(tyvar_id);
                                    }
                                }
                            }
                        }

                        // Build function signature for method lookup
                        // Pass current module so type names like `Token` resolve to THIS module's Token
                        // Pass self_ty so that `Self` type references in signatures get resolved
                        let mut param_types = Vec::new();
                        for (i, param) in method.params.iter().enumerate() {
                            if i == 0 && !is_static {
                                // Self parameter - use self_ty with reference if needed
                                let param_ty = self.resolve_type_with_generics_impl(ctx, &module.interner, &param.ty, &method_type_params, Some(module_def_id), Some(&self_ty));
                                param_types.push(param_ty);
                            } else {
                                // Regular parameter - also pass self_ty in case Self is used in param types
                                let param_ty = self.resolve_type_with_generics_impl(ctx, &module.interner, &param.ty, &method_type_params, Some(module_def_id), Some(&self_ty));
                                param_types.push(param_ty);
                            }
                        }

                        let return_ty = method.return_type.as_ref()
                            .map(|ty| self.resolve_type_with_generics_impl(ctx, &module.interner, ty, &method_type_params, Some(module_def_id), Some(&self_ty)))
                            .unwrap_or_else(Type::unit);

                        // Create and insert the function signature
                        let sig = crate::hir::FnSig {
                            inputs: param_types.clone(),
                            output: return_ty.clone(),
                            is_const: method.qualifiers.is_const,
                            is_async: method.qualifiers.is_async,
                            is_unsafe: method.qualifiers.is_unsafe,
                            generics: method_generics,
                        };

                        // Debug output for stdlib method signature collection
                        if std::env::var("BLOOD_COLLECT_VERBOSE").is_ok() && (method_name == "spanned_ident" || method_name.contains("spanned") || method_name == "clone" || method_name == "advance" || method_name == "current") {
                            eprintln!("[STDLIB_COLLECT_IMPL] method '{}' on {:?} signature:", method_name, self_ty);
                            for (i, ty) in param_types.iter().enumerate() {
                                eprintln!("  param[{}]: {:?}", i, ty);
                            }
                            eprintln!("  return: {:?}", return_ty);
                        }

                        ctx.fn_sigs.insert(method_def_id, sig);

                        methods.push(ImplMethodInfo {
                            def_id: method_def_id,
                            name: method_name.clone(),
                            is_static,
                        });

                        // Register the method's self type for `self` resolution
                        ctx.method_self_types.insert(method_def_id, self_ty.clone());

                        // Add method to module items so it can be imported
                        if let Some(module_info) = ctx.module_defs.get_mut(&module_def_id) {
                            module_info.items.push(method_def_id);
                            module_info.items_by_name.insert(method_name, method_def_id);
                        }

                        method_count += 1;
                    }

                    // Create the ImplBlockInfo
                    let impl_info = ImplBlockInfo {
                        self_ty,
                        trait_ref,
                        generics: impl_generics,
                        methods,
                        assoc_types: Vec::new(),
                        assoc_consts: Vec::new(),
                        span: impl_block.span,
                        from_stdlib: true, // Stdlib impls don't have where clause info
                    };

                    if verbose {
                        let trait_name = trait_ref
                            .and_then(|id| ctx.trait_defs.get(&id))
                            .map(|t| t.name.clone())
                            .unwrap_or_else(|| "inherent".to_string());
                        eprintln!("    Registered impl {} for {:?} in '{}'", trait_name, impl_info.self_ty, module_path);
                    }
                    ctx.impl_blocks.push(impl_info);
                    impl_count += 1;
                }
            }
        }

        if verbose {
            eprintln!("    Processed {} impl blocks with {} methods", impl_count, method_count);
        }

        Ok(())
    }

    /// Resolve an impl block's self type to a HIR Type.
    fn resolve_impl_self_type(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        ast_type: &ast::Type,
    ) -> Type {
        match &ast_type.kind {
            ast::TypeKind::Path(path) => {
                // For single-segment paths, check if it's a primitive type first
                if path.segments.len() == 1 {
                    if let Some(name) = interner.resolve(path.segments[0].name.node) {
                        if let Some(prim_ty) = self.name_to_primitive(name) {
                            return Type::new(TypeKind::Primitive(prim_ty));
                        }
                    }
                }

                // Try to resolve the path to a DefId
                if let Some(def_id) = self.resolve_type_path_to_def_id(ctx, interner, path) {
                    // Check if it's a struct or enum
                    if let Some(info) = ctx.resolver.def_info.get(&def_id) {
                        match info.kind {
                            DefKind::Struct => return Type::adt(def_id, Vec::new()),
                            DefKind::Enum => return Type::adt(def_id, Vec::new()),
                            _ => {}
                        }
                    }
                }
                Type::error()
            }
            ast::TypeKind::Reference { inner, mutable, .. } => {
                let inner_ty = self.resolve_impl_self_type(ctx, interner, inner);
                Type::new(TypeKind::Ref { inner: inner_ty, mutable: *mutable })
            }
            ast::TypeKind::Tuple(elements) if elements.is_empty() => {
                // Unit type ()
                Type::unit()
            }
            _ => Type::error(),
        }
    }

    /// Resolve a type to a HIR Type, handling type parameters.
    ///
    /// This is similar to `resolve_impl_self_type` but also resolves:
    /// - Type parameters (e.g., `T` in `impl<T> Box<T>`)
    /// - Type arguments (e.g., `Box<T>` includes the `T` type argument)
    fn resolve_type_with_generics(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        ast_type: &ast::Type,
        type_params: &HashMap<String, TyVarId>,
    ) -> Type {
        self.resolve_type_with_generics_in_module(ctx, interner, ast_type, type_params, None)
    }

    /// Resolve a type with generics, with awareness of the current module.
    /// The current_module parameter ensures that single-segment type names (like `Token`)
    /// are resolved to the type defined in that module first, rather than potentially
    /// finding a type with the same name in a different module.
    fn resolve_type_with_generics_in_module(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        ast_type: &ast::Type,
        type_params: &HashMap<String, TyVarId>,
        current_module: Option<DefId>,
    ) -> Type {
        self.resolve_type_with_generics_impl(ctx, interner, ast_type, type_params, current_module, None)
    }

    /// Resolve a type with generics, with awareness of the current module and optional Self type.
    /// The self_ty parameter is used to resolve `Self` type references in impl blocks.
    fn resolve_type_with_generics_impl(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        ast_type: &ast::Type,
        type_params: &HashMap<String, TyVarId>,
        current_module: Option<DefId>,
        self_ty: Option<&Type>,
    ) -> Type {
        match &ast_type.kind {
            ast::TypeKind::Path(path) => {
                // For single-segment paths, check type parameters first
                if path.segments.len() == 1 {
                    if let Some(name) = interner.resolve(path.segments[0].name.node) {
                        // Check if it's the Self type
                        if name == "Self" {
                            if let Some(st) = self_ty {
                                return st.clone();
                            }
                        }
                        // Check if it's a type parameter
                        if let Some(&tyvar_id) = type_params.get(name) {
                            return Type::new(TypeKind::Param(tyvar_id));
                        }
                        // Check if it's a primitive type
                        if let Some(prim_ty) = self.name_to_primitive(name) {
                            return Type::new(TypeKind::Primitive(prim_ty));
                        }
                    }
                }

                // Try to resolve the path to a DefId, checking current module first
                if let Some(def_id) = self.resolve_type_path_to_def_id_in_module(ctx, interner, path, current_module) {
                    // Check if it's a struct, enum, or type alias
                    if let Some(info) = ctx.resolver.def_info.get(&def_id) {
                        match info.kind {
                            DefKind::Struct | DefKind::Enum | DefKind::TypeAlias => {
                                // Resolve type arguments if present
                                // Note: Type aliases are NOT expanded here - they remain nominal types
                                // so that impl blocks for `impl Ident` work correctly.
                                let type_args = self.resolve_type_args_in_module(ctx, interner, path, type_params, current_module);
                                return Type::adt(def_id, type_args);
                            }
                            _ => {}
                        }
                    }
                }
                Type::error()
            }
            ast::TypeKind::Reference { inner, mutable, .. } => {
                let inner_ty = self.resolve_type_with_generics_impl(ctx, interner, inner, type_params, current_module, self_ty);
                Type::new(TypeKind::Ref { inner: inner_ty, mutable: *mutable })
            }
            ast::TypeKind::Tuple(elements) if elements.is_empty() => {
                Type::unit()
            }
            ast::TypeKind::Tuple(elements) => {
                let element_types: Vec<Type> = elements.iter()
                    .map(|elem| self.resolve_type_with_generics_impl(ctx, interner, elem, type_params, current_module, self_ty))
                    .collect();
                Type::new(TypeKind::Tuple(element_types))
            }
            ast::TypeKind::Slice { element } => {
                let elem_ty = self.resolve_type_with_generics_impl(ctx, interner, element, type_params, current_module, self_ty);
                Type::new(TypeKind::Slice { element: elem_ty })
            }
            ast::TypeKind::Array { element, size: _ } => {
                let elem_ty = self.resolve_type_with_generics_impl(ctx, interner, element, type_params, current_module, self_ty);
                // For now, use a placeholder size - array size handling is complex
                Type::new(TypeKind::Array { element: elem_ty, size: 0 })
            }
            ast::TypeKind::Function { params, return_type, effects: _ } => {
                let param_types: Vec<Type> = params.iter()
                    .map(|p| self.resolve_type_with_generics_impl(ctx, interner, p, type_params, current_module, self_ty))
                    .collect();
                let ret_ty = self.resolve_type_with_generics_impl(ctx, interner, return_type, type_params, current_module, self_ty);
                Type::new(TypeKind::Fn {
                    params: param_types,
                    ret: ret_ty,
                    effects: Vec::new(),
                })
            }
            _ => Type::error(),
        }
    }

    /// Resolve type arguments from a type path.
    fn resolve_type_args(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::TypePath,
        type_params: &HashMap<String, TyVarId>,
    ) -> Vec<Type> {
        self.resolve_type_args_in_module(ctx, interner, path, type_params, None)
    }

    fn resolve_type_args_in_module(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::TypePath,
        type_params: &HashMap<String, TyVarId>,
        current_module: Option<DefId>,
    ) -> Vec<Type> {
        // Get the last segment which contains the type arguments
        if let Some(segment) = path.segments.last() {
            if let Some(args) = &segment.args {
                return args.args.iter().filter_map(|arg| {
                    match arg {
                        ast::TypeArg::Type(ty) => {
                            Some(self.resolve_type_with_generics_in_module(ctx, interner, ty, type_params, current_module))
                        }
                        _ => None, // Skip lifetimes, consts, etc. for now
                    }
                }).collect();
            }
        }
        Vec::new()
    }

    /// Convert a type name to a primitive type.
    fn name_to_primitive(&self, name: &str) -> Option<PrimitiveTy> {
        match name {
            "bool" => Some(PrimitiveTy::Bool),
            "char" => Some(PrimitiveTy::Char),
            "i8" => Some(PrimitiveTy::Int(IntTy::I8)),
            "i16" => Some(PrimitiveTy::Int(IntTy::I16)),
            "i32" => Some(PrimitiveTy::Int(IntTy::I32)),
            "i64" => Some(PrimitiveTy::Int(IntTy::I64)),
            "i128" => Some(PrimitiveTy::Int(IntTy::I128)),
            "isize" => Some(PrimitiveTy::Int(IntTy::Isize)),
            "u8" => Some(PrimitiveTy::Uint(UintTy::U8)),
            "u16" => Some(PrimitiveTy::Uint(UintTy::U16)),
            "u32" => Some(PrimitiveTy::Uint(UintTy::U32)),
            "u64" => Some(PrimitiveTy::Uint(UintTy::U64)),
            "u128" => Some(PrimitiveTy::Uint(UintTy::U128)),
            "usize" => Some(PrimitiveTy::Uint(UintTy::Usize)),
            "f32" => Some(PrimitiveTy::Float(crate::hir::FloatTy::F32)),
            "f64" => Some(PrimitiveTy::Float(crate::hir::FloatTy::F64)),
            "str" => Some(PrimitiveTy::Str),
            "unit" => Some(PrimitiveTy::Unit),
            _ => None,
        }
    }

    /// Resolve a type path to a DefId by looking through registered modules.
    fn resolve_type_path_to_def_id(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::TypePath,
    ) -> Option<DefId> {
        self.resolve_type_path_to_def_id_in_module(ctx, interner, path, None)
    }

    /// Resolve a type path to a DefId, optionally checking a specific module first.
    /// This is important for single-segment paths in impl blocks to find the type
    /// defined in the same module.
    fn resolve_type_path_to_def_id_in_module(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::TypePath,
        current_module: Option<DefId>,
    ) -> Option<DefId> {
        if path.segments.is_empty() {
            return None;
        }

        // Single-segment path: look for a type in current module or global types
        if path.segments.len() == 1 {
            let name = interner.resolve(path.segments[0].name.node)
                .map(|s| s.to_string())?;

            // Check if it's a builtin type
            if let Some(crate::typeck::resolve::Binding::Def(def_id)) = ctx.resolver.lookup(&name) {
                return Some(def_id);
            }

            // Check type bindings
            if let Some(def_id) = ctx.resolver.lookup_type(&name) {
                return Some(def_id);
            }

            let verbose = std::env::var("BLOOD_TYPE_RESOLVE_VERBOSE").is_ok();

            // IMPORTANT: Check current module FIRST for single-segment paths
            // This ensures that `impl Clone for Token` in lexer.blood finds the Token
            // defined in lexer.blood, not a Token from another module.
            if let Some(module_def_id) = current_module {
                if let Some(module_info) = ctx.module_defs.get(&module_def_id) {
                    if let Some(&def_id) = module_info.items_by_name.get(&name) {
                        if let Some(info) = ctx.resolver.def_info.get(&def_id) {
                            if matches!(info.kind, DefKind::Struct | DefKind::Enum | DefKind::TypeAlias | DefKind::Trait) {
                                if verbose {
                                    eprintln!("    resolve_type_path: found '{}' -> DefId({}) in CURRENT module DefId({})", name, def_id.index(), module_def_id.index());
                                }
                                return Some(def_id);
                            }
                        }
                    }
                }
            }

            // Search through all modules for this type name
            for (module_def_id, module_info) in &ctx.module_defs {
                if let Some(&def_id) = module_info.items_by_name.get(&name) {
                    if let Some(info) = ctx.resolver.def_info.get(&def_id) {
                        // Include Trait in the match to properly resolve trait references in impl blocks
                        if matches!(info.kind, DefKind::Struct | DefKind::Enum | DefKind::TypeAlias | DefKind::Trait) {
                            if verbose {
                                eprintln!("    resolve_type_path: found '{}' -> DefId({}) in module DefId({})", name, def_id.index(), module_def_id.index());
                            }
                            return Some(def_id);
                        }
                    }
                }
            }

            return None;
        }

        // Multi-segment path: resolve as module path
        let first_name = interner.resolve(path.segments[0].name.node)
            .map(|s| s.to_string())?;

        // Find the root module
        let mut current_def_id = None;
        for (&def_id, info) in &ctx.module_defs {
            if info.is_external && info.name == first_name {
                current_def_id = Some(def_id);
                break;
            }
        }

        let mut current_def_id = current_def_id?;

        // Walk remaining segments
        for segment in &path.segments[1..] {
            let name = interner.resolve(segment.name.node)
                .map(|s| s.to_string())?;

            if let Some(module_info) = ctx.module_defs.get(&current_def_id) {
                if let Some(&item_def_id) = module_info.items_by_name.get(&name) {
                    current_def_id = item_def_id;
                } else {
                    // Check in items list
                    let mut found = false;
                    for &item_id in &module_info.items {
                        if let Some(info) = ctx.resolver.def_info.get(&item_id) {
                            if info.name == name {
                                current_def_id = item_id;
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        return None;
                    }
                }
            } else {
                return None;
            }
        }

        Some(current_def_id)
    }

    /// Process pub use re-exports for all stdlib modules.
    ///
    /// This handles declarations like `pub use node::Span;` in mod.blood files,
    /// allowing parent modules to re-export items from child modules.
    ///
    /// Modules are processed in topological order based on their glob re-export
    /// dependencies, ensuring that when processing `pub use A::*`, module A's
    /// re-exports have already been resolved.
    fn process_reexports<'a>(&self, ctx: &mut TypeContext<'a>) -> Result<(), Vec<StdlibError>> {
        // Build dependency graph and get topologically sorted order
        let deps = self.build_reexport_graph();
        let sorted = match self.topological_sort_modules(&deps) {
            Ok(s) => s,
            Err(e) => return Err(vec![e]),
        };

        // Process in dependency order (dependencies first)
        for module_path in sorted {
            let module = match self.modules.get(&module_path) {
                Some(m) => m,
                None => continue,
            };
            let module_def_id = match module.def_id {
                Some(id) => id,
                None => continue,
            };

            let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
            if verbose {
                eprintln!("    Processing imports for module '{}' (DefId {}), {} imports, {} declarations",
                    module_path, module_def_id.index(), module.ast.imports.len(), module.ast.declarations.len());
            }

            // Helper function to process a single import
            let process_import = |import: &ast::Import, ctx: &mut TypeContext<'a>, loader: &Self| {
                let (visibility, path, items_opt, alias_opt) = match import {
                    ast::Import::Simple { visibility, path, alias, .. } => {
                        (*visibility, path, None, alias.as_ref())
                    }
                    ast::Import::Group { visibility, path, items, .. } => {
                        (*visibility, path, Some(items.as_slice()), None)
                    }
                    ast::Import::Glob { visibility, path, .. } => {
                        // Glob re-exports need special handling
                        if *visibility == ast::Visibility::Public {
                            if verbose {
                                eprintln!("      Glob reexport: pub use {:?}::*", path);
                            }
                            loader.process_glob_reexport(ctx, module_def_id, &module_path, &module.interner, path);
                        }
                        return;
                    }
                };

                if visibility != ast::Visibility::Public {
                    return;
                }

                // Resolve the import path relative to this module
                if let Some(items) = items_opt {
                    if verbose {
                        eprintln!("      Group reexport: pub use {:?}::{{...}}", path);
                    }
                    // Group import: pub use path::{Item1, Item2};
                    loader.process_group_reexport(ctx, module_def_id, &module_path, &module.interner, path, items);
                } else {
                    if verbose {
                        eprintln!("      Simple reexport: pub use {:?}", path);
                    }
                    // Simple import: pub use path::Item;
                    loader.process_simple_reexport(ctx, module_def_id, &module_path, &module.interner, path, alias_opt);
                }
            };

            // Process imports array
            for import in &module.ast.imports {
                process_import(import, ctx, self);
            }

            // Also process Declaration::Use from declarations array
            // (pub use statements may appear as declarations rather than imports)
            let mut use_count = 0;
            for decl in &module.ast.declarations {
                if let ast::Declaration::Use(import) = decl {
                    use_count += 1;
                    process_import(import, ctx, self);
                }
            }
            if verbose {
                if module_path == "std.collections" {
                    // Debug: show all declaration types in the collections module
                    eprintln!("      std.collections use_count={}, declarations:", use_count);
                    for (i, decl) in module.ast.declarations.iter().enumerate() {
                        match decl {
                            ast::Declaration::Use(imp) => {
                                let vis = match imp {
                                    ast::Import::Simple { visibility, .. } => visibility,
                                    ast::Import::Group { visibility, .. } => visibility,
                                    ast::Import::Glob { visibility, .. } => visibility,
                                };
                                eprintln!("        [{}] Use: visibility={:?}", i, vis);
                            }
                            ast::Declaration::Module(_) => eprintln!("        [{}] Module", i),
                            ast::Declaration::Trait(_) => eprintln!("        [{}] Trait", i),
                            ast::Declaration::Impl(_) => eprintln!("        [{}] Impl", i),
                            ast::Declaration::Struct(_) => eprintln!("        [{}] Struct", i),
                            ast::Declaration::Enum(_) => eprintln!("        [{}] Enum", i),
                            _ => eprintln!("        [{}] Other", i),
                        }
                    }
                } else if use_count > 0 {
                    eprintln!("      Found {} Declaration::Use items", use_count);
                }
            }
        }

        Ok(())
    }

    /// Process a simple pub use re-export.
    fn process_simple_reexport<'a>(
        &self,
        ctx: &mut TypeContext<'a>,
        module_def_id: DefId,
        module_path: &str,
        interner: &DefaultStringInterner,
        path: &ast::ModulePath,
        alias: Option<&crate::span::Spanned<ast::Symbol>>,
    ) {
        let resolve_symbol = |sym: ast::Symbol| -> String {
            interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
        };

        // Try to resolve the path to find the target item
        // Use the current module as base for relative paths
        if let Some(def_id) = self.resolve_import_path_relative(ctx, interner, path, module_def_id, module_path) {
            // Determine the local name
            let local_name = if let Some(alias_spanned) = alias {
                resolve_symbol(alias_spanned.node)
            } else if let Some(last) = path.segments.last() {
                resolve_symbol(last.node)
            } else {
                return;
            };

            // Add to the module's re-exports
            if let Some(module_info) = ctx.module_defs.get_mut(&module_def_id) {
                module_info.reexports.push((local_name, def_id));
            }
        }
    }

    /// Process a group pub use re-export.
    fn process_group_reexport<'a>(
        &self,
        ctx: &mut TypeContext<'a>,
        module_def_id: DefId,
        module_path: &str,
        interner: &DefaultStringInterner,
        base_path: &ast::ModulePath,
        items: &[ast::ImportItem],
    ) {
        let resolve_symbol = |sym: ast::Symbol| -> String {
            interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
        };

        // Resolve the base path to a module (supports relative paths from current module)
        let path_str: String = base_path.segments.iter()
            .map(|s| resolve_symbol(s.node))
            .collect::<Vec<_>>()
            .join(".");
        let base_module_id = match self.resolve_module_path_relative(ctx, interner, base_path, module_def_id, module_path) {
            Some(id) => {
                let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
                if verbose && module_path == "std.collections" {
                    eprintln!("      RESOLVED: '{}' from '{}' -> DefId({})", path_str, module_path, id.index());
                }
                id
            }
            None => {
                let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
                if verbose {
                    eprintln!("      WARNING: Failed to resolve base path '{}' from module '{}'", path_str, module_path);
                }
                return;
            }
        };

        self.process_import_items_reexport(
            ctx,
            module_def_id,
            interner,
            base_module_id,
            items,
            &resolve_symbol,
        );
    }

    /// Recursively process import items (handles both simple and nested items).
    fn process_import_items_reexport<'a, F>(
        &self,
        ctx: &mut TypeContext<'a>,
        module_def_id: DefId,
        interner: &DefaultStringInterner,
        base_module_id: DefId,
        items: &[ast::ImportItem],
        resolve_symbol: &F,
    )
    where
        F: Fn(ast::Symbol) -> String,
    {
        for item in items {
            match item {
                ast::ImportItem::Simple { name, alias } => {
                    let item_name = resolve_symbol(name.node);
                    let local_name = alias
                        .as_ref()
                        .map(|a| resolve_symbol(a.node))
                        .unwrap_or_else(|| item_name.clone());

                    // Look up the item in the base module
                    if let Some(item_def_id) = self.lookup_in_module(ctx, base_module_id, &item_name) {
                        let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
                        if verbose && item_name == "HashMap" {
                            eprintln!("      REEXPORT: Found '{}' -> DefId({}) in module DefId({})",
                                item_name, item_def_id.index(), base_module_id.index());
                        }
                        if let Some(module_info) = ctx.module_defs.get_mut(&module_def_id) {
                            module_info.reexports.push((local_name.clone(), item_def_id));
                            if verbose && item_name == "HashMap" {
                                eprintln!("      REEXPORT: Registered '{}' as reexport in module DefId({})",
                                    local_name, module_def_id.index());
                            }
                        }
                    } else {
                        let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();
                        if verbose && item_name == "HashMap" {
                            eprintln!("      REEXPORT: FAILED to find '{}' in base_module DefId({}) (reexporting to DefId({}))",
                                item_name, base_module_id.index(), module_def_id.index());
                        }
                    }
                }
                ast::ImportItem::Nested { path, items: nested_items } => {
                    // Resolve the nested path from the base module
                    if let Some(nested_base_id) = self.resolve_nested_path(ctx, interner, base_module_id, path) {
                        self.process_import_items_reexport(
                            ctx,
                            module_def_id,
                            interner,
                            nested_base_id,
                            nested_items,
                            resolve_symbol,
                        );
                    }
                }
            }
        }
    }

    /// Resolve a nested path starting from a base module.
    fn resolve_nested_path(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        base_module_id: DefId,
        path_segments: &[crate::Spanned<ast::Symbol>],
    ) -> Option<DefId> {
        let mut current_id = base_module_id;

        for segment in path_segments {
            let segment_name = interner.resolve(segment.node)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            // Look up the segment as a submodule
            current_id = self.lookup_submodule(ctx, current_id, &segment_name)?;
        }

        Some(current_id)
    }

    /// Look up a submodule within a parent module.
    /// Uses the existing lookup_in_module which checks items_by_name.
    fn lookup_submodule(&self, ctx: &TypeContext<'_>, parent_id: DefId, name: &str) -> Option<DefId> {
        // Use the same mechanism as lookup_in_module - submodules are registered as items
        self.lookup_in_module(ctx, parent_id, name)
    }

    /// Process a glob pub use re-export.
    fn process_glob_reexport<'a>(
        &self,
        ctx: &mut TypeContext<'a>,
        module_def_id: DefId,
        _module_path: &str,
        interner: &DefaultStringInterner,
        path: &ast::ModulePath,
    ) {
        // Resolve the path to a module
        let source_module_id = match self.resolve_module_path(ctx, interner, path) {
            Some(id) => id,
            None => return,
        };

        // Get all public items from the source module
        let items = self.get_module_public_items(ctx, source_module_id);

        for (name, def_id) in items {
            if let Some(module_info) = ctx.module_defs.get_mut(&module_def_id) {
                module_info.reexports.push((name, def_id));
            }
        }
    }

    /// Resolve an import path to find the target item.
    fn resolve_import_path(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::ModulePath,
    ) -> Option<DefId> {
        let resolve_symbol = |sym: ast::Symbol| -> String {
            interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
        };

        if path.segments.is_empty() {
            return None;
        }

        // Walk path segments to find the final item
        let first_segment = resolve_symbol(path.segments[0].node);

        // Find the starting module
        let mut current_id = match first_segment.as_str() {
            // "std" -> look up the std module
            "std" => {
                self.modules.get("std").and_then(|m| m.def_id)?
            }
            // Other starting points could be added (crate, super, etc.)
            _ => return None,
        };

        // Walk remaining segments
        for (i, segment) in path.segments.iter().enumerate().skip(1) {
            let segment_name = resolve_symbol(segment.node);

            if i < path.segments.len() - 1 {
                // Intermediate segment - should be a module
                current_id = self.lookup_in_module(ctx, current_id, &segment_name)?;
            } else {
                // Last segment - should be an item
                return self.lookup_in_module(ctx, current_id, &segment_name);
            }
        }

        Some(current_id)
    }

    /// Resolve an import path, handling relative paths from the current module.
    /// For paths starting with "std", uses absolute resolution.
    /// For other paths, resolves relative to the base_module_id.
    fn resolve_import_path_relative(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::ModulePath,
        base_module_id: DefId,
        base_module_path: &str,
    ) -> Option<DefId> {
        let resolve_symbol = |sym: ast::Symbol| -> String {
            interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
        };

        if path.segments.is_empty() {
            return None;
        }

        let first_segment = resolve_symbol(path.segments[0].node);
        let verbose = std::env::var("BLOOD_STDLIB_VERBOSE").is_ok();

        // Determine the starting module and track the current module path
        let (mut current_id, mut current_module_path) = match first_segment.as_str() {
            // Absolute paths starting with "std"
            "std" => {
                let std_id = self.modules.get("std").and_then(|m| m.def_id)?;
                (std_id, "std".to_string())
            }
            // Relative paths start from the current module
            _ => {
                // First segment could be a submodule of the base module
                // Try submodule first: base_module_path.first_segment
                let submodule_path = format!("{}.{}", base_module_path, first_segment);
                if let Some(submodule_id) = self.modules.get(&submodule_path).and_then(|m| m.def_id) {
                    if verbose && base_module_path == "std.collections" {
                        eprintln!("      resolve_import_path_relative: '{}' is submodule at '{}'", first_segment, submodule_path);
                    }
                    (submodule_id, submodule_path)
                } else {
                    // Fall back to item lookup in base module
                    if let Some(item_id) = self.lookup_in_module(ctx, base_module_id, &first_segment) {
                        // If the path has only one segment, this is the item we want
                        if path.segments.len() == 1 {
                            return Some(item_id);
                        }
                        // Otherwise, item must be a container we can look into
                        (item_id, base_module_path.to_string())
                    } else {
                        if verbose && base_module_path == "std.collections" {
                            eprintln!("      resolve_import_path_relative: FAILED to find '{}' as submodule '{}' or item in '{}'",
                                first_segment, submodule_path, base_module_path);
                        }
                        return None;
                    }
                }
            }
        };

        // Walk remaining segments
        let skip_count = if first_segment == "std" { 1 } else { 1 }; // Skip first segment (already processed)

        for (i, segment) in path.segments.iter().enumerate().skip(skip_count) {
            let segment_name = resolve_symbol(segment.node);
            let is_last = i == path.segments.len() - 1;

            if is_last {
                // Last segment - should be an item in the current module
                let result = self.lookup_in_module(ctx, current_id, &segment_name);
                if verbose && base_module_path == "std.collections" {
                    eprintln!("      resolve_import_path_relative: looking up item '{}' in module '{}' -> {:?}",
                        segment_name, current_module_path, result.map(|id| id.index()));
                }
                return result;
            } else {
                // Intermediate segment - should be a submodule
                let submodule_path = format!("{}.{}", current_module_path, segment_name);
                if let Some(submodule_id) = self.modules.get(&submodule_path).and_then(|m| m.def_id) {
                    current_id = submodule_id;
                    current_module_path = submodule_path;
                } else {
                    // Fall back to item lookup
                    if let Some(id) = self.lookup_in_module(ctx, current_id, &segment_name) {
                        current_id = id;
                    } else {
                        return None;
                    }
                }
            }
        }

        Some(current_id)
    }

    /// Resolve a module path to find the module DefId.
    fn resolve_module_path(
        &self,
        ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::ModulePath,
    ) -> Option<DefId> {
        let resolve_symbol = |sym: ast::Symbol| -> String {
            interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
        };

        if path.segments.is_empty() {
            return None;
        }

        // Build full path string
        let full_path: String = path.segments.iter()
            .map(|s| resolve_symbol(s.node))
            .collect::<Vec<_>>()
            .join(".");

        // Look up in our modules
        self.modules.get(&full_path).and_then(|m| m.def_id)
            .or_else(|| {
                // Try to find by walking the path
                let first_segment = resolve_symbol(path.segments[0].node);
                let mut current_id = match first_segment.as_str() {
                    "std" => self.modules.get("std").and_then(|m| m.def_id)?,
                    _ => return None,
                };

                for segment in path.segments.iter().skip(1) {
                    let segment_name = resolve_symbol(segment.node);
                    current_id = self.lookup_in_module(ctx, current_id, &segment_name)?;
                }

                Some(current_id)
            })
    }

    /// Resolve a module path, supporting relative paths from a base module.
    /// For paths starting with "std", uses absolute resolution.
    /// For other paths, resolves relative to the base module using the module_path_str.
    ///
    /// Resolution order for relative paths:
    /// 1. Try as child of current module: module_path.relative_path
    /// 2. Try as sibling (under parent): parent_module.relative_path
    fn resolve_module_path_relative(
        &self,
        _ctx: &TypeContext<'_>,
        interner: &DefaultStringInterner,
        path: &ast::ModulePath,
        _base_module_id: DefId,
        module_path_str: &str,
    ) -> Option<DefId> {
        let resolve_symbol = |sym: ast::Symbol| -> String {
            interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
        };

        if path.segments.is_empty() {
            return None;
        }

        let first_segment = resolve_symbol(path.segments[0].node);

        // Determine starting point based on whether path is absolute or relative
        match first_segment.as_str() {
            "std" => {
                // Absolute path starting with "std"
                let full_path: String = path.segments.iter()
                    .map(|s| resolve_symbol(s.node))
                    .collect::<Vec<_>>()
                    .join(".");
                self.modules.get(&full_path).and_then(|m| m.def_id)
            }
            _ => {
                // Relative path - try multiple resolution strategies
                let relative_path: String = path.segments.iter()
                    .map(|s| resolve_symbol(s.node))
                    .collect::<Vec<_>>()
                    .join(".");

                // Strategy 1: Try as child of current module (e.g., std.collections.hashmap)
                let child_path = format!("{}.{}", module_path_str, relative_path);
                if let Some(result) = self.modules.get(&child_path).and_then(|m| m.def_id) {
                    return Some(result);
                }

                // Strategy 2: Try as sibling under parent module (e.g., std.core from std.prelude)
                if let Some(dot_pos) = module_path_str.rfind('.') {
                    let parent_path = &module_path_str[..dot_pos];
                    let sibling_path = format!("{}.{}", parent_path, relative_path);
                    if let Some(result) = self.modules.get(&sibling_path).and_then(|m| m.def_id) {
                        return Some(result);
                    }
                }

                None
            }
        }
    }

    /// Look up an item in a module.
    ///
    /// Uses O(1) HashMap lookup when the items_by_name index is populated,
    /// falling back to linear search for re-exports.
    fn lookup_in_module(&self, ctx: &TypeContext<'_>, module_id: DefId, name: &str) -> Option<DefId> {
        if let Some(module_def) = ctx.module_defs.get(&module_id) {
            // O(1) lookup in items index
            if let Some(&def_id) = module_def.items_by_name.get(name) {
                return Some(def_id);
            }

            // Check re-exports (still linear, but typically much smaller)
            for (reexport_name, def_id) in &module_def.reexports {
                if reexport_name == name {
                    return Some(*def_id);
                }
            }
        }
        None
    }

    /// Get all public items from a module.
    fn get_module_public_items(&self, ctx: &TypeContext<'_>, module_id: DefId) -> Vec<(String, DefId)> {
        let mut items = Vec::new();

        if let Some(module_def) = ctx.module_defs.get(&module_id) {
            for &item_id in &module_def.items {
                if let Some(info) = ctx.resolver.def_info.get(&item_id) {
                    match info.visibility {
                        ast::Visibility::Public | ast::Visibility::PublicCrate => {
                            items.push((info.name.clone(), item_id));
                        }
                        _ => {}
                    }
                }
            }

            // Include existing re-exports
            for (name, def_id) in &module_def.reexports {
                items.push((name.clone(), *def_id));
            }
        }

        items
    }

    /// Get a loaded module by path.
    pub fn get_module(&self, path: &str) -> Option<&LoadedModule> {
        self.modules.get(path)
    }

    /// Get all loaded module paths.
    pub fn module_paths(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(|s| s.as_str())
    }

    /// Get the number of loaded modules.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }
}

/// Errors that can occur during stdlib loading.
#[derive(Debug)]
pub enum StdlibError {
    PathNotFound(PathBuf),
    IoError(String),
    ParseError { file: PathBuf, message: String },
    /// Circular dependency detected in glob re-exports.
    CyclicReexport {
        /// The cycle path (e.g., ["std.a", "std.b", "std.a"])
        cycle: Vec<String>,
    },
}

impl std::fmt::Display for StdlibError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StdlibError::PathNotFound(path) => {
                write!(f, "stdlib path not found: {}", path.display())
            }
            StdlibError::IoError(msg) => write!(f, "I/O error: {}", msg),
            StdlibError::ParseError { file, message } => {
                write!(f, "parse error in {}: {}", file.display(), message)
            }
            StdlibError::CyclicReexport { cycle } => {
                write!(f, "circular dependency in glob re-exports: {}", cycle.join(" -> "))
            }
        }
    }
}

impl std::error::Error for StdlibError {}

/// Create a DefId for a declaration.
/// This is a free function to avoid borrow checker issues when iterating over modules.
///
/// If the item already exists as a builtin, returns the existing DefId.
/// This allows stdlib modules to extend or shadow builtins.
fn create_item_def_id<'a>(
    ctx: &mut TypeContext<'a>,
    interner: &DefaultStringInterner,
    decl: &ast::Declaration,
    verbose: bool,
) -> Option<DefId> {
    // Helper to resolve a symbol using the module's interner
    let resolve_symbol = |sym: crate::ast::Symbol| -> String {
        interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
    };

    let (name, kind, kind_str) = match decl {
        ast::Declaration::Function(f) => {
            let name = resolve_symbol(f.name.node);
            (name, DefKind::Fn, "fn")
        }
        ast::Declaration::Struct(s) => {
            let name = resolve_symbol(s.name.node);
            (name, DefKind::Struct, "struct")
        }
        ast::Declaration::Enum(e) => {
            let name = resolve_symbol(e.name.node);
            (name, DefKind::Enum, "enum")
        }
        ast::Declaration::Trait(t) => {
            let name = resolve_symbol(t.name.node);
            (name, DefKind::Trait, "trait")
        }
        ast::Declaration::Type(t) => {
            let name = resolve_symbol(t.name.node);
            (name, DefKind::TypeAlias, "type")
        }
        ast::Declaration::Effect(e) => {
            let name = resolve_symbol(e.name.node);
            (name, DefKind::Effect, "effect")
        }
        ast::Declaration::Const(c) => {
            let name = resolve_symbol(c.name.node);
            (name, DefKind::Const, "const")
        }
        ast::Declaration::Static(s) => {
            let name = resolve_symbol(s.name.node);
            (name, DefKind::Static, "static")
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

    // Check if item already exists (e.g., builtins like Option, Result)
    // If so, reuse the existing DefId
    if let Some(crate::typeck::resolve::Binding::Def(def_id)) = ctx.resolver.lookup(&name) {
        if verbose {
            eprintln!("        ~ {} {} -> existing DefId({}) (builtin)", kind_str, name, def_id.index());
        }
        return Some(def_id);
    }

    // Use define_namespaced_item to create DefId WITHOUT adding to root scope.
    // Stdlib items should only be accessible through their module paths (e.g., std.compiler.lexer::Token),
    // not directly in the root scope. This prevents name collisions when checking files
    // that define their own types with the same names.
    let def_id = ctx.resolver.define_namespaced_item(name.clone(), kind, Span::dummy());
    if verbose {
        eprintln!("        ✓ {} {} -> DefId({}) (namespaced)", kind_str, name, def_id.index());
    }
    Some(def_id)
}

/// Register type information for a declaration.
/// This populates struct_defs/enum_defs so type checking works for external types.
fn register_type_info<'a>(
    ctx: &mut TypeContext<'a>,
    interner: &DefaultStringInterner,
    def_id: DefId,
    decl: &ast::Declaration,
) {
    let resolve_symbol = |sym: crate::ast::Symbol| -> String {
        interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
    };

    match decl {
        ast::Declaration::Struct(s) => {
            // Skip if struct info already exists (e.g., from builtins)
            if ctx.struct_defs.contains_key(&def_id) {
                return;
            }

            let name = resolve_symbol(s.name.node);

            // Convert fields
            let fields = match &s.body {
                ast::StructBody::Record(fields) => {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let field_name = resolve_symbol(f.name.node);
                            let ty = ast_type_to_basic_type(interner, &f.ty);
                            FieldInfo {
                                name: field_name,
                                ty,
                                index: i as u32,
                            }
                        })
                        .collect()
                }
                ast::StructBody::Tuple(fields) => {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(i, field)| {
                            let ty = ast_type_to_basic_type(interner, &field.ty);
                            FieldInfo {
                                name: format!("{i}"),
                                ty,
                                index: i as u32,
                            }
                        })
                        .collect()
                }
                ast::StructBody::Unit => Vec::new(),
            };

            // Extract generics (simplified - just track count for now)
            let generics: Vec<TyVarId> = if let Some(ref params) = s.type_params {
                params.params.iter().enumerate().filter_map(|(i, p)| {
                    match p {
                        ast::GenericParam::Type(_) => Some(TyVarId(i as u32)),
                        _ => None,
                    }
                }).collect()
            } else {
                Vec::new()
            };

            ctx.struct_defs.insert(def_id, StructInfo {
                name,
                fields,
                generics,
            });
        }
        ast::Declaration::Enum(e) => {
            // Skip if enum info already exists (e.g., from builtins like Option, Result)
            if ctx.enum_defs.contains_key(&def_id) {
                return;
            }

            let name = resolve_symbol(e.name.node);

            // Convert variants - we need to create DefIds for each variant
            let mut variants = Vec::new();
            for (i, v) in e.variants.iter().enumerate() {
                let variant_name = resolve_symbol(v.name.node);
                let fields = match &v.body {
                    ast::StructBody::Unit => Vec::new(),
                    ast::StructBody::Tuple(fields) => {
                        fields
                            .iter()
                            .enumerate()
                            .map(|(fi, field)| {
                                FieldInfo {
                                    name: format!("{fi}"),
                                    ty: ast_type_to_basic_type(interner, &field.ty),
                                    index: fi as u32,
                                }
                            })
                            .collect()
                    }
                    ast::StructBody::Record(fields) => {
                        fields
                            .iter()
                            .enumerate()
                            .map(|(fi, f)| {
                                FieldInfo {
                                    name: resolve_symbol(f.name.node),
                                    ty: ast_type_to_basic_type(interner, &f.ty),
                                    index: fi as u32,
                                }
                            })
                            .collect()
                    }
                };

                // Create a DefId for this variant WITHOUT adding to root scope.
                // Stdlib enum variants should only be accessible via their enum type
                // (e.g., LLVMTypeKind::Token), not directly in root scope.
                let variant_def_id = ctx.resolver.define_namespaced_item(
                    variant_name.clone(),
                    DefKind::Variant,
                    Span::dummy(),
                );

                variants.push(VariantInfo {
                    name: variant_name,
                    index: i as u32,
                    fields,
                    def_id: variant_def_id,
                });
            }

            // Extract generics
            let generics: Vec<TyVarId> = if let Some(ref params) = e.type_params {
                params.params.iter().enumerate().filter_map(|(i, p)| {
                    match p {
                        ast::GenericParam::Type(_) => Some(TyVarId(i as u32)),
                        _ => None,
                    }
                }).collect()
            } else {
                Vec::new()
            };

            ctx.enum_defs.insert(def_id, EnumInfo {
                name,
                variants,
                generics,
            });
        }
        ast::Declaration::Type(t) => {
            // Skip if type alias info already exists
            if ctx.type_aliases.contains_key(&def_id) {
                return;
            }

            let name = resolve_symbol(t.name.node);

            // Extract generics from the type alias declaration
            let generics: Vec<TyVarId> = if let Some(ref params) = t.type_params {
                params.params.iter().enumerate().filter_map(|(i, p)| {
                    match p {
                        ast::GenericParam::Type(_) => Some(TyVarId(i as u32)),
                        _ => None,
                    }
                }).collect()
            } else {
                Vec::new()
            };

            // Convert the aliased type
            // For complex types like Spanned<Symbol>, we need to resolve the DefIds
            if std::env::var("BLOOD_DEBUG_TYPE_ALIAS").is_ok() {
                eprintln!("[type_alias] Processing type alias '{}' DefId({})", name, def_id.index());
            }
            let aliased_ty = if let Some(ref ty_val) = t.ty {
                let ty = ast_type_to_type_with_ctx(ctx, interner, ty_val);
                if std::env::var("BLOOD_DEBUG_TYPE_ALIAS").is_ok() {
                    eprintln!("[type_alias] '{}' resolved to {:?}", name, ty);
                }
                ty
            } else {
                Type::error()
            };

            ctx.type_aliases.insert(def_id, TypeAliasInfo {
                name,
                ty: aliased_ty,
                generics,
            });
        }
        // Other declarations don't need type info registration
        _ => {}
    }
}

/// Register a function signature for a top-level function declaration.
/// This allows imported functions to be called with proper type checking.
fn register_fn_sig<'a>(
    ctx: &mut TypeContext<'a>,
    interner: &DefaultStringInterner,
    def_id: DefId,
    decl: &ast::Declaration,
) {
    if let ast::Declaration::Function(f) = decl {
        // Skip if fn_sig already exists
        if ctx.fn_sigs.contains_key(&def_id) {
            return;
        }

        // Convert parameter types
        let mut param_types = Vec::new();
        for param in &f.params {
            let param_ty = ast_type_to_basic_type(interner, &param.ty);
            param_types.push(param_ty);
        }

        // Convert return type
        let return_ty = f.return_type.as_ref()
            .map(|ty| ast_type_to_basic_type(interner, ty))
            .unwrap_or_else(Type::unit);

        // Create and insert the function signature
        let sig = crate::hir::FnSig {
            inputs: param_types,
            output: return_ty,
            is_const: f.qualifiers.is_const,
            is_async: f.qualifiers.is_async,
            is_unsafe: f.qualifiers.is_unsafe,
            generics: Vec::new(), // TODO: handle function generics
        };
        ctx.fn_sigs.insert(def_id, sig);
    }
}

/// Convert an AST type to HIR type with access to the type context.
/// This can resolve user-defined types by looking up their DefIds.
fn ast_type_to_type_with_ctx<'a>(
    ctx: &TypeContext<'a>,
    interner: &DefaultStringInterner,
    ty: &ast::Type,
) -> Type {
    let resolve_symbol = |sym: crate::ast::Symbol| -> String {
        interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
    };

    match &ty.kind {
        ast::TypeKind::Path(path) => {
            if path.segments.len() == 1 {
                let name = resolve_symbol(path.segments[0].name.node);

                // Handle primitive types first
                if let Some(prim_ty) = match name.as_str() {
                    "i8" => Some(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8)))),
                    "i16" => Some(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16)))),
                    "i32" => Some(Type::i32()),
                    "i64" => Some(Type::i64()),
                    "i128" => Some(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128)))),
                    "isize" => Some(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::Isize)))),
                    "u8" => Some(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8)))),
                    "u16" => Some(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16)))),
                    "u32" => Some(Type::u32()),
                    "u64" => Some(Type::u64()),
                    "u128" => Some(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128)))),
                    "usize" => Some(Type::usize()),
                    "f32" => Some(Type::f32()),
                    "f64" => Some(Type::f64()),
                    "bool" => Some(Type::bool()),
                    "char" => Some(Type::char()),
                    "str" => Some(Type::str()),
                    "String" => Some(Type::string()),
                    "()" => Some(Type::unit()),
                    _ => None,
                } {
                    return prim_ty;
                }

                // Try to resolve as a user-defined type via resolver first
                let mut found_def_id: Option<DefId> = ctx.resolver.lookup_type(&name);

                // If resolver lookup fails, search through registered struct_defs and enum_defs
                if found_def_id.is_none() {
                    // Search struct_defs
                    for (&def_id, info) in &ctx.struct_defs {
                        if info.name == name {
                            found_def_id = Some(def_id);
                            break;
                        }
                    }
                }
                if found_def_id.is_none() {
                    // Search enum_defs
                    for (&def_id, info) in &ctx.enum_defs {
                        if info.name == name {
                            found_def_id = Some(def_id);
                            break;
                        }
                    }
                }
                if found_def_id.is_none() {
                    // Search type_aliases
                    for (&def_id, info) in &ctx.type_aliases {
                        if info.name == name {
                            found_def_id = Some(def_id);
                            break;
                        }
                    }
                }

                if std::env::var("BLOOD_DEBUG_TYPE_ALIAS").is_ok() {
                    eprintln!("[type_alias] Looking up type '{}' -> {:?}", name, found_def_id.map(|d| d.index()));
                }
                if let Some(def_id) = found_def_id {
                    // Collect type arguments if any
                    let type_args: Vec<Type> = if let Some(ref args) = path.segments[0].args {
                        args.args.iter().filter_map(|arg| {
                            if let ast::TypeArg::Type(arg_ty) = arg {
                                Some(ast_type_to_type_with_ctx(ctx, interner, arg_ty))
                            } else {
                                None
                            }
                        }).collect()
                    } else {
                        Vec::new()
                    };

                    return Type::adt(def_id, type_args);
                }

                // Fallback to error
                Type::error()
            } else if path.segments.len() == 2 {
                // Two-segment path: Module::Type
                let module_name = resolve_symbol(path.segments[0].name.node);
                let type_name = resolve_symbol(path.segments[1].name.node);

                // Look up the module
                for (module_def_id, module_info) in &ctx.module_defs {
                    if module_info.name == module_name {
                        if let Some(&def_id) = module_info.items_by_name.get(&type_name) {
                            // Collect type arguments if any
                            let type_args: Vec<Type> = if let Some(ref args) = path.segments[1].args {
                                args.args.iter().filter_map(|arg| {
                                    if let ast::TypeArg::Type(arg_ty) = arg {
                                        Some(ast_type_to_type_with_ctx(ctx, interner, arg_ty))
                                    } else {
                                        None
                                    }
                                }).collect()
                            } else {
                                Vec::new()
                            };

                            return Type::adt(def_id, type_args);
                        }
                    }
                }

                Type::error()
            } else {
                Type::error()
            }
        }
        ast::TypeKind::Reference { inner, mutable, .. } => {
            let inner_ty = ast_type_to_type_with_ctx(ctx, interner, inner);
            Type::new(TypeKind::Ref {
                inner: inner_ty,
                mutable: *mutable
            })
        }
        ast::TypeKind::Pointer { inner, mutable } => {
            let inner_ty = ast_type_to_type_with_ctx(ctx, interner, inner);
            Type::new(TypeKind::Ptr {
                inner: inner_ty,
                mutable: *mutable
            })
        }
        ast::TypeKind::Tuple(types) => {
            let tys: Vec<Type> = types
                .iter()
                .map(|t| ast_type_to_type_with_ctx(ctx, interner, t))
                .collect();
            Type::new(TypeKind::Tuple(tys))
        }
        ast::TypeKind::Slice { element } => {
            let elem_ty = ast_type_to_type_with_ctx(ctx, interner, element);
            Type::new(TypeKind::Slice { element: elem_ty })
        }
        ast::TypeKind::Array { element, size } => {
            let elem_ty = ast_type_to_type_with_ctx(ctx, interner, element);
            // Extract size from the expression if it's a literal integer
            let size_val = if let ast::ExprKind::Literal(lit) = &size.kind {
                if let ast::LiteralKind::Int { value, .. } = &lit.kind {
                    *value as u64
                } else {
                    0
                }
            } else {
                0
            };
            Type::new(TypeKind::Array { element: elem_ty, size: size_val })
        }
        _ => Type::error(),
    }
}

/// Convert an AST type to a basic HIR type.
/// This handles primitive types and falls back to an error type for complex types.
/// Full type resolution happens later during type checking.
fn ast_type_to_basic_type(
    interner: &DefaultStringInterner,
    ty: &ast::Type,
) -> Type {
    let resolve_symbol = |sym: crate::ast::Symbol| -> String {
        interner.resolve(sym).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string())
    };

    match &ty.kind {
        ast::TypeKind::Path(path) => {
            if path.segments.len() == 1 && path.segments[0].args.is_none() {
                let name = resolve_symbol(path.segments[0].name.node);

                // Handle primitive types
                match name.as_str() {
                    "i8" => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8))),
                    "i16" => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16))),
                    "i32" => Type::i32(),
                    "i64" => Type::i64(),
                    "i128" => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128))),
                    "isize" => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::Isize))),
                    "u8" => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8))),
                    "u16" => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16))),
                    "u32" => Type::u32(),
                    "u64" => Type::u64(),
                    "u128" => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128))),
                    "usize" => Type::usize(),
                    "f32" => Type::f32(),
                    "f64" => Type::f64(),
                    "bool" => Type::bool(),
                    "char" => Type::char(),
                    "str" => Type::str(),
                    "String" => Type::string(),
                    "()" => Type::unit(),
                    // For non-primitive types, use a placeholder
                    // These will be resolved during actual type checking
                    _ => Type::error(),
                }
            } else {
                // Complex paths (generic types, module paths, etc.)
                Type::error()
            }
        }
        ast::TypeKind::Reference { inner, mutable, .. } => {
            let inner_ty = ast_type_to_basic_type(interner, inner);
            Type::new(TypeKind::Ref {
                inner: inner_ty,
                mutable: *mutable
            })
        }
        ast::TypeKind::Pointer { inner, mutable } => {
            let inner_ty = ast_type_to_basic_type(interner, inner);
            Type::new(TypeKind::Ptr {
                inner: inner_ty,
                mutable: *mutable
            })
        }
        ast::TypeKind::Tuple(types) => {
            let tys: Vec<Type> = types
                .iter()
                .map(|t| ast_type_to_basic_type(interner, t))
                .collect();
            Type::new(TypeKind::Tuple(tys))
        }
        ast::TypeKind::Slice { element } => {
            let elem_ty = ast_type_to_basic_type(interner, element);
            Type::new(TypeKind::Slice { element: elem_ty })
        }
        // For other complex types, use a placeholder
        _ => Type::error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_stdlib() -> TempDir {
        let temp = TempDir::new().unwrap();
        let std_dir = temp.path().join("std");

        // Create stdlib structure
        fs::create_dir_all(&std_dir).unwrap();
        fs::create_dir_all(std_dir.join("compiler")).unwrap();

        // Create lib.blood
        fs::write(
            std_dir.join("lib.blood"),
            "pub const VERSION: &str = \"0.1.0\";",
        ).unwrap();

        // Create compiler/lexer.blood
        fs::write(
            std_dir.join("compiler/lexer.blood"),
            r#"
pub struct Token {
    kind: i32,
}

pub fn tokenize(source: &str) -> [Token] {
    []
}
"#,
        ).unwrap();

        temp
    }

    #[test]
    fn test_discover_modules() {
        let temp = create_test_stdlib();
        let mut loader = StdlibLoader::new(temp.path().join("std"));

        loader.discover().unwrap();

        // Should find std (from lib.blood) and std.compiler.lexer
        assert!(loader.modules.contains_key("std"));
        assert!(loader.modules.contains_key("std.compiler.lexer"));
    }

    #[test]
    fn test_parse_modules() {
        let temp = create_test_stdlib();
        let mut loader = StdlibLoader::new(temp.path().join("std"));

        loader.discover().unwrap();
        loader.parse_all().unwrap();

        // Check that parsing succeeded
        let lexer_module = loader.get_module("std.compiler.lexer").unwrap();
        assert!(!lexer_module.ast.declarations.is_empty());
    }

    #[test]
    fn test_build_reexport_graph_no_deps() {
        // A module with no glob re-exports should have empty dependencies
        let temp = TempDir::new().unwrap();
        let std_dir = temp.path().join("std");
        fs::create_dir_all(&std_dir).unwrap();

        fs::write(
            std_dir.join("lib.blood"),
            "pub const X: i32 = 1;",
        ).unwrap();

        let mut loader = StdlibLoader::new(std_dir);
        loader.discover().unwrap();
        loader.parse_all().unwrap();

        let deps = loader.build_reexport_graph();
        assert!(deps.get("std").map_or(true, |d| d.is_empty()));
    }

    #[test]
    fn test_build_reexport_graph_with_deps() {
        // Module with `pub use child::*` should have child as dependency
        let temp = TempDir::new().unwrap();
        let std_dir = temp.path().join("std");
        let child_dir = std_dir.join("child");
        fs::create_dir_all(&child_dir).unwrap();

        fs::write(
            std_dir.join("lib.blood"),
            "pub use std.child::*;",
        ).unwrap();

        fs::write(
            child_dir.join("mod.blood"),
            "pub const Y: i32 = 2;",
        ).unwrap();

        let mut loader = StdlibLoader::new(std_dir);
        loader.discover().unwrap();
        loader.parse_all().unwrap();

        let deps = loader.build_reexport_graph();
        let std_deps = deps.get("std").unwrap();
        assert!(std_deps.contains(&"std.child".to_string()));
    }

    #[test]
    fn test_topological_sort_simple_chain() {
        // c -> b -> a (a depends on nothing, b depends on a, c depends on b)
        // Should sort as: a, b, c
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        deps.insert("a".to_string(), Vec::new());
        deps.insert("b".to_string(), vec!["a".to_string()]);
        deps.insert("c".to_string(), vec!["b".to_string()]);

        let loader = StdlibLoader::new(PathBuf::new());
        let sorted = loader.topological_sort_modules(&deps).unwrap();

        // a should come before b, b before c
        let pos_a = sorted.iter().position(|x| x == "a").unwrap();
        let pos_b = sorted.iter().position(|x| x == "b").unwrap();
        let pos_c = sorted.iter().position(|x| x == "c").unwrap();

        assert!(pos_a < pos_b, "a should be processed before b");
        assert!(pos_b < pos_c, "b should be processed before c");
    }

    #[test]
    fn test_topological_sort_parallel_deps() {
        // d has no deps
        // c depends on d
        // b depends on d
        // a depends on b and c
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        deps.insert("d".to_string(), Vec::new());
        deps.insert("c".to_string(), vec!["d".to_string()]);
        deps.insert("b".to_string(), vec!["d".to_string()]);
        deps.insert("a".to_string(), vec!["b".to_string(), "c".to_string()]);

        let loader = StdlibLoader::new(PathBuf::new());
        let sorted = loader.topological_sort_modules(&deps).unwrap();

        let pos_d = sorted.iter().position(|x| x == "d").unwrap();
        let pos_c = sorted.iter().position(|x| x == "c").unwrap();
        let pos_b = sorted.iter().position(|x| x == "b").unwrap();
        let pos_a = sorted.iter().position(|x| x == "a").unwrap();

        // d must come before b and c
        assert!(pos_d < pos_b, "d should be processed before b");
        assert!(pos_d < pos_c, "d should be processed before c");
        // b and c must come before a
        assert!(pos_b < pos_a, "b should be processed before a");
        assert!(pos_c < pos_a, "c should be processed before a");
    }

    #[test]
    fn test_topological_sort_detects_cycle() {
        // a -> b -> a (cycle)
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["a".to_string()]);

        let loader = StdlibLoader::new(PathBuf::new());
        let result = loader.topological_sort_modules(&deps);

        assert!(result.is_err(), "Should detect cycle");
        if let Err(StdlibError::CyclicReexport { cycle }) = result {
            // Cycle should contain both a and b
            assert!(cycle.contains(&"a".to_string()) || cycle.contains(&"b".to_string()));
        } else {
            panic!("Expected CyclicReexport error");
        }
    }

    #[test]
    fn test_topological_sort_detects_longer_cycle() {
        // a -> b -> c -> a (3-way cycle)
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["c".to_string()]);
        deps.insert("c".to_string(), vec!["a".to_string()]);

        let loader = StdlibLoader::new(PathBuf::new());
        let result = loader.topological_sort_modules(&deps);

        assert!(result.is_err(), "Should detect cycle");
        match result {
            Err(StdlibError::CyclicReexport { .. }) => { /* expected */ }
            _ => panic!("Expected CyclicReexport error"),
        }
    }
}
