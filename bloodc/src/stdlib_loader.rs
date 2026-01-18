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

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use string_interner::DefaultStringInterner;

use crate::ast;
use crate::hir::{DefId, DefKind, Type, TypeKind, PrimitiveTy, IntTy, UintTy, TyVarId};
use crate::parser::Parser;
use crate::span::Span;
use crate::typeck::TypeContext;
use crate::typeck::context::{StructInfo, FieldInfo, EnumInfo, VariantInfo};

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
}

impl StdlibLoader {
    /// Create a new stdlib loader.
    pub fn new(stdlib_root: PathBuf) -> Self {
        Self {
            stdlib_root,
            modules: HashMap::new(),
            children: HashMap::new(),
        }
    }

    /// Discover all modules in the stdlib.
    ///
    /// This walks the stdlib directory tree and creates LoadedModule
    /// entries for each .blood file found.
    pub fn discover(&mut self) -> Result<(), StdlibError> {
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

                // Skip hidden directories and build artifacts
                if dir_name.starts_with('.') || dir_name.ends_with("_objs") {
                    continue;
                }

                let child_prefix = format!("{}.{}", module_prefix, dir_name);
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
    pub fn parse_all(&mut self) -> Result<(), Vec<StdlibError>> {
        let mut errors = Vec::new();

        // Get list of module paths to parse
        let paths: Vec<String> = self.modules.keys().cloned().collect();

        for module_path in paths {
            // Need to temporarily take the module out to parse it
            if let Some(mut module) = self.modules.remove(&module_path) {
                let mut parser = Parser::new(&module.source);
                match parser.parse_program() {
                    Ok(ast) => {
                        module.ast = ast;
                        module.interner = parser.take_interner();
                        self.modules.insert(module_path, module);
                    }
                    Err(parse_errors) => {
                        for err in parse_errors {
                            errors.push(StdlibError::ParseError {
                                file: module.file_path.clone(),
                                message: err.message,
                            });
                        }
                        // Still insert the module (with empty AST) so we don't lose track
                        module.interner = parser.take_interner();
                        self.modules.insert(module_path, module);
                    }
                }
            }
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
        let errors: Vec<StdlibError> = Vec::new();

        // First, create DefIds for all modules
        self.create_module_def_ids(ctx)?;

        // Then, collect declarations for each module
        for (module_path, module) in &mut self.modules {
            // Skip modules that failed to parse
            if module.ast.declarations.is_empty() && !module.source.is_empty() {
                continue;
            }

            let def_id = match module.def_id {
                Some(id) => id,
                None => continue,
            };

            // Collect declarations from this module's AST
            // We need to run the collection phase for each module
            // For now, we'll extract just the top-level item names

            for decl in &module.ast.declarations {
                if let Some(item_def_id) = create_item_def_id(ctx, &module.interner, decl) {
                    module.items.push(item_def_id);
                    // Register struct/enum type info for type system integration
                    register_type_info(ctx, &module.interner, item_def_id, decl);
                }
            }

            // Register the module with its items
            ctx.register_external_module(
                module_path.clone(),
                def_id,
                module.items.clone(),
                Span::dummy(),
            );
        }

        // Build module hierarchy (parent modules contain child modules)
        self.register_module_hierarchy(ctx)?;

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

        for module_path in paths {
            if let Some(module) = self.modules.get_mut(&module_path) {
                // Extract just the last segment of the path for the item name
                // e.g., "std.compiler.lexer" -> "lexer"
                let simple_name = module_path
                    .rsplit('.')
                    .next()
                    .unwrap_or(&module_path)
                    .to_string();

                // Create a DefId for this module
                let def_id = match ctx.resolver.define_item(
                    simple_name,
                    DefKind::Mod,
                    Span::dummy(),
                ) {
                    Ok(id) => id,
                    Err(_) => {
                        // Module might already be defined, try to look it up
                        continue;
                    }
                };

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
        }
    }
}

impl std::error::Error for StdlibError {}

/// Create a DefId for a declaration.
/// This is a free function to avoid borrow checker issues when iterating over modules.
fn create_item_def_id<'a>(
    ctx: &mut TypeContext<'a>,
    interner: &DefaultStringInterner,
    decl: &ast::Declaration,
) -> Option<DefId> {
    // Helper to resolve a symbol using the module's interner
    let resolve_symbol = |sym: crate::ast::Symbol| -> String {
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
    };

    ctx.resolver.define_item(name, kind, Span::dummy()).ok()
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
                ast::StructBody::Tuple(types) => {
                    types
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| {
                            let ty = ast_type_to_basic_type(interner, ty);
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
            let name = resolve_symbol(e.name.node);

            // Convert variants - we need to create DefIds for each variant
            let mut variants = Vec::new();
            for (i, v) in e.variants.iter().enumerate() {
                let variant_name = resolve_symbol(v.name.node);
                let fields = match &v.body {
                    ast::StructBody::Unit => Vec::new(),
                    ast::StructBody::Tuple(types) => {
                        types
                            .iter()
                            .enumerate()
                            .map(|(fi, ty)| {
                                FieldInfo {
                                    name: format!("{fi}"),
                                    ty: ast_type_to_basic_type(interner, ty),
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

                // Create a DefId for this variant
                let variant_def_id = ctx.resolver.define_item(
                    variant_name.clone(),
                    DefKind::Variant,
                    Span::dummy(),
                ).unwrap_or(DefId::new(0)); // Fallback if already defined

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
        // Other declarations don't need type info registration
        _ => {}
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
}
