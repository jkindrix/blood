//! Declaration collection for the type checker.
//!
//! This module contains methods for collecting top-level declarations
//! (functions, structs, enums, effects, handlers, impl blocks, traits)
//! and registering them in the type context.

use std::collections::HashMap;

use crate::ast;
use crate::hir::{self, DefId, Type, TyVarId};
use crate::diagnostics::Diagnostic;

use super::{
    TypeContext, StructInfo, FieldInfo, EnumInfo, VariantInfo, TypeAliasInfo,
    EffectInfo, OperationInfo, EffectRef, ImplBlockInfo, ImplMethodInfo,
    ImplAssocTypeInfo, ImplAssocConstInfo, TraitInfo, TraitMethodInfo,
    TraitAssocTypeInfo, TraitAssocConstInfo,
};
use super::super::error::{TypeError, TypeErrorKind};
use super::super::resolve::{Binding, ScopeKind};

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
        // Phase 1: Pre-register all type names so forward references work.
        // This allows struct A { b: B } to compile when B is defined after A.
        for decl in &program.declarations {
            if let Err(e) = self.register_type_name(decl) {
                self.errors.push(e);
            }
        }

        // Phase 2: Collect all top-level definitions (now that all type names are known)
        for decl in &program.declarations {
            if let Err(e) = self.collect_declaration(decl) {
                self.errors.push(e);
            }
        }

        // Phase 3: Resolve imports (after all declarations are collected)
        for import in &program.imports {
            if let Err(e) = self.resolve_import(import) {
                self.errors.push(e);
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors.iter().map(|e| e.to_diagnostic()).collect());
        }

        Ok(())
    }

    /// Pre-register type names for forward reference support.
    /// This only registers the name -> DefId mapping, without resolving field types.
    fn register_type_name(&mut self, decl: &ast::Declaration) -> Result<(), TypeError> {
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

    /// Add a re-export to the current module's reexports list.
    /// Called when processing `pub use` imports.
    fn add_reexport_to_current_module(&mut self, name: String, def_id: DefId) {
        if let Some(module_def_id) = self.current_module {
            if let Some(module_info) = self.module_defs.get_mut(&module_def_id) {
                module_info.reexports.push((name, def_id));
            }
        }
        // If we're not in a module (root level), the re-export is tracked
        // at the crate level. For now, we don't have a separate crate-level
        // exports list, so root-level pub use is a no-op for re-export tracking.
        // TODO: Add crate-level exports for pub use at root level
    }

    /// Resolve an import statement.
    fn resolve_import(&mut self, import: &ast::Import) -> Result<(), TypeError> {
        match import {
            ast::Import::Simple { visibility, path, alias, span } => {
                self.resolve_simple_import(*visibility, path, alias.as_ref(), *span)
            }
            ast::Import::Group { visibility, path, items, span } => {
                self.resolve_group_import(*visibility, path, items, *span)
            }
            ast::Import::Glob { visibility, path, span } => {
                self.resolve_glob_import(*visibility, path, *span)
            }
        }
    }

    /// Resolve a simple import: `use std.mem.allocate;` or `use std.mem.allocate as alloc;`
    /// If visibility is Public, this is a re-export (`pub use`).
    fn resolve_simple_import(
        &mut self,
        visibility: ast::Visibility,
        path: &ast::ModulePath,
        alias: Option<&crate::span::Spanned<ast::Symbol>>,
        span: crate::span::Span,
    ) -> Result<(), TypeError> {
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
                return Err(TypeError::new(
                    TypeErrorKind::ImportError {
                        message: "empty import path".to_string(),
                    },
                    span,
                ));
            }
        };

        // Determine if this is a type or value import based on the DefKind
        if let Some(info) = self.resolver.def_info.get(&def_id) {
            match info.kind {
                hir::DefKind::Struct | hir::DefKind::Enum | hir::DefKind::TypeAlias
                | hir::DefKind::Effect | hir::DefKind::Trait => {
                    self.resolver.import_type_binding(local_name.clone(), def_id, span)?;
                }
                _ => {
                    self.resolver.import_binding(local_name.clone(), def_id, span)?;
                }
            }
        } else {
            return Err(TypeError::new(
                TypeErrorKind::ImportError {
                    message: format!("cannot find definition for import path"),
                },
                span,
            ));
        }

        // Track public re-exports
        if visibility == ast::Visibility::Public {
            self.add_reexport_to_current_module(local_name, def_id);
        }

        Ok(())
    }

    /// Resolve a group import: `use std.iter::{Iterator, IntoIterator};`
    /// If visibility is Public, this is a re-export (`pub use`).
    fn resolve_group_import(
        &mut self,
        visibility: ast::Visibility,
        path: &ast::ModulePath,
        items: &[ast::ImportItem],
        span: crate::span::Span,
    ) -> Result<(), TypeError> {
        // For group imports, we need to resolve the base path as a module
        // and then import each item from that module
        let base_module_id = self.resolve_module_path(path)?;

        self.resolve_import_items(visibility, base_module_id, items, span)
    }

    /// Recursively resolve import items (handles both simple and nested items).
    fn resolve_import_items(
        &mut self,
        visibility: ast::Visibility,
        base_module_id: DefId,
        items: &[ast::ImportItem],
        span: crate::span::Span,
    ) -> Result<(), TypeError> {
        for item in items {
            match item {
                ast::ImportItem::Simple { name, alias } => {
                    let item_name = self.symbol_to_string(name.node);

                    // Look up the item in the module
                    if let Some(def_id) = self.lookup_in_module(base_module_id, &item_name) {
                        let local_name = if let Some(a) = alias {
                            self.symbol_to_string(a.node)
                        } else {
                            item_name.clone()
                        };

                        // Import based on def kind
                        if let Some(info) = self.resolver.def_info.get(&def_id) {
                            match info.kind {
                                hir::DefKind::Struct | hir::DefKind::Enum | hir::DefKind::TypeAlias
                                | hir::DefKind::Effect | hir::DefKind::Trait => {
                                    self.resolver.import_type_binding(local_name.clone(), def_id, span)?;
                                }
                                _ => {
                                    self.resolver.import_binding(local_name.clone(), def_id, span)?;
                                }
                            }
                        }

                        // Track public re-exports
                        if visibility == ast::Visibility::Public {
                            self.add_reexport_to_current_module(local_name, def_id);
                        }
                    } else {
                        return Err(TypeError::new(
                            TypeErrorKind::ImportError {
                                message: format!("cannot find `{}` in module", item_name),
                            },
                            span,
                        ));
                    }
                }
                ast::ImportItem::Nested { path: nested_path, items: nested_items } => {
                    // Resolve the nested path from the base module
                    let nested_module_id = self.resolve_nested_import_path(base_module_id, nested_path, span)?;
                    self.resolve_import_items(visibility, nested_module_id, nested_items, span)?;
                }
            }
        }

        Ok(())
    }

    /// Resolve a nested import path from a base module.
    fn resolve_nested_import_path(
        &self,
        base_module_id: DefId,
        path_segments: &[crate::Spanned<ast::Symbol>],
        span: crate::span::Span,
    ) -> Result<DefId, TypeError> {
        let mut current_id = base_module_id;

        for segment in path_segments {
            let segment_name = self.symbol_to_string(segment.node);

            if let Some(child_id) = self.lookup_submodule(current_id, &segment_name) {
                current_id = child_id;
            } else {
                return Err(TypeError::new(
                    TypeErrorKind::ImportError {
                        message: format!("cannot find module `{}`", segment_name),
                    },
                    span,
                ));
            }
        }

        Ok(current_id)
    }

    /// Look up a submodule within a parent module.
    /// Uses the existing lookup_in_module which checks module items.
    fn lookup_submodule(&self, parent_id: DefId, name: &str) -> Option<DefId> {
        // Submodules are registered as items in their parent module
        self.lookup_in_module(parent_id, name)
    }

    /// Resolve a glob import: `use std.ops::*;`
    /// If visibility is Public, this is a re-export (`pub use`).
    fn resolve_glob_import(
        &mut self,
        visibility: ast::Visibility,
        path: &ast::ModulePath,
        span: crate::span::Span,
    ) -> Result<(), TypeError> {
        // Resolve the path as a module and import all public items
        let module_id = self.resolve_module_path(path)?;

        // Get all public items from the module
        let items = self.get_module_public_items(module_id);

        for (name, def_id) in items {
            // Import based on def kind (skip if already exists)
            if let Some(info) = self.resolver.def_info.get(&def_id) {
                let result = match info.kind {
                    hir::DefKind::Struct | hir::DefKind::Enum | hir::DefKind::TypeAlias
                    | hir::DefKind::Effect | hir::DefKind::Trait => {
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

                // Track public re-exports
                if visibility == ast::Visibility::Public {
                    self.add_reexport_to_current_module(name, def_id);
                }
            }
        }

        Ok(())
    }

    /// Resolve an import path to find the target definition.
    fn resolve_import_path(&mut self, path: &ast::ModulePath) -> Result<DefId, TypeError> {
        if path.segments.is_empty() {
            return Err(TypeError::new(
                TypeErrorKind::ImportError {
                    message: "empty import path".to_string(),
                },
                path.span,
            ));
        }

        // Walk the path segments
        let mut current_scope_names: Vec<String> = path.segments.iter()
            .map(|s| self.symbol_to_string(s.node))
            .collect();

        // The last segment is the item we're importing
        let item_name = current_scope_names.pop().unwrap();

        if current_scope_names.is_empty() {
            // Simple case: just a name (e.g., `use foo;`)
            // Look it up in the current scope
            if let Some(Binding::Def(def_id)) = self.resolver.lookup(&item_name) {
                return Ok(def_id);
            }
            if let Some(def_id) = self.resolver.lookup_type(&item_name) {
                return Ok(def_id);
            }
            return Err(TypeError::new(
                TypeErrorKind::ImportError {
                    message: format!("cannot find `{}` in scope", item_name),
                },
                path.span,
            ));
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
            Err(TypeError::new(
                TypeErrorKind::ImportError {
                    message: format!("cannot find `{}` in module", item_name),
                },
                path.span,
            ))
        }
    }

    /// Resolve a module path to find the module DefId.
    fn resolve_module_path(&self, path: &ast::ModulePath) -> Result<DefId, TypeError> {
        if path.segments.is_empty() {
            return Err(TypeError::new(
                TypeErrorKind::ImportError {
                    message: "empty module path".to_string(),
                },
                path.span,
            ));
        }

        let first_name = self.symbol_to_string(path.segments[0].node);

        // Look up the first segment in multiple places:
        // 1. Value bindings in current scope
        // 2. Type bindings in current scope
        // 3. External modules registered via register_external_module
        let mut current_id = if let Some(Binding::Def(def_id)) = self.resolver.lookup(&first_name) {
            def_id
        } else if let Some(def_id) = self.resolver.lookup_type(&first_name) {
            def_id
        } else if let Some(def_id) = self.find_root_module_by_name(&first_name) {
            // Check for external modules like "std"
            def_id
        } else {
            return Err(TypeError::new(
                TypeErrorKind::ImportError {
                    message: format!("cannot find module `{}`", first_name),
                },
                path.span,
            ));
        };

        // Walk the remaining segments
        for segment in &path.segments[1..] {
            let name = self.symbol_to_string(segment.node);

            if let Some(def_id) = self.lookup_in_module(current_id, &name) {
                current_id = def_id;
            } else {
                return Err(TypeError::new(
                    TypeErrorKind::ImportError {
                        message: format!("cannot find `{}` in module path", name),
                    },
                    path.span,
                ));
            }
        }

        Ok(current_id)
    }

    /// Look up an item by name in a module.
    fn lookup_in_module(&self, module_id: DefId, name: &str) -> Option<DefId> {
        // Check if we have module info for this DefId
        if let Some(module_def) = self.module_defs.get(&module_id) {
            // Look in the module's items
            for &item_id in &module_def.items {
                if let Some(info) = self.resolver.def_info.get(&item_id) {
                    if info.name == name {
                        return Some(item_id);
                    }
                }
            }

            // Also check re-exports (pub use)
            for (reexport_name, def_id) in &module_def.reexports {
                if reexport_name == name {
                    return Some(*def_id);
                }
            }
        }
        None
    }

    /// Get all public items from a module (including re-exports).
    fn get_module_public_items(&self, module_id: DefId) -> Vec<(String, DefId)> {
        use crate::ast::Visibility;

        let mut items = Vec::new();

        if let Some(module_def) = self.module_defs.get(&module_id) {
            // Include public items defined in the module
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

            // Include re-exports (pub use)
            for (name, def_id) in &module_def.reexports {
                items.push((name.clone(), *def_id));
            }
        }

        items
    }

    /// Find a root-level module by name.
    ///
    /// This searches through registered external modules to find one with the given name.
    /// Used for resolving the first segment of import paths like `std.compiler.lexer`.
    fn find_root_module_by_name(&self, name: &str) -> Option<DefId> {
        // Search through all registered modules
        for (&def_id, info) in &self.module_defs {
            // Only consider root-level external modules
            if info.is_external && info.name == name {
                return Some(def_id);
            }
        }
        None
    }

    /// Collect a declaration.
    pub(crate) fn collect_declaration(&mut self, decl: &ast::Declaration) -> Result<(), TypeError> {
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
            // Macro invocations should be expanded before type checking
            ast::Declaration::MacroInvocation(_) => Ok(()),
            // Use declarations are handled during import resolution
            ast::Declaration::Use(_) => Ok(()),
        }
    }

    /// Collect a macro declaration.
    ///
    /// Macros are expanded before type checking, so this just registers the macro
    /// name in the current scope for later lookup during expansion.
    pub(crate) fn collect_macro(&mut self, _macro_decl: &ast::MacroDecl) -> Result<(), TypeError> {
        // TODO: Register macro in macro namespace for expansion phase
        // For now, macros are handled separately in the expansion phase
        Ok(())
    }

    /// Collect a bridge declaration (FFI).
    pub(crate) fn collect_bridge(&mut self, bridge: &ast::BridgeDecl) -> Result<(), TypeError> {
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
                    let fields: Vec<_> = s.fields.iter()
                        .map(|f| {
                            let field_name = self.symbol_to_string(f.name.node);
                            let ty = self.ast_type_to_hir_type(&f.ty)?;
                            Ok(BridgeFieldInfo {
                                name: field_name,
                                ty,
                                span: f.span,
                            })
                        })
                        .collect::<Result<_, TypeError>>()?;

                    // Extract packed and align from attributes
                    let (is_packed, align) = self.extract_struct_attrs(&s.attrs);

                    structs.push(BridgeStructInfo {
                        def_id,
                        name,
                        fields,
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
                        .collect::<Result<_, TypeError>>()?;
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
    pub(crate) fn collect_function(&mut self, func: &ast::FnDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(func.name.node);
        // Use define_function for multiple dispatch support
        let def_id = self.resolver.define_function(
            name.clone(),
            func.span,
            func.vis,
        )?;

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

        // Restore previous generic params scope (after processing signature including effects)
        self.generic_params = saved_generic_params;
        self.const_params = saved_const_params;
        self.lifetime_params = saved_lifetime_params;

        let mut sig = hir::FnSig::new(param_types, return_type);
        sig.generics = generics_vec;
        self.fn_sigs.insert(def_id, sig);

        // Queue function for later body type-checking
        if func.body.is_some() {
            self.pending_bodies.push((def_id, func.clone(), self.current_module));
        }

        Ok(())
    }

    /// Collect a struct declaration.
    pub(crate) fn collect_struct(&mut self, struct_decl: &ast::StructDecl) -> Result<(), TypeError> {
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
                    .collect::<Result<Vec<_>, TypeError>>()?
            }
            ast::StructBody::Tuple(fields) => {
                fields
                    .iter()
                    .enumerate()
                    .map(|(i, field)| {
                        let ty = self.ast_type_to_hir_type(&field.ty)?;
                        Ok(FieldInfo {
                            name: format!("{i}"),
                            ty,
                            index: i as u32,
                        })
                    })
                    .collect::<Result<Vec<_>, TypeError>>()?
            }
            ast::StructBody::Unit => Vec::new(),
        };

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
    pub(crate) fn collect_type_alias(&mut self, type_decl: &ast::TypeDecl) -> Result<(), TypeError> {
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
        // For associated type declarations without a definition, skip
        let Some(ref ty) = type_decl.ty else {
            // Restore previous generic params scope
            self.generic_params = saved_generic_params;
            self.const_params = saved_const_params;
            self.lifetime_params = saved_lifetime_params;
            return Ok(());
        };
        let aliased_ty = self.ast_type_to_hir_type(ty)?;

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
    pub(crate) fn collect_enum(&mut self, enum_decl: &ast::EnumDecl) -> Result<(), TypeError> {
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
                        .collect::<Result<Vec<_>, TypeError>>()?
                }
                ast::StructBody::Tuple(fields) => {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(j, field)| {
                            let ty = self.ast_type_to_hir_type(&field.ty)?;
                            Ok(FieldInfo {
                                name: format!("{j}"),
                                ty,
                                index: j as u32,
                            })
                        })
                        .collect::<Result<Vec<_>, TypeError>>()?
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
    pub(crate) fn collect_const(&mut self, const_decl: &ast::ConstDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(const_decl.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Const,
            const_decl.span,
        )?;

        // Convert the declared type
        let ty = self.ast_type_to_hir_type(&const_decl.ty)?;

        // Set the type in def_info so it can be looked up during expression type inference
        if let Some(def_info) = self.resolver.def_info.get_mut(&def_id) {
            def_info.ty = Some(ty.clone());
        }

        // Queue for body type-checking (the value expression)
        self.pending_consts.push((def_id, const_decl.clone()));

        // Store a placeholder - body_id will be assigned during check_const_body
        // We use a dummy body_id here that will be replaced
        let placeholder_body_id = hir::BodyId::new(u32::MAX);
        self.const_defs.insert(def_id, super::ConstInfo {
            name,
            ty,
            body_id: placeholder_body_id,
            span: const_decl.span,
        });

        Ok(())
    }

    /// Collect a static declaration.
    ///
    /// This registers the static item and queues its value expression for type checking.
    pub(crate) fn collect_static(&mut self, static_decl: &ast::StaticDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(static_decl.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Static,
            static_decl.span,
        )?;

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
    pub(crate) fn collect_effect(&mut self, effect: &ast::EffectDecl) -> Result<(), TypeError> {
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
    pub(crate) fn collect_handler(&mut self, handler: &ast::HandlerDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(handler.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Handler,
            handler.span,
        )?;

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
            .ok_or_else(|| TypeError::new(
                TypeErrorKind::NotAnEffect { name: "unknown".to_string() },
                handler.effect.span,
            ))?;
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
        });

        // Queue handler for body type-checking
        self.pending_handlers.push((def_id, handler.clone()));

        Ok(())
    }

    /// Collect an impl block declaration.
    pub(crate) fn collect_impl_block(&mut self, impl_block: &ast::ImplBlock) -> Result<(), TypeError> {
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
                        return Err(TypeError::new(
                            TypeErrorKind::TypeNotFound { name: "empty trait path".to_string() },
                            impl_block.span,
                        ));
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
                                    return Err(TypeError::new(
                                        TypeErrorKind::TraitNotFound { name: trait_name },
                                        trait_ty.span,
                                    ));
                                }
                            } else {
                                return Err(TypeError::new(
                                    TypeErrorKind::TraitNotFound { name: trait_name },
                                    trait_ty.span,
                                ));
                            }
                        }
                        _ => {
                            return Err(TypeError::new(
                                TypeErrorKind::TraitNotFound { name: trait_name },
                                trait_ty.span,
                            ));
                        }
                    }
                }
                _ => {
                    return Err(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::unit(), // Placeholder
                            found: Type::unit(),
                        },
                        trait_ty.span,
                    ));
                }
            }
        } else {
            None
        };

        // Process impl items (methods, associated types, associated constants)
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        let mut assoc_consts = Vec::new();

        for item in &impl_block.items {
            match item {
                ast::ImplItem::Function(func) => {
                    let method_name = self.symbol_to_string(func.name.node);

                    // Create a qualified name for the method: Type::method_name
                    let qualified_name = format!("{}::{}", self.type_to_string(&self_ty), method_name);

                    // Register the method as an associated function
                    let method_def_id = self.resolver.define_item(
                        qualified_name.clone(),
                        hir::DefKind::AssocFn,
                        func.span,
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
                    };

                    self.fn_sigs.insert(method_def_id, sig);

                    // Queue method body for type-checking
                    // Include module context if this impl is inside a module
                    self.pending_bodies.push((method_def_id, func.clone(), self.current_module));

                    methods.push(ImplMethodInfo {
                        def_id: method_def_id,
                        name: method_name,
                        is_static,
                    });
                }
                ast::ImplItem::Type(type_decl) => {
                    let type_name = self.symbol_to_string(type_decl.name.node);
                    // Associated types in impl blocks must have a type value
                    let Some(ref ast_ty) = type_decl.ty else {
                        return Err(TypeError {
                            kind: TypeErrorKind::MissingAssocType {
                                trait_name: "impl".to_string(),
                                type_name: type_name.clone(),
                            },
                            span: type_decl.span,
                            help: Some("associated types in impl blocks must have a value".to_string()),
                        });
                    };
                    let ty = self.ast_type_to_hir_type(ast_ty)?;
                    assoc_types.push(ImplAssocTypeInfo {
                        name: type_name,
                        ty,
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
    pub(crate) fn collect_trait(&mut self, trait_decl: &ast::TraitDecl) -> Result<(), TypeError> {
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
                                        return Err(TypeError::new(
                                            TypeErrorKind::TraitNotFound { name: supertrait_name },
                                            supertrait.span,
                                        ));
                                    }
                                }
                            }
                            _ => {
                                return Err(TypeError::new(
                                    TypeErrorKind::TraitNotFound { name: supertrait_name },
                                    supertrait.span,
                                ));
                            }
                        }
                    }
                }
                _ => {
                    return Err(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "complex supertrait bounds".to_string(),
                        },
                        supertrait.span,
                    ));
                }
            }
        }

        // Process trait items
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        let mut assoc_consts = Vec::new();

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
                ast::TraitItem::Type(type_decl) => {
                    let type_name = self.symbol_to_string(type_decl.name.node);
                    // For associated types, the `ty` field in TypeDecl is the default (if present)
                    let default = if let Some(ref ast_ty) = type_decl.ty {
                        // Try to convert the type - if it's just a name binding, there's no default
                        match self.ast_type_to_hir_type(ast_ty) {
                            Ok(ty) if !ty.is_error() => Some(ty),
                            _ => None,
                        }
                    } else {
                        None
                    };

                    assoc_types.push(TraitAssocTypeInfo {
                        name: type_name,
                        default,
                    });
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
    pub(crate) fn parse_effect_row(&mut self, effect_row: &ast::EffectRow) -> Result<(Vec<EffectRef>, Option<String>), TypeError> {
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
    pub(crate) fn resolve_effect_type(&mut self, ty: &ast::Type) -> Result<Option<EffectRef>, TypeError> {
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
                    ast::TypeKind::ImplTrait { .. } => "impl trait type",
                    ast::TypeKind::DynTrait { .. } => "dyn trait type",
                    ast::TypeKind::MaybeUnsized { .. } => "relaxed bound type",
                    ast::TypeKind::Wildcard => "wildcard type",
                    ast::TypeKind::Path(_) => unreachable!("Path type should be handled by the match above")
                };
                Err(TypeError::new(
                    TypeErrorKind::InvalidEffectType { found: found.to_string() },
                    ty.span,
                ))
            }
        }
    }

    /// Register effect operations in the current scope based on a function's declared effects.
    ///
    /// This makes effect operations like `get()` and `put()` available within functions
    /// that declare they use those effects (e.g., `fn counter() / {State<i32>}`).
    pub(crate) fn register_effect_operations_in_scope(&mut self, fn_def_id: DefId) -> Result<(), TypeError> {
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
    /// For external modules (`mod foo;`), we would need to load the file - not yet implemented.
    pub(crate) fn collect_module(&mut self, module: &ast::ModItemDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(module.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Mod,
            module.span,
        )?;

        match &module.body {
            Some(declarations) => {
                // Inline module - push a new scope and collect declarations
                self.resolver.push_scope(ScopeKind::Module, module.span);

                // Track the current module for functions defined inside
                let saved_module = self.current_module;
                self.current_module = Some(def_id);

                // Track the starting DefId counter
                let def_id_start = self.resolver.current_def_id_counter();

                for decl in declarations {
                    if let Err(e) = self.collect_declaration(decl) {
                        self.errors.push(e);
                    }
                }

                // Collect DefIds of items defined during this module's collection
                let def_id_end = self.resolver.current_def_id_counter();
                let item_def_ids: Vec<DefId> = (def_id_start..def_id_end)
                    .map(DefId::new)
                    .collect();

                // Restore previous module context
                self.current_module = saved_module;
                self.resolver.pop_scope();

                // Build index for O(1) lookup by name
                let mut items_by_name = std::collections::HashMap::new();
                for &item_id in &item_def_ids {
                    if let Some(info) = self.resolver.def_info.get(&item_id) {
                        items_by_name.insert(info.name.clone(), item_id);
                    }
                }

                // Store module info
                self.module_defs.insert(def_id, super::ModuleInfo {
                    name,
                    items: item_def_ids,
                    reexports: Vec::new(),
                    is_external: false,
                    span: module.span,
                    items_by_name,
                });
            }
            None => {
                // External module - load from file
                let source_dir = match &self.source_path {
                    Some(path) => path.parent().map(|p| p.to_path_buf()),
                    None => None,
                };

                let source_dir = source_dir.ok_or_else(|| TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: format!(
                            "external modules (`mod {};`) - no source path available. \
                            Use `with_source_path()` when creating the type context.",
                            name
                        ),
                    },
                    module.span,
                ))?;

                // Try to find the module file: first `name.blood`, then `name/mod.blood`
                let file_path = source_dir.join(format!("{}.blood", name));
                let alt_path = source_dir.join(&name).join("mod.blood");

                let module_path = if file_path.exists() {
                    file_path
                } else if alt_path.exists() {
                    alt_path
                } else {
                    return Err(TypeError::new(
                        TypeErrorKind::ModuleNotFound {
                            name: name.clone(),
                            searched_paths: vec![
                                file_path.display().to_string(),
                                alt_path.display().to_string(),
                            ],
                        },
                        module.span,
                    ));
                };

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
                            return Err(TypeError::new(
                                TypeErrorKind::ParseError {
                                    message: first_err.message,
                                },
                                module.span,
                            ));
                        }
                        return Err(TypeError::new(
                            TypeErrorKind::ParseError {
                                message: "unknown parse error".to_string(),
                            },
                            module.span,
                        ));
                    }
                };

                // Process the declarations in a new scope
                self.resolver.push_scope(ScopeKind::Module, module.span);

                // Track the current module for functions defined inside
                let saved_module = self.current_module;
                self.current_module = Some(def_id);

                // Track the starting DefId counter
                let def_id_start = self.resolver.current_def_id_counter();

                for decl in &module_ast.declarations {
                    if let Err(e) = self.collect_declaration(decl) {
                        self.errors.push(e);
                    }
                }

                // Collect DefIds of items defined during this module's collection
                let def_id_end = self.resolver.current_def_id_counter();
                let item_def_ids: Vec<DefId> = (def_id_start..def_id_end)
                    .map(DefId::new)
                    .collect();

                // Restore previous module context
                self.current_module = saved_module;
                self.resolver.pop_scope();

                // Build index for O(1) lookup by name
                let mut items_by_name = std::collections::HashMap::new();
                for &item_id in &item_def_ids {
                    if let Some(info) = self.resolver.def_info.get(&item_id) {
                        items_by_name.insert(info.name.clone(), item_id);
                    }
                }

                // Store module info
                self.module_defs.insert(def_id, super::ModuleInfo {
                    name,
                    items: item_def_ids,
                    reexports: Vec::new(),
                    is_external: true,
                    span: module.span,
                    items_by_name,
                });
            }
        }

        Ok(())
    }
}
