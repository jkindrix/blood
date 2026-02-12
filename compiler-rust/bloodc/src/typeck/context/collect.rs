//! Declaration collection for the type checker.
//!
//! This module contains methods for collecting top-level declarations
//! (functions, structs, enums, effects, handlers, impl blocks, traits)
//! and registering them in the type context.

use std::collections::{HashMap, HashSet};

use crate::ast;
use crate::hir::{self, DefId, Type, TypeKind, TyVarId};
use crate::diagnostics::Diagnostic;

use super::{
    TypeContext, StructInfo, FieldInfo, EnumInfo, VariantInfo, TypeAliasInfo,
    EffectInfo, OperationInfo, EffectRef, ImplBlockInfo, ImplMethodInfo,
    ImplAssocTypeInfo, ImplAssocConstInfo, TraitInfo, TraitMethodInfo,
    TraitAssocTypeInfo, TraitAssocConstInfo,
};
use super::super::error::{TypeError, TypeErrorKind};
use super::super::resolve::{Binding, ScopeKind};
use crate::ast::Visibility;
use crate::typeck::const_eval;

impl<'a> TypeContext<'a> {
    /// Expand derive macros after collection but before type checking bodies.
    ///
    /// This generates impl blocks with derived methods (Debug, Clone, Eq, etc.)
    /// for all types with #[derive(...)] attributes. Must be called after
    /// resolve_program() and before check_all_bodies().
    pub fn expand_derives(&mut self) {
        if self.pending_derives.is_empty() {
            return;
        }

        // Calculate next DefId by finding max existing DefId + 1
        let next_def_id = self.resolver.def_info.keys()
            .map(|d| d.index)
            .max()
            .unwrap_or(0) + 1;

        let mut expander = crate::derive::DeriveExpander::new(
            &self.struct_defs,
            &self.enum_defs,
            &mut self.impl_blocks,
            &mut self.fn_sigs,
            &mut self.bodies,
            &mut self.method_self_types,
            &mut self.fn_bodies,
            next_def_id,
            self.next_body_id,
        );

        let pending = std::mem::take(&mut self.pending_derives);
        let generated_methods = expander.expand_all(pending);

        // Update next_body_id to avoid conflicts with bodies created during check_all_bodies
        self.next_body_id = expander.next_body_id();

        // Register generated methods with the resolver so they appear in HIR
        use super::super::resolve::DefInfo;
        use crate::hir::DefKind;
        use crate::ast::Visibility;

        for method in generated_methods {
            self.resolver.def_info.insert(method.def_id, DefInfo {
                kind: DefKind::Fn,
                name: method.name,
                span: method.span,
                ty: None, // Type is in fn_sigs
                parent: None,
                visibility: Visibility::Public,
            });
        }
    }

    /// Resolve names in a program.
    pub fn resolve_program(&mut self, program: &ast::Program) -> Result<(), Vec<Diagnostic>> {
        // Phase 0: Import prelude items (unless --no-std is set)
        if !self.no_std {
            self.import_prelude();
        }

        // Phase 1: Pre-register all type names so forward references work.
        // This allows struct A { b: B } to compile when B is defined after A.
        for decl in &program.declarations {
            if let Err(e) = self.register_type_name(decl) {
                self.errors.push(*e);
            }
        }

        // Phase 2: Collect all top-level definitions (now that all type names are known)
        for decl in &program.declarations {
            if let Err(e) = self.collect_declaration(decl) {
                self.errors.push(*e);
            }
        }

        // Phase 3: Resolve imports (after all declarations are collected)
        for import in &program.imports {
            if let Err(e) = self.resolve_import(import) {
                self.errors.push(*e);
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors.iter().map(|e| e.to_diagnostic()).collect());
        }

        Ok(())
    }

    /// Check for recursive types with infinite size.
    ///
    /// A struct that directly or indirectly contains itself without indirection
    /// (Box, &, *const, *mut) has infinite size and is rejected.
    pub fn check_recursive_types(&self) -> Result<(), Vec<Diagnostic>> {
        let mut errors = Vec::new();

        for (&def_id, struct_info) in &self.struct_defs {
            let mut visiting = HashSet::new();
            visiting.insert(def_id);
            if self.type_contains_cycle(def_id, &visiting) {
                let span = self.resolver.def_info.get(&def_id)
                    .map(|info| info.span)
                    .unwrap_or_else(crate::span::Span::dummy);
                errors.push(TypeError::new(
                    TypeErrorKind::RecursiveType { name: struct_info.name.clone() },
                    span,
                ).to_diagnostic());
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check if a type definition contains a cycle through its fields.
    /// `visiting` tracks the set of struct DefIds currently being explored (for cycle detection).
    fn type_contains_cycle(&self, check_def_id: DefId, visiting: &HashSet<DefId>) -> bool {
        if let Some(struct_info) = self.struct_defs.get(&check_def_id) {
            for field in &struct_info.fields {
                if self.field_type_has_cycle(&field.ty, visiting) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a field type creates a recursive cycle.
    /// Types behind indirection (Box, Vec, &, *const, *mut) are fine — they have
    /// a fixed stack size regardless of the inner type.
    fn field_type_has_cycle(&self, ty: &Type, visiting: &HashSet<DefId>) -> bool {
        match ty.kind() {
            // ADT types — check if this creates a cycle
            TypeKind::Adt { def_id, args } => {
                if visiting.contains(def_id) {
                    return true;
                }
                // Box<T> and Vec<T> are heap-allocated indirection types with fixed
                // stack sizes (pointer / ptr+len+cap). Their type arguments don't
                // contribute to the parent type's stack layout, so recursive
                // references through them are not infinite-size cycles.
                if self.is_heap_indirection(*def_id) {
                    return false;
                }
                // Check through this struct's fields recursively
                if let Some(struct_info) = self.struct_defs.get(def_id) {
                    let mut new_visiting = visiting.clone();
                    new_visiting.insert(*def_id);
                    for field in &struct_info.fields {
                        if self.field_type_has_cycle(&field.ty, &new_visiting) {
                            return true;
                        }
                    }
                }
                // Also check through enum variants
                if let Some(enum_info) = self.enum_defs.get(def_id) {
                    let mut new_visiting = visiting.clone();
                    new_visiting.insert(*def_id);
                    for variant in &enum_info.variants {
                        for field in &variant.fields {
                            if self.field_type_has_cycle(&field.ty, &new_visiting) {
                                return true;
                            }
                        }
                    }
                }
                // Check type arguments (e.g., Wrapper<S> where S is recursive)
                for arg in args {
                    if self.field_type_has_cycle(arg, visiting) {
                        return true;
                    }
                }
                false
            }
            // References and pointers provide indirection — no infinite size
            TypeKind::Ref { .. } | TypeKind::Ptr { .. } => false,
            // Tuples — check each element
            TypeKind::Tuple(elems) => {
                elems.iter().any(|e| self.field_type_has_cycle(e, visiting))
            }
            // Arrays — check element type
            TypeKind::Array { element, .. } => {
                self.field_type_has_cycle(element, visiting)
            }
            // Everything else (primitives, fn types, inference vars, etc.) — no cycle
            _ => false,
        }
    }

    /// Check if a DefId refers to a known heap-indirection type (Box, Vec).
    ///
    /// These types are heap-allocated with fixed stack sizes regardless of their
    /// type arguments: Box<T> is a single pointer (8 bytes on 64-bit), Vec<T> is
    /// {ptr, len, cap} (24 bytes). Recursive type references through them do not
    /// create infinite-size types.
    fn is_heap_indirection(&self, def_id: DefId) -> bool {
        self.box_def_id == Some(def_id) || self.vec_def_id == Some(def_id)
    }

    /// Import prelude items from the standard library.
    ///
    /// This automatically imports commonly used items so they're available
    /// without explicit `use` statements. Called automatically unless `--no-std`
    /// is specified.
    ///
    /// The prelude is loaded from `prelude.blood` or `prelude/mod.blood` in the
    /// stdlib path. All public items from the prelude are imported into the
    /// global scope.
    ///
    /// If no stdlib_path is set or the prelude doesn't exist, this silently
    /// succeeds (having no prelude is valid).
    fn import_prelude(&mut self) {
        // Built-in macros (println!, print!, format!, etc.) are already globally
        // available through builtin function registration in TypeContext::new().
        // The prelude adds standard library types and traits.

        // Check if stdlib_path is set
        let stdlib_path = match &self.stdlib_path {
            Some(p) => p.clone(),
            None => return, // No stdlib configured, nothing to import
        };

        // Look for prelude file: try prelude.blood, then prelude/mod.blood
        let prelude_file = if stdlib_path.is_file() {
            // stdlib_path points to a single file (e.g., std.blood)
            // Look for prelude.blood in the same directory
            if let Some(parent) = stdlib_path.parent() {
                let prelude_path = parent.join("prelude.blood");
                let alt_path = parent.join("prelude").join("mod.blood");
                if prelude_path.exists() {
                    prelude_path
                } else if alt_path.exists() {
                    alt_path
                } else {
                    return; // No prelude file found, that's fine
                }
            } else {
                return;
            }
        } else {
            // stdlib_path is a directory
            let prelude_path = stdlib_path.join("prelude.blood");
            let alt_path = stdlib_path.join("prelude").join("mod.blood");
            if prelude_path.exists() {
                prelude_path
            } else if alt_path.exists() {
                alt_path
            } else {
                return; // No prelude file found, that's fine
            }
        };

        // Read and parse the prelude file
        let prelude_source = match std::fs::read_to_string(&prelude_file) {
            Ok(s) => s,
            Err(_) => return, // Can't read prelude, silently continue
        };

        // Parse the prelude
        let interner = std::mem::take(&mut self.interner);
        let mut parser = crate::parser::Parser::with_interner(&prelude_source, interner);
        let prelude_ast = parser.parse_program();
        self.interner = parser.take_interner();

        let prelude_ast = match prelude_ast {
            Ok(ast) => ast,
            Err(_) => return, // Parse error in prelude, silently continue
        };

        // Collect prelude declarations in a temporary scope, then re-export to global
        // This ensures proper DefId assignment while making items globally available
        let prelude_span = crate::span::Span::new(0, 0, 0, 0);
        self.resolver.push_scope(ScopeKind::Module, prelude_span);

        // Pre-register type names first (for forward references within prelude)
        for decl in &prelude_ast.declarations {
            if let Err(e) = self.register_type_name(decl) {
                self.errors.push(*e);
            }
        }

        // Collect all declarations
        for decl in &prelude_ast.declarations {
            if let Err(e) = self.collect_declaration(decl) {
                self.errors.push(*e);
            }
        }

        // Get all bindings from the prelude scope before popping
        let prelude_bindings = self.resolver.get_current_scope_bindings();
        let prelude_type_bindings = self.resolver.get_current_scope_type_bindings();

        self.resolver.pop_scope();

        // Re-import public value bindings into global scope
        // This makes prelude items available without qualification
        for (name, binding) in prelude_bindings {
            // Extract DefId(s) from the binding
            let def_ids: Vec<DefId> = match &binding {
                Binding::Def(def_id) => vec![*def_id],
                Binding::Methods(def_ids) => def_ids.clone(),
                Binding::Local { .. } => continue, // Don't import locals
            };

            for def_id in def_ids {
                // Only import public items (check visibility in def_info)
                if let Some(def_info) = self.resolver.def_info.get(&def_id) {
                    if def_info.visibility == crate::ast::Visibility::Public {
                        // Import into global scope
                        if let Err(e) = self.resolver.import_binding(name.clone(), def_id, prelude_span) {
                            // Ignore conflicts with user-defined items (user takes precedence)
                            // DuplicateDefinition errors are expected when user code shadows prelude
                            if !matches!(e.kind, TypeErrorKind::DuplicateDefinition { .. }) {
                                self.errors.push(*e);
                            }
                        }
                        // Only import the first def_id for a given name to avoid conflicts
                        break;
                    }
                }
            }
        }

        // Re-import public type bindings into global scope
        for (name, def_id) in prelude_type_bindings {
            // Only import public types (check visibility in def_info)
            if let Some(def_info) = self.resolver.def_info.get(&def_id) {
                if def_info.visibility == crate::ast::Visibility::Public {
                    // Import type into global scope
                    if let Err(e) = self.resolver.import_type_binding(name.clone(), def_id, prelude_span) {
                        // Ignore conflicts with user-defined types (user takes precedence)
                        if !matches!(e.kind, TypeErrorKind::DuplicateDefinition { .. }) {
                            self.errors.push(*e);
                        }
                    }
                }
            }
        }
    }

    /// Pre-register type names for forward reference support.
    /// This only registers the name -> DefId mapping, without resolving field types.
    fn register_type_name(&mut self, decl: &ast::Declaration) -> Result<(), Box<TypeError>> {
        match decl {
            ast::Declaration::Struct(s) => {
                let name = self.symbol_to_string(s.name.node);
                let def_id = self.resolver.define_item(
                    name.clone(),
                    hir::DefKind::Struct,
                    s.span,
                )?;
                self.resolver.define_type(name, def_id, s.span)?;
            }
            ast::Declaration::Enum(e) => {
                let name = self.symbol_to_string(e.name.node);
                let def_id = self.resolver.define_item(
                    name.clone(),
                    hir::DefKind::Enum,
                    e.span,
                )?;
                self.resolver.define_type(name, def_id, e.span)?;
            }
            ast::Declaration::Type(t) => {
                let name = self.symbol_to_string(t.name.node);
                let def_id = self.resolver.define_item(
                    name.clone(),
                    hir::DefKind::TypeAlias,
                    t.span,
                )?;
                self.resolver.define_type(name, def_id, t.span)?;
            }
            ast::Declaration::Effect(e) => {
                let name = self.symbol_to_string(e.name.node);
                let def_id = self.resolver.define_item(
                    name.clone(),
                    hir::DefKind::Effect,
                    e.span,
                )?;
                self.resolver.define_type(name, def_id, e.span)?;
            }
            ast::Declaration::Trait(t) => {
                let name = self.symbol_to_string(t.name.node);
                let def_id = self.resolver.define_item(
                    name.clone(),
                    hir::DefKind::Trait,
                    t.span,
                )?;
                self.resolver.define_type(name, def_id, t.span)?;
            }
            // Other declarations don't introduce type names that can be forward-referenced
            _ => {}
        }
        Ok(())
    }

    /// Resolve an import statement.
    fn resolve_import(&mut self, import: &ast::Import) -> Result<(), Box<TypeError>> {
        match import {
            ast::Import::Simple { path, alias, visibility, span } => {
                self.resolve_simple_import(path, alias.as_ref(), *visibility, *span)
            }
            ast::Import::Group { path, items, visibility, span } => {
                self.resolve_group_import(path, items, *visibility, *span)
            }
            ast::Import::Glob { path, visibility, span } => {
                self.resolve_glob_import(path, *visibility, *span)
            }
        }
    }

    /// Resolve a simple import: `use std.mem.allocate;` or `use std.mem.allocate as alloc;`
    fn resolve_simple_import(
        &mut self,
        path: &ast::ModulePath,
        alias: Option<&crate::span::Spanned<ast::Symbol>>,
        visibility: ast::Visibility,
        span: crate::span::Span,
    ) -> Result<(), Box<TypeError>> {
        // Resolve the path to find the target definition
        let def_id = self.resolve_import_path(path)?;

        // Determine the local name (last segment or alias)
        let local_name = if let Some(alias_spanned) = alias {
            self.symbol_to_string(alias_spanned.node)
        } else {
            // Use the last segment of the path as the name
            if let Some(last) = path.segments.last() {
                self.symbol_to_string(last.node)
            } else {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::ImportError {
                        message: "empty import path".to_string(),
                    },
                    span,
                )));
            }
        };

        // Determine if this is a type or value import based on the DefKind
        // Struct and Enum need to be imported in BOTH namespaces:
        //   - Type namespace: for type annotations like `let x: Point`
        //   - Value namespace: for constructors like `Point { x: 0 }`
        if let Some(info) = self.resolver.def_info.get(&def_id) {
            match info.kind {
                hir::DefKind::Struct | hir::DefKind::Enum => {
                    // Import as type (for type annotations)
                    self.resolver.import_type_binding(local_name.clone(), def_id, span)?;
                    // Also import as value (for constructors)
                    // Ignore duplicate error since it might already exist in value namespace
                    let _ = self.resolver.import_binding(local_name.clone(), def_id, span);
                }
                hir::DefKind::TypeAlias | hir::DefKind::Effect | hir::DefKind::Trait => {
                    // Pure types - only type namespace
                    self.resolver.import_type_binding(local_name.clone(), def_id, span)?;
                }
                _ => {
                    // Values (functions, constants, etc.) - only value namespace
                    self.resolver.import_binding(local_name.clone(), def_id, span)?;
                }
            }
        } else {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::ImportError {
                    message: "cannot find definition for import path".to_string(),
                },
                span,
            )));
        }

        // For `pub use` re-exports, add this item to the current module's exports.
        // For private `use` imports, track them so they're visible inside the module.
        if matches!(visibility, ast::Visibility::Public | ast::Visibility::PublicCrate) {
            self.register_reexport(local_name, def_id, visibility, span);
        } else {
            self.register_private_import(local_name, def_id);
        }

        Ok(())
    }

