//! Type checking context.
//!
//! The TypeContext is the main entry point for type checking. It coordinates
//! name resolution, type collection, and type inference.

use std::collections::HashMap;

use string_interner::{DefaultStringInterner, Symbol as _};

use crate::ast;
use crate::hir::{self, DefId, LocalId, Type, TypeKind, PrimitiveTy, TyVarId};
use crate::hir::def::{IntTy, UintTy};
use crate::span::Span;
use crate::diagnostics::Diagnostic;
use crate::ice;

use super::error::{TypeError, TypeErrorKind};
use super::resolve::{Resolver, ScopeKind, Binding};
use super::unify::Unifier;

/// The main type checking context.
pub struct TypeContext<'a> {
    /// The source code (reserved for future use in error messages).
    _source: &'a str,
    /// The string interner for resolving symbols.
    interner: DefaultStringInterner,
    /// The name resolver.
    pub resolver: Resolver<'a>,
    /// The type unifier.
    pub unifier: Unifier,
    /// Type signatures for functions.
    fn_sigs: HashMap<DefId, hir::FnSig>,
    /// Struct definitions.
    struct_defs: HashMap<DefId, StructInfo>,
    /// Enum definitions.
    enum_defs: HashMap<DefId, EnumInfo>,
    /// Type alias definitions.
    type_aliases: HashMap<DefId, TypeAliasInfo>,
    /// Functions to type-check (includes full declaration for parameter names).
    pending_bodies: Vec<(DefId, ast::FnDecl)>,
    /// The current function's return type.
    return_type: Option<Type>,
    /// The current function's DefId (for effect checking).
    current_fn: Option<DefId>,
    /// Stack of currently handled effects (from enclosing with...handle blocks).
    handled_effects: Vec<DefId>,
    /// Errors encountered.
    errors: Vec<TypeError>,
    /// Compiled bodies.
    bodies: HashMap<hir::BodyId, hir::Body>,
    /// Mapping from function DefId to its body.
    fn_bodies: HashMap<DefId, hir::BodyId>,
    /// Next body ID.
    next_body_id: u32,
    /// Local variables in current function.
    locals: Vec<hir::Local>,
    /// Current generic type parameters in scope (name -> TyVarId).
    /// This is populated when entering a generic function/struct/enum
    /// and cleared when leaving.
    generic_params: HashMap<String, TyVarId>,
    /// Next type parameter ID for generating unique TyVarIds.
    next_type_param_id: u32,
    /// Builtin function names (DefId -> function name).
    /// Used by codegen to resolve runtime function calls.
    builtin_fns: HashMap<DefId, String>,
    /// Effect definitions.
    effect_defs: HashMap<DefId, EffectInfo>,
    /// Handler definitions.
    handler_defs: HashMap<DefId, HandlerInfo>,
    /// Effect annotations for functions (DefId -> list of effects the function uses).
    fn_effects: HashMap<DefId, Vec<EffectRef>>,
    /// Handlers to type-check (includes full declaration for operation bodies).
    pending_handlers: Vec<(DefId, ast::HandlerDecl)>,
}

/// Information about a struct.
#[derive(Debug, Clone)]
pub struct StructInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub generics: Vec<TyVarId>,
}

/// Information about a field.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Type,
    pub index: u32,
}

/// Information about an enum.
#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub name: String,
    pub variants: Vec<VariantInfo>,
    pub generics: Vec<TyVarId>,
}

/// Information about a type alias.
#[derive(Debug, Clone)]
pub struct TypeAliasInfo {
    pub name: String,
    pub ty: Type,
    pub generics: Vec<TyVarId>,
}

/// Information about a variant.
#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub index: u32,
    pub def_id: DefId,
}

/// Information about an effect.
#[derive(Debug, Clone)]
pub struct EffectInfo {
    pub name: String,
    pub operations: Vec<OperationInfo>,
    pub generics: Vec<TyVarId>,
}

/// Information about an effect operation.
#[derive(Debug, Clone)]
pub struct OperationInfo {
    pub name: String,
    pub params: Vec<Type>,
    pub return_ty: Type,
    pub def_id: DefId,
}

/// Information about a handler.
#[derive(Debug, Clone)]
pub struct HandlerInfo {
    pub name: String,
    /// Handler kind: deep (reify continuation) or shallow (single shot).
    pub kind: ast::HandlerKind,
    /// The effect this handler handles (DefId of the effect).
    pub effect_id: DefId,
    /// The operations implemented by this handler (name, body_id).
    pub operations: Vec<(String, hir::BodyId)>,
    pub generics: Vec<TyVarId>,
    /// State fields in the handler (used for struct-like initialization).
    pub fields: Vec<FieldInfo>,
    /// Return clause body ID, if present.
    pub return_clause_body_id: Option<hir::BodyId>,
}

/// A reference to an effect with type arguments.
/// For example, `State<i32>` would be EffectRef { def_id: State's DefId, type_args: [i32] }
#[derive(Debug, Clone)]
pub struct EffectRef {
    /// The effect definition this refers to.
    pub def_id: DefId,
    /// Type arguments instantiating the effect's generics.
    pub type_args: Vec<Type>,
}

impl<'a> TypeContext<'a> {
    /// Create a new type context.
    pub fn new(source: &'a str, interner: DefaultStringInterner) -> Self {
        let mut ctx = Self {
            _source: source,
            interner,
            resolver: Resolver::new(source),
            unifier: Unifier::new(),
            fn_sigs: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            type_aliases: HashMap::new(),
            pending_bodies: Vec::new(),
            return_type: None,
            current_fn: None,
            handled_effects: Vec::new(),
            errors: Vec::new(),
            bodies: HashMap::new(),
            fn_bodies: HashMap::new(),
            next_body_id: 0,
            locals: Vec::new(),
            generic_params: HashMap::new(),
            next_type_param_id: 0,
            builtin_fns: HashMap::new(),
            effect_defs: HashMap::new(),
            handler_defs: HashMap::new(),
            fn_effects: HashMap::new(),
            pending_handlers: Vec::new(),
        };
        ctx.register_builtins();
        ctx
    }

    /// Register built-in runtime functions.
    fn register_builtins(&mut self) {
        let unit_ty = Type::unit();
        let bool_ty = Type::bool();
        let i32_ty = Type::i32();
        let i64_ty = Type::i64();
        let string_ty = Type::string();
        let never_ty = Type::never();

        // === I/O Functions ===

        // print(String) -> () - convenience function (maps to runtime print_str)
        self.register_builtin_fn_aliased("print", "print_str", vec![string_ty.clone()], unit_ty.clone());

        // println(String) -> () - convenience function (prints string + newline, maps to runtime println_str)
        self.register_builtin_fn_aliased("println", "println_str", vec![string_ty.clone()], unit_ty.clone());

        // print_int(i32) -> ()
        self.register_builtin_fn("print_int", vec![i32_ty.clone()], unit_ty.clone());

        // println_int(i32) -> ()
        self.register_builtin_fn("println_int", vec![i32_ty.clone()], unit_ty.clone());

        // print_str(String) -> () - legacy name, same as print
        self.register_builtin_fn("print_str", vec![string_ty.clone()], unit_ty.clone());

        // println_str(String) -> () - legacy name, same as println
        self.register_builtin_fn("println_str", vec![string_ty.clone()], unit_ty.clone());

        // print_char(i32) -> ()  (char as i32 for now)
        self.register_builtin_fn("print_char", vec![i32_ty.clone()], unit_ty.clone());

        // print_newline() -> () - prints just a newline
        self.register_builtin_fn("print_newline", vec![], unit_ty.clone());

        // print_bool(bool) -> ()
        self.register_builtin_fn("print_bool", vec![bool_ty.clone()], unit_ty.clone());

        // println_bool(bool) -> ()
        self.register_builtin_fn("println_bool", vec![bool_ty.clone()], unit_ty.clone());

        // === Control Flow / Assertions ===

        // panic(String) -> !
        self.register_builtin_fn("panic", vec![string_ty.clone()], never_ty.clone());

        // assert(bool) -> ()
        self.register_builtin_fn("assert", vec![bool_ty.clone()], unit_ty.clone());

        // assert_eq(i32, i32) -> ()
        self.register_builtin_fn("assert_eq_int", vec![i32_ty.clone(), i32_ty.clone()], unit_ty.clone());

        // assert_eq(bool, bool) -> ()
        self.register_builtin_fn("assert_eq_bool", vec![bool_ty.clone(), bool_ty.clone()], unit_ty.clone());

        // unreachable() -> !
        self.register_builtin_fn("unreachable", vec![], never_ty.clone());

        // todo() -> !
        self.register_builtin_fn("todo", vec![], never_ty.clone());

        // === Memory Functions ===

        // size_of_i32() -> i64
        self.register_builtin_fn("size_of_i32", vec![], i64_ty.clone());

        // size_of_i64() -> i64
        self.register_builtin_fn("size_of_i64", vec![], i64_ty.clone());

        // size_of_bool() -> i64
        self.register_builtin_fn("size_of_bool", vec![], i64_ty.clone());

        // === Conversion Functions ===

        // int_to_string(i32) -> String
        self.register_builtin_fn("int_to_string", vec![i32_ty.clone()], string_ty.clone());

        // bool_to_string(bool) -> String
        self.register_builtin_fn("bool_to_string", vec![bool_ty.clone()], string_ty.clone());

        // i32_to_i64(i32) -> i64
        self.register_builtin_fn("i32_to_i64", vec![i32_ty.clone()], i64_ty.clone());

        // i64_to_i32(i64) -> i32
        self.register_builtin_fn("i64_to_i32", vec![i64_ty.clone()], i32_ty.clone());
    }

    /// Register a single built-in function.
    fn register_builtin_fn(&mut self, name: &str, inputs: Vec<Type>, output: Type) {
        self.register_builtin_fn_aliased(name, name, inputs, output);
    }

