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
use super::super::resolve::Binding;

impl<'a> TypeContext<'a> {
    /// Resolve names in a program.
    pub fn resolve_program(&mut self, program: &ast::Program) -> Result<(), Vec<Diagnostic>> {
        // First pass: collect all top-level definitions
        for decl in &program.declarations {
            if let Err(e) = self.collect_declaration(decl) {
                self.errors.push(e);
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors.iter().map(|e| e.to_diagnostic()).collect());
        }

        Ok(())
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
        }
    }

    /// Collect a function declaration.
    pub(crate) fn collect_function(&mut self, func: &ast::FnDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(func.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Fn,
            func.span,
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

        // Restore previous generic params scope
        self.generic_params = saved_generic_params;
        self.const_params = saved_const_params;
        self.lifetime_params = saved_lifetime_params;

        let mut sig = hir::FnSig::new(param_types, return_type);
        sig.generics = generics_vec;
        self.fn_sigs.insert(def_id, sig);

        // Parse and store the function's effect annotation
        if let Some(ref effect_row) = func.effects {
            let effects = self.parse_effect_row(effect_row)?;
            if !effects.is_empty() {
                self.fn_effects.insert(def_id, effects);
            }
        }

        // Queue function for later body type-checking
        if func.body.is_some() {
            self.pending_bodies.push((def_id, func.clone()));
        }

        Ok(())
    }

    /// Collect a struct declaration.
    pub(crate) fn collect_struct(&mut self, struct_decl: &ast::StructDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(struct_decl.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Struct,
            struct_decl.span,
        )?;

        // Also define as a type
        self.resolver.define_type(name.clone(), def_id, struct_decl.span)?;

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
                    .collect::<Result<Vec<_>, TypeError>>()?
            }
            ast::StructBody::Unit => Vec::new(),
        };

        // Restore previous generic params scope
        self.generic_params = saved_generic_params;
        self.const_params = saved_const_params;
        self.lifetime_params = saved_lifetime_params;

        self.struct_defs.insert(def_id, StructInfo {
            name,
            fields,
            generics: generics_vec,
        });

        Ok(())
    }

    /// Collect a type alias declaration.
    pub(crate) fn collect_type_alias(&mut self, type_decl: &ast::TypeDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(type_decl.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::TypeAlias,
            type_decl.span,
        )?;

        // Also define as a type so it can be referenced
        self.resolver.define_type(name.clone(), def_id, type_decl.span)?;

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
        let aliased_ty = self.ast_type_to_hir_type(&type_decl.ty)?;

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
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Enum,
            enum_decl.span,
        )?;

        // Also define as a type
        self.resolver.define_type(name.clone(), def_id, enum_decl.span)?;

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

            // Define variant in scope
            let variant_def_id = self.resolver.define_item(
                variant_name.clone(),
                hir::DefKind::Variant,
                variant.span,
            )?;

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
            name,
            variants,
            generics: generics_vec,
        });

        Ok(())
    }

    /// Collect a const declaration.
    ///
    /// Currently, const items are parsed but not yet fully implemented in codegen.
    /// This function reports an explicit error rather than silently ignoring the declaration.
    pub(crate) fn collect_const(&mut self, const_decl: &ast::ConstDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(const_decl.name.node);
        Err(TypeError::new(
            TypeErrorKind::UnsupportedFeature {
                feature: format!("const item `{}` - const declarations are not yet implemented", name),
            },
            const_decl.span,
        ))
    }

    /// Collect a static declaration.
    ///
    /// Currently, static items are parsed but not yet fully implemented in codegen.
    /// This function reports an explicit error rather than silently ignoring the declaration.
    pub(crate) fn collect_static(&mut self, static_decl: &ast::StaticDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(static_decl.name.node);
        Err(TypeError::new(
            TypeErrorKind::UnsupportedFeature {
                feature: format!("static item `{}` - static declarations are not yet implemented", name),
            },
            static_decl.span,
        ))
    }

    /// Collect an effect declaration.
    pub(crate) fn collect_effect(&mut self, effect: &ast::EffectDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(effect.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Effect,
            effect.span,
        )?;

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
                    self.pending_bodies.push((method_def_id, func.clone()));

                    methods.push(ImplMethodInfo {
                        def_id: method_def_id,
                        name: method_name,
                        is_static,
                    });
                }
                ast::ImplItem::Type(type_decl) => {
                    let type_name = self.symbol_to_string(type_decl.name.node);
                    let ty = self.ast_type_to_hir_type(&type_decl.ty)?;
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

        Ok(())
    }

    /// Collect a trait declaration.
    pub(crate) fn collect_trait(&mut self, trait_decl: &ast::TraitDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(trait_decl.name.node);

        // Register the trait
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Trait,
            trait_decl.span,
        )?;

        // Save current generic params
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let mut generics_vec = Vec::new();

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
                        self.pending_bodies.push((method_def_id, func.clone()));
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
                    // For associated types, the `ty` field in TypeDecl is the default
                    // Check if it's meaningful (not just a placeholder)
                    let default = if type_decl.type_params.is_none() {
                        // Try to convert the type - if it's just a name binding, there's no default
                        match self.ast_type_to_hir_type(&type_decl.ty) {
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

        // Restore generic params
        self.generic_params = saved_generic_params;

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

    /// Parse an effect row annotation into a list of EffectRefs.
    pub(crate) fn parse_effect_row(&mut self, effect_row: &ast::EffectRow) -> Result<Vec<EffectRef>, TypeError> {
        match &effect_row.kind {
            ast::EffectRowKind::Pure => Ok(Vec::new()),
            ast::EffectRowKind::Var(_) => {
                // Effect polymorphism - Phase 2+
                Ok(Vec::new())
            }
            ast::EffectRowKind::Effects { effects, rest: _ } => {
                let mut result = Vec::new();
                for effect_ty in effects {
                    if let Some(effect_ref) = self.resolve_effect_type(effect_ty)? {
                        result.push(effect_ref);
                    }
                }
                Ok(result)
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
                if let Some(binding) = self.resolver.lookup(&effect_name) {
                    if let Binding::Def(def_id) = binding {
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
}