    /// Resolve a group import: `use std.iter::{Iterator, IntoIterator};`
    fn resolve_group_import(
        &mut self,
        path: &ast::ModulePath,
        items: &[ast::ImportItem],
        visibility: ast::Visibility,
        span: crate::span::Span,
    ) -> Result<(), Box<TypeError>> {
        // For group imports, we need to resolve the base path as a module
        // and then import each item from that module
        let base_module_id = self.resolve_module_path(path)?;

        for item in items {
            let item_name = self.symbol_to_string(item.name.node);

            // Look up the item in the module
            if let Some(def_id) = self.lookup_in_module(base_module_id, &item_name) {
                let local_name = if let Some(alias) = &item.alias {
                    self.symbol_to_string(alias.node)
                } else {
                    item_name.clone()
                };

                // Import based on def kind (struct/enum need both namespaces)
                if let Some(info) = self.resolver.def_info.get(&def_id) {
                    match info.kind {
                        hir::DefKind::Struct | hir::DefKind::Enum => {
                            // Import as type and value (for annotations and constructors)
                            self.resolver.import_type_binding(local_name.clone(), def_id, span)?;
                            let _ = self.resolver.import_binding(local_name.clone(), def_id, span);
                        }
                        hir::DefKind::TypeAlias | hir::DefKind::Effect | hir::DefKind::Trait => {
                            self.resolver.import_type_binding(local_name.clone(), def_id, span)?;
                        }
                        _ => {
                            self.resolver.import_binding(local_name.clone(), def_id, span)?;
                        }
                    }
                }

                // For `pub use` re-exports, add this item to the current module's exports.
                // For private `use` imports, track them so they're visible inside the module.
                if matches!(visibility, ast::Visibility::Public | ast::Visibility::PublicCrate) {
                    self.register_reexport(local_name, def_id, visibility, span);
                } else {
                    self.register_private_import(local_name, def_id);
                }
            } else {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::ImportError {
                        message: format!("cannot find `{}` in module", item_name),
                    },
                    span,
                )));
            }
        }

        Ok(())
    }

    /// Resolve a glob import: `use std.ops::*;`
    fn resolve_glob_import(
        &mut self,
        path: &ast::ModulePath,
        visibility: ast::Visibility,
        span: crate::span::Span,
    ) -> Result<(), Box<TypeError>> {
        // Resolve the path as a module and import all public items
        let module_id = self.resolve_module_path(path)?;

        // Get all public items from the module
        let items = self.get_module_public_items(module_id);

        for (name, def_id) in items {
            // Import based on def kind (skip if already exists)
            // Struct and enum need both type and value namespaces
            if let Some(info) = self.resolver.def_info.get(&def_id) {
                let result = match info.kind {
                    hir::DefKind::Struct | hir::DefKind::Enum => {
                        // Import as type first
                        let type_result = self.resolver.import_type_binding(name.clone(), def_id, span);
                        // Also import as value (ignore duplicate)
                        let _ = self.resolver.import_binding(name.clone(), def_id, span);
                        type_result
                    }
                    hir::DefKind::TypeAlias | hir::DefKind::Effect | hir::DefKind::Trait => {
                        self.resolver.import_type_binding(name.clone(), def_id, span)
                    }
                    _ => {
                        self.resolver.import_binding(name.clone(), def_id, span)
                    }
                };

                // For glob imports, we silently skip duplicates
                if let Err(e) = result {
                    if !matches!(e.kind, TypeErrorKind::DuplicateDefinition { .. }) {
                        return Err(e);
                    }
                }

                // For `pub use *` re-exports, add all items to the current module's exports.
                // For private `use *` imports, track them so they're visible inside the module.
                if matches!(visibility, ast::Visibility::Public | ast::Visibility::PublicCrate) {
                    self.register_reexport(name, def_id, visibility, span);
                } else {
                    self.register_private_import(name, def_id);
                }
            }
        }

        Ok(())
    }

    /// Register a re-exported item in the current module.
    ///
    /// When `pub use path::item;` is used, the imported item should be
    /// re-exported from the current module, making it accessible to external
    /// modules that import from this module.
    fn register_reexport(
        &mut self,
        name: String,
        def_id: DefId,
        visibility: ast::Visibility,
        _span: crate::span::Span,
    ) {
        // Track re-exports in the current module's reexports list.
        // This is stored separately during collection and merged into
        // the ModuleInfo when the module is finalized.
        //
        // The key insight is that the DefId already exists (from the original
        // definition), we just need to make it visible under a new name in
        // the current module's namespace.
        if self.current_module.is_some() {
            // Add to current module's reexports list
            self.current_module_reexports.push((name, def_id, visibility));
        }
        // For top-level re-exports (not inside a module), the item is already
        // in global scope from the import, so no additional action needed.
    }

    /// Register a private import in the current module.
    ///
    /// When `use path::item;` (without `pub`) is used inside a module, the imported
    /// item must be visible to function bodies within that module, even though it's
    /// not exported to external modules. Private imports are tracked separately and
    /// injected into scope during body checking via `inject_module_bindings`.
    fn register_private_import(&mut self, name: String, def_id: DefId) {
        if self.current_module.is_some() {
            self.current_module_private_imports.push((name, def_id));
        }
        // For top-level private imports (not inside a module), the item is already
        // in global scope from the import, so no additional action needed.
    }

    /// Resolve an import path to find the target definition.
    fn resolve_import_path(&mut self, path: &ast::ModulePath) -> Result<DefId, Box<TypeError>> {
        if path.segments.is_empty() {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::ImportError {
                    message: "empty import path".to_string(),
                },
                path.span,
            )));
        }

        // Walk the path segments
        let mut current_scope_names: Vec<String> = path.segments.iter()
            .map(|s| self.symbol_to_string(s.node))
            .collect();

        // Guard: empty path is invalid
        if current_scope_names.is_empty() {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::ImportError {
                    message: "empty import path".to_string(),
                },
                path.span,
            )));
        }

        // The last segment is the item we're importing
        // SAFETY: checked non-empty above
        let item_name = current_scope_names.pop().expect("BUG: checked non-empty above");

        if current_scope_names.is_empty() {
            // Simple case: just a name (e.g., `use foo;`)
            // Look it up in the current scope
            if let Some(Binding::Def(def_id)) = self.resolver.lookup(&item_name) {
                return Ok(def_id);
            }
            if let Some(def_id) = self.resolver.lookup_type(&item_name) {
                return Ok(def_id);
            }
            return Err(Box::new(TypeError::new(
                TypeErrorKind::ImportError {
                    message: format!("cannot find `{}` in scope", item_name),
                },
                path.span,
            )));
        }

        // For paths like `std.mem.allocate`, we need to resolve the module path first
        // then look up the item in that module
        let module_path = ast::ModulePath {
            segments: path.segments[..path.segments.len() - 1].to_vec(),
            span: path.span,
        };

        let module_id = self.resolve_module_path(&module_path)?;

        // Look up the item in the module
        if let Some(def_id) = self.lookup_in_module(module_id, &item_name) {
            Ok(def_id)
        } else {
            Err(Box::new(TypeError::new(
                TypeErrorKind::ImportError {
                    message: format!("cannot find `{}` in module", item_name),
                },
                path.span,
            )))
        }
    }

    /// Resolve a module path to find the module DefId.
    fn resolve_module_path(&self, path: &ast::ModulePath) -> Result<DefId, Box<TypeError>> {
        if path.segments.is_empty() {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::ImportError {
                    message: "empty module path".to_string(),
                },
                path.span,
            )));
        }

        let first_name = self.symbol_to_string(path.segments[0].node);

        // Look up the first segment
        let mut current_id = if let Some(Binding::Def(def_id)) = self.resolver.lookup(&first_name) {
            def_id
        } else if let Some(def_id) = self.resolver.lookup_type(&first_name) {
            def_id
        } else {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::ImportError {
                    message: format!("cannot find module `{}`", first_name),
                },
                path.span,
            )));
        };

        // Walk the remaining segments
        for segment in &path.segments[1..] {
            let name = self.symbol_to_string(segment.node);

            if let Some(def_id) = self.lookup_in_module(current_id, &name) {
                current_id = def_id;
            } else {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::ImportError {
                        message: format!("cannot find `{}` in module path", name),
                    },
                    path.span,
                )));
            }
        }

        Ok(current_id)
    }

    /// Look up an item by name in a module.
    fn lookup_in_module(&self, module_id: DefId, name: &str) -> Option<DefId> {
        // Check if we have module info for this DefId
        if let Some(module_def) = self.module_defs.get(&module_id) {
            // Look in the module's directly defined items
            for &item_id in &module_def.items {
                if let Some(info) = self.resolver.def_info.get(&item_id) {
                    if info.name == name {
                        return Some(item_id);
                    }
                }
            }

            // Also look in re-exported items (from `pub use`)
            for (reexport_name, def_id, _vis) in &module_def.reexports {
                if reexport_name == name {
                    return Some(*def_id);
                }
            }
        }
        None
    }

    /// Get all public items from a module.
    fn get_module_public_items(&self, module_id: DefId) -> Vec<(String, DefId)> {
        use crate::ast::Visibility;

        let mut items = Vec::new();

        if let Some(module_def) = self.module_defs.get(&module_id) {
            // Include directly defined public items
            for &item_id in &module_def.items {
                if let Some(info) = self.resolver.def_info.get(&item_id) {
                    // Only include public items (Public or PublicCrate from outside)
                    match info.visibility {
                        Visibility::Public | Visibility::PublicCrate => {
                            items.push((info.name.clone(), item_id));
                        }
                        Visibility::Private | Visibility::PublicSuper | Visibility::PublicSelf => {
                            // Private items are not exported
                        }
                    }
                }
            }

            // Include re-exported items (from `pub use`)
            for (name, def_id, vis) in &module_def.reexports {
                match vis {
                    Visibility::Public | Visibility::PublicCrate => {
                        items.push((name.clone(), *def_id));
                    }
                    Visibility::Private | Visibility::PublicSuper | Visibility::PublicSelf => {
                        // Non-public re-exports are not visible externally
                    }
                }
            }
        }

        items
    }

    /// Collect a declaration.
    pub(crate) fn collect_declaration(&mut self, decl: &ast::Declaration) -> Result<(), Box<TypeError>> {
        match decl {
            ast::Declaration::Function(f) => self.collect_function(f),
            ast::Declaration::Struct(s) => self.collect_struct(s),
            ast::Declaration::Enum(e) => self.collect_enum(e),
            ast::Declaration::Const(c) => self.collect_const(c),
            ast::Declaration::Static(s) => self.collect_static(s),
            ast::Declaration::Effect(e) => self.collect_effect(e),
            ast::Declaration::Handler(h) => self.collect_handler(h),
            ast::Declaration::Type(t) => self.collect_type_alias(t),
            ast::Declaration::Impl(i) => self.collect_impl_block(i),
            ast::Declaration::Trait(t) => self.collect_trait(t),
            ast::Declaration::Bridge(b) => self.collect_bridge(b),
            ast::Declaration::Module(m) => self.collect_module(m),
            ast::Declaration::Macro(m) => self.collect_macro(m),
            ast::Declaration::Use(import) => self.resolve_import(import),
        }
    }

    /// Collect a macro declaration.
    ///
    /// Macros are expanded in a separate pre-typeck phase (macro_expand.rs).
    /// The type checker does not need to register macros — this function exists
    /// because the declaration visitor visits all declaration types. By the time
    /// type checking runs, all macro calls have already been expanded in place.
    pub(crate) fn collect_macro(&mut self, _macro_decl: &ast::MacroDecl) -> Result<(), Box<TypeError>> {
        Ok(())
    }

    /// Collect a bridge declaration (FFI).
    pub(crate) fn collect_bridge(&mut self, bridge: &ast::BridgeDecl) -> Result<(), Box<TypeError>> {
        use super::{
            BridgeInfo, BridgeLinkSpec, BridgeLinkKind, BridgeFnInfo, BridgeOpaqueInfo,
            BridgeTypeAliasInfo, BridgeStructInfo, BridgeFieldInfo, BridgeEnumInfo,
            BridgeEnumVariantInfo, BridgeUnionInfo, BridgeConstInfo, BridgeCallbackInfo,
        };

        let bridge_name = self.symbol_to_string(bridge.name.node);
        let abi = bridge.language.node.clone();

        let mut link_specs = Vec::new();
        let mut extern_fns = Vec::new();
        let mut opaque_types = Vec::new();
        let mut type_aliases = Vec::new();
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        let mut unions = Vec::new();
        let mut consts = Vec::new();
        let mut callbacks = Vec::new();

        for item in &bridge.items {
            match item {
                ast::BridgeItem::Link(link) => {
                    let kind = match link.kind {
                        Some(ast::LinkKind::Dylib) => BridgeLinkKind::Dylib,
                        Some(ast::LinkKind::Static) => BridgeLinkKind::Static,
                        Some(ast::LinkKind::Framework) => BridgeLinkKind::Framework,
                        None => BridgeLinkKind::Dylib, // Default to dylib
                    };
                    link_specs.push(BridgeLinkSpec {
                        name: link.name.clone(),
                        kind,
                        wasm_import_module: link.wasm_import_module.clone(),
                    });
                }
                ast::BridgeItem::Function(func) => {
                    let name = self.symbol_to_string(func.name.node);
                    // Use define_namespaced_item to avoid conflicts with global builtins.
                    // Bridge functions are accessed via namespace (e.g., libc::abs).
                    let def_id = self.resolver.define_namespaced_item(
                        name.clone(),
                        hir::DefKind::Fn,
                        func.span,
                    );

                    // Convert parameter types
                    let params: Vec<_> = func.params.iter()
                        .map(|p| self.ast_type_to_hir_type(&p.ty))
                        .collect::<Result<_, _>>()?;

                    let return_ty = match &func.return_type {
                        Some(ty) => self.ast_type_to_hir_type(ty)?,
                        None => hir::Type::unit(),
                    };

                    // Store function signature for type checking
                    self.fn_sigs.insert(def_id, hir::FnSig::new(params.clone(), return_ty.clone()));

                    // Extract link_name from attributes if present
                    let link_name = self.extract_link_name_from_attrs(&func.attrs);

                    extern_fns.push(BridgeFnInfo {
                        def_id,
                        name,
                        params,
                        return_ty,
                        link_name,
                        is_variadic: func.is_variadic,
                        span: func.span,
                    });
                }
                ast::BridgeItem::OpaqueType(opaque) => {
                    let name = self.symbol_to_string(opaque.name.node);
                    let def_id = self.resolver.define_item(
                        name.clone(),
                        hir::DefKind::Struct,
                        opaque.span,
                    )?;
                    opaque_types.push(BridgeOpaqueInfo {
                        def_id,
                        name,
                        span: opaque.span,
                    });
                }
                ast::BridgeItem::TypeAlias(alias) => {
                    let name = self.symbol_to_string(alias.name.node);
                    let def_id = self.resolver.define_item(
                        name.clone(),
                        hir::DefKind::TypeAlias,
                        alias.span,
                    )?;
                    let ty = self.ast_type_to_hir_type(&alias.ty)?;
                    type_aliases.push(BridgeTypeAliasInfo {
                        def_id,
                        name,
                        ty,
                        span: alias.span,
                    });
                }
                ast::BridgeItem::Struct(s) => {
                    let name = self.symbol_to_string(s.name.node);
                    let def_id = self.resolver.define_item(
                        name.clone(),
                        hir::DefKind::Struct,
                        s.span,
                    )?;
                    let bridge_fields: Vec<_> = s.fields.iter()
                        .map(|f| {
                            let field_name = self.symbol_to_string(f.name.node);
                            let ty = self.ast_type_to_hir_type(&f.ty)?;
                            Ok(BridgeFieldInfo {
                                name: field_name,
                                ty,
                                span: f.span,
                            })
                        })
                        .collect::<Result<_, Box<TypeError>>>()?;

                    // Also register in struct_defs for field access and struct literals
                    let struct_fields: Vec<super::FieldInfo> = bridge_fields.iter()
                        .enumerate()
                        .map(|(i, f)| super::FieldInfo {
                            name: f.name.clone(),
                            ty: f.ty.clone(),
                            index: i as u32,
                        })
                        .collect();

                    self.struct_defs.insert(def_id, super::StructInfo {
                        name: name.clone(),
                        fields: struct_fields,
                        generics: Vec::new(), // Bridge structs don't have generics
                    });

                    // Extract packed and align from attributes
                    let (is_packed, align) = self.extract_struct_attrs(&s.attrs);

                    structs.push(BridgeStructInfo {
                        def_id,
                        name,
                        fields: bridge_fields,
                        is_packed,
                        align,
                        span: s.span,
                    });
                }
                ast::BridgeItem::Enum(e) => {
                    let name = self.symbol_to_string(e.name.node);
                    let def_id = self.resolver.define_item(
                        name.clone(),
                        hir::DefKind::Enum,
                        e.span,
                    )?;

                    // Extract repr type from attributes, default to i32
                    let repr = self.extract_repr_from_attrs(&e.attrs)
                        .unwrap_or_else(hir::Type::i32);

                    let variants: Vec<_> = e.variants.iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let var_name = self.symbol_to_string(v.name.node);
                            // If discriminant is specified, try to parse it, otherwise use index
                            let value = v.discriminant.as_ref()
                                .and_then(Self::literal_to_i64)
                                .unwrap_or(i as i64);
                            BridgeEnumVariantInfo {
                                name: var_name,
                                value,
                                span: v.span,
                            }
                        })
                        .collect();
                    enums.push(BridgeEnumInfo {
                        def_id,
                        name,
                        repr,
                        variants,
                        span: e.span,
                    });
                }
                ast::BridgeItem::Union(u) => {
                    let name = self.symbol_to_string(u.name.node);
                    let def_id = self.resolver.define_item(
                        name.clone(),
                        hir::DefKind::Struct, // Unions are similar to structs in DefKind
                        u.span,
                    )?;
                    let fields: Vec<_> = u.fields.iter()
                        .map(|f| {
                            let field_name = self.symbol_to_string(f.name.node);
                            let ty = self.ast_type_to_hir_type(&f.ty)?;
                            Ok(BridgeFieldInfo {
                                name: field_name,
                                ty,
                                span: f.span,
                            })
                        })
                        .collect::<Result<_, Box<TypeError>>>()?;
                    unions.push(BridgeUnionInfo {
                        def_id,
                        name,
                        fields,
                        span: u.span,
                    });
                }
                ast::BridgeItem::Callback(cb) => {
                    let name = self.symbol_to_string(cb.name.node);
                    let def_id = self.resolver.define_item(
                        name.clone(),
                        hir::DefKind::TypeAlias,
                        cb.span,
                    )?;
                    let params: Vec<_> = cb.params.iter()
                        .map(|ty| self.ast_type_to_hir_type(ty))
                        .collect::<Result<_, _>>()?;
                    let return_ty = match &cb.return_type {
                        Some(ty) => self.ast_type_to_hir_type(ty)?,
                        None => hir::Type::unit(),
                    };
                    callbacks.push(BridgeCallbackInfo {
                        def_id,
                        name,
                        params,
                        return_ty,
                        span: cb.span,
                    });
                }
                ast::BridgeItem::Const(c) => {
                    let name = self.symbol_to_string(c.name.node);
                    // Use define_namespaced_item to avoid conflicts with global names.
                    // Bridge constants are accessed via namespace (e.g., libc::MAX_VALUE).
                    let def_id = self.resolver.define_namespaced_item(
                        name.clone(),
                        hir::DefKind::Const,
                        c.span,
                    );
                    let ty = self.ast_type_to_hir_type(&c.ty)?;
                    // Convert literal to i64
                    let value = Self::literal_to_i64(&c.value).unwrap_or(0);
                    consts.push(BridgeConstInfo {
                        def_id,
                        name,
                        ty,
                        value,
                        span: c.span,
                    });
                }
            }
        }

        self.bridge_defs.push(BridgeInfo {
            name: bridge_name,
            abi,
            link_specs,
            extern_fns,
            opaque_types,
            type_aliases,
            structs,
            enums,
            unions,
            consts,
            callbacks,
            span: bridge.span,
        });

        Ok(())
    }

    /// Extract link_name attribute from a list of attributes.
    ///
    /// Parses: `#[link_name = "actual_name"]`
    fn extract_link_name_from_attrs(&self, attrs: &[ast::Attribute]) -> Option<String> {
        for attr in attrs {
            // Check if this is a link_name attribute
            if attr.path.len() == 1 {
                let name = self.symbol_to_string(attr.path[0].node);
                if name == "link_name" {
                    if let Some(ast::AttributeArgs::Eq(lit)) = &attr.args {
                        if let ast::LiteralKind::String(s) = &lit.kind {
                            return Some(s.clone());
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract is_packed and align from struct attributes.
    ///
    /// Parses:
    /// - `#[repr(packed)]` -> is_packed = true
    /// - `#[repr(align(N))]` -> align = Some(N)
    /// - `#[repr(C, packed)]` -> is_packed = true
    fn extract_struct_attrs(&self, attrs: &[ast::Attribute]) -> (bool, Option<u32>) {
        let mut is_packed = false;
        let mut align = None;

        for attr in attrs {
            if attr.path.len() == 1 {
                let name = self.symbol_to_string(attr.path[0].node);
                if name == "repr" {
                    if let Some(ast::AttributeArgs::List(args)) = &attr.args {
                        for arg in args {
                            match arg {
                                ast::AttributeArg::Ident(ident) => {
                                    let ident_name = self.symbol_to_string(ident.node);
                                    if ident_name == "packed" {
                                        is_packed = true;
                                    }
                                }
                                ast::AttributeArg::KeyValue(key, value) => {
                                    let key_name = self.symbol_to_string(key.node);
                                    if key_name == "align" {
                                        if let ast::LiteralKind::Int { value: n, .. } = &value.kind {
                                            align = Some(*n as u32);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        (is_packed, align)
    }

    /// Extract repr type from enum attributes.
    ///
    /// Parses:
    /// - `#[repr(i8)]`, `#[repr(i16)]`, `#[repr(i32)]`, `#[repr(i64)]`
    /// - `#[repr(u8)]`, `#[repr(u16)]`, `#[repr(u32)]`, `#[repr(u64)]`
    /// - `#[repr(isize)]`, `#[repr(usize)]`
    fn extract_repr_from_attrs(&self, attrs: &[ast::Attribute]) -> Option<hir::Type> {
        use crate::hir::ty::{TypeKind, PrimitiveTy};
        use crate::hir::def::{IntTy, UintTy};

        for attr in attrs {
            if attr.path.len() == 1 {
                let name = self.symbol_to_string(attr.path[0].node);
                if name == "repr" {
                    if let Some(ast::AttributeArgs::List(args)) = &attr.args {
                        for arg in args {
                            if let ast::AttributeArg::Ident(ident) = arg {
                                let ident_name = self.symbol_to_string(ident.node);
                                return match ident_name.as_str() {
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
                                    // Skip C and other non-type specifiers
                                    "C" | "packed" | "transparent" => continue,
                                    _ => None,
                                };
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract derive macros from attributes.
    ///
    /// Parses: `#[derive(Debug, Clone, Eq, PartialEq, Default, Hash)]`
    fn extract_derives(&self, attrs: &[ast::Attribute]) -> Vec<crate::derive::DeriveKind> {
        use crate::derive::DeriveKind;

        let mut derives = Vec::new();

        for attr in attrs {
            if attr.path.len() == 1 {
                let name = self.symbol_to_string(attr.path[0].node);
                if name == "derive" {
                    if let Some(ast::AttributeArgs::List(args)) = &attr.args {
                        for arg in args {
                            if let ast::AttributeArg::Ident(ident) = arg {
                                let ident_name = self.symbol_to_string(ident.node);
                                if let Some(kind) = DeriveKind::from_str(&ident_name) {
                                    derives.push(kind);
                                }
                            }
                        }
                    }
                }
            }
        }

        derives
    }

    /// Convert a literal to i64 if possible.
    fn literal_to_i64(lit: &ast::Literal) -> Option<i64> {
        match &lit.kind {
            ast::LiteralKind::Int { value, .. } => Some(*value as i64),
            ast::LiteralKind::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }

    /// Collect a function declaration.
    ///
    /// This uses `define_function` which supports multiple dispatch - multiple
    /// functions with the same name are allowed and will form a method family.
    pub(crate) fn collect_function(&mut self, func: &ast::FnDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(func.name.node);
        // Use define_function for multiple dispatch support
        let def_id = self.resolver.define_function(
            name.clone(),
            func.span,
            func.vis,
        )?;

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Register generic type parameters before processing parameter types
        // This allows type references like `T` to be resolved in the function signature
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let saved_const_params = std::mem::take(&mut self.const_params);
        let saved_lifetime_params = std::mem::take(&mut self.lifetime_params);
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = func.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var_id = TyVarId(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var_id);
                        generics_vec.push(ty_var_id);
                        // Register trait bounds for method lookup on type parameters
                        self.register_type_param_bounds(ty_var_id, &type_param.bounds);
                    }
                    ast::GenericParam::Lifetime(lifetime_param) => {
                        let param_name = self.symbol_to_string(lifetime_param.name.node);
                        let lifetime_id = hir::LifetimeId::new(self.next_lifetime_id);
                        self.next_lifetime_id += 1;
                        self.lifetime_params.insert(param_name, lifetime_id);
                    }
                    ast::GenericParam::Const(const_param) => {
                        let param_name = self.symbol_to_string(const_param.name.node);
                        let const_id = hir::ConstParamId::new(self.next_const_param_id);
                        self.next_const_param_id += 1;
                        self.const_params.insert(param_name, const_id);
                    }
                }
            }
        }

        // Build function signature (now with generics in scope)
        let mut param_types = Vec::new();
        for param in &func.params {
            let ty = self.ast_type_to_hir_type(&param.ty)?;
            param_types.push(ty);
        }

        let return_type = if let Some(ref ret) = func.return_type {
            self.ast_type_to_hir_type(ret)?
        } else {
            Type::unit()
        };

        // Parse and store the function's effect annotation BEFORE restoring generic params
        // This ensures type parameters like T are in scope for effect annotations like {Emit<T>}
        if let Some(ref effect_row) = func.effects {
            let (effects, row_var) = self.parse_effect_row(effect_row)?;
            if !effects.is_empty() || row_var.is_some() {
                self.fn_effects.insert(def_id, effects);
            }
            // Store the row variable if present (for effect row polymorphism)
            if row_var.is_some() {
                self.fn_effect_row_var.insert(def_id, row_var);
            }
        }

        // Process where clause for trait bounds enforcement
        // This is done BEFORE restoring generic params so type parameters are in scope
        if let Some(ref where_clause) = func.where_clause {
            let mut predicates = Vec::new();
            for predicate in &where_clause.predicates {
                match predicate {
                    ast::WherePredicate::TypeBound { ty, bounds, span: bound_span } => {
                        // Convert the subject type (e.g., T in `T: Trait`)
                        let subject_ty = self.ast_type_to_hir_type(ty)?;

                        // Convert each bound to a trait DefId using resolve_trait_bound
                        let mut trait_bounds = Vec::new();
                        for bound in bounds {
                            if let Some(trait_def_id) = self.resolve_trait_bound(bound) {
                                trait_bounds.push(trait_def_id);
                            } else {
                                // Report error for unknown trait in where clause
                                let trait_name = match &bound.kind {
                                    ast::TypeKind::Path(type_path) => {
                                        self.symbol_to_string(type_path.segments[0].name.node)
                                    }
                                    _ => "unknown".to_string(),
                                };
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::TraitNotFound { name: trait_name },
                                    *bound_span,
                                )));
                            }
                        }

                        if !trait_bounds.is_empty() {
                            predicates.push(super::WhereClausePredicate {
                                subject_ty,
                                trait_bounds,
                            });
                        }
                    }
                    ast::WherePredicate::Lifetime { .. } => {
                        // Lifetime bounds (e.g., 'a: 'b) are parsed but not enforced.
                        // Blood uses generational pointers for memory safety rather than
                        // Rust-style borrow checking, so lifetime outlives constraints
                        // are accepted syntactically but do not affect type checking.
                    }
                }
            }
            if !predicates.is_empty() {
                self.fn_where_bounds.insert(def_id, predicates);
            }
        }

        // Collect const generics before restoring scope
        let const_generics_vec: Vec<hir::ConstParamId> = self.const_params.values().copied().collect();

        // Restore previous generic params scope (after processing signature including effects)
        self.generic_params = saved_generic_params;
        self.const_params = saved_const_params;
        self.lifetime_params = saved_lifetime_params;

        let mut sig = hir::FnSig::new(param_types, return_type);
        sig.generics = generics_vec;
        sig.const_generics = const_generics_vec;
        self.fn_sigs.insert(def_id, sig);

        // Check for duplicate function signatures (same name AND same parameter types).
        // Multiple dispatch allows same-name functions with different signatures,
        // but identical signatures are an error.
        if let Some(Binding::Methods(ref methods)) = self.resolver.current_scope().bindings.get(&name) {
            for &other_id in methods {
                if other_id == def_id {
                    continue;
                }
                if let Some(other_sig) = self.fn_sigs.get(&other_id) {
                    let current_sig = &self.fn_sigs[&def_id];
                    if other_sig.inputs.len() == current_sig.inputs.len()
                        && other_sig.inputs.iter().zip(current_sig.inputs.iter()).all(|(a, b)| a == b)
                    {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::DuplicateDefinition { name },
                            func.span,
                        )));
                    }
                }
            }
        }

        // Reject functions without a body outside traits/extern blocks.
        // This method is only called for free-standing Declaration::Function items,
        // not for trait methods (handled in collect_trait) or extern fns (collect_bridge).
        if func.body.is_none() {
            let name = self.symbol_to_string(func.name.node);
            return Err(Box::new(TypeError::new(
                TypeErrorKind::UnsupportedFeature {
                    feature: format!(
                        "function `{}` declared without a body; only trait methods and extern functions may omit the body",
                        name
                    ),
                },
                func.span,
            )));
        }

        // Queue function for later body type-checking
        self.pending_bodies.push((def_id, func.clone(), self.current_module));

        Ok(())
    }

    /// Collect a struct declaration.
    pub(crate) fn collect_struct(&mut self, struct_decl: &ast::StructDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(struct_decl.name.node);

        // Use pre-registered DefId if it exists (from forward reference support phase),
        // otherwise define the item now
        let def_id = if let Some(existing_def_id) = self.resolver.lookup_type(&name) {
            existing_def_id
        } else {
            let def_id = self.resolver.define_item(
                name.clone(),
                hir::DefKind::Struct,
                struct_decl.span,
            )?;
            self.resolver.define_type(name.clone(), def_id, struct_decl.span)?;
            def_id
        };

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Register generic type parameters before processing field types
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let saved_const_params = std::mem::take(&mut self.const_params);
        let saved_lifetime_params = std::mem::take(&mut self.lifetime_params);
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = struct_decl.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var_id = TyVarId(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var_id);
                        generics_vec.push(ty_var_id);
                        // Register trait bounds for method lookup on type parameters
                        self.register_type_param_bounds(ty_var_id, &type_param.bounds);
                    }
                    ast::GenericParam::Lifetime(lifetime_param) => {
                        let param_name = self.symbol_to_string(lifetime_param.name.node);
                        let lifetime_id = hir::LifetimeId::new(self.next_lifetime_id);
                        self.next_lifetime_id += 1;
                        self.lifetime_params.insert(param_name, lifetime_id);
                    }
                    ast::GenericParam::Const(const_param) => {
                        let param_name = self.symbol_to_string(const_param.name.node);
                        let const_id = hir::ConstParamId::new(self.next_const_param_id);
                        self.next_const_param_id += 1;
                        self.const_params.insert(param_name, const_id);
                    }
                }
            }
        }

        // Collect fields (now with generics in scope)
        let fields = match &struct_decl.body {
            ast::StructBody::Record(fields) => {
                fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let field_name = self.symbol_to_string(f.name.node);
                        let ty = self.ast_type_to_hir_type(&f.ty)?;
                        Ok(FieldInfo {
                            name: field_name,
                            ty,
                            index: i as u32,
                        })
                    })
                    .collect::<Result<Vec<_>, Box<TypeError>>>()?
            }
            ast::StructBody::Tuple(types) => {
                types
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| {
                        let ty = self.ast_type_to_hir_type(ty)?;
                        Ok(FieldInfo {
                            name: format!("{i}"),
                            ty,
                            index: i as u32,
                        })
                    })
                    .collect::<Result<Vec<_>, Box<TypeError>>>()?
            }
            ast::StructBody::Unit => Vec::new(),
        };

        // Check for duplicate field names
        {
            let mut seen: HashSet<&str> = HashSet::new();
            for field in &fields {
                if !seen.insert(&field.name) {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::DuplicateDefinition { name: field.name.clone() },
                        struct_decl.span,
                    )));
                }
            }
        }

        // Restore previous generic params scope
        self.generic_params = saved_generic_params;
        self.const_params = saved_const_params;
        self.lifetime_params = saved_lifetime_params;

        self.struct_defs.insert(def_id, StructInfo {
            name: name.clone(),
            fields,
            generics: generics_vec,
        });

        // Extract derive macros from attributes
        let derives = self.extract_derives(&struct_decl.attrs);
        if !derives.is_empty() {
            self.pending_derives.push(crate::derive::DeriveRequest {
                type_def_id: def_id,
                type_name: name,
                derives,
                span: struct_decl.span,
            });
        }

        Ok(())
    }

    /// Collect a type alias declaration.
    pub(crate) fn collect_type_alias(&mut self, type_decl: &ast::TypeDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(type_decl.name.node);

        // Use pre-registered DefId if it exists (from forward reference support phase),
        // otherwise define the item now
        let def_id = if let Some(existing_def_id) = self.resolver.lookup_type(&name) {
            existing_def_id
        } else {
            let def_id = self.resolver.define_item(
                name.clone(),
                hir::DefKind::TypeAlias,
                type_decl.span,
            )?;
            self.resolver.define_type(name.clone(), def_id, type_decl.span)?;
            def_id
        };

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Register generic type parameters before processing the aliased type
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let saved_const_params = std::mem::take(&mut self.const_params);
        let saved_lifetime_params = std::mem::take(&mut self.lifetime_params);
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = type_decl.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var_id = TyVarId(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var_id);
                        generics_vec.push(ty_var_id);
                        // Register trait bounds for method lookup on type parameters
                        self.register_type_param_bounds(ty_var_id, &type_param.bounds);
                    }
                    ast::GenericParam::Lifetime(lifetime_param) => {
                        let param_name = self.symbol_to_string(lifetime_param.name.node);
                        let lifetime_id = hir::LifetimeId::new(self.next_lifetime_id);
                        self.next_lifetime_id += 1;
                        self.lifetime_params.insert(param_name, lifetime_id);
                    }
                    ast::GenericParam::Const(const_param) => {
                        let param_name = self.symbol_to_string(const_param.name.node);
                        let const_id = hir::ConstParamId::new(self.next_const_param_id);
                        self.next_const_param_id += 1;
                        self.const_params.insert(param_name, const_id);
                    }
                }
            }
        }

        // Convert the aliased type (now with generics in scope)
        // Top-level type aliases must have a type (only trait items may omit it)
        let ty_ast = type_decl.ty.as_ref().ok_or_else(|| {
            let name = self.symbol_to_string(type_decl.name.node);
            Box::new(TypeError::new(
                TypeErrorKind::UnsupportedFeature {
                    feature: format!(
                        "type alias `{}` declared without a type; only trait associated types may omit the type",
                        name
                    ),
                },
                type_decl.span,
            ))
        })?;
        let aliased_ty = self.ast_type_to_hir_type(ty_ast)?;

        // Restore previous generic params scope
        self.generic_params = saved_generic_params;
        self.const_params = saved_const_params;
        self.lifetime_params = saved_lifetime_params;

        self.type_aliases.insert(def_id, TypeAliasInfo {
            name,
            ty: aliased_ty,
            generics: generics_vec,
        });

        Ok(())
    }

    /// Collect an enum declaration.
    pub(crate) fn collect_enum(&mut self, enum_decl: &ast::EnumDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(enum_decl.name.node);

        // Use pre-registered DefId if it exists (from forward reference support phase),
        // otherwise define the item now
        let def_id = if let Some(existing_def_id) = self.resolver.lookup_type(&name) {
            existing_def_id
        } else {
            let def_id = self.resolver.define_item(
                name.clone(),
                hir::DefKind::Enum,
                enum_decl.span,
            )?;
            self.resolver.define_type(name.clone(), def_id, enum_decl.span)?;
            def_id
        };

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Register generic type parameters before processing variant types
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let saved_const_params = std::mem::take(&mut self.const_params);
        let saved_lifetime_params = std::mem::take(&mut self.lifetime_params);
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = enum_decl.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var_id = TyVarId(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var_id);
                        generics_vec.push(ty_var_id);
                        // Register trait bounds for method lookup on type parameters
                        self.register_type_param_bounds(ty_var_id, &type_param.bounds);
                    }
                    ast::GenericParam::Lifetime(lifetime_param) => {
                        let param_name = self.symbol_to_string(lifetime_param.name.node);
                        let lifetime_id = hir::LifetimeId::new(self.next_lifetime_id);
                        self.next_lifetime_id += 1;
                        self.lifetime_params.insert(param_name, lifetime_id);
                    }
                    ast::GenericParam::Const(const_param) => {
                        let param_name = self.symbol_to_string(const_param.name.node);
                        let const_id = hir::ConstParamId::new(self.next_const_param_id);
                        self.next_const_param_id += 1;
                        self.const_params.insert(param_name, const_id);
                    }
                }
            }
        }

        // Collect variants (now with generics in scope)
        let mut variants = Vec::new();
        for (i, variant) in enum_decl.variants.iter().enumerate() {
            let variant_name = self.symbol_to_string(variant.name.node);

            // Define variant as a namespaced item (not in global scope).
            // Variants are accessed via qualified paths like EnumName::VariantName,
            // so they shouldn't pollute the global namespace.
            let variant_def_id = self.resolver.define_namespaced_item(
                variant_name.clone(),
                hir::DefKind::Variant,
                variant.span,
            );

            // Set the parent to the enum def_id for qualified path resolution
            if let Some(def_info) = self.resolver.def_info.get_mut(&variant_def_id) {
                def_info.parent = Some(def_id);
            }

            let fields = match &variant.body {
                ast::StructBody::Record(fields) => {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(j, f)| {
                            let field_name = self.symbol_to_string(f.name.node);
                            let ty = self.ast_type_to_hir_type(&f.ty)?;
                            Ok(FieldInfo {
                                name: field_name,
                                ty,
                                index: j as u32,
                            })
                        })
                        .collect::<Result<Vec<_>, Box<TypeError>>>()?
                }
                ast::StructBody::Tuple(types) => {
                    types
                        .iter()
                        .enumerate()
                        .map(|(j, ty)| {
                            let ty = self.ast_type_to_hir_type(ty)?;
                            Ok(FieldInfo {
                                name: format!("{j}"),
                                ty,
                                index: j as u32,
                            })
                        })
                        .collect::<Result<Vec<_>, Box<TypeError>>>()?
                }
                ast::StructBody::Unit => Vec::new(),
            };

            variants.push(VariantInfo {
                name: variant_name,
                fields,
                index: i as u32,
                def_id: variant_def_id,
            });
        }

        // Check for duplicate variant names
        {
            let mut seen: HashSet<&str> = HashSet::new();
            for variant in &variants {
                if !seen.insert(&variant.name) {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::DuplicateDefinition { name: variant.name.clone() },
                        enum_decl.span,
                    )));
                }
            }
        }

        // Restore previous generic params scope
        self.generic_params = saved_generic_params;
        self.const_params = saved_const_params;
        self.lifetime_params = saved_lifetime_params;

        self.enum_defs.insert(def_id, EnumInfo {
            name: name.clone(),
            variants,
            generics: generics_vec,
        });

        // Extract derive macros from attributes
        let derives = self.extract_derives(&enum_decl.attrs);
        if !derives.is_empty() {
            self.pending_derives.push(crate::derive::DeriveRequest {
                type_def_id: def_id,
                type_name: name,
                derives,
                span: enum_decl.span,
            });
        }

        Ok(())
    }

    /// Collect a const declaration.
    ///
    /// This registers the const item and queues its value expression for type checking.
    pub(crate) fn collect_const(&mut self, const_decl: &ast::ConstDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(const_decl.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Const,
            const_decl.span,
        )?;

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Convert the declared type
        let ty = self.ast_type_to_hir_type(&const_decl.ty)?;

        // Set the type in def_info so it can be looked up during expression type inference
        if let Some(def_info) = self.resolver.def_info.get_mut(&def_id) {
            def_info.ty = Some(ty.clone());
        }

        // Try to evaluate the const value for use in const contexts (e.g., array sizes).
        // Path resolution failures are expected during collection (other consts may not be
        // collected yet), but genuine evaluation errors (div-by-zero, overflow) must propagate.
        match const_eval::eval_const_expr(&const_decl.value) {
            Ok(value) => {
                self.const_values.insert(def_id, value);
            }
            Err(e) => {
                if !matches!(&e.kind, TypeErrorKind::ConstEvalError { reason }
                    if reason.contains("cannot evaluate path"))
                {
                    return Err(e);
                }
            }
        }

        // Queue for body type-checking (the value expression)
        self.pending_consts.push((def_id, const_decl.clone()));

        // Store a placeholder - body_id will be assigned during check_const_body
        // We use a dummy body_id here that will be replaced
        let placeholder_body_id = hir::BodyId::new(u32::MAX);
        self.const_defs.insert(def_id, super::ConstInfo {
            name: name.clone(),
            ty,
            body_id: placeholder_body_id,
            span: const_decl.span,
        });

        Ok(())
    }

    /// Collect a static declaration.
    ///
    /// This registers the static item and queues its value expression for type checking.
    pub(crate) fn collect_static(&mut self, static_decl: &ast::StaticDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(static_decl.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Static,
            static_decl.span,
        )?;

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Convert the declared type
        let ty = self.ast_type_to_hir_type(&static_decl.ty)?;

        // Set the type in def_info so it can be looked up during expression type inference
        if let Some(def_info) = self.resolver.def_info.get_mut(&def_id) {
            def_info.ty = Some(ty.clone());
        }

        // Queue for body type-checking (the value expression)
        self.pending_statics.push((def_id, static_decl.clone()));

        // Store a placeholder - body_id will be assigned during check_static_body
        // We use a dummy body_id here that will be replaced
        let placeholder_body_id = hir::BodyId::new(u32::MAX);
        self.static_defs.insert(def_id, super::StaticInfo {
            name,
            ty,
            is_mut: static_decl.is_mut,
            body_id: placeholder_body_id,
            span: static_decl.span,
        });

        Ok(())
    }

    /// Collect an effect declaration.
    pub(crate) fn collect_effect(&mut self, effect: &ast::EffectDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(effect.name.node);

        // Use pre-registered DefId if it exists (from forward reference support phase),
        // otherwise define the item now
        let def_id = if let Some(existing_def_id) = self.resolver.lookup_type(&name) {
            existing_def_id
        } else {
            let def_id = self.resolver.define_item(
                name.clone(),
                hir::DefKind::Effect,
                effect.span,
            )?;
            self.resolver.define_type(name.clone(), def_id, effect.span)?;
            def_id
        };

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Collect generic parameters
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let mut generics_vec = Vec::new();
        if let Some(ref type_params) = effect.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var = TyVarId(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var);
                        generics_vec.push(ty_var);
                    }
                    ast::GenericParam::Lifetime(_) => {}
                    ast::GenericParam::Const(_) => {}
                }
            }
        }

        // Collect operations
        let mut operations = Vec::new();
        for (index, op) in effect.operations.iter().enumerate() {
            let op_name = self.symbol_to_string(op.name.node);

            // Generate a DefId for the operation WITHOUT registering it globally.
            // Effect operations are only available within functions that declare the effect.
            // They will be registered in scope when checking function bodies.
            let op_def_id = self.resolver.next_def_id();

            // Register def_info for the operation (but NOT in any scope)
            self.resolver.def_info.insert(op_def_id, super::super::resolve::DefInfo {
                kind: hir::DefKind::Fn,
                name: op_name.clone(),
                span: op.span,
                ty: None,
                parent: Some(def_id),  // Parent is the effect
                visibility: crate::ast::Visibility::Public,
            });

            // Collect parameter types
            let params: Vec<Type> = op.params.iter()
                .map(|p| self.ast_type_to_hir_type(&p.ty))
                .collect::<Result<_, _>>()?;

            // Get return type
            let return_ty = self.ast_type_to_hir_type(&op.return_type)?;

            operations.push(OperationInfo {
                name: op_name.clone(),
                params,
                return_ty,
                def_id: op_def_id,
            });

            // Also register the operation signature for type checking
            // Include the effect's type parameters as generics so they can be instantiated
            self.fn_sigs.insert(op_def_id, hir::FnSig {
                inputs: operations[index].params.clone(),
                output: operations[index].return_ty.clone(),
                is_const: false,
                is_async: false,
                is_unsafe: false,
                generics: generics_vec.clone(),
                const_generics: Vec::new(),
            });

            // Note: Effect operations are not builtins - they will be handled
            // by the effect handler at runtime. For now, we just register the
            // signature. Full effect codegen is Phase 2.
        }

        // Restore generic params
        self.generic_params = saved_generic_params;

        self.effect_defs.insert(def_id, EffectInfo {
            name,
            operations,
            generics: generics_vec,
        });

        Ok(())
    }

    /// Collect a handler declaration.
    pub(crate) fn collect_handler(&mut self, handler: &ast::HandlerDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(handler.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Handler,
            handler.span,
        )?;

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Collect generic parameters
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let mut generics_vec = Vec::new();
        if let Some(ref type_params) = handler.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var = TyVarId(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var);
                        generics_vec.push(ty_var);
                    }
                    ast::GenericParam::Lifetime(_) => {}
                    ast::GenericParam::Const(_) => {}
                }
            }
        }

        // Find the effect this handler handles
        // The effect is a Type, we need to resolve it to a DefId
        let effect_ref = self.resolve_effect_type(&handler.effect)?
            .ok_or_else(|| Box::new(TypeError::new(
                TypeErrorKind::NotAnEffect { name: "unknown".to_string() },
                handler.effect.span,
            )))?;
        let effect_id = effect_ref.def_id;

        // Collect operation names implemented by this handler
        let operations: Vec<String> = handler.operations.iter()
            .map(|op| self.symbol_to_string(op.name.node))
            .collect();

        // Validate that the handler implements all required operations from the effect
        if let Some(effect_info) = self.effect_defs.get(&effect_id) {
            let effect_op_names: Vec<&str> = effect_info.operations.iter()
                .map(|op| op.name.as_str())
                .collect();

            // Check for missing operations
            for effect_op in &effect_op_names {
                if !operations.iter().any(|op| op == *effect_op) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidHandler {
                            reason: format!(
                                "handler `{}` missing operation `{}` from effect",
                                name, effect_op
                            ),
                        },
                        handler.span,
                    ));
                }
            }

            // Check for unknown operations
            for handler_op in &operations {
                if !effect_op_names.contains(&handler_op.as_str()) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidHandler {
                            reason: format!(
                                "handler `{}` defines unknown operation `{}`",
                                name, handler_op
                            ),
                        },
                        handler.span,
                    ));
                }
            }
        }

        // Collect state fields (while generic params are still in scope)
        let mut fields = Vec::new();
        for (i, state_field) in handler.state.iter().enumerate() {
            let field_name = self.symbol_to_string(state_field.name.node);
            let field_ty = self.ast_type_to_hir_type(&state_field.ty)?;
            fields.push(FieldInfo {
                name: field_name,
                ty: field_ty,
                index: i as u32,
            });
        }

        // Restore generic params
        self.generic_params = saved_generic_params;

        // Create inference variables early so they're available at handle sites
        // (which are type-checked before handler bodies in Phase 5 vs Phase 6).
        let continuation_result_ty = match handler.kind {
            crate::ast::HandlerKind::Deep => Some(self.unifier.fresh_var()),
            crate::ast::HandlerKind::Shallow => None,
        };
        let return_clause_input_ty = if handler.return_clause.is_some() {
            Some(self.unifier.fresh_var())
        } else {
            None
        };
        let return_clause_output_ty = if handler.return_clause.is_some() {
            Some(self.unifier.fresh_var())
        } else {
            None
        };

        // Store handler with empty operations initially - bodies will be type-checked later
        self.handler_defs.insert(def_id, super::HandlerInfo {
            name,
            kind: handler.kind,
            effect_id,
            effect_type_args: effect_ref.type_args.clone(),
            operations: Vec::new(), // Will be populated during body type-checking
            generics: generics_vec,
            fields,
            return_clause_body_id: None, // Will be populated during body type-checking
            continuation_result_ty,
            return_clause_input_ty,
            return_clause_output_ty,
        });

        // Queue handler for body type-checking
        self.pending_handlers.push((def_id, handler.clone()));

        Ok(())
    }

    /// Collect an impl block declaration.
    pub(crate) fn collect_impl_block(&mut self, impl_block: &ast::ImplBlock) -> Result<(), Box<TypeError>> {
        // Save current generic params
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let mut generics_vec = Vec::new();

        // Register type parameters from the impl block
        if let Some(ref type_params) = impl_block.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var_id = TyVarId::new(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var_id);
                        generics_vec.push(ty_var_id);
                        // Register trait bounds for method lookup on type parameters
                        self.register_type_param_bounds(ty_var_id, &type_param.bounds);
                    }
                    ast::GenericParam::Lifetime(_) => {}
                    ast::GenericParam::Const(_) => {}
                }
            }
        }

        // Convert self type to HIR type
        let self_ty = self.ast_type_to_hir_type(&impl_block.self_ty)?;

        // Set current_impl_self_ty so that `Self` can be resolved in method signatures
        self.current_impl_self_ty = Some(self_ty.clone());

        // Check if this is a trait impl and resolve the trait (if any)
        let trait_ref = if let Some(ref trait_ty) = impl_block.trait_ty {
            // For now, only support simple trait paths
            match &trait_ty.kind {
                ast::TypeKind::Path(path) => {
                    if path.segments.is_empty() {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::TypeNotFound { name: "empty trait path".to_string() },
                            impl_block.span,
                        )));
                    }
                    let trait_name = self.symbol_to_string(path.segments[0].name.node);
                    // Look up the trait by name
                    match self.resolver.lookup(&trait_name) {
                        Some(Binding::Def(def_id)) => {
                            // Verify this is actually a trait
                            if let Some(info) = self.resolver.def_info.get(&def_id) {
                                if matches!(info.kind, hir::DefKind::Trait) {
                                    Some(def_id)
                                } else {
                                    return Err(Box::new(TypeError::new(
                                        TypeErrorKind::TraitNotFound { name: trait_name },
                                        trait_ty.span,
                                    )));
                                }
                            } else {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::TraitNotFound { name: trait_name },
                                    trait_ty.span,
                                )));
                            }
                        }
                        _ => {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::TraitNotFound { name: trait_name },
                                trait_ty.span,
                            )));
                        }
                    }
                }
                _ => {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::unit(), // Placeholder
                            found: Type::unit(),
                        },
                        trait_ty.span,
                    )));
                }
            }
        } else {
            None
        };

        // Process impl items in two passes:
        // 1. First collect associated types so Self::AssocType can be resolved in methods
        // 2. Then collect methods and constants
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        let mut assoc_consts = Vec::new();

        // Clear and prepare current_impl_assoc_types for this impl block
        self.current_impl_assoc_types.clear();

        // First pass: collect associated types
        for item in &impl_block.items {
            if let ast::ImplItem::Type(type_decl) = item {
                let type_name = self.symbol_to_string(type_decl.name.node);
                // Impl associated types must have a concrete type
                let ty_ast = type_decl.ty.as_ref().ok_or_else(|| {
                    Box::new(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: format!(
                                "associated type `{}` in impl block must have a concrete type",
                                type_name
                            ),
                        },
                        type_decl.span,
                    ))
                })?;
                let ty = self.ast_type_to_hir_type(ty_ast)?;
                let assoc_type_info = ImplAssocTypeInfo {
                    name: type_name,
                    ty,
                };
                // Add to current_impl_assoc_types so Self::AssocType can be resolved
                self.current_impl_assoc_types.push(assoc_type_info.clone());
                assoc_types.push(assoc_type_info);
            }
        }

        // Second pass: collect methods and constants
        for item in &impl_block.items {
            match item {
                ast::ImplItem::Type(_) => {
                    // Already processed in first pass
                }
                ast::ImplItem::Function(func) => {
                    let method_name = self.symbol_to_string(func.name.node);

                    // Create a qualified name for the method: Type::method_name
                    let qualified_name = format!("{}::{}", self.type_to_string(&self_ty), method_name);

                    // Register the method as an associated function, supporting method overloading
                    let method_def_id = self.resolver.define_assoc_function(
                        qualified_name.clone(),
                        func.span,
                        Visibility::Public,
                    )?;

                    // Check if this is a static method (no self parameter)
                    let is_static = func.params.first().map_or(true, |p| {
                        match &p.pattern.kind {
                            ast::PatternKind::Ident { name, .. } => {
                                let name_str = self.symbol_to_string(name.node);
                                name_str != "self"
                            }
                            _ => true,
                        }
                    });

                    // Store the Self type for this method
                    self.method_self_types.insert(method_def_id, self_ty.clone());

                    // Handle method-level type parameters
                    // Note: impl-level params are already in scope from earlier
                    let mut method_generics = Vec::new();
                    if let Some(ref type_params) = func.type_params {
                        for generic_param in &type_params.params {
                            match generic_param {
                                ast::GenericParam::Type(type_param) => {
                                    let param_name = self.symbol_to_string(type_param.name.node);
                                    let ty_var_id = TyVarId(self.next_type_param_id);
                                    self.next_type_param_id += 1;
                                    self.generic_params.insert(param_name, ty_var_id);
                                    method_generics.push(ty_var_id);
                                }
                                ast::GenericParam::Lifetime(_) => {}
                                ast::GenericParam::Const(_) => {}
                            }
                        }
                    }

                    // Build the function signature
                    let mut param_types = Vec::new();
                    for (i, param) in func.params.iter().enumerate() {
                        if i == 0 && !is_static {
                            // This is the self parameter
                            // The type is in param.ty - check if it's a reference type
                            // (e.g., &Self or &mut Self)
                            param_types.push(self.ast_type_to_hir_type(&param.ty)?);
                        } else {
                            // Regular parameter
                            param_types.push(self.ast_type_to_hir_type(&param.ty)?);
                        }
                    }

                    let return_type = match &func.return_type {
                        Some(ty) => self.ast_type_to_hir_type(ty)?,
                        None => Type::unit(),
                    };

                    // Create function signature with method generics
                    let sig = hir::FnSig {
                        inputs: param_types,
                        output: return_type,
                        is_const: func.qualifiers.is_const,
                        is_async: func.qualifiers.is_async,
                        is_unsafe: func.qualifiers.is_unsafe,
                        generics: method_generics,
                        const_generics: Vec::new(),
                    };

                    self.fn_sigs.insert(method_def_id, sig);

                    // Check for duplicate method signatures (same name AND same parameter types).
                    // Multiple dispatch allows same-name methods with different signatures,
                    // but identical signatures are an error.
                    if let Some(Binding::Methods(ref method_ids)) = self.resolver.current_scope().bindings.get(&qualified_name) {
                        for &other_id in method_ids {
                            if other_id == method_def_id {
                                continue;
                            }
                            if let Some(other_sig) = self.fn_sigs.get(&other_id) {
                                let current_sig = &self.fn_sigs[&method_def_id];
                                if other_sig.inputs.len() == current_sig.inputs.len()
                                    && other_sig.inputs.iter().zip(current_sig.inputs.iter()).all(|(a, b)| a == b)
                                {
                                    return Err(Box::new(TypeError::new(
                                        TypeErrorKind::DuplicateDefinition { name: qualified_name },
                                        func.span,
                                    )));
                                }
                            }
                        }
                    }

                    // Queue method body for type-checking
                    // Include module context if this impl is inside a module
                    self.pending_bodies.push((method_def_id, func.clone(), self.current_module));

                    methods.push(ImplMethodInfo {
                        def_id: method_def_id,
                        name: method_name,
                        is_static,
                    });
                }
                ast::ImplItem::Const(const_decl) => {
                    let const_name = self.symbol_to_string(const_decl.name.node);
                    let qualified_name = format!("{}::{}", self.type_to_string(&self_ty), const_name);

                    let const_def_id = self.resolver.define_item(
                        qualified_name,
                        hir::DefKind::AssocConst,
                        const_decl.span,
                    )?;

                    let ty = self.ast_type_to_hir_type(&const_decl.ty)?;

                    assoc_consts.push(ImplAssocConstInfo {
                        def_id: const_def_id,
                        name: const_name,
                        ty,
                    });
                }
            }
        }

        // Restore generic params
        self.generic_params = saved_generic_params;

        // Validate trait impl: check that all required methods are provided
        if let Some(trait_id) = trait_ref {
            self.validate_trait_impl(
                trait_id,
                &methods,
                &assoc_types,
                impl_block.span,
            )?;
        }

        // Store the impl block with its source span for error reporting
        self.impl_blocks.push(ImplBlockInfo {
            self_ty,
            trait_ref,
            generics: generics_vec,
            methods,
            assoc_types,
            assoc_consts,
            span: impl_block.span,
        });

        // Clear current_impl_self_ty now that we're done with this impl block
        self.current_impl_self_ty = None;

        Ok(())
    }

    /// Collect a trait declaration.
    pub(crate) fn collect_trait(&mut self, trait_decl: &ast::TraitDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(trait_decl.name.node);

        // Use pre-registered DefId if it exists (from forward reference support phase),
        // otherwise define the item now
        let def_id = if let Some(existing_def_id) = self.resolver.lookup_type(&name) {
            existing_def_id
        } else {
            let def_id = self.resolver.define_item(
                name.clone(),
                hir::DefKind::Trait,
                trait_decl.span,
            )?;
            self.resolver.define_type(name.clone(), def_id, trait_decl.span)?;
            def_id
        };

        // Track this item as belonging to the current module
        self.current_module_items.push(def_id);

        // Save current generic params and current_impl_self_ty
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let saved_impl_self_ty = self.current_impl_self_ty.take();
        let mut generics_vec = Vec::new();

        // Create a type parameter for `Self` - this represents "the implementing type"
        // When the trait is implemented, Self will be substituted with the concrete type
        let self_ty_var_id = TyVarId::new(self.next_type_param_id);
        self.next_type_param_id += 1;
        self.current_impl_self_ty = Some(Type::param(self_ty_var_id));

        // Register type parameters
        if let Some(ref type_params) = trait_decl.type_params {
            for generic_param in &type_params.params {
                match generic_param {
                    ast::GenericParam::Type(type_param) => {
                        let param_name = self.symbol_to_string(type_param.name.node);
                        let ty_var_id = TyVarId::new(self.next_type_param_id);
                        self.next_type_param_id += 1;
                        self.generic_params.insert(param_name, ty_var_id);
                        generics_vec.push(ty_var_id);
                        // Register trait bounds for method lookup on type parameters
                        self.register_type_param_bounds(ty_var_id, &type_param.bounds);
                    }
                    ast::GenericParam::Lifetime(_) => {}
                    ast::GenericParam::Const(_) => {}
                }
            }
        }

        // Resolve supertraits
        let mut supertraits = Vec::new();
        for supertrait in &trait_decl.supertraits {
            match &supertrait.kind {
                ast::TypeKind::Path(path) => {
                    if !path.segments.is_empty() {
                        let supertrait_name = self.symbol_to_string(path.segments[0].name.node);
                        match self.resolver.lookup(&supertrait_name) {
                            Some(Binding::Def(supertrait_def_id)) => {
                                if let Some(info) = self.resolver.def_info.get(&supertrait_def_id) {
                                    if matches!(info.kind, hir::DefKind::Trait) {
                                        supertraits.push(supertrait_def_id);
                                    } else {
                                        return Err(Box::new(TypeError::new(
                                            TypeErrorKind::TraitNotFound { name: supertrait_name },
                                            supertrait.span,
                                        )));
                                    }
                                }
                            }
                            _ => {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::TraitNotFound { name: supertrait_name },
                                    supertrait.span,
                                )));
                            }
                        }
                    }
                }
                _ => {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "complex supertrait bounds".to_string(),
                        },
                        supertrait.span,
                    )));
                }
            }
        }

        // Process trait items in two passes:
        // 1. First collect associated types so Self::AssocType can be resolved in method signatures
        // 2. Then collect methods and constants
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        let mut assoc_consts = Vec::new();

        // First pass: collect associated types
        for item in &trait_decl.items {
            if let ast::TraitItem::Type(type_decl) = item {
                let type_name = self.symbol_to_string(type_decl.name.node);
                let default = match &type_decl.ty {
                    Some(ty_ast) => {
                        match self.ast_type_to_hir_type(ty_ast) {
                            Ok(ty) if !ty.is_error() => Some(ty),
                            _ => None,
                        }
                    }
                    None => None, // Bare `type Item;` — no default
                };

                assoc_types.push(TraitAssocTypeInfo {
                    name: type_name,
                    default,
                });
            }
        }

        // Set trait associated types context so Self::AssocType resolves in method signatures
        self.current_trait_assoc_types = assoc_types.clone();

        // Second pass: collect methods and constants
        for item in &trait_decl.items {
            match item {
                ast::TraitItem::Function(func) => {
                    let method_name = self.symbol_to_string(func.name.node);
                    let qualified_name = format!("{}::{}", name, method_name);

                    let method_def_id = self.resolver.define_item(
                        qualified_name,
                        hir::DefKind::AssocFn,
                        func.span,
                    )?;

                    // Handle method-level type parameters
                    let mut method_generics = Vec::new();
                    if let Some(ref type_params) = func.type_params {
                        for generic_param in &type_params.params {
                            match generic_param {
                                ast::GenericParam::Type(type_param) => {
                                    let param_name = self.symbol_to_string(type_param.name.node);
                                    let ty_var_id = TyVarId(self.next_type_param_id);
                                    self.next_type_param_id += 1;
                                    self.generic_params.insert(param_name, ty_var_id);
                                    method_generics.push(ty_var_id);
                                }
                                ast::GenericParam::Lifetime(_) => {}
                                ast::GenericParam::Const(_) => {}
                            }
                        }
                    }

                    // Build parameter types
                    let mut param_types = Vec::new();
                    for param in &func.params {
                        param_types.push(self.ast_type_to_hir_type(&param.ty)?);
                    }

                    let return_type = match &func.return_type {
                        Some(ty) => self.ast_type_to_hir_type(ty)?,
                        None => Type::unit(),
                    };

                    let sig = hir::FnSig {
                        inputs: param_types,
                        output: return_type,
                        is_const: func.qualifiers.is_const,
                        is_async: func.qualifiers.is_async,
                        is_unsafe: func.qualifiers.is_unsafe,
                        generics: method_generics,
                        const_generics: Vec::new(),
                    };

                    self.fn_sigs.insert(method_def_id, sig.clone());

                    // Check if this has a default implementation
                    let has_default = func.body.is_some();
                    if has_default {
                        // Queue the default body for type-checking
                        // Trait methods don't need module context
                        self.pending_bodies.push((method_def_id, func.clone(), None));
                    }

                    methods.push(TraitMethodInfo {
                        def_id: method_def_id,
                        name: method_name,
                        sig,
                        has_default,
                    });
                }
                ast::TraitItem::Type(_) => {
                    // Already processed in first pass
                }
                ast::TraitItem::Const(const_decl) => {
                    let const_name = self.symbol_to_string(const_decl.name.node);
                    let qualified_name = format!("{}::{}", name, const_name);

                    let const_def_id = self.resolver.define_item(
                        qualified_name,
                        hir::DefKind::AssocConst,
                        const_decl.span,
                    )?;

                    let ty = self.ast_type_to_hir_type(&const_decl.ty)?;
                    // In the AST, trait constants always have a value (the parser requires it)
                    // The presence of a value means it has a default
                    let has_default = true;

                    assoc_consts.push(TraitAssocConstInfo {
                        def_id: const_def_id,
                        name: const_name,
                        ty,
                        has_default,
                    });
                }
            }
        }

        // Clear trait associated types context
        self.current_trait_assoc_types.clear();

        // Restore generic params and current_impl_self_ty
        self.generic_params = saved_generic_params;
        self.current_impl_self_ty = saved_impl_self_ty;

        // Store the trait info
        self.trait_defs.insert(def_id, TraitInfo {
            name,
            generics: generics_vec,
            supertraits,
            methods,
            assoc_types,
            assoc_consts,
        });

        Ok(())
    }

    /// Parse an effect row annotation into a list of EffectRefs and optional row variable.
    ///
    /// Returns (effects, row_variable_name) where:
    /// - `effects` is the list of concrete effects in the row
    /// - `row_variable_name` is Some(name) if the row has a polymorphic tail (e.g., `{IO | e}` or just `e`)
    pub(crate) fn parse_effect_row(&mut self, effect_row: &ast::EffectRow) -> Result<(Vec<EffectRef>, Option<String>), Box<TypeError>> {
        match &effect_row.kind {
            ast::EffectRowKind::Pure => Ok((Vec::new(), None)),
            ast::EffectRowKind::Var(var) => {
                // Just a row variable: `fn foo() / e`
                let var_name = self.symbol_to_string(var.node);
                Ok((Vec::new(), Some(var_name)))
            }
            ast::EffectRowKind::Effects { effects, rest } => {
                let mut result = Vec::new();
                for effect_ty in effects {
                    if let Some(effect_ref) = self.resolve_effect_type(effect_ty)? {
                        result.push(effect_ref);
                    }
                }
                // Extract row variable name if present
                let row_var = rest.as_ref().map(|r| self.symbol_to_string(r.node));
                Ok((result, row_var))
            }
        }
    }

    /// Resolve an effect type (like `State<i32>`) to an EffectRef.
    pub(crate) fn resolve_effect_type(&mut self, ty: &ast::Type) -> Result<Option<EffectRef>, Box<TypeError>> {
        match &ty.kind {
            ast::TypeKind::Path(path) => {
                if path.segments.is_empty() {
                    return Ok(None);
                }

                let effect_name = self.symbol_to_string(path.segments[0].name.node);

                // IO is a built-in effect, not user-defined
                if effect_name == "IO" {
                    return Ok(None);
                }

                // Look up the effect by name in the global bindings
                if let Some(Binding::Def(def_id)) = self.resolver.lookup(&effect_name) {
                    // Verify it's actually an effect
                    if self.effect_defs.contains_key(&def_id) {
                        // Parse type arguments
                        let type_args = if let Some(ref args) = path.segments[0].args {
                            let mut parsed_args = Vec::new();
                            for arg in &args.args {
                                if let ast::TypeArg::Type(arg_ty) = arg {
                                    parsed_args.push(self.ast_type_to_hir_type(arg_ty)?);
                                }
                            }
                            parsed_args
                        } else {
                            Vec::new()
                        };

                        return Ok(Some(EffectRef { def_id, type_args }));
                    } else {
                        // Name resolves to something, but it's not an effect
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotAnEffect { name: effect_name },
                            ty.span,
                        )));
                    }
                }

                Ok(None)
            }
            other => {
                // Non-path types are invalid effect types
                let found = match other {
                    ast::TypeKind::Reference { .. } => "reference type",
                    ast::TypeKind::Pointer { .. } => "pointer type",
                    ast::TypeKind::Array { .. } => "array type",
                    ast::TypeKind::Slice { .. } => "slice type",
                    ast::TypeKind::Tuple(_) => "tuple type",
                    ast::TypeKind::Function { .. } => "function type",
                    ast::TypeKind::Record { .. } => "record type",
                    ast::TypeKind::Ownership { .. } => "ownership-qualified type",
                    ast::TypeKind::Never => "never type",
                    ast::TypeKind::Infer => "inferred type",
                    ast::TypeKind::Paren(inner) => {
                        // For parenthesized types, recurse
                        return self.resolve_effect_type(inner);
                    }
                    ast::TypeKind::Forall { .. } => "forall type",
                    ast::TypeKind::Path(_) => unreachable!("Path type should be handled by the match above")
                };
                Err(Box::new(TypeError::new(
                    TypeErrorKind::InvalidEffectType { found: found.to_string() },
                    ty.span,
                )))
            }
        }
    }

    /// Register effect operations in the current scope based on a function's declared effects.
    ///
    /// This makes effect operations like `get()` and `put()` available within functions
    /// that declare they use those effects (e.g., `fn counter() / {State<i32>}`).
    pub(crate) fn register_effect_operations_in_scope(&mut self, fn_def_id: DefId) -> Result<(), Box<TypeError>> {
        use crate::ice;

        // Get the function's declared effects
        let effects = match self.fn_effects.get(&fn_def_id) {
            Some(effects) => effects.clone(),
            None => return Ok(()), // No effects declared
        };

        for effect_ref in effects {
            // Get the effect definition
            let effect_info = match self.effect_defs.get(&effect_ref.def_id) {
                Some(info) => info.clone(),
                None => {
                    // This indicates an internal compiler error - the effect was registered
                    // in fn_effects but not found in effect_defs
                    ice!("effect registered in fn_effects but not found in effect_defs";
                         "effect_def_id" => effect_ref.def_id);
                    continue;
                }
            };

            // Build a substitution map from effect's generic params to concrete types
            let mut substitution: HashMap<TyVarId, Type> = HashMap::new();
            for (i, &generic_var) in effect_info.generics.iter().enumerate() {
                if let Some(concrete_ty) = effect_ref.type_args.get(i) {
                    substitution.insert(generic_var, concrete_ty.clone());
                }
            }

            // Register each operation in the current scope for name lookup.
            // Note: We don't overwrite fn_sigs here - the generic signature is kept
            // and substitution happens at the Perform expression site.
            for op_info in &effect_info.operations {
                // Add the operation to the current scope so it can be called by name
                self.resolver.current_scope_mut()
                    .bindings
                    .insert(op_info.name.clone(), Binding::Def(op_info.def_id));
            }
        }

        Ok(())
    }

    /// Collect a module declaration.
    ///
    /// For inline modules (`mod foo { ... }`), recursively collect all declarations.
    /// For external modules (`mod foo;`), loads the module from `name.blood` or `name/mod.blood`
    /// relative to the current source file. For `mod std;`, also checks the stdlib_path.
    pub(crate) fn collect_module(&mut self, module: &ast::ModItemDecl) -> Result<(), Box<TypeError>> {
        let name = self.symbol_to_string(module.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Mod,
            module.span,
        )?;

        // Track this module as belonging to the PARENT module's items.
        // This must happen BEFORE we save/clear current_module_items for the nested module.
        self.current_module_items.push(def_id);

        match &module.body {
            Some(declarations) => {
                // Inline module - push a new scope and collect declarations
                self.resolver.push_scope(ScopeKind::Module, module.span);

                // Track the current module for functions defined inside
                let saved_module = self.current_module;
                self.current_module = Some(def_id);

                // Save and clear current_module_items to track only this module's items.
                // This fixes the name collision bug where items from submodules would
                // be incorrectly included in the parent module's item list, causing
                // same-named items to shadow each other during inject_module_bindings.
                let saved_module_items = std::mem::take(&mut self.current_module_items);
                let saved_module_reexports = std::mem::take(&mut self.current_module_reexports);
                let saved_module_private_imports = std::mem::take(&mut self.current_module_private_imports);

                // Phase 1: Pre-register all type names so forward references work within the module
                for decl in declarations {
                    if let Err(e) = self.register_type_name(decl) {
                        self.errors.push(*e);
                    }
                }

                // Phase 2: Collect all declarations (now that all type names are known)
                for decl in declarations {
                    if let Err(e) = self.collect_declaration(decl) {
                        self.errors.push(*e);
                    }
                }

                // Get the items, reexports, and private imports collected for THIS module only
                let item_def_ids = std::mem::take(&mut self.current_module_items);
                let reexports = std::mem::take(&mut self.current_module_reexports);
                let private_imports = std::mem::take(&mut self.current_module_private_imports);

                // Restore previous module context, items, reexports, and private imports
                self.current_module = saved_module;
                self.current_module_items = saved_module_items;
                self.current_module_reexports = saved_module_reexports;
                self.current_module_private_imports = saved_module_private_imports;
                self.resolver.pop_scope();

                // Store module info
                self.module_defs.insert(def_id, super::ModuleInfo {
                    name,
                    items: item_def_ids,
                    is_external: false,
                    span: module.span,
                    source_path: None,
                    source_content: None,
                    reexports,
                    private_imports,
                });
            }
            None => {
                // External module - load from file
                let source_dir = match &self.source_path {
                    Some(path) => path.parent().map(|p| p.to_path_buf()),
                    None => None,
                };

                let source_dir = source_dir.ok_or_else(|| Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: format!(
                            "external modules (`mod {};`) - no source path available. \
                            Use `with_source_path()` when creating the type context.",
                            name
                        ),
                    },
                    module.span,
                )))?;

                // Try to find the module file.
                // For nested modules (when inside foo.blood), we check:
                // 1. sibling dir: foo/name.blood
                // 2. sibling dir mod: foo/name/mod.blood
                // 3. parent dir: name.blood (for root-level modules)
                // 4. parent dir mod: name/mod.blood
                let sibling_dir = self.source_path.as_ref().and_then(|p| {
                    p.file_stem().map(|stem| source_dir.join(stem))
                });

                let sibling_path = sibling_dir.as_ref().map(|d| d.join(format!("{}.blood", name)));
                let sibling_alt = sibling_dir.as_ref().map(|d| d.join(&name).join("mod.blood"));
                let file_path = source_dir.join(format!("{}.blood", name));
                let alt_path = source_dir.join(&name).join("mod.blood");

                // For "std" module, also check stdlib_path if set
                let stdlib_file_path = if name == "std" {
                    self.stdlib_path.as_ref().map(|p| {
                        if p.is_file() {
                            p.clone()
                        } else {
                            p.join("mod.blood")
                        }
                    })
                } else {
                    None
                };

                let mut searched_paths = Vec::new();
                if let Some(ref sp) = sibling_path {
                    searched_paths.push(sp.display().to_string());
                }
                if let Some(ref sa) = sibling_alt {
                    searched_paths.push(sa.display().to_string());
                }
                searched_paths.push(file_path.display().to_string());
                searched_paths.push(alt_path.display().to_string());
                if let Some(ref sp) = stdlib_file_path {
                    searched_paths.push(sp.display().to_string());
                }

                // Search in order: sibling paths first (for nested modules), then parent paths
                let module_path = if sibling_path.as_ref().is_some_and(|p| p.exists()) {
                    // SAFETY: condition checks sibling_path.as_ref() is Some and exists
                    sibling_path.expect("BUG: just checked sibling_path is Some above")
                } else if sibling_alt.as_ref().is_some_and(|p| p.exists()) {
                    // SAFETY: condition checks sibling_alt.as_ref() is Some and exists
                    sibling_alt.expect("BUG: just checked sibling_alt is Some above")
                } else if file_path.exists() {
                    file_path
                } else if alt_path.exists() {
                    alt_path
                } else if let Some(ref sp) = stdlib_file_path {
                    if sp.exists() {
                        sp.clone()
                    } else {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::ModuleNotFound {
                                name: name.clone(),
                                searched_paths,
                            },
                            module.span,
                        )));
                    }
                } else {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::ModuleNotFound {
                            name: name.clone(),
                            searched_paths,
                        },
                        module.span,
                    )));
                };

                // Canonicalize the module path to handle diamond dependencies
                let canonical_path = module_path.canonicalize().unwrap_or_else(|_| module_path.clone());

                // Check if this module has already been loaded (diamond dependency case)
                if let Some(&existing_def_id) = self.loaded_modules.get(&canonical_path) {
                    // Module already loaded - reuse existing definitions
                    // Just create an alias in the current scope pointing to the existing module
                    if let Some(existing_info) = self.module_defs.get(&existing_def_id).cloned() {
                        // Store module info with the new def_id pointing to existing items
                        self.module_defs.insert(def_id, super::ModuleInfo {
                            name,
                            items: existing_info.items,
                            is_external: true,
                            span: module.span,
                            source_path: existing_info.source_path,
                            source_content: existing_info.source_content,
                            reexports: existing_info.reexports,
                            private_imports: existing_info.private_imports,
                        });
                        return Ok(());
                    }
                }

                // Read the module file
                let module_source = std::fs::read_to_string(&module_path).map_err(|e| {
                    TypeError::new(
                        TypeErrorKind::IoError {
                            message: format!("failed to read module file '{}': {}", module_path.display(), e),
                        },
                        module.span,
                    )
                })?;

                // Temporarily take the interner, parse, and restore
                let interner = std::mem::take(&mut self.interner);
                let mut parser = crate::parser::Parser::with_interner(&module_source, interner);
                let module_ast = parser.parse_program();
                self.interner = parser.take_interner();

                let module_ast = match module_ast {
                    Ok(ast) => ast,
                    Err(errors) => {
                        // Collect parse errors and return the first one
                        if let Some(first_err) = errors.into_iter().next() {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::ParseError {
                                    message: first_err.message,
                                },
                                module.span,
                            )));
                        }
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::ParseError {
                                message: "unknown parse error".to_string(),
                            },
                            module.span,
                        )));
                    }
                };

                // Process the declarations in a new scope
                self.resolver.push_scope(ScopeKind::Module, module.span);

                // Track the current module for functions defined inside
                let saved_module = self.current_module;
                self.current_module = Some(def_id);

                // Save and update source path for nested module resolution
                let saved_source_path = self.source_path.take();
                self.source_path = Some(module_path.clone());

                // Save and clear current_module_items to track only this module's items.
                // This fixes the name collision bug where items from submodules would
                // be incorrectly included in the parent module's item list, causing
                // same-named items to shadow each other during inject_module_bindings.
                let saved_module_items = std::mem::take(&mut self.current_module_items);
                let saved_module_reexports = std::mem::take(&mut self.current_module_reexports);
                let saved_module_private_imports = std::mem::take(&mut self.current_module_private_imports);

                // Phase 1: Pre-register all type names so forward references work within the module
                for decl in &module_ast.declarations {
                    if let Err(e) = self.register_type_name(decl) {
                        let e_with_source = e.with_source_file(module_path.clone(), module_source.clone());
                        self.errors.push(e_with_source);
                    }
                }

                // Phase 2: Collect all declarations (now that all type names are known)
                for decl in &module_ast.declarations {
                    if let Err(e) = self.collect_declaration(decl) {
                        // Attach source file info to errors from external modules
                        let e_with_source = e.with_source_file(module_path.clone(), module_source.clone());
                        self.errors.push(e_with_source);
                    }
                }

                // Get the items, reexports, and private imports collected for THIS module only
                let item_def_ids = std::mem::take(&mut self.current_module_items);
                let reexports = std::mem::take(&mut self.current_module_reexports);
                let private_imports = std::mem::take(&mut self.current_module_private_imports);

                // Restore previous module context, source path, items, reexports, and private imports
                self.source_path = saved_source_path;
                self.current_module = saved_module;
                self.current_module_items = saved_module_items;
                self.current_module_reexports = saved_module_reexports;
                self.current_module_private_imports = saved_module_private_imports;
                self.resolver.pop_scope();

                // Store module info with source for error reporting
                self.module_defs.insert(def_id, super::ModuleInfo {
                    name,
                    items: item_def_ids,
                    is_external: true,
                    span: module.span,
                    source_path: Some(module_path),
                    source_content: Some(module_source),
                    reexports,
                    private_imports,
                });

                // Cache this module by canonical path for future diamond dependency detection
                self.loaded_modules.insert(canonical_path, def_id);
            }
        }

        Ok(())
    }
}