    /// Register a builtin function with a user-facing name that maps to a different runtime name.
    /// E.g., `println(String)` maps to runtime function `println_str`.
    fn register_builtin_fn_aliased(&mut self, user_name: &str, runtime_name: &str, inputs: Vec<Type>, output: Type) {
        let def_id = self.resolver.define_item(
            user_name.to_string(),
            hir::DefKind::Fn,
            Span::dummy(),
        ).expect("BUG: builtin registration failed - this indicates a name collision in builtin definitions");

        self.fn_sigs.insert(def_id, hir::FnSig {
            inputs,
            output,
            is_const: false,
            is_async: false,
            is_unsafe: false,
            generics: Vec::new(),
        });

        // Track runtime function name for codegen to resolve runtime function calls
        self.builtin_fns.insert(def_id, runtime_name.to_string());
    }

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
    fn collect_declaration(&mut self, decl: &ast::Declaration) -> Result<(), TypeError> {
        match decl {
            ast::Declaration::Function(f) => self.collect_function(f),
            ast::Declaration::Struct(s) => self.collect_struct(s),
            ast::Declaration::Enum(e) => self.collect_enum(e),
            ast::Declaration::Const(c) => self.collect_const(c),
            ast::Declaration::Static(s) => self.collect_static(s),
            ast::Declaration::Effect(e) => self.collect_effect(e),
            ast::Declaration::Handler(h) => self.collect_handler(h),
            ast::Declaration::Type(t) => self.collect_type_alias(t),
            ast::Declaration::Impl(i) => {
                // Impl blocks - Phase 2+
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "impl blocks are not yet supported".to_string(),
                    },
                    i.span,
                ))
            }
            ast::Declaration::Trait(t) => {
                // Trait declarations - Phase 2+
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "trait declarations are not yet supported".to_string(),
                    },
                    t.span,
                ))
            }
        }
    }

    /// Collect a function declaration.
    fn collect_function(&mut self, func: &ast::FnDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(func.name.node);
        let def_id = self.resolver.define_item(
            name.clone(),
            hir::DefKind::Fn,
            func.span,
        )?;

        // Register generic type parameters before processing parameter types
        // This allows type references like `T` to be resolved in the function signature
        let saved_generic_params = std::mem::take(&mut self.generic_params);
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = func.type_params {
            for type_param in &type_params.params {
                let param_name = self.symbol_to_string(type_param.name.node);
                let ty_var_id = TyVarId(self.next_type_param_id);
                self.next_type_param_id += 1;
                self.generic_params.insert(param_name, ty_var_id);
                generics_vec.push(ty_var_id);
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

    /// Parse an effect row annotation into a list of EffectRefs.
    fn parse_effect_row(&mut self, effect_row: &ast::EffectRow) -> Result<Vec<EffectRef>, TypeError> {
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
    fn resolve_effect_type(&mut self, ty: &ast::Type) -> Result<Option<EffectRef>, TypeError> {
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
                    if let super::resolve::Binding::Def(def_id) = binding {
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
    fn register_effect_operations_in_scope(&mut self, fn_def_id: DefId) -> Result<(), TypeError> {
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
                    .insert(op_info.name.clone(), super::resolve::Binding::Def(op_info.def_id));
            }
        }

        Ok(())
    }

    /// Collect a struct declaration.
    fn collect_struct(&mut self, struct_decl: &ast::StructDecl) -> Result<(), TypeError> {
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
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = struct_decl.type_params {
            for type_param in &type_params.params {
                let param_name = self.symbol_to_string(type_param.name.node);
                let ty_var_id = TyVarId(self.next_type_param_id);
                self.next_type_param_id += 1;
                self.generic_params.insert(param_name, ty_var_id);
                generics_vec.push(ty_var_id);
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

        self.struct_defs.insert(def_id, StructInfo {
            name,
            fields,
            generics: generics_vec,
        });

        Ok(())
    }

    /// Collect a type alias declaration.
    fn collect_type_alias(&mut self, type_decl: &ast::TypeDecl) -> Result<(), TypeError> {
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
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = type_decl.type_params {
            for type_param in &type_params.params {
                let param_name = self.symbol_to_string(type_param.name.node);
                let ty_var_id = TyVarId(self.next_type_param_id);
                self.next_type_param_id += 1;
                self.generic_params.insert(param_name, ty_var_id);
                generics_vec.push(ty_var_id);
            }
        }

        // Convert the aliased type (now with generics in scope)
        let aliased_ty = self.ast_type_to_hir_type(&type_decl.ty)?;

        // Restore previous generic params scope
        self.generic_params = saved_generic_params;

        self.type_aliases.insert(def_id, TypeAliasInfo {
            name,
            ty: aliased_ty,
            generics: generics_vec,
        });

        Ok(())
    }

    /// Collect an enum declaration.
    fn collect_enum(&mut self, enum_decl: &ast::EnumDecl) -> Result<(), TypeError> {
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
        let mut generics_vec = Vec::new();

        if let Some(ref type_params) = enum_decl.type_params {
            for type_param in &type_params.params {
                let param_name = self.symbol_to_string(type_param.name.node);
                let ty_var_id = TyVarId(self.next_type_param_id);
                self.next_type_param_id += 1;
                self.generic_params.insert(param_name, ty_var_id);
                generics_vec.push(ty_var_id);
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

        self.enum_defs.insert(def_id, EnumInfo {
            name,
            variants,
            generics: generics_vec,
        });

        Ok(())
    }

    /// Collect a const declaration.
    fn collect_const(&mut self, const_decl: &ast::ConstDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(const_decl.name.node);
        let _def_id = self.resolver.define_item(
            name,
            hir::DefKind::Const,
            const_decl.span,
        )?;
        // Type-checking const values is deferred
        Ok(())
    }

    /// Collect a static declaration.
    fn collect_static(&mut self, static_decl: &ast::StaticDecl) -> Result<(), TypeError> {
        let name = self.symbol_to_string(static_decl.name.node);
        let _def_id = self.resolver.define_item(
            name,
            hir::DefKind::Static,
            static_decl.span,
        )?;
        Ok(())
    }

    /// Collect an effect declaration.
    fn collect_effect(&mut self, effect: &ast::EffectDecl) -> Result<(), TypeError> {
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
            for param in &type_params.params {
                let param_name = self.symbol_to_string(param.name.node);
                let ty_var = TyVarId(self.next_type_param_id);
                self.next_type_param_id += 1;
                self.generic_params.insert(param_name, ty_var);
                generics_vec.push(ty_var);
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
            self.resolver.def_info.insert(op_def_id, super::resolve::DefInfo {
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
    fn collect_handler(&mut self, handler: &ast::HandlerDecl) -> Result<(), TypeError> {
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
            for param in &type_params.params {
                let param_name = self.symbol_to_string(param.name.node);
                let ty_var = TyVarId(self.next_type_param_id);
                self.next_type_param_id += 1;
                self.generic_params.insert(param_name, ty_var);
                generics_vec.push(ty_var);
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
                    self.resolver.error(TypeError::new(
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
                    self.resolver.error(TypeError::new(
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
        self.handler_defs.insert(def_id, HandlerInfo {
            name,
            kind: handler.kind,
            effect_id,
            operations: Vec::new(), // Will be populated during body type-checking
            generics: generics_vec,
            fields,
            return_clause_body_id: None, // Will be populated during body type-checking
        });

        // Queue handler for body type-checking
        self.pending_handlers.push((def_id, handler.clone()));

        Ok(())
    }

    /// Type-check all queued bodies.
    pub fn check_all_bodies(&mut self) -> Result<(), Vec<Diagnostic>> {
        // Type-check function bodies
        let pending = std::mem::take(&mut self.pending_bodies);
        for (def_id, func) in pending {
            if let Err(e) = self.check_function_body(def_id, &func) {
                self.errors.push(e);
            }
        }

        // Type-check handler operation bodies
        let pending_handlers = std::mem::take(&mut self.pending_handlers);
        for (def_id, handler) in pending_handlers {
            if let Err(e) = self.check_handler_body(def_id, &handler) {
                self.errors.push(e);
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors.iter().map(|e| e.to_diagnostic()).collect());
        }

        Ok(())
    }

    /// Type-check a function body.
    fn check_function_body(&mut self, def_id: DefId, func: &ast::FnDecl) -> Result<(), TypeError> {
        let body = func.body.as_ref().ok_or_else(|| TypeError::new(
            TypeErrorKind::NotFound { name: format!("body for {def_id}") },
            func.span,
        ))?;

        let sig = self.fn_sigs.get(&def_id).cloned()
            .ok_or_else(|| TypeError::new(
                TypeErrorKind::NotFound { name: format!("fn sig for {def_id}") },
                func.span,
            ))?;

        // Set up function scope
        self.resolver.push_scope(ScopeKind::Function, body.span);
        self.resolver.reset_local_ids();
        // Skip LocalId(0) which is reserved for the return place
        let _ = self.resolver.next_local_id();
        self.locals.clear();

        // Register effect operations in scope based on function's declared effects
        self.register_effect_operations_in_scope(def_id)?;

        // Add return place
        self.locals.push(hir::Local {
            id: LocalId::RETURN_PLACE,
            ty: sig.output.clone(),
            mutable: false,
            name: None,
            span: body.span,
        });

        // Set return type for return statements
        self.return_type = Some(sig.output.clone());

        // Set current function for effect checking
        self.current_fn = Some(def_id);

        // Add parameters as locals with their actual names from the AST
        for (i, param) in func.params.iter().enumerate() {
            let param_ty = sig.inputs.get(i).cloned().ok_or_else(|| {
                // This indicates a bug: parameter count mismatch between AST and signature
                TypeError::new(
                    TypeErrorKind::WrongArity {
                        expected: sig.inputs.len(),
                        found: func.params.len(),
                    },
                    param.span,
                )
            })?;

            // Handle the parameter pattern
            match &param.pattern.kind {
                ast::PatternKind::Ident { name, mutable, .. } => {
                    let param_name = self.symbol_to_string(name.node);
                    let local_id = self.resolver.define_local(
                        param_name.clone(),
                        param_ty.clone(),
                        *mutable,
                        param.span,
                    )?;

                    self.locals.push(hir::Local {
                        id: local_id,
                        ty: param_ty,
                        mutable: *mutable,
                        name: Some(param_name),
                        span: param.span,
                    });
                }
                ast::PatternKind::Wildcard => {
                    // Anonymous parameter - generate a unique name but don't expose it
                    let param_name = format!("_param{i}");
                    let local_id = self.resolver.next_local_id();

                    self.locals.push(hir::Local {
                        id: local_id,
                        ty: param_ty,
                        mutable: false,
                        name: Some(param_name),
                        span: param.span,
                    });
                }
                ast::PatternKind::Tuple { fields, .. } => {
                    // Tuple destructuring in parameters: fn foo((x, y): (i32, i32))
                    // Create a hidden parameter local, then define the pattern bindings
                    let hidden_name = format!("__param{i}");
                    let _hidden_local_id = self.resolver.define_local(
                        hidden_name.clone(),
                        param_ty.clone(),
                        false,
                        param.span,
                    )?;

                    // Now define the pattern bindings from the tuple type
                    let elem_types = match param_ty.kind() {
                        hir::TypeKind::Tuple(elems) => elems.clone(),
                        _ => {
                            return Err(TypeError::new(
                                TypeErrorKind::NotATuple { ty: param_ty.clone() },
                                param.span,
                            ));
                        }
                    };

                    if fields.len() != elem_types.len() {
                        return Err(TypeError::new(
                            TypeErrorKind::WrongArity {
                                expected: elem_types.len(),
                                found: fields.len(),
                            },
                            param.span,
                        ));
                    }

                    // Define each element of the tuple pattern
                    for (field_pat, elem_ty) in fields.iter().zip(elem_types.iter()) {
                        self.define_pattern(field_pat, elem_ty.clone())?;
                    }

                    // Add a local for the whole parameter (for param_count tracking)
                    self.locals.push(hir::Local {
                        id: self.resolver.next_local_id(),
                        ty: param_ty,
                        mutable: false,
                        name: Some(hidden_name),
                        span: param.span,
                    });
                }
                _ => {
                    // Other complex patterns - return an error instead of silently failing
                    return Err(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "complex patterns in function parameters (only identifiers and tuples supported)".to_string(),
                        },
                        param.span,
                    ));
                }
            }
        }

        // Type-check the body
        let body_expr = self.check_block(body, &sig.output)?;

        // Create body
        let body_id = hir::BodyId::new(self.next_body_id);
        self.next_body_id += 1;

        let hir_body = hir::Body {
            locals: std::mem::take(&mut self.locals),
            param_count: sig.inputs.len(),
            expr: body_expr,
            span: body.span,
        };

        self.bodies.insert(body_id, hir_body);
        self.fn_bodies.insert(def_id, body_id);

        // Clean up
        self.return_type = None;
        self.resolver.pop_scope();

        Ok(())
    }

    /// Type-check a handler's operation and return clause bodies.
    fn check_handler_body(&mut self, handler_def_id: DefId, handler: &ast::HandlerDecl) -> Result<(), TypeError> {
        // Get the handler info to find the effect
        let handler_info = self.handler_defs.get(&handler_def_id).cloned()
            .ok_or_else(|| TypeError::new(
                TypeErrorKind::NotFound { name: format!("handler info for {handler_def_id}") },
                handler.span,
            ))?;

        let effect_id = handler_info.effect_id;
        let effect_info = self.effect_defs.get(&effect_id).cloned()
            .ok_or_else(|| TypeError::new(
                TypeErrorKind::NotAnEffect { name: format!("effect {effect_id}") },
                handler.span,
            ))?;

        // Collect operation body IDs
        let mut operation_body_ids: Vec<(String, hir::BodyId)> = Vec::new();

        // Type-check each operation body
        for op_impl in &handler.operations {
            let op_name = self.symbol_to_string(op_impl.name.node);

            // Find the effect operation info to get the expected signature
            let effect_op = effect_info.operations.iter()
                .find(|op| op.name == op_name)
                .ok_or_else(|| TypeError::new(
                    TypeErrorKind::InvalidHandler {
                        reason: format!("operation `{}` not found in effect", op_name),
                    },
                    op_impl.span,
                ))?;

            // Set up scope for operation body
            // Use ScopeKind::Handler so that `resume` is recognized as being in a handler
            self.resolver.push_scope(ScopeKind::Handler, op_impl.span);
            self.resolver.reset_local_ids();
            let _ = self.resolver.next_local_id(); // Skip LocalId(0) for return place
            self.locals.clear();

            // Add return place (unit type for operations that use resume)
            // Note: The actual return happens through resume, not normal return
            self.locals.push(hir::Local {
                id: LocalId::RETURN_PLACE,
                ty: effect_op.return_ty.clone(),
                mutable: false,
                name: None,
                span: op_impl.span,
            });

            // Add handler state fields to scope
            for (i, state_field) in handler.state.iter().enumerate() {
                let field_name = self.symbol_to_string(state_field.name.node);
                let field_ty = handler_info.fields[i].ty.clone();

                let local_id = self.resolver.define_local(
                    field_name.clone(),
                    field_ty.clone(),
                    state_field.is_mut,
                    state_field.span,
                )?;
                self.locals.push(hir::Local {
                    id: local_id,
                    ty: field_ty,
                    mutable: state_field.is_mut,
                    name: Some(field_name),
                    span: state_field.span,
                });
            }

            // Add operation parameters to scope
            for (param_idx, param) in op_impl.params.iter().enumerate() {
                if let ast::PatternKind::Ident { name, mutable, .. } = &param.kind {
                    let param_name = self.symbol_to_string(name.node);
                    let param_ty = if param_idx < effect_op.params.len() {
                        effect_op.params[param_idx].clone()
                    } else {
                        // Should not happen if effect validation passed
                        Type::unit()
                    };

                    let local_id = self.resolver.define_local(
                        param_name.clone(),
                        param_ty.clone(),
                        *mutable,
                        param.span,
                    )?;
                    self.locals.push(hir::Local {
                        id: local_id,
                        ty: param_ty,
                        mutable: *mutable,
                        name: Some(param_name),
                        span: param.span,
                    });
                }
            }

            // Add 'resume' to scope as a special continuation function
            // Resume is a function that takes the return value and returns unit
            // (the actual continuation handling happens at runtime)
            let resume_ty = Type::function(vec![effect_op.return_ty.clone()], Type::unit());
            let resume_local_id = self.resolver.define_local(
                "resume".to_string(),
                resume_ty.clone(),
                false,
                op_impl.span,
            )?;
            self.locals.push(hir::Local {
                id: resume_local_id,
                ty: resume_ty,
                mutable: false,
                name: Some("resume".to_string()),
                span: op_impl.span,
            });

            // Type-check the body (expected type is unit since resume handles the return)
            let body_expr = self.check_block(&op_impl.body, &Type::unit())?;

            // Create body
            let body_id = hir::BodyId::new(self.next_body_id);
            self.next_body_id += 1;

            let hir_body = hir::Body {
                locals: std::mem::take(&mut self.locals),
                param_count: effect_op.params.len(),
                expr: body_expr,
                span: op_impl.body.span,
            };

            self.bodies.insert(body_id, hir_body);
            operation_body_ids.push((op_name, body_id));

            self.resolver.pop_scope();
        }

        // Type-check return clause if present
        let return_clause_body_id = if let Some(ret_clause) = &handler.return_clause {
            self.resolver.push_scope(ScopeKind::Function, ret_clause.span);
            self.resolver.reset_local_ids();
            let _ = self.resolver.next_local_id(); // Skip LocalId(0)
            self.locals.clear();

            // Return clause takes the result and transforms it
            // For Phase 1, use i32 as the result type (most common)
            let result_ty = Type::i32();

            // Add return place
            self.locals.push(hir::Local {
                id: LocalId::RETURN_PLACE,
                ty: result_ty.clone(),
                mutable: false,
                name: None,
                span: ret_clause.span,
            });

            // Add the result parameter
            let param_name = self.symbol_to_string(ret_clause.param.node);
            let local_id = self.resolver.define_local(
                param_name.clone(),
                result_ty.clone(),
                false,
                ret_clause.param.span,
            )?;
            self.locals.push(hir::Local {
                id: local_id,
                ty: result_ty.clone(),
                mutable: false,
                name: Some(param_name),
                span: ret_clause.param.span,
            });

            // Type-check return clause body
            let body_expr = self.check_block(&ret_clause.body, &result_ty)?;

            let body_id = hir::BodyId::new(self.next_body_id);
            self.next_body_id += 1;

            let hir_body = hir::Body {
                locals: std::mem::take(&mut self.locals),
                param_count: 1, // Just the result parameter
                expr: body_expr,
                span: ret_clause.body.span,
            };

            self.bodies.insert(body_id, hir_body);
            self.resolver.pop_scope();

            Some(body_id)
        } else {
            None
        };

        // Update handler info with body IDs
        if let Some(info) = self.handler_defs.get_mut(&handler_def_id) {
            info.operations = operation_body_ids;
            info.return_clause_body_id = return_clause_body_id;
        }

        Ok(())
    }

    /// Type-check a block.
    fn check_block(&mut self, block: &ast::Block, expected: &Type) -> Result<hir::Expr, TypeError> {
        self.resolver.push_scope(ScopeKind::Block, block.span);

        let mut stmts = Vec::new();

        for stmt in &block.statements {
            match stmt {
                ast::Statement::Let { pattern, ty, value, span } => {
                    let local_ty = if let Some(ty) = ty {
                        self.ast_type_to_hir_type(ty)?
                    } else if let Some(value) = value {
                        // Infer from value
                        let inferred = self.infer_expr(value)?;
                        inferred.ty.clone()
                    } else {
                        // No type annotation and no value - error
                        return Err(TypeError::new(
                            TypeErrorKind::CannotInfer,
                            *span,
                        ));
                    };

                    // Handle the pattern (simplified: just identifiers for Phase 1)
                    let local_id = self.define_pattern(pattern, local_ty.clone())?;

                    let init = if let Some(value) = value {
                        Some(self.check_expr(value, &local_ty)?)
                    } else {
                        None
                    };

                    stmts.push(hir::Stmt::Let { local_id, init });
                }
                ast::Statement::Expr { expr, has_semi: _ } => {
                    let hir_expr = self.infer_expr(expr)?;
                    stmts.push(hir::Stmt::Expr(hir_expr));
                }
                ast::Statement::Item(decl) => {
                    // Nested items - collect them
                    self.collect_declaration(decl)?;
                }
            }
        }

        // Type-check trailing expression
        let expr = if let Some(expr) = &block.expr {
            self.check_expr(expr, expected)?
        } else {
            // No trailing expression - block returns unit
            if !expected.is_unit() && !expected.is_never() {
                // Check if expected is a type variable
                if let TypeKind::Infer(_) = expected.kind() {
                    self.unifier.unify(&Type::unit(), expected, block.span)?;
                }
            }
            hir::Expr::new(
                hir::ExprKind::Tuple(Vec::new()),
                Type::unit(),
                block.span,
            )
        };

        self.resolver.pop_scope();

        Ok(hir::Expr::new(
            hir::ExprKind::Block {
                stmts,
                expr: Some(Box::new(expr.clone())),
            },
            expr.ty.clone(),
            block.span,
        ))
    }

    /// Define a pattern, returning the local ID for simple patterns.
    fn define_pattern(&mut self, pattern: &ast::Pattern, ty: Type) -> Result<LocalId, TypeError> {
        match &pattern.kind {
            ast::PatternKind::Ident { name, mutable, .. } => {
                let name_str = self.symbol_to_string(name.node);
                let local_id = self.resolver.define_local(
                    name_str.clone(),
                    ty.clone(),
                    *mutable,
                    pattern.span,
                )?;

                self.locals.push(hir::Local {
                    id: local_id,
                    ty,
                    mutable: *mutable,
                    name: Some(name_str),
                    span: pattern.span,
                });

                Ok(local_id)
            }
            ast::PatternKind::Wildcard => {
                // Anonymous local
                let local_id = self.resolver.next_local_id();
                self.locals.push(hir::Local {
                    id: local_id,
                    ty,
                    mutable: false,
                    name: None,
                    span: pattern.span,
                });
                Ok(local_id)
            }
            ast::PatternKind::Tuple { fields, .. } => {
                // Tuple destructuring pattern: let (x, y) = ...
                // Need to extract element types from the tuple type
                let elem_types = match ty.kind() {
                    hir::TypeKind::Tuple(elems) => elems.clone(),
                    hir::TypeKind::Infer(_) => {
                        // Type not yet known - create fresh variables for each element
                        (0..fields.len())
                            .map(|_| self.unifier.fresh_var())
                            .collect::<Vec<_>>()
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotATuple { ty: ty.clone() },
                            pattern.span,
                        ));
                    }
                };

                // Check arity matches
                if fields.len() != elem_types.len() {
                    return Err(TypeError::new(
                        TypeErrorKind::WrongArity {
                            expected: elem_types.len(),
                            found: fields.len(),
                        },
                        pattern.span,
                    ));
                }

                // If we inferred element types, unify with the original type
                if matches!(ty.kind(), hir::TypeKind::Infer(_)) {
                    let tuple_ty = Type::tuple(elem_types.clone());
                    self.unifier.unify(&ty, &tuple_ty, pattern.span)?;
                }

                // Recursively define each element pattern
                // Return the first local_id (for the overall pattern binding)
                let mut first_local_id = None;
                for (field_pat, elem_ty) in fields.iter().zip(elem_types.iter()) {
                    let local_id = self.define_pattern(field_pat, elem_ty.clone())?;
                    if first_local_id.is_none() {
                        first_local_id = Some(local_id);
                    }
                }

                // Return the first local id, or create a placeholder if no fields
                Ok(first_local_id.unwrap_or_else(|| {
                    let local_id = self.resolver.next_local_id();
                    self.locals.push(hir::Local {
                        id: local_id,
                        ty: Type::unit(),
                        mutable: false,
                        name: None,
                        span: pattern.span,
                    });
                    local_id
                }))
            }
            ast::PatternKind::Paren(inner) => {
                // Parenthesized pattern - just unwrap
                self.define_pattern(inner, ty)
            }
            ast::PatternKind::Literal(_) => {
                // Literal patterns in let bindings don't make sense (can't bind)
                // But they should be allowed in match arms. For let, error out.
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "literal patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Path(_) => {
                // Path patterns (like enum variants) in let bindings - error
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "path patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::TupleStruct { .. } => {
                // Tuple struct patterns in let - error (use match)
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "tuple struct patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Rest => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "rest patterns (..) in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Ref { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "reference patterns (&x) in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Struct { path, fields, rest } => {
                // Struct pattern: let Point { x, y } = point;
                // First verify the type is a struct matching the pattern's path
                if path.segments.len() != 1 {
                    return Err(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "qualified struct pattern paths".to_string(),
                        },
                        pattern.span,
                    ));
                }
                let struct_name = self.symbol_to_string(path.segments[0].name.node);

                // Get the struct definition from the type
                let struct_def_id = match ty.kind() {
                    TypeKind::Adt { def_id, .. } => *def_id,
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: ty.clone() },
                            pattern.span,
                        ));
                    }
                };

                let struct_info = self.struct_defs.get(&struct_def_id).cloned().ok_or_else(|| {
                    TypeError::new(
                        TypeErrorKind::TypeNotFound { name: struct_name.clone() },
                        pattern.span,
                    )
                })?;

                // Create a hidden local for the whole struct value
                let hidden_name = format!("__struct_{}", pattern.span.start);
                let hidden_local = self.resolver.next_local_id();
                self.locals.push(hir::Local {
                    id: hidden_local,
                    name: Some(hidden_name),
                    ty: ty.clone(),
                    mutable: false,
                    span: pattern.span,
                });

                // Process each field pattern
                let mut bound_fields = std::collections::HashSet::new();
                for field_pattern in fields {
                    let field_name = self.symbol_to_string(field_pattern.name.node);

                    // Look up the field in the struct
                    let field_info = struct_info.fields.iter()
                        .find(|f| f.name == field_name)
                        .ok_or_else(|| TypeError::new(
                            TypeErrorKind::NoField {
                                ty: ty.clone(),
                                field: field_name.clone(),
                            },
                            field_pattern.span,
                        ))?;

                    bound_fields.insert(field_name.clone());

                    // Define the pattern for this field
                    if let Some(ref inner_pattern) = field_pattern.pattern {
                        // Field with explicit pattern: `x: pat`
                        self.define_pattern(inner_pattern, field_info.ty.clone())?;
                    } else {
                        // Shorthand field: `x` means `x: x`
                        let local_id = self.resolver.define_local(
                            field_name.clone(),
                            field_info.ty.clone(),
                            false, // mutable
                            pattern.span,
                        )?;
                        self.locals.push(hir::Local {
                            id: local_id,
                            name: Some(field_name),
                            ty: field_info.ty.clone(),
                            mutable: false,
                            span: field_pattern.span,
                        });
                    }
                }

                // If not using rest (..), verify all fields are bound
                if !*rest {
                    for field_info in &struct_info.fields {
                        if !bound_fields.contains(&field_info.name) {
                            return Err(TypeError::new(
                                TypeErrorKind::MissingField {
                                    ty: ty.clone(),
                                    field: field_info.name.clone(),
                                },
                                pattern.span,
                            ));
                        }
                    }
                }

                Ok(hidden_local)
            }
            ast::PatternKind::Slice { elements, rest_pos } => {
                // Slice pattern: let [first, second, ..] = arr;
                // Get the element type from the array/slice type
                let elem_ty = match ty.kind() {
                    TypeKind::Array { element, size } => {
                        // Validate: number of non-rest patterns must be <= array size
                        let num_patterns = if rest_pos.is_some() { elements.len() - 1 } else { elements.len() };
                        if num_patterns as u64 > *size {
                            return Err(TypeError::new(
                                TypeErrorKind::PatternMismatch {
                                    expected: ty.clone(),
                                    pattern: format!("slice pattern with {} elements", num_patterns),
                                },
                                pattern.span,
                            ));
                        }
                        element.clone()
                    }
                    TypeKind::Slice { element } => element.clone(),
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotIndexable { ty: ty.clone() },
                            pattern.span,
                        ));
                    }
                };

                // Create a hidden local for the whole array/slice value
                let hidden_name = format!("__slice_{}", pattern.span.start);
                let hidden_local = self.resolver.next_local_id();
                self.locals.push(hir::Local {
                    id: hidden_local,
                    name: Some(hidden_name),
                    ty: ty.clone(),
                    mutable: false,
                    span: pattern.span,
                });

                // Process each element pattern
                for (i, elem_pattern) in elements.iter().enumerate() {
                    // Handle rest pattern (..)
                    if rest_pos.is_some() && Some(i) == *rest_pos {
                        // Rest pattern - skip (it binds the middle slice, handled elsewhere)
                        if let ast::PatternKind::Rest = &elem_pattern.kind {
                            continue;
                        }
                    }
                    self.define_pattern(elem_pattern, elem_ty.clone())?;
                }

                Ok(hidden_local)
            }
            ast::PatternKind::Or { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "or patterns (a | b) in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Range { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "range patterns in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
        }
    }

    /// Check an expression against an expected type.
    fn check_expr(&mut self, expr: &ast::Expr, expected: &Type) -> Result<hir::Expr, TypeError> {
        let inferred = self.infer_expr(expr)?;

        // Unify inferred type with expected
        self.unifier.unify(&inferred.ty, expected, expr.span)?;

        Ok(inferred)
    }

    /// Infer the type of an expression.
    fn infer_expr(&mut self, expr: &ast::Expr) -> Result<hir::Expr, TypeError> {
        match &expr.kind {
            ast::ExprKind::Literal(lit) => self.infer_literal(lit, expr.span),
            ast::ExprKind::Path(path) => self.infer_path(path, expr.span),
            ast::ExprKind::Binary { op, left, right } => {
                self.infer_binary(*op, left, right, expr.span)
            }
            ast::ExprKind::Unary { op, operand } => {
                self.infer_unary(*op, operand, expr.span)
            }
            ast::ExprKind::Call { callee, args } => {
                self.infer_call(callee, args, expr.span)
            }
            ast::ExprKind::If { condition, then_branch, else_branch } => {
                self.infer_if(condition, then_branch, else_branch.as_ref(), expr.span)
            }
            ast::ExprKind::Block(block) => {
                let expected = self.unifier.fresh_var();
                self.check_block(block, &expected)
            }
            ast::ExprKind::Return(value) => {
                self.infer_return(value.as_deref(), expr.span)
            }
            ast::ExprKind::Tuple(exprs) => {
                self.infer_tuple(exprs, expr.span)
            }
            ast::ExprKind::Paren(inner) => {
                self.infer_expr(inner)
            }
            ast::ExprKind::Assign { target, value } => {
                self.infer_assign(target, value, expr.span)
            }
            ast::ExprKind::Loop { body, .. } => {
                self.infer_loop(body, expr.span)
            }
            ast::ExprKind::While { condition, body, .. } => {
                self.infer_while(condition, body, expr.span)
            }
            ast::ExprKind::For { pattern, iter, body, .. } => {
                self.infer_for(pattern, iter, body, expr.span)
            }
            ast::ExprKind::Break { value, .. } => {
                self.infer_break(value.as_deref(), expr.span)
            }
            ast::ExprKind::Continue { .. } => {
                Ok(hir::Expr::new(
                    hir::ExprKind::Continue { label: None },
                    Type::never(),
                    expr.span,
                ))
            }
            ast::ExprKind::Match { scrutinee, arms } => {
                self.infer_match(scrutinee, arms, expr.span)
            }
            ast::ExprKind::Record { path, fields, base } => {
                self.infer_record(path.as_ref(), fields, base.as_deref(), expr.span)
            }
            ast::ExprKind::Field { base: field_base, field } => {
                self.infer_field_access(field_base, field, expr.span)
            }
            ast::ExprKind::Closure { is_move, params, return_type, effects: _, body } => {
                self.infer_closure(*is_move, params, return_type.as_ref(), body, expr.span)
            }
            ast::ExprKind::WithHandle { handler, body } => {
                // Handle expression: runs body with an effect handler installed.
                //
                // 1. Type-check the handler expression
                // 2. Extract the handler def_id from its type
                // 3. Look up the handler to find which effect it handles
                // 4. Push that effect onto handled_effects
                // 5. Type-check the body
                // 6. Pop the effect

                // Type-check the handler expression first
                let handler_expr = self.infer_expr(handler)?;

                // Extract handler def_id from the type (handlers are ADTs)
                let handled_effect = match handler_expr.ty.kind() {
                    hir::TypeKind::Adt { def_id: handler_def_id, .. } => {
                        self.handler_defs.get(handler_def_id).map(|h| h.effect_id)
                    }
                    _ => None,
                };

                // Push the handled effect onto the stack
                if let Some(effect_id) = handled_effect {
                    self.handled_effects.push(effect_id);
                }

                let body_block = match &body.kind {
                    ast::ExprKind::Block(block) => block,
                    _ => {
                        // Pop effect if we pushed it
                        if handled_effect.is_some() {
                            self.handled_effects.pop();
                        }
                        return Err(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "Handle body must be a block".into(),
                            },
                            body.span,
                        ));
                    }
                };

                // Push a handler scope and register effect operations
                self.resolver.push_scope(ScopeKind::Handler, body.span);

                // Register the handled effect's operations in this scope
                //
                // NOTE: Operations are registered with their generic signatures (DefId only).
                // Type argument substitution happens at the call site when the operation is invoked.
                // This is consistent with how register_effect_operations_in_scope works for functions.
                //
                // For full handler type argument support, we would:
                // 1. Extract type args from handler_expr.ty (e.g., Handler<i32> has type arg i32)
                // 2. Build substitution map: effect generic param -> handler type arg
                // 3. Pre-substitute operation signatures for better error messages
                //
                // Current behavior works because unification at call sites resolves the types.
                if let Some(effect_id) = handled_effect {
                    if let Some(effect_info) = self.effect_defs.get(&effect_id).cloned() {
                        for op_info in &effect_info.operations {
                            self.resolver.current_scope_mut()
                                .bindings
                                .insert(op_info.name.clone(), super::resolve::Binding::Def(op_info.def_id));
                        }
                    }
                }

                // Type-check the block
                let expected = self.unifier.fresh_var();
                let result = self.check_block(body_block, &expected);

                // Pop the handler scope
                self.resolver.pop_scope();

                // Pop the handled effect
                if handled_effect.is_some() {
                    self.handled_effects.pop();
                }

                // Extract handler_id from handler expression type
                let handler_id = match handler_expr.ty.kind() {
                    hir::TypeKind::Adt { def_id, .. } => *def_id,
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "Handler must be an ADT type".into(),
                            },
                            expr.span,
                        ));
                    }
                };

                // Wrap the body in a Handle expression
                let body_expr = result?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Handle {
                        body: Box::new(body_expr),
                        handler_id,
                        handler_instance: Box::new(handler_expr),
                    },
                    ty: expected.clone(),
                    span: expr.span,
                })
            }
            ast::ExprKind::Perform { effect, operation, args } => {
                // Effect operation: performs an operation from an effect.
                //
                // This requires:
                // 1. Looking up the effect definition (from explicit path or scope)
                // 2. Finding the operation within the effect
                // 3. Type checking arguments against operation signature
                // 4. Returning the operation's return type

                let op_name = self.symbol_to_string(operation.node);

                // Try to find the operation - either from explicit effect path or from scope
                let (effect_id, op_index, op_def_id, type_args) = if let Some(effect_path) = effect {
                    // Explicit effect specified: `perform Effect<T>.op(args)`
                    // Extract base name (without type args)
                    let effect_name = if let Some(first_seg) = effect_path.segments.first() {
                        self.symbol_to_string(first_seg.name.node)
                    } else {
                        String::new()
                    };

                    // Extract type arguments from the first segment
                    let type_args: Vec<Type> = if let Some(first_seg) = effect_path.segments.first() {
                        if let Some(ref args) = first_seg.args {
                            args.args.iter()
                                .filter_map(|arg| {
                                    if let ast::TypeArg::Type(ty) = arg {
                                        self.ast_type_to_hir_type(ty).ok()
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };

                    // Look up the effect by name
                    let effect_def_id = self.effect_defs.iter()
                        .find(|(_, info)| info.name == effect_name)
                        .map(|(def_id, _)| *def_id);

                    match effect_def_id {
                        Some(eff_id) => {
                            let effect_info = self.effect_defs.get(&eff_id).cloned();
                            match effect_info {
                                Some(info) => {
                                    // Find the operation by name
                                    let op_result = info.operations.iter().enumerate()
                                        .find(|(_, op)| op.name == op_name);
                                    match op_result {
                                        Some((idx, op)) => (eff_id, idx as u32, op.def_id, type_args),
                                        None => {
                                            self.resolver.error(TypeError::new(
                                                TypeErrorKind::NotFound { name: format!("{}.{}", effect_name, op_name) },
                                                operation.span,
                                            ));
                                            return Ok(hir::Expr::new(
                                                hir::ExprKind::Error,
                                                Type::error(),
                                                expr.span,
                                            ));
                                        }
                                    }
                                }
                                None => {
                                    self.resolver.error(TypeError::new(
                                        TypeErrorKind::TypeNotFound { name: effect_name },
                                        expr.span,
                                    ));
                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Error,
                                        Type::error(),
                                        expr.span,
                                    ));
                                }
                            }
                        }
                        None => {
                            self.resolver.error(TypeError::new(
                                TypeErrorKind::TypeNotFound { name: effect_name },
                                expr.span,
                            ));
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Error,
                                Type::error(),
                                expr.span,
                            ));
                        }
                    }
                } else {
                    // No explicit effect - look up operation in scope
                    // (it should be registered by register_effect_operations_in_scope)
                    // Get type args from current function's effect declaration
                    let binding = self.resolver.lookup(&op_name);
                    match binding {
                        Some(super::resolve::Binding::Def(op_def_id)) => {
                            // Found the operation - now find its parent effect
                            let def_info = self.resolver.def_info.get(&op_def_id);
                            match def_info.and_then(|info| info.parent) {
                                Some(effect_def_id) => {
                                    // Find operation index in the effect
                                    let effect_info = self.effect_defs.get(&effect_def_id);

                                    // Get type args from current function's effect declaration
                                    let type_args = if let Some(fn_id) = self.current_fn {
                                        if let Some(fn_effects) = self.fn_effects.get(&fn_id) {
                                            fn_effects.iter()
                                                .find(|er| er.def_id == effect_def_id)
                                                .map(|er| er.type_args.clone())
                                                .unwrap_or_default()
                                        } else {
                                            Vec::new()
                                        }
                                    } else {
                                        Vec::new()
                                    };

                                    match effect_info {
                                        Some(info) => {
                                            let op_idx = info.operations.iter()
                                                .position(|op| op.def_id == op_def_id);
                                            match op_idx {
                                                Some(idx) => (effect_def_id, idx as u32, op_def_id, type_args),
                                                None => {
                                                    self.resolver.error(TypeError::new(
                                                        TypeErrorKind::NotFound { name: op_name },
                                                        operation.span,
                                                    ));
                                                    return Ok(hir::Expr::new(
                                                        hir::ExprKind::Error,
                                                        Type::error(),
                                                        expr.span,
                                                    ));
                                                }
                                            }
                                        }
                                        None => {
                                            self.resolver.error(TypeError::new(
                                                TypeErrorKind::NotFound { name: op_name },
                                                operation.span,
                                            ));
                                            return Ok(hir::Expr::new(
                                                hir::ExprKind::Error,
                                                Type::error(),
                                                expr.span,
                                            ));
                                        }
                                    }
                                }
                                None => {
                                    self.resolver.error(TypeError::new(
                                        TypeErrorKind::NotFound { name: op_name },
                                        operation.span,
                                    ));
                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Error,
                                        Type::error(),
                                        expr.span,
                                    ));
                                }
                            }
                        }
                        _ => {
                            self.resolver.error(TypeError::new(
                                TypeErrorKind::NotFound { name: op_name },
                                operation.span,
                            ));
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Error,
                                Type::error(),
                                expr.span,
                            ));
                        }
                    }
                };

                // Get the operation's signature for type checking
                let fn_sig = self.fn_sigs.get(&op_def_id).cloned();
                let (param_types, return_ty) = match fn_sig {
                    Some(sig) => {
                        // Substitute type arguments into the signature if provided
                        let effect_info = self.effect_defs.get(&effect_id);
                        if !type_args.is_empty() {
                            if let Some(info) = effect_info {
                                // Create substitution map from effect's generic params to provided type args
                                let substitution: std::collections::HashMap<TyVarId, Type> = info.generics.iter()
                                    .zip(type_args.iter())
                                    .map(|(&var_id, ty)| (var_id, ty.clone()))
                                    .collect();

                                // Substitute in parameter types and return type
                                let subst_params: Vec<Type> = sig.inputs.iter()
                                    .map(|ty| self.substitute_type_vars(ty, &substitution))
                                    .collect();
                                let subst_return = self.substitute_type_vars(&sig.output, &substitution);
                                (subst_params, subst_return)
                            } else {
                                (sig.inputs.clone(), sig.output.clone())
                            }
                        } else {
                            (sig.inputs.clone(), sig.output.clone())
                        }
                    }
                    None => {
                        // Fallback: get from EffectInfo
                        let effect_info = self.effect_defs.get(&effect_id);
                        match effect_info.and_then(|info| info.operations.get(op_index as usize)) {
                            Some(op_info) => (op_info.params.clone(), op_info.return_ty.clone()),
                            None => {
                                return Ok(hir::Expr::new(
                                    hir::ExprKind::Error,
                                    Type::error(),
                                    expr.span,
                                ));
                            }
                        }
                    }
                };

                // Type-check arguments
                if args.len() != param_types.len() {
                    self.resolver.error(TypeError::new(
                        TypeErrorKind::WrongArity {
                            expected: param_types.len(),
                            found: args.len(),
                        },
                        expr.span,
                    ));
                }

                let mut hir_args = Vec::with_capacity(args.len());
                for (i, arg) in args.iter().enumerate() {
                    let arg_expr = self.infer_expr(arg)?;
                    if let Some(expected_ty) = param_types.get(i) {
                        self.unifier.unify(&arg_expr.ty, expected_ty, arg.span)?;
                    }
                    hir_args.push(arg_expr);
                }

                Ok(hir::Expr::new(
                    hir::ExprKind::Perform {
                        effect_id,
                        op_index,
                        args: hir_args,
                    },
                    return_ty,
                    expr.span,
                ))
            }
            ast::ExprKind::Resume(value) => {
                // Resume continuation in a handler.
                //
                // This should only appear inside a handler operation body (with...handle block).
                // The resume expression passes the value back to the caller and continues
                // the computation. The type of the resume expression itself depends on what
                // the caller does after the operation completes.

                // Validate we're inside a handler scope
                if !self.resolver.in_handler() {
                    self.resolver.error(TypeError::new(
                        TypeErrorKind::InvalidHandler {
                            reason: "`resume` can only be used inside an effect handler".to_string(),
                        },
                        expr.span,
                    ));
                    return Ok(hir::Expr::new(
                        hir::ExprKind::Error,
                        Type::error(),
                        expr.span,
                    ));
                }

                let value_expr = self.infer_expr(value)?;

                // The type of the resume expression depends on the continuation's return type.
                // For deep handlers, this is typically the handler's overall return type.
                // For now, use a fresh type variable to allow inference.
                let resume_ty = self.unifier.fresh_var();

                Ok(hir::Expr::new(
                    hir::ExprKind::Resume {
                        value: Some(Box::new(value_expr)),
                    },
                    resume_ty,
                    expr.span,
                ))
            }
            ast::ExprKind::MethodCall { receiver, method, type_args: _, args } => {
                // Method call: x.foo(y)
                // Look up method on receiver's type, type-check arguments
                self.infer_method_call(receiver, &method.node, args, expr.span)
            }
            ast::ExprKind::Index { base, index } => {
                // Index expression: x[i]
                // Check base is indexable, check index type, return element type
                self.infer_index(base, index, expr.span)
            }
            ast::ExprKind::Array(array_expr) => {
                // Array expression: [1, 2, 3] or [0; 10]
                self.infer_array(array_expr, expr.span)
            }
            ast::ExprKind::Cast { expr: inner, ty } => {
                // Cast expression: x as T
                let inner_expr = self.infer_expr(inner)?;
                let target_ty = self.ast_type_to_hir_type(ty)?;

                // For now, allow casts between numeric types and pointer types
                // A full implementation would validate the cast is legal
                Ok(hir::Expr::new(
                    hir::ExprKind::Cast {
                        expr: Box::new(inner_expr),
                        target_ty: target_ty.clone(),
                    },
                    target_ty,
                    expr.span,
                ))
            }
            ast::ExprKind::AssignOp { op, target, value } => {
                // Compound assignment: x += y
                // Desugar to x = x op y
                let target_expr = self.infer_expr(target)?;
                let value_expr = self.infer_expr(value)?;

                // For arithmetic ops, both operands must have same type
                self.unifier.unify(&target_expr.ty, &value_expr.ty, expr.span)?;
                let result_ty = target_expr.ty.clone();

                Ok(hir::Expr::new(
                    hir::ExprKind::Assign {
                        target: Box::new(target_expr.clone()),
                        value: Box::new(hir::Expr::new(
                            hir::ExprKind::Binary {
                                op: *op,
                                left: Box::new(target_expr),
                                right: Box::new(value_expr),
                            },
                            result_ty,
                            expr.span,
                        )),
                    },
                    Type::unit(),
                    expr.span,
                ))
            }
            ast::ExprKind::Unsafe(block) => {
                // Unsafe block: @unsafe { ... }
                // Type-check the block, mark it as unsafe context
                let expected = self.unifier.fresh_var();
                let block_expr = self.check_block(block, &expected)?;
                let result_ty = block_expr.ty.clone();

                Ok(hir::Expr::new(
                    hir::ExprKind::Unsafe(Box::new(block_expr)),
                    result_ty,
                    expr.span,
                ))
            }
            ast::ExprKind::Region { body, .. } => {
                // Region block: region 'a { ... }
                // Type-check the block (region tracking is a Phase 2+ feature)
                // For now, just return the block's result
                let expected = self.unifier.fresh_var();
                self.check_block(body, &expected)
            }
            ast::ExprKind::Range { start, end, inclusive: _ } => {
                // Range expressions outside of for loops are not yet supported.
                // In for loops, the range is desugared to a while loop.
                // For standalone ranges (e.g., for iterators), we'd need Range<T> stdlib type.

                // Type-check start and end for error reporting
                if let Some(s) = start {
                    let _ = self.infer_expr(s)?;
                }
                if let Some(e) = end {
                    let _ = self.infer_expr(e)?;
                }

                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "Range expressions outside of for loops require Range<T> type (not yet implemented)".into(),
                    },
                    expr.span,
                ))
            }
        }
    }

    /// Infer type of a method call.
    fn infer_method_call(
        &mut self,
        receiver: &ast::Expr,
        method: &ast::Symbol,
        args: &[ast::CallArg],
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let receiver_expr = self.infer_expr(receiver)?;
        let method_name = self.symbol_to_string(*method);

        // Look up method on receiver type
        // This would normally use trait resolution, but for now we'll check:
        // 1. Struct impl methods
        // 2. Built-in methods on primitive types

        // Type-check arguments
        let mut hir_args = Vec::with_capacity(args.len());
        for arg in args {
            let arg_expr = self.infer_expr(&arg.value)?;
            hir_args.push(arg_expr);
        }

        // Try to find method signature
        let (method_def_id, return_ty) = self.resolve_method(&receiver_expr.ty, &method_name, &hir_args, span)?;

        Ok(hir::Expr::new(
            hir::ExprKind::MethodCall {
                receiver: Box::new(receiver_expr),
                method: method_def_id,
                args: hir_args,
            },
            return_ty,
            span,
        ))
    }

    /// Resolve a method on a type.
    fn resolve_method(
        &mut self,
        receiver_ty: &Type,
        method_name: &str,
        _args: &[hir::Expr],
        span: Span,
    ) -> Result<(DefId, Type), TypeError> {
        // For ADTs, look up method in associated functions
        match receiver_ty.kind() {
            TypeKind::Adt { def_id: _, .. } => {
                // Look for method in impl blocks (Phase 2+)
                // For now, return an error
                Err(TypeError::new(
                    TypeErrorKind::NoField {
                        ty: receiver_ty.clone(),
                        field: method_name.to_string(),
                    },
                    span,
                ))
            }
            TypeKind::Primitive(prim) => {
                // Built-in methods on primitives (e.g., i32::abs, str::len)
                // For now, return an error - this requires trait resolution
                Err(TypeError::new(
                    TypeErrorKind::NotFound {
                        name: format!("{}::{}", prim, method_name),
                    },
                    span,
                ))
            }
            _ => {
                Err(TypeError::new(
                    TypeErrorKind::NotFound {
                        name: format!("<type>::{}", method_name),
                    },
                    span,
                ))
            }
        }
    }

    /// Infer type of an index expression.
    fn infer_index(
        &mut self,
        base: &ast::Expr,
        index: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let base_expr = self.infer_expr(base)?;
        let index_expr = self.infer_expr(index)?;

        // Check that index is a usize or integer type
        match index_expr.ty.kind() {
            TypeKind::Primitive(PrimitiveTy::Int(_) | PrimitiveTy::Uint(_)) => {}
            TypeKind::Infer(_) => {
                // Inferred type - will be resolved later, just default to i32 for index
                self.unifier.unify(&index_expr.ty, &Type::i32(), span)?;
            }
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::NotIndexable { ty: index_expr.ty.clone() },
                    span,
                ));
            }
        }

        // Determine element type based on base type
        let elem_ty = match base_expr.ty.kind() {
            TypeKind::Array { element, .. } => element.clone(),
            TypeKind::Slice { element } => element.clone(),
            TypeKind::Ref { inner, .. } => {
                // Deref and check inner type
                match inner.kind() {
                    TypeKind::Array { element, .. } => element.clone(),
                    TypeKind::Slice { element } => element.clone(),
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                            span,
                        ));
                    }
                }
            }
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                    span,
                ));
            }
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Index {
                base: Box::new(base_expr),
                index: Box::new(index_expr),
            },
            elem_ty,
            span,
        ))
    }

    /// Infer type of an array expression.
    fn infer_array(
        &mut self,
        array_expr: &ast::ArrayExpr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        match array_expr {
            ast::ArrayExpr::List(elements) => {
                if elements.is_empty() {
                    // Empty array - need type annotation
                    let elem_ty = self.unifier.fresh_var();
                    return Ok(hir::Expr::new(
                        hir::ExprKind::Array(vec![]),
                        Type::array(elem_ty, 0),
                        span,
                    ));
                }

                // Type-check all elements
                let mut hir_elements = Vec::with_capacity(elements.len());
                let first_elem = self.infer_expr(&elements[0])?;
                let elem_ty = first_elem.ty.clone();
                hir_elements.push(first_elem);

                for elem in &elements[1..] {
                    let elem_expr = self.infer_expr(elem)?;
                    // Unify element types
                    self.unifier.unify(&elem_expr.ty, &elem_ty, elem.span)?;
                    hir_elements.push(elem_expr);
                }

                let size = hir_elements.len() as u64;
                Ok(hir::Expr::new(
                    hir::ExprKind::Array(hir_elements),
                    Type::array(elem_ty, size),
                    span,
                ))
            }
            ast::ArrayExpr::Repeat { value, count } => {
                // [value; count] - repeat value count times
                let value_expr = self.infer_expr(value)?;
                let count_expr = self.infer_expr(count)?;

                // Verify count is an integer type
                self.unifier.unify(&count_expr.ty, &Type::i32(), count.span)?;

                // Count must be a constant integer (const eval required for non-literals)
                let size = match &count.kind {
                    ast::ExprKind::Literal(ast::Literal {
                        kind: ast::LiteralKind::Int { value, .. },
                        ..
                    }) => *value as u64,
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "array repeat count must be a literal integer (const evaluation not yet supported)".to_string(),
                            },
                            count.span,
                        ));
                    }
                };

                Ok(hir::Expr::new(
                    hir::ExprKind::Repeat {
                        value: Box::new(value_expr.clone()),
                        count: size,
                    },
                    Type::array(value_expr.ty, size),
                    span,
                ))
            }
        }
    }

    /// Infer type of a literal.
    fn infer_literal(&mut self, lit: &ast::Literal, span: Span) -> Result<hir::Expr, TypeError> {
        let (value, ty) = match &lit.kind {
            ast::LiteralKind::Int { value, suffix } => {
                let ty = match suffix {
                    Some(ast::IntSuffix::I8) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8))),
                    Some(ast::IntSuffix::I16) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16))),
                    Some(ast::IntSuffix::I32) => Type::i32(),
                    Some(ast::IntSuffix::I64) => Type::i64(),
                    Some(ast::IntSuffix::I128) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128))),
                    Some(ast::IntSuffix::U8) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8))),
                    Some(ast::IntSuffix::U16) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16))),
                    Some(ast::IntSuffix::U32) => Type::u32(),
                    Some(ast::IntSuffix::U64) => Type::u64(),
                    Some(ast::IntSuffix::U128) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128))),
                    None => Type::i32(), // Default to i32
                };
                (hir::LiteralValue::Int(*value as i128), ty)
            }
            ast::LiteralKind::Float { value, suffix } => {
                let ty = match suffix {
                    Some(ast::FloatSuffix::F32) => Type::f32(),
                    Some(ast::FloatSuffix::F64) | None => Type::f64(),
                };
                (hir::LiteralValue::Float(value.0), ty)
            }
            ast::LiteralKind::Bool(b) => (hir::LiteralValue::Bool(*b), Type::bool()),
            ast::LiteralKind::Char(c) => (hir::LiteralValue::Char(*c), Type::char()),
            ast::LiteralKind::String(s) => (hir::LiteralValue::String(s.clone()), Type::string()),
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Literal(value),
            ty,
            span,
        ))
    }

    /// Infer type of a path (variable/function reference).
    fn infer_path(&mut self, path: &ast::ExprPath, span: Span) -> Result<hir::Expr, TypeError> {
        // For now, handle simple single-segment paths
        if path.segments.len() == 1 {
            let name = self.symbol_to_string(path.segments[0].name.node);

            match self.resolver.lookup(&name) {
                Some(Binding::Local { local_id, ty, .. }) => {
                    Ok(hir::Expr::new(
                        hir::ExprKind::Local(local_id),
                        ty,
                        span,
                    ))
                }
                Some(Binding::Def(def_id)) => {
                    // Look up the type of the definition
                    if let Some(sig) = self.fn_sigs.get(&def_id).cloned() {
                        // For generic functions, instantiate fresh type variables
                        let fn_ty = if sig.generics.is_empty() {
                            Type::function(sig.inputs.clone(), sig.output.clone())
                        } else {
                            self.instantiate_fn_sig(&sig)
                        };
                        Ok(hir::Expr::new(
                            hir::ExprKind::Def(def_id),
                            fn_ty,
                            span,
                        ))
                    } else if let Some(def_info) = self.resolver.def_info.get(&def_id) {
                        // Look up type from def_info (constants, statics, etc.)
                        let ty = if let Some(ty) = &def_info.ty {
                            ty.clone()
                        } else {
                            // Type not yet known - use fresh type variable for inference
                            self.unifier.fresh_var()
                        };
                        Ok(hir::Expr::new(
                            hir::ExprKind::Def(def_id),
                            ty,
                            span,
                        ))
                    } else {
                        // No def_info - internal error, should not happen
                        Err(TypeError::new(
                            TypeErrorKind::NotFound { name },
                            span,
                        ))
                    }
                }
                None => {
                    Err(TypeError::new(
                        TypeErrorKind::NotFound { name },
                        span,
                    ))
                }
            }
        } else {
            // Multi-segment paths - Phase 2+
            Err(TypeError::new(
                TypeErrorKind::NotFound { name: format!("{:?}", path) },
                span,
            ))
        }
    }

    /// Infer type of a binary expression.
    fn infer_binary(
        &mut self,
        op: ast::BinOp,
        left: &ast::Expr,
        right: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let left_expr = self.infer_expr(left)?;
        let right_expr = self.infer_expr(right)?;

        // Check operator compatibility and determine result type
        let result_ty = match op {
            // Arithmetic operators
            ast::BinOp::Add | ast::BinOp::Sub | ast::BinOp::Mul | ast::BinOp::Div | ast::BinOp::Rem => {
                // Both operands must be the same numeric type
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
                left_expr.ty.clone()
            }
            // Comparison operators
            ast::BinOp::Eq | ast::BinOp::Ne | ast::BinOp::Lt | ast::BinOp::Le | ast::BinOp::Gt | ast::BinOp::Ge => {
                // Operands must be comparable
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
                Type::bool()
            }
            // Logical operators
            ast::BinOp::And | ast::BinOp::Or => {
                self.unifier.unify(&left_expr.ty, &Type::bool(), span)?;
                self.unifier.unify(&right_expr.ty, &Type::bool(), span)?;
                Type::bool()
            }
            // Bitwise operators
            ast::BinOp::BitAnd | ast::BinOp::BitOr | ast::BinOp::BitXor | ast::BinOp::Shl | ast::BinOp::Shr => {
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
                left_expr.ty.clone()
            }
            // Pipe operator
            ast::BinOp::Pipe => {
                // left |> right === right(left)
                // right must be a function taking left as argument
                // result type is the function's return type
                match right_expr.ty.kind() {
                    TypeKind::Fn { params, ret } => {
                        // Verify the function takes at least one parameter
                        if params.is_empty() {
                            return Err(TypeError::new(
                                TypeErrorKind::WrongArity {
                                    expected: 1,
                                    found: 0,
                                },
                                span,
                            ));
                        }
                        // Verify the left operand type matches the first parameter
                        self.unifier.unify(&left_expr.ty, &params[0], span)?;
                        // Result is the function's return type
                        ret.clone()
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotAFunction { ty: right_expr.ty.clone() },
                            span,
                        ));
                    }
                }
            }
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Binary {
                op,
                left: Box::new(left_expr),
                right: Box::new(right_expr),
            },
            result_ty,
            span,
        ))
    }

    /// Infer type of a unary expression.
    fn infer_unary(
        &mut self,
        op: ast::UnaryOp,
        operand: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let operand_expr = self.infer_expr(operand)?;

        let result_ty = match op {
            ast::UnaryOp::Neg => {
                // Operand must be numeric
                operand_expr.ty.clone()
            }
            ast::UnaryOp::Not => {
                // Operand must be bool or integer
                operand_expr.ty.clone()
            }
            ast::UnaryOp::Deref => {
                // Operand must be a reference
                match operand_expr.ty.kind() {
                    TypeKind::Ref { inner, .. } => inner.clone(),
                    TypeKind::Ptr { inner, .. } => inner.clone(),
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::CannotDeref { ty: operand_expr.ty.clone() },
                            span,
                        ));
                    }
                }
            }
            ast::UnaryOp::Ref => {
                Type::reference(operand_expr.ty.clone(), false)
            }
            ast::UnaryOp::RefMut => {
                Type::reference(operand_expr.ty.clone(), true)
            }
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Unary {
                op,
                operand: Box::new(operand_expr),
            },
            result_ty,
            span,
        ))
    }

    /// Infer type of a function call.
    fn infer_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::CallArg],
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let callee_expr = self.infer_expr(callee)?;

        // Check that callee is a function
        let (param_types, return_type) = match callee_expr.ty.kind() {
            TypeKind::Fn { params, ret } => (params.clone(), ret.clone()),
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::NotAFunction { ty: callee_expr.ty.clone() },
                    span,
                ));
            }
        };

        // Check arity
        if args.len() != param_types.len() {
            return Err(TypeError::new(
                TypeErrorKind::WrongArity {
                    expected: param_types.len(),
                    found: args.len(),
                },
                span,
            ));
        }

        // Check effect compatibility: callee's effects must be subset of caller's effects
        if let hir::ExprKind::Def(callee_def_id) = &callee_expr.kind {
            self.check_effect_compatibility(*callee_def_id, span)?;
        }

        // Type-check arguments
        let mut hir_args = Vec::new();
        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
            let arg_expr = self.check_expr(&arg.value, param_ty)?;
            hir_args.push(arg_expr);
        }

        Ok(hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(callee_expr),
                args: hir_args,
            },
            return_type,
            span,
        ))
    }

    /// Check that calling a function is effect-compatible with the current context.
    ///
    /// A function can only call another function if the callee's effects are
    /// a subset of the caller's effects (or are handled by an enclosing handler).
    fn check_effect_compatibility(&self, callee_def_id: DefId, span: Span) -> Result<(), TypeError> {
        // Get callee's effects
        let callee_effects = self.fn_effects.get(&callee_def_id);

        // If callee has no effects, it's always compatible
        let callee_effects = match callee_effects {
            Some(effects) if !effects.is_empty() => effects,
            _ => return Ok(()),
        };

        // Get current function's effects
        let current_fn = match self.current_fn {
            Some(def_id) => def_id,
            None => {
                // Not inside a function - this is unexpected during effect checking
                // This shouldn't happen but we gracefully return Ok to avoid spurious errors
                return Ok(());
            }
        };

        let caller_effects = self.fn_effects.get(&current_fn);
        let caller_effect_ids: Vec<DefId> = caller_effects
            .map(|effects| effects.iter().map(|e| e.def_id).collect())
            .unwrap_or_default();

        // Check that each of callee's effects is either:
        // 1. In caller's declared effects, OR
        // 2. Handled by an enclosing with...handle block
        for effect_ref in callee_effects {
            let effect_id = effect_ref.def_id;

            let in_caller_effects = caller_effect_ids.contains(&effect_id);
            let is_handled = self.handled_effects.contains(&effect_id);

            if !in_caller_effects && !is_handled {
                // Get effect name for error message
                let effect_name = self.effect_defs
                    .get(&effect_id)
                    .map(|info| info.name.clone())
                    .unwrap_or_else(|| format!("{:?}", effect_id));

                return Err(TypeError::new(
                    TypeErrorKind::UnhandledEffect { effect: effect_name },
                    span,
                ));
            }
        }

        Ok(())
    }

    /// Infer type of an if expression.
    fn infer_if(
        &mut self,
        condition: &ast::Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ast::ElseBranch>,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Condition must be bool
        let cond_expr = self.check_expr(condition, &Type::bool())?;

        // Both branches must have same type
        let expected = self.unifier.fresh_var();
        let then_expr = self.check_block(then_branch, &expected)?;

        let else_expr = if let Some(else_branch) = else_branch {
            match else_branch {
                ast::ElseBranch::Block(block) => {
                    Some(Box::new(self.check_block(block, &expected)?))
                }
                ast::ElseBranch::If(if_expr) => {
                    Some(Box::new(self.check_expr(if_expr, &expected)?))
                }
            }
        } else {
            // No else branch - result is unit
            self.unifier.unify(&expected, &Type::unit(), span)?;
            None
        };

        let result_ty = self.unifier.resolve(&expected);

        Ok(hir::Expr::new(
            hir::ExprKind::If {
                condition: Box::new(cond_expr),
                then_branch: Box::new(then_expr),
                else_branch: else_expr,
            },
            result_ty,
            span,
        ))
    }

    /// Infer type of a return expression.
    fn infer_return(&mut self, value: Option<&ast::Expr>, span: Span) -> Result<hir::Expr, TypeError> {
        let return_type = self.return_type.clone().ok_or_else(|| {
            TypeError::new(TypeErrorKind::ReturnOutsideFunction, span)
        })?;

        let value_expr = if let Some(value) = value {
            Some(Box::new(self.check_expr(value, &return_type)?))
        } else {
            // return; is only valid if return type is unit
            self.unifier.unify(&return_type, &Type::unit(), span)?;
            None
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Return(value_expr),
            Type::never(),
            span,
        ))
    }

    /// Infer type of a tuple expression.
    fn infer_tuple(&mut self, exprs: &[ast::Expr], span: Span) -> Result<hir::Expr, TypeError> {
        let mut hir_exprs = Vec::new();
        let mut types = Vec::new();

        for expr in exprs {
            let hir_expr = self.infer_expr(expr)?;
            types.push(hir_expr.ty.clone());
            hir_exprs.push(hir_expr);
        }

        Ok(hir::Expr::new(
            hir::ExprKind::Tuple(hir_exprs),
            Type::tuple(types),
            span,
        ))
    }

    /// Infer type of an assignment.
    fn infer_assign(&mut self, target: &ast::Expr, value: &ast::Expr, span: Span) -> Result<hir::Expr, TypeError> {
        let target_expr = self.infer_expr(target)?;
        let value_expr = self.check_expr(value, &target_expr.ty)?;

        Ok(hir::Expr::new(
            hir::ExprKind::Assign {
                target: Box::new(target_expr),
                value: Box::new(value_expr),
            },
            Type::unit(),
            span,
        ))
    }

    /// Infer type of a loop.
    fn infer_loop(&mut self, body: &ast::Block, span: Span) -> Result<hir::Expr, TypeError> {
        self.resolver.push_scope(ScopeKind::Loop, span);

        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        // Loop type depends on break values
        Ok(hir::Expr::new(
            hir::ExprKind::Loop {
                body: Box::new(body_expr),
                label: None,
            },
            Type::never(), // Unless there's a break
            span,
        ))
    }

    /// Infer type of a while loop.
    fn infer_while(&mut self, condition: &ast::Expr, body: &ast::Block, span: Span) -> Result<hir::Expr, TypeError> {
        self.resolver.push_scope(ScopeKind::Loop, span);

        let cond_expr = self.check_expr(condition, &Type::bool())?;
        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        Ok(hir::Expr::new(
            hir::ExprKind::While {
                condition: Box::new(cond_expr),
                body: Box::new(body_expr),
                label: None,
            },
            Type::unit(),
            span,
        ))
    }

    /// Infer type of a for loop.
    ///
    /// Desugars `for i in start..end { body }` to:
    /// ```text
    /// {
    ///     let mut _idx = start;
    ///     while _idx < end {  // or <= for inclusive
    ///         let i = _idx;
    ///         body;
    ///         _idx = _idx + 1;
    ///     }
    /// }
    /// ```
    fn infer_for(
        &mut self,
        pattern: &ast::Pattern,
        iter: &ast::Expr,
        body: &ast::Block,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Extract range bounds from the iterator expression
        let (start, end, inclusive) = match &iter.kind {
            ast::ExprKind::Range { start, end, inclusive } => {
                let start = start.as_ref().ok_or_else(|| {
                    TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "For loop requires range with start bound".into(),
                        },
                        iter.span,
                    )
                })?;
                let end = end.as_ref().ok_or_else(|| {
                    TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "For loop requires range with end bound".into(),
                        },
                        iter.span,
                    )
                })?;
                (start, end, *inclusive)
            }
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "For loop currently only supports range expressions (e.g., 1..10 or 1..=10)".into(),
                    },
                    iter.span,
                ));
            }
        };

        // Get the loop variable name from the pattern
        let var_name = match &pattern.kind {
            ast::PatternKind::Ident { name, .. } => self.symbol_to_string(name.node),
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "For loop currently only supports simple identifier patterns".into(),
                    },
                    pattern.span,
                ));
            }
        };

        self.resolver.push_scope(ScopeKind::Loop, span);

        // Infer the start expression - this determines the loop variable type
        let start_expr = self.infer_expr(start)?;
        let idx_ty = start_expr.ty.clone();

        // Check end expression against the same type
        let end_expr = self.check_expr(end, &idx_ty)?;

        // Create the loop index variable (_idx) - internal, not user-visible
        let idx_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: idx_local_id,
            ty: idx_ty.clone(),
            mutable: true,
            name: Some("_loop_idx".to_string()),
            span,
        });

        // Register the user's loop variable in the resolver (creates binding in scope)
        // This creates the LocalId AND adds it to the scope
        let var_local_id = self.resolver.define_local(
            var_name.clone(),
            idx_ty.clone(),
            false,
            pattern.span,
        )?;

        // Also add to our locals list for HIR generation
        self.locals.push(hir::Local {
            id: var_local_id,
            ty: idx_ty.clone(),
            mutable: false,
            name: Some(var_name),
            span: pattern.span,
        });

        // Type check the body
        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        // Build the desugared while loop structure:
        //
        // {
        //     let mut _idx = start;
        //     while _idx < end {
        //         let i = _idx;
        //         body
        //         _idx = _idx + 1;
        //     }
        // }

        // Create condition: _idx < end (or _idx <= end for inclusive)
        let comparison_op = if inclusive { ast::BinOp::Le } else { ast::BinOp::Lt };
        let condition = hir::Expr::new(
            hir::ExprKind::Binary {
                op: comparison_op,
                left: Box::new(hir::Expr::new(
                    hir::ExprKind::Local(idx_local_id),
                    idx_ty.clone(),
                    span,
                )),
                right: Box::new(end_expr),
            },
            Type::bool(),
            span,
        );

        // Create: let i = _idx;
        let bind_stmt = hir::Stmt::Let {
            local_id: var_local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::Local(idx_local_id),
                idx_ty.clone(),
                span,
            )),
        };

        // Create: _idx = _idx + 1;
        let increment = hir::Expr::new(
            hir::ExprKind::Assign {
                target: Box::new(hir::Expr::new(
                    hir::ExprKind::Local(idx_local_id),
                    idx_ty.clone(),
                    span,
                )),
                value: Box::new(hir::Expr::new(
                    hir::ExprKind::Binary {
                        op: ast::BinOp::Add,
                        left: Box::new(hir::Expr::new(
                            hir::ExprKind::Local(idx_local_id),
                            idx_ty.clone(),
                            span,
                        )),
                        right: Box::new(hir::Expr::new(
                            hir::ExprKind::Literal(hir::LiteralValue::Int(1)),
                            idx_ty.clone(),
                            span,
                        )),
                    },
                    idx_ty.clone(),
                    span,
                )),
            },
            Type::unit(),
            span,
        );

        // Combine body: { let i = _idx; body; _idx = _idx + 1; }
        let while_body = hir::Expr::new(
            hir::ExprKind::Block {
                stmts: vec![bind_stmt, hir::Stmt::Expr(body_expr), hir::Stmt::Expr(increment)],
                expr: None,
            },
            Type::unit(),
            span,
        );

        // Create while loop
        let while_loop = hir::Expr::new(
            hir::ExprKind::While {
                condition: Box::new(condition),
                body: Box::new(while_body),
                label: None,
            },
            Type::unit(),
            span,
        );

        // Create: let mut _idx = start;
        let init_stmt = hir::Stmt::Let {
            local_id: idx_local_id,
            init: Some(start_expr),
        };

        // Wrap in block
        Ok(hir::Expr::new(
            hir::ExprKind::Block {
                stmts: vec![init_stmt],
                expr: Some(Box::new(while_loop)),
            },
            Type::unit(),
            span,
        ))
    }

    /// Infer type of a break.
    fn infer_break(&mut self, value: Option<&ast::Expr>, span: Span) -> Result<hir::Expr, TypeError> {
        if !self.resolver.in_loop() {
            return Err(TypeError::new(TypeErrorKind::BreakOutsideLoop, span));
        }

        let value_expr = if let Some(value) = value {
            Some(Box::new(self.infer_expr(value)?))
        } else {
            None
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Break {
                label: None,
                value: value_expr,
            },
            Type::never(),
            span,
        ))
    }

    /// Infer type of a match expression.
    fn infer_match(
        &mut self,
        scrutinee: &ast::Expr,
        arms: &[ast::MatchArm],
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let scrutinee_expr = self.infer_expr(scrutinee)?;

        if arms.is_empty() {
            return Ok(hir::Expr::new(
                hir::ExprKind::Match {
                    scrutinee: Box::new(scrutinee_expr),
                    arms: Vec::new(),
                },
                Type::never(),
                span,
            ));
        }

        let result_ty = self.unifier.fresh_var();
        let mut hir_arms = Vec::new();

        for arm in arms {
            self.resolver.push_scope(ScopeKind::MatchArm, arm.span);

            // Phase 2+: Implement exhaustiveness and usefulness checking for patterns.
            // Currently we lower the pattern but don't verify that the pattern fully
            // covers all variants of the scrutinee type or detect unreachable arms.
            let pattern = self.lower_pattern(&arm.pattern, &scrutinee_expr.ty)?;

            let guard = if let Some(ref guard) = arm.guard {
                Some(self.check_expr(guard, &Type::bool())?)
            } else {
                None
            };

            let body = self.check_expr(&arm.body, &result_ty)?;

            self.resolver.pop_scope();

            hir_arms.push(hir::MatchArm {
                pattern,
                guard,
                body,
            });
        }

        let final_ty = self.unifier.resolve(&result_ty);

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(scrutinee_expr),
                arms: hir_arms,
            },
            final_ty,
            span,
        ))
    }

    /// Lower a pattern from AST to HIR.
    fn lower_pattern(&mut self, pattern: &ast::Pattern, expected_ty: &Type) -> Result<hir::Pattern, TypeError> {
        let kind = match &pattern.kind {
            ast::PatternKind::Wildcard => hir::PatternKind::Wildcard,
            ast::PatternKind::Ident { name, mutable, .. } => {
                let name_str = self.symbol_to_string(name.node);
                let local_id = self.resolver.define_local(
                    name_str.clone(),
                    expected_ty.clone(),
                    *mutable,
                    pattern.span,
                )?;

                self.locals.push(hir::Local {
                    id: local_id,
                    ty: expected_ty.clone(),
                    mutable: *mutable,
                    name: Some(name_str),
                    span: pattern.span,
                });

                hir::PatternKind::Binding {
                    local_id,
                    mutable: *mutable,
                    subpattern: None,
                }
            }
            ast::PatternKind::Literal(lit) => {
                hir::PatternKind::Literal(hir::LiteralValue::from_ast(&lit.kind))
            }
            ast::PatternKind::Tuple { fields, rest_pos } => {
                // Check if the expected type is a tuple
                match expected_ty.kind() {
                    TypeKind::Tuple(elem_types) => {
                        if rest_pos.is_some() {
                            return Err(TypeError::new(
                                TypeErrorKind::UnsupportedFeature {
                                    feature: "rest patterns in tuples are not yet supported".to_string(),
                                },
                                pattern.span,
                            ));
                        }
                        if fields.len() != elem_types.len() {
                            return Err(TypeError::new(
                                TypeErrorKind::PatternMismatch {
                                    expected: expected_ty.clone(),
                                    pattern: format!("tuple pattern with {} elements", fields.len()),
                                },
                                pattern.span,
                            ));
                        }
                        let mut hir_fields = Vec::new();
                        for (field, elem_ty) in fields.iter().zip(elem_types.iter()) {
                            hir_fields.push(self.lower_pattern(field, elem_ty)?);
                        }
                        hir::PatternKind::Tuple(hir_fields)
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotATuple { ty: expected_ty.clone() },
                            pattern.span,
                        ));
                    }
                }
            }
            ast::PatternKind::TupleStruct { path, fields, .. } => {
                // Resolve the path to find the variant
                let path_str = if let Some(seg) = path.segments.first() {
                    self.symbol_to_string(seg.name.node)
                } else {
                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{path:?}") },
                        pattern.span,
                    ));
                };

                // Look up the variant in scope
                match self.resolver.lookup(&path_str) {
                    Some(super::resolve::Binding::Def(variant_def_id)) => {
                        // Check if it's an enum variant
                        if let Some(info) = self.resolver.def_info.get(&variant_def_id) {
                            if info.kind == hir::DefKind::Variant {
                                // Get the parent enum def_id
                                let enum_def_id = info.parent.ok_or_else(|| TypeError::new(
                                    TypeErrorKind::NotFound { name: format!("parent of variant {}", path_str) },
                                    pattern.span,
                                ))?;

                                // Look up the enum to find variant info
                                let enum_info = self.enum_defs.get(&enum_def_id).ok_or_else(|| TypeError::new(
                                    TypeErrorKind::NotFound { name: format!("enum for variant {}", path_str) },
                                    pattern.span,
                                ))?;

                                // Find the variant in the enum
                                let variant_info = enum_info.variants.iter()
                                    .find(|v| v.def_id == variant_def_id)
                                    .ok_or_else(|| TypeError::new(
                                        TypeErrorKind::NotFound { name: format!("variant {} in enum", path_str) },
                                        pattern.span,
                                    ))?;

                                let variant_idx = variant_info.index;
                                // Clone field types to avoid borrow conflict
                                let variant_field_types: Vec<Type> = variant_info.fields.iter()
                                    .map(|f| f.ty.clone())
                                    .collect();

                                // Check field count matches
                                if fields.len() != variant_field_types.len() {
                                    return Err(TypeError::new(
                                        TypeErrorKind::WrongArity {
                                            expected: variant_field_types.len(),
                                            found: fields.len(),
                                        },
                                        pattern.span,
                                    ));
                                }

                                // Use actual variant field types
                                let mut hir_fields = Vec::new();
                                for (field, field_ty) in fields.iter().zip(variant_field_types.iter()) {
                                    hir_fields.push(self.lower_pattern(field, field_ty)?);
                                }

                                hir::PatternKind::Variant {
                                    def_id: variant_def_id,
                                    variant_idx,
                                    fields: hir_fields,
                                }
                            } else {
                                return Err(TypeError::new(
                                    TypeErrorKind::NotFound { name: path_str },
                                    pattern.span,
                                ));
                            }
                        } else {
                            return Err(TypeError::new(
                                TypeErrorKind::NotFound { name: path_str },
                                pattern.span,
                            ));
                        }
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotFound { name: path_str },
                            pattern.span,
                        ));
                    }
                }
            }
            ast::PatternKind::Rest => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "rest patterns (..) are not yet supported".to_string(),
                    },
                    pattern.span,
                ));
            }
            ast::PatternKind::Ref { .. } => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "reference patterns (&x) are not yet supported".to_string(),
                    },
                    pattern.span,
                ));
            }
            ast::PatternKind::Struct { .. } => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "struct patterns are not yet supported".to_string(),
                    },
                    pattern.span,
                ));
            }
            ast::PatternKind::Slice { .. } => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "slice patterns are not yet supported".to_string(),
                    },
                    pattern.span,
                ));
            }
            ast::PatternKind::Or(_) => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "or patterns (A | B) are not yet supported".to_string(),
                    },
                    pattern.span,
                ));
            }
            ast::PatternKind::Range { .. } => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "range patterns are not yet supported".to_string(),
                    },
                    pattern.span,
                ));
            }
            ast::PatternKind::Path(path) => {
                // Unit variant pattern like `None`
                let path_str = if let Some(seg) = path.segments.first() {
                    self.symbol_to_string(seg.name.node)
                } else {
                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{path:?}") },
                        pattern.span,
                    ));
                };

                match self.resolver.lookup(&path_str) {
                    Some(super::resolve::Binding::Def(def_id)) => {
                        if let Some(info) = self.resolver.def_info.get(&def_id) {
                            if info.kind == hir::DefKind::Variant {
                                hir::PatternKind::Variant {
                                    def_id,
                                    variant_idx: 0, // Simplified
                                    fields: vec![],
                                }
                            } else {
                                return Err(TypeError::new(
                                    TypeErrorKind::NotFound { name: path_str },
                                    pattern.span,
                                ));
                            }
                        } else {
                            return Err(TypeError::new(
                                TypeErrorKind::NotFound { name: path_str },
                                pattern.span,
                            ));
                        }
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotFound { name: path_str },
                            pattern.span,
                        ));
                    }
                }
            }
            ast::PatternKind::Paren(inner) => {
                // Parenthesized pattern - just unwrap it
                return self.lower_pattern(inner, expected_ty);
            }
        };

        Ok(hir::Pattern {
            kind,
            ty: expected_ty.clone(),
            span: pattern.span,
        })
    }

    /// Instantiate a generic function signature with fresh type variables.
    ///
    /// For each generic type parameter in the signature, creates a fresh inference
    /// variable and substitutes it throughout the input and output types.
    fn instantiate_fn_sig(&mut self, sig: &hir::FnSig) -> Type {
        if sig.generics.is_empty() {
            return Type::function(sig.inputs.clone(), sig.output.clone());
        }

        // Create a mapping from old type vars to fresh ones
        let mut substitution: HashMap<TyVarId, Type> = HashMap::new();
        for &old_var in &sig.generics {
            let fresh_var = self.unifier.fresh_var();
            substitution.insert(old_var, fresh_var);
        }

        // Substitute in inputs and output
        let inputs: Vec<Type> = sig.inputs.iter()
            .map(|ty| self.substitute_type_vars(ty, &substitution))
            .collect();
        let output = self.substitute_type_vars(&sig.output, &substitution);

        Type::function(inputs, output)
    }

    /// Substitute type variables in a type according to a substitution map.
    fn substitute_type_vars(&self, ty: &Type, subst: &HashMap<TyVarId, Type>) -> Type {
        match ty.kind() {
            TypeKind::Param(var_id) => {
                if let Some(replacement) = subst.get(var_id) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            TypeKind::Fn { params, ret } => {
                let new_params: Vec<Type> = params.iter()
                    .map(|p| self.substitute_type_vars(p, subst))
                    .collect();
                let new_ret = self.substitute_type_vars(ret, subst);
                Type::function(new_params, new_ret)
            }
            TypeKind::Tuple(elems) => {
                let new_elems: Vec<Type> = elems.iter()
                    .map(|e| self.substitute_type_vars(e, subst))
                    .collect();
                Type::tuple(new_elems)
            }
            TypeKind::Array { element, size } => {
                let new_elem = self.substitute_type_vars(element, subst);
                Type::array(new_elem, *size)
            }
            TypeKind::Slice { element } => {
                let new_elem = self.substitute_type_vars(element, subst);
                Type::slice(new_elem)
            }
            TypeKind::Ref { inner, mutable } => {
                let new_inner = self.substitute_type_vars(inner, subst);
                Type::reference(new_inner, *mutable)
            }
            TypeKind::Ptr { inner, mutable } => {
                let new_inner = self.substitute_type_vars(inner, subst);
                Type::new(TypeKind::Ptr { inner: new_inner, mutable: *mutable })
            }
            TypeKind::Adt { def_id, args } => {
                let new_args: Vec<Type> = args.iter()
                    .map(|a| self.substitute_type_vars(a, subst))
                    .collect();
                Type::adt(*def_id, new_args)
            }
            // Primitives, Never, Error, Infer don't need substitution
            _ => ty.clone(),
        }
    }

    /// Convert an AST type to an HIR type.
    fn ast_type_to_hir_type(&mut self, ty: &ast::Type) -> Result<Type, TypeError> {
        match &ty.kind {
            ast::TypeKind::Path(path) => {
                if path.segments.len() == 1 && path.segments[0].args.is_none() {
                    let name = self.symbol_to_string(path.segments[0].name.node);

                    // Check for primitive types
                    if let Some(prim) = PrimitiveTy::from_name(&name) {
                        return Ok(Type::new(TypeKind::Primitive(prim)));
                    }

                    // Check for generic type parameters in current scope
                    if let Some(&ty_var_id) = self.generic_params.get(&name) {
                        return Ok(Type::new(TypeKind::Param(ty_var_id)));
                    }

                    // Look up user-defined types
                    if let Some(def_id) = self.resolver.lookup_type(&name) {
                        // Check if this is a type alias
                        if let Some(alias_info) = self.type_aliases.get(&def_id).cloned() {
                            // For type aliases without arguments, return the aliased type directly
                            if alias_info.generics.is_empty() {
                                return Ok(alias_info.ty);
                            } else {
                                // Type alias with generics but no arguments provided - error
                                return Err(TypeError::new(
                                    TypeErrorKind::GenericArgsMismatch {
                                        expected: alias_info.generics.len(),
                                        found: 0,
                                    },
                                    ty.span,
                                ));
                            }
                        }
                        return Ok(Type::adt(def_id, Vec::new()));
                    }

                    return Err(TypeError::new(
                        TypeErrorKind::TypeNotFound { name },
                        ty.span,
                    ));
                }

                // Handle paths with type arguments (generic instantiation)
                if path.segments.len() == 1 {
                    let segment = &path.segments[0];
                    let name = self.symbol_to_string(segment.name.node);

                    // Look up the base type
                    if let Some(def_id) = self.resolver.lookup_type(&name) {
                        // Convert type arguments if present
                        let type_args = if let Some(ref args) = segment.args {
                            let mut converted = Vec::new();
                            for arg in &args.args {
                                match arg {
                                    ast::TypeArg::Type(arg_ty) => {
                                        converted.push(self.ast_type_to_hir_type(arg_ty)?);
                                    }
                                    ast::TypeArg::Const(_) => {
                                        // Const generics - Phase 2+
                                    }
                                    ast::TypeArg::Lifetime(_) => {
                                        // Lifetime parameters - Phase 2+
                                    }
                                }
                            }
                            converted
                        } else {
                            Vec::new()
                        };

                        // Check if this is a type alias with type arguments
                        if let Some(alias_info) = self.type_aliases.get(&def_id).cloned() {
                            if alias_info.generics.len() != type_args.len() {
                                return Err(TypeError::new(
                                    TypeErrorKind::GenericArgsMismatch {
                                        expected: alias_info.generics.len(),
                                        found: type_args.len(),
                                    },
                                    ty.span,
                                ));
                            }
                            // Substitute type parameters with provided arguments
                            let subst: HashMap<TyVarId, Type> = alias_info.generics
                                .iter()
                                .zip(type_args.iter())
                                .map(|(&param, arg)| (param, arg.clone()))
                                .collect();
                            return Ok(self.substitute_type_vars(&alias_info.ty, &subst));
                        }

                        return Ok(Type::adt(def_id, type_args));
                    }
                }

                // Multi-segment path - Phase 2+
                Err(TypeError::new(
                    TypeErrorKind::TypeNotFound { name: format!("{path:?}") },
                    ty.span,
                ))
            }
            ast::TypeKind::Reference { inner, mutable, .. } => {
                let inner_ty = self.ast_type_to_hir_type(inner)?;
                Ok(Type::reference(inner_ty, *mutable))
            }
            ast::TypeKind::Pointer { inner, mutable } => {
                let inner_ty = self.ast_type_to_hir_type(inner)?;
                Ok(Type::new(TypeKind::Ptr {
                    inner: inner_ty,
                    mutable: *mutable,
                }))
            }
            ast::TypeKind::Tuple(types) => {
                let hir_types: Result<Vec<_>, _> = types
                    .iter()
                    .map(|t| self.ast_type_to_hir_type(t))
                    .collect();
                Ok(Type::tuple(hir_types?))
            }
            ast::TypeKind::Array { element, size } => {
                let elem_ty = self.ast_type_to_hir_type(element)?;
                // Array size must be a constant integer (const eval required for non-literals)
                let size_val = match &size.kind {
                    ast::ExprKind::Literal(ast::Literal { kind: ast::LiteralKind::Int { value, .. }, .. }) => {
                        *value as u64
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "array size must be a literal integer (const evaluation not yet supported)".to_string(),
                            },
                            size.span,
                        ));
                    }
                };
                Ok(Type::array(elem_ty, size_val))
            }
            ast::TypeKind::Slice { element } => {
                let elem_ty = self.ast_type_to_hir_type(element)?;
                Ok(Type::slice(elem_ty))
            }
            ast::TypeKind::Function { params, return_type, .. } => {
                let param_types: Result<Vec<_>, _> = params
                    .iter()
                    .map(|t| self.ast_type_to_hir_type(t))
                    .collect();
                let ret_ty = self.ast_type_to_hir_type(return_type)?;
                Ok(Type::function(param_types?, ret_ty))
            }
            ast::TypeKind::Never => Ok(Type::never()),
            ast::TypeKind::Infer => Ok(self.unifier.fresh_var()),
            ast::TypeKind::Paren(inner) => self.ast_type_to_hir_type(inner),
            ast::TypeKind::Record { .. } => {
                // Record types (structural records) - Phase 2+
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature { feature: "record types".to_string() },
                    ty.span,
                ))
            }
            ast::TypeKind::Ownership { .. } => {
                // Ownership types (linear, affine) - Phase 2+
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature { feature: "ownership types".to_string() },
                    ty.span,
                ))
            }
        }
    }

    /// Convert a Symbol to a String.
    ///
    /// Note: This is a temporary implementation. In the real implementation,
    /// we'd use the string interner from the parser.
    fn symbol_to_string(&self, symbol: ast::Symbol) -> String {
        self.interner.resolve(symbol)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("sym_{}", symbol.to_usize()))
    }

    /// Convert to HIR crate.
    pub fn into_hir(self) -> hir::Crate {
        let mut items = HashMap::new();

        // Convert collected definitions to HIR items
        for (def_id, info) in self.resolver.def_info {
            let kind = match info.kind {
                hir::DefKind::Fn => {
                    if let Some(sig) = self.fn_sigs.get(&def_id) {
                        hir::ItemKind::Fn(hir::FnDef {
                            sig: sig.clone(),
                            body_id: self.fn_bodies.get(&def_id).copied(),
                            generics: hir::Generics::empty(),
                        })
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Struct => {
                    if let Some(struct_info) = self.struct_defs.get(&def_id) {
                        let fields: Vec<_> = struct_info.fields.iter().map(|f| {
                            hir::FieldDef {
                                index: f.index,
                                name: Some(f.name.clone()),
                                ty: f.ty.clone(),
                                vis: ast::Visibility::Public,
                                span: info.span,
                            }
                        }).collect();

                        hir::ItemKind::Struct(hir::StructDef {
                            generics: hir::Generics::empty(),
                            kind: hir::StructKind::Record(fields),
                        })
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Enum => {
                    if let Some(enum_info) = self.enum_defs.get(&def_id) {
                        let variants: Vec<_> = enum_info.variants.iter().map(|v| {
                            let fields: Vec<_> = v.fields.iter().map(|f| {
                                hir::FieldDef {
                                    index: f.index,
                                    name: Some(f.name.clone()),
                                    ty: f.ty.clone(),
                                    vis: ast::Visibility::Public,
                                    span: info.span,
                                }
                            }).collect();

                            hir::Variant {
                                index: v.index,
                                name: v.name.clone(),
                                fields: if fields.is_empty() {
                                    hir::StructKind::Unit
                                } else {
                                    hir::StructKind::Record(fields)
                                },
                                def_id: v.def_id,
                                span: info.span,
                            }
                        }).collect();

                        hir::ItemKind::Enum(hir::EnumDef {
                            generics: hir::Generics::empty(),
                            variants,
                        })
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Effect => {
                    if let Some(effect_info) = self.effect_defs.get(&def_id) {
                        let operations: Vec<_> = effect_info.operations.iter().map(|op| {
                            hir::EffectOp {
                                name: op.name.clone(),
                                inputs: op.params.clone(),
                                output: op.return_ty.clone(),
                                def_id: op.def_id,
                                span: Span::default(),
                            }
                        }).collect();

                        hir::ItemKind::Effect {
                            generics: hir::Generics::empty(),
                            operations,
                        }
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Handler => {
                    if let Some(handler_info) = self.handler_defs.get(&def_id) {
                        // Convert handler state fields
                        let state: Vec<_> = handler_info.fields.iter().map(|f| {
                            hir::HandlerState {
                                name: f.name.clone(),
                                ty: f.ty.clone(),
                                mutable: true, // Handler state is typically mutable
                                default: None, // Default values handled at instantiation
                            }
                        }).collect();

                        // Convert ast::HandlerKind to hir::HandlerKind
                        let hir_kind = match handler_info.kind {
                            ast::HandlerKind::Deep => hir::HandlerKind::Deep,
                            ast::HandlerKind::Shallow => hir::HandlerKind::Shallow,
                        };

                        // Build operation list from body IDs
                        let operations: Vec<hir::HandlerOp> = handler_info.operations.iter()
                            .map(|(name, body_id)| hir::HandlerOp {
                                name: name.clone(),
                                body_id: *body_id,
                                span: info.span,
                            })
                            .collect();

                        // Build return clause if present
                        let return_clause = handler_info.return_clause_body_id.map(|body_id| {
                            hir::ReturnClause {
                                param: "x".to_string(), // Default param name
                                body_id,
                                span: info.span,
                            }
                        });

                        hir::ItemKind::Handler {
                            generics: hir::Generics::empty(),
                            kind: hir_kind,
                            effect: Type::adt(handler_info.effect_id, Vec::new()),
                            state,
                            operations,
                            return_clause,
                        }
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Const => {
                    // Constants - get type and body from collected info
                    if let (Some(ty), Some(body_id)) = (&info.ty, self.fn_bodies.get(&def_id).copied()) {
                        hir::ItemKind::Const {
                            ty: ty.clone(),
                            body_id,
                        }
                    } else {
                        // Constants must have both type and body - skip if missing
                        continue;
                    }
                }
                hir::DefKind::Static => {
                    // Statics - get type and body from collected info
                    if let (Some(ty), Some(body_id)) = (&info.ty, self.fn_bodies.get(&def_id).copied()) {
                        hir::ItemKind::Static {
                            ty: ty.clone(),
                            mutable: false, // Would need to track this
                            body_id,
                        }
                    } else {
                        // Statics must have both type and body - skip if missing
                        continue;
                    }
                }
                // Variants are part of enums, not top-level items
                hir::DefKind::Variant => continue,
                // Associated items in impl blocks - not standalone items
                hir::DefKind::AssocType | hir::DefKind::AssocConst | hir::DefKind::AssocFn => continue,
                // TypeAlias, Trait not yet implemented
                hir::DefKind::TypeAlias | hir::DefKind::Trait => continue,
                // Closures are inline, not top-level items
                hir::DefKind::Closure => continue,
                // Type/lifetime/const params are not items
                hir::DefKind::TyParam | hir::DefKind::LifetimeParam | hir::DefKind::ConstParam => continue,
                // Local variables are not items
                hir::DefKind::Local => continue,
                // Effect operations are part of effects, not standalone
                hir::DefKind::EffectOp => continue,
                // Fields are part of structs, not standalone
                hir::DefKind::Field => continue,
            };

            items.insert(def_id, hir::Item {
                def_id,
                kind,
                vis: ast::Visibility::Public,
                name: info.name,
                span: info.span,
            });
        }

        // Find main function
        let entry = items.iter()
            .find(|(_, item)| item.name == "main" || item.name.ends_with("_main"))
            .map(|(id, _)| *id);

        hir::Crate {
            items,
            bodies: self.bodies,
            entry,
            builtin_fns: self.builtin_fns,
        }
    }

    /// Infer type of a record (struct) construction expression.
    fn infer_record(
        &mut self,
        path: Option<&ast::TypePath>,
        fields: &[ast::RecordExprField],
        _base: Option<&ast::Expr>,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Get the struct type from the path
        let (def_id, struct_info, result_ty) = if let Some(path) = path {
            if path.segments.len() == 1 {
                let name = self.symbol_to_string(path.segments[0].name.node);

                // Look up the struct definition
                if let Some(binding) = self.resolver.lookup(&name) {
                    match binding {
                        Binding::Def(def_id) => {
                            if let Some(struct_info) = self.struct_defs.get(&def_id) {
                                let result_ty = Type::adt(def_id, Vec::new());
                                (def_id, struct_info.clone(), result_ty)
                            } else if let Some(handler_info) = self.handler_defs.get(&def_id) {
                                // Handler instantiation uses struct-like syntax
                                let struct_info = StructInfo {
                                    name: handler_info.name.clone(),
                                    fields: handler_info.fields.clone(),
                                    generics: handler_info.generics.clone(),
                                };
                                let result_ty = Type::adt(def_id, Vec::new());
                                (def_id, struct_info, result_ty)
                            } else {
                                return Err(TypeError::new(
                                    TypeErrorKind::NotAStruct { ty: Type::error() },
                                    span,
                                ));
                            }
                        }
                        Binding::Local { .. } => {
                            return Err(TypeError::new(
                                TypeErrorKind::NotAStruct { ty: Type::error() },
                                span,
                            ));
                        }
                    }
                } else {
                    return Err(TypeError::new(
                        TypeErrorKind::UnresolvedName { name },
                        span,
                    ));
                }
            } else {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature { feature: "qualified struct paths".to_string() },
                    span,
                ));
            }
        } else {
            // Anonymous record - not supported in Phase 1
            return Err(TypeError::new(
                TypeErrorKind::UnsupportedFeature { feature: "anonymous records".to_string() },
                span,
            ));
        };

        // Type-check each field
        let mut hir_fields = Vec::new();
        for field in fields {
            let field_name = self.symbol_to_string(field.name.node);

            // Find the field in the struct definition
            let field_info = struct_info.fields.iter()
                .find(|f| f.name == field_name)
                .ok_or_else(|| TypeError::new(
                    TypeErrorKind::UnknownField {
                        ty: result_ty.clone(),
                        field: field_name.clone(),
                    },
                    field.span,
                ))?;

            // Type-check the field value
            let value = if let Some(ref value_expr) = field.value {
                self.check_expr(value_expr, &field_info.ty)?
            } else {
                // Shorthand: `{ x }` means `{ x: x }`
                self.infer_path(&ast::ExprPath {
                    segments: vec![ast::ExprPathSegment {
                        name: field.name.clone(),
                        args: None,
                    }],
                    span: field.span,
                }, field.span)?
            };

            hir_fields.push(hir::FieldExpr {
                field_idx: field_info.index,
                value,
            });
        }

        Ok(hir::Expr::new(
            hir::ExprKind::Struct {
                def_id,
                fields: hir_fields,
                base: None,
            },
            result_ty,
            span,
        ))
    }

    /// Infer type of a closure expression.
    ///
    /// Closures are type-checked as follows:
    /// 1. Create a new closure scope
    /// 2. Determine parameter types (from annotations or inference)
    /// 3. Type-check the body expression
    /// 4. Determine return type (from annotation or body type)
    /// 5. Analyze captured variables
    /// 6. Create HIR closure with function type
    fn infer_closure(
        &mut self,
        is_move: bool,
        params: &[ast::ClosureParam],
        return_type: Option<&ast::Type>,
        body: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Save current locals and create fresh ones for closure
        let outer_locals = std::mem::take(&mut self.locals);
        let outer_return_type = self.return_type.take();

        // Push closure scope (don't reset local IDs - closures share outer function's ID space)
        self.resolver.push_scope(ScopeKind::Closure, span);

        // Add return place - use the next available LocalId for this closure's body
        // (Different from the outer function's return place)
        let return_local_id = self.resolver.next_local_id();
        let expected_return_ty = if let Some(ret_ty) = return_type {
            self.ast_type_to_hir_type(ret_ty)?
        } else {
            self.unifier.fresh_var()
        };

        self.locals.push(hir::Local {
            id: return_local_id,
            ty: expected_return_ty.clone(),
            mutable: false,
            name: None,
            span,
        });

        // Process closure parameters
        let mut param_types = Vec::new();
        for param in params {
            let param_ty = if let Some(ty) = &param.ty {
                self.ast_type_to_hir_type(ty)?
            } else {
                // Create inference variable for parameter without annotation
                self.unifier.fresh_var()
            };

            // Extract name and mutability from parameter pattern
            let (param_name, mutable) = match &param.pattern.kind {
                ast::PatternKind::Ident { name, mutable, .. } => {
                    (self.symbol_to_string(name.node), *mutable)
                }
                ast::PatternKind::Wildcard => {
                    (format!("_param{}", param_types.len()), false)
                }
                _ => {
                    // Complex pattern - generate placeholder
                    (format!("param{}", param_types.len()), false)
                }
            };

            // Define parameter in closure scope
            let local_id = self.resolver.define_local(
                param_name.clone(),
                param_ty.clone(),
                mutable,
                param.span,
            )?;

            self.locals.push(hir::Local {
                id: local_id,
                ty: param_ty.clone(),
                mutable,
                name: Some(param_name),
                span: param.span,
            });

            param_types.push(param_ty);
        }

        // Type-check the closure body
        let body_expr = self.infer_expr(body)?;

        // Unify body type with expected return type
        self.unifier.unify(&body_expr.ty, &expected_return_ty, body.span)?;

        // Resolve all types now that inference is done
        let resolved_return_ty = self.unifier.resolve(&expected_return_ty);
        let resolved_param_types: Vec<Type> = param_types
            .iter()
            .map(|t| self.unifier.resolve(t))
            .collect();

        // Analyze captures (simplified: find all referenced outer locals)
        let captures = self.analyze_closure_captures(&body_expr, is_move);

        // Create closure body
        let body_id = hir::BodyId::new(self.next_body_id);
        self.next_body_id += 1;

        let hir_body = hir::Body {
            locals: std::mem::take(&mut self.locals),
            param_count: params.len(),
            expr: body_expr,
            span,
        };

        self.bodies.insert(body_id, hir_body);

        // Pop closure scope
        self.resolver.pop_scope();

        // Restore outer context
        self.locals = outer_locals;
        self.return_type = outer_return_type;

        // Build the closure type: Fn(params) -> ret
        let closure_ty = Type::function(resolved_param_types, resolved_return_ty);

        Ok(hir::Expr::new(
            hir::ExprKind::Closure {
                body_id,
                captures,
            },
            closure_ty,
            span,
        ))
    }

    /// Analyze which variables a closure captures.
    ///
    /// This is a simplified analysis that finds all local variable references
    /// in the closure body that refer to outer scopes.
    fn analyze_closure_captures(&self, body: &hir::Expr, is_move: bool) -> Vec<hir::Capture> {
        let mut captures = Vec::new();
        let mut seen = std::collections::HashSet::new();
        self.collect_captures(body, is_move, &mut captures, &mut seen);
        captures
    }

    /// Recursively collect captured variables from an expression.
    fn collect_captures(
        &self,
        expr: &hir::Expr,
        is_move: bool,
        captures: &mut Vec<hir::Capture>,
        seen: &mut std::collections::HashSet<LocalId>,
    ) {
        match &expr.kind {
            hir::ExprKind::Local(local_id) => {
                // Check if this local is from an outer scope
                // We consider any local with ID lower than the closure's locals as a capture
                // Note: This is a simplified heuristic; full implementation would track scope depths
                if !seen.contains(local_id) {
                    // Check if this local exists in the current closure's locals
                    let is_closure_local = self.locals.iter().any(|l| l.id == *local_id);
                    if !is_closure_local {
                        seen.insert(*local_id);
                        captures.push(hir::Capture {
                            local_id: *local_id,
                            by_move: is_move,
                        });
                    }
                }
            }
            hir::ExprKind::Binary { left, right, .. } => {
                self.collect_captures(left, is_move, captures, seen);
                self.collect_captures(right, is_move, captures, seen);
            }
            hir::ExprKind::Unary { operand, .. } => {
                self.collect_captures(operand, is_move, captures, seen);
            }
            hir::ExprKind::Call { callee, args } => {
                self.collect_captures(callee, is_move, captures, seen);
                for arg in args {
                    self.collect_captures(arg, is_move, captures, seen);
                }
            }
            hir::ExprKind::If { condition, then_branch, else_branch } => {
                self.collect_captures(condition, is_move, captures, seen);
                self.collect_captures(then_branch, is_move, captures, seen);
                if let Some(else_expr) = else_branch {
                    self.collect_captures(else_expr, is_move, captures, seen);
                }
            }
            hir::ExprKind::Block { stmts, expr: tail } => {
                for stmt in stmts {
                    match stmt {
                        hir::Stmt::Let { init: Some(init), .. } => {
                            self.collect_captures(init, is_move, captures, seen);
                        }
                        hir::Stmt::Expr(e) => {
                            self.collect_captures(e, is_move, captures, seen);
                        }
                        _ => {}
                    }
                }
                if let Some(tail_expr) = tail {
                    self.collect_captures(tail_expr, is_move, captures, seen);
                }
            }
            hir::ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.collect_captures(elem, is_move, captures, seen);
                }
            }
            hir::ExprKind::Field { base, .. } => {
                self.collect_captures(base, is_move, captures, seen);
            }
            hir::ExprKind::Index { base, index } => {
                self.collect_captures(base, is_move, captures, seen);
                self.collect_captures(index, is_move, captures, seen);
            }
            hir::ExprKind::Assign { target, value } => {
                self.collect_captures(target, is_move, captures, seen);
                self.collect_captures(value, is_move, captures, seen);
            }
            hir::ExprKind::Return(opt_expr) => {
                if let Some(e) = opt_expr {
                    self.collect_captures(e, is_move, captures, seen);
                }
            }
            hir::ExprKind::Loop { body, .. } | hir::ExprKind::While { body, .. } => {
                self.collect_captures(body, is_move, captures, seen);
            }
            hir::ExprKind::Break { value, .. } => {
                if let Some(e) = value {
                    self.collect_captures(e, is_move, captures, seen);
                }
            }
            hir::ExprKind::Match { scrutinee, arms } => {
                self.collect_captures(scrutinee, is_move, captures, seen);
                for arm in arms {
                    self.collect_captures(&arm.body, is_move, captures, seen);
                }
            }
            hir::ExprKind::Struct { fields, base, .. } => {
                for field in fields {
                    self.collect_captures(&field.value, is_move, captures, seen);
                }
                if let Some(base_expr) = base {
                    self.collect_captures(base_expr, is_move, captures, seen);
                }
            }
            hir::ExprKind::Closure { .. } => {
                // Nested closures have their own capture analysis
            }
            hir::ExprKind::Borrow { expr: inner, .. }
            | hir::ExprKind::Deref(inner)
            | hir::ExprKind::AddrOf { expr: inner, .. }
            | hir::ExprKind::Unsafe(inner) => {
                self.collect_captures(inner, is_move, captures, seen);
            }
            hir::ExprKind::Let { init, .. } => {
                self.collect_captures(init, is_move, captures, seen);
            }
            hir::ExprKind::Resume { value, .. } => {
                if let Some(v) = value {
                    self.collect_captures(v, is_move, captures, seen);
                }
            }
            hir::ExprKind::Handle { body, .. } => {
                self.collect_captures(body, is_move, captures, seen);
            }
            hir::ExprKind::Perform { args, .. } => {
                for arg in args {
                    self.collect_captures(arg, is_move, captures, seen);
                }
            }
            hir::ExprKind::MethodCall { receiver, args, .. } => {
                self.collect_captures(receiver, is_move, captures, seen);
                for arg in args {
                    self.collect_captures(arg, is_move, captures, seen);
                }
            }
            hir::ExprKind::Array(elements) => {
                for elem in elements {
                    self.collect_captures(elem, is_move, captures, seen);
                }
            }
            hir::ExprKind::Repeat { value, .. } => {
                self.collect_captures(value, is_move, captures, seen);
            }
            hir::ExprKind::Variant { fields, .. } => {
                for field in fields {
                    self.collect_captures(field, is_move, captures, seen);
                }
            }
            hir::ExprKind::Cast { expr: inner, .. } => {
                self.collect_captures(inner, is_move, captures, seen);
            }
            // These don't contain local references directly
            hir::ExprKind::Literal(_)
            | hir::ExprKind::Def(_)
            | hir::ExprKind::Continue { .. }
            | hir::ExprKind::Error => {}
        }
    }

    /// Infer type of a field access expression.
    fn infer_field_access(
        &mut self,
        base: &ast::Expr,
        field: &ast::FieldAccess,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let base_expr = self.infer_expr(base)?;
        let base_ty = self.unifier.resolve(&base_expr.ty);

        match field {
            ast::FieldAccess::Named(name) => {
                let field_name = self.symbol_to_string(name.node);

                // Check if it's a struct type
                if let TypeKind::Adt { def_id, .. } = base_ty.kind() {
                    // Look up the struct definition
                    if let Some(struct_info) = self.struct_defs.get(def_id) {
                        // Find the field
                        if let Some(field_info) = struct_info.fields.iter().find(|f| f.name == field_name) {
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Field {
                                    base: Box::new(base_expr),
                                    field_idx: field_info.index,
                                },
                                field_info.ty.clone(),
                                span,
                            ));
                        } else {
                            return Err(TypeError::new(
                                TypeErrorKind::UnknownField {
                                    ty: base_ty,
                                    field: field_name,
                                },
                                span,
                            ));
                        }
                    }
                }

                // Not a struct or unknown type
                Err(TypeError::new(
                    TypeErrorKind::NotAStruct { ty: base_ty },
                    span,
                ))
            }
            ast::FieldAccess::Index(index, _span) => {
                // Tuple field access
                if let TypeKind::Tuple(types) = base_ty.kind() {
                    if (*index as usize) < types.len() {
                        let field_ty = types[*index as usize].clone();
                        return Ok(hir::Expr::new(
                            hir::ExprKind::Field {
                                base: Box::new(base_expr),
                                field_idx: *index,
                            },
                            field_ty,
                            span,
                        ));
                    }
                }

                Err(TypeError::new(
                    TypeErrorKind::NotATuple { ty: base_ty },
                    span,
                ))
            }
        }
    }
}
