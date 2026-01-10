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
    /// Functions to type-check (includes full declaration for parameter names).
    pending_bodies: Vec<(DefId, ast::FnDecl)>,
    /// The current function's return type.
    return_type: Option<Type>,
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

/// Information about a variant.
#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub index: u32,
    pub def_id: DefId,
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
            pending_bodies: Vec::new(),
            return_type: None,
            errors: Vec::new(),
            bodies: HashMap::new(),
            fn_bodies: HashMap::new(),
            next_body_id: 0,
            locals: Vec::new(),
            generic_params: HashMap::new(),
            next_type_param_id: 0,
            builtin_fns: HashMap::new(),
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
            // Type alias, trait, impl, effect, handler - Phase 2+
            _ => Ok(()), // Skip for now
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

        // Queue function for later body type-checking
        if func.body.is_some() {
            self.pending_bodies.push((def_id, func.clone()));
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

    /// Type-check all queued bodies.
    pub fn check_all_bodies(&mut self) -> Result<(), Vec<Diagnostic>> {
        let pending = std::mem::take(&mut self.pending_bodies);

        for (def_id, func) in pending {
            if let Err(e) = self.check_function_body(def_id, &func) {
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

        // Add parameters as locals with their actual names from the AST
        for (i, param) in func.params.iter().enumerate() {
            let param_ty = sig.inputs.get(i).cloned().unwrap_or_else(Type::error);

            // Extract name and mutability from the parameter pattern
            let (param_name, mutable) = match &param.pattern.kind {
                ast::PatternKind::Ident { name, mutable, .. } => {
                    (self.symbol_to_string(name.node), *mutable)
                }
                ast::PatternKind::Wildcard => {
                    // Anonymous parameter - generate a unique name
                    (format!("_param{i}"), false)
                }
                _ => {
                    // Complex pattern - generate a placeholder name for now.
                    // Phase 2+: Implement destructuring patterns in function parameters,
                    // allowing patterns like `fn foo((x, y): (i32, i32))` to directly
                    // bind tuple elements. This requires generating hidden temporaries
                    // and pattern-matching code in the function prologue.
                    (format!("param{i}"), false)
                }
            };

            // Register the parameter in the resolver so it can be looked up by name
            let local_id = self.resolver.define_local(
                param_name.clone(),
                param_ty.clone(),
                mutable,
                param.span,
            )?;

            self.locals.push(hir::Local {
                id: local_id,
                ty: param_ty,
                mutable,
                name: Some(param_name),
                span: param.span,
            });
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
            // More complex patterns - Phase 2+
            _ => {
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
            ast::ExprKind::WithHandle { handler: _, body } => {
                // Handle expression: runs body with an effect handler installed.
                //
                // Full effect handling requires:
                // 1. Looking up the handler definition
                // 2. Verifying the handler covers the effects used in body
                // 3. Setting up evidence passing
                //
                // For now, we just type-check the body and return it directly.
                // This allows the code to compile without full effect handling.
                // TODO: Implement full effect handler type checking (Phase 2.3)
                let body_block = match &body.kind {
                    ast::ExprKind::Block(block) => block,
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "Handle body must be a block".into(),
                            },
                            body.span,
                        ));
                    }
                };
                // Type-check the block with unknown expected type
                // The actual return type will be inferred from the body
                self.check_block(body_block, &Type::unit())
            }
            ast::ExprKind::Perform { effect: _, operation, args } => {
                // Effect operation: performs an operation from an effect.
                //
                // Full effect handling requires:
                // 1. Looking up the effect definition
                // 2. Finding the operation
                // 3. Type checking arguments
                //
                // For now, return a placeholder that compiles to a runtime error.
                // TODO: Implement full effect perform (Phase 2.3)
                let _op_name = self.symbol_to_string(operation.node);
                for arg in args {
                    let _ = self.infer_expr(arg)?;
                }
                Ok(hir::Expr::new(
                    hir::ExprKind::Error,
                    Type::error(),
                    expr.span,
                ))
            }
            ast::ExprKind::Resume(value) => {
                // Resume continuation in a handler.
                //
                // This should only appear inside a handler operation body.
                // For now, just type-check the value.
                // TODO: Implement full resume type checking (Phase 2.3)
                let value_expr = self.infer_expr(value)?;
                Ok(hir::Expr::new(
                    hir::ExprKind::Resume {
                        value: Some(Box::new(value_expr)),
                    },
                    Type::unit(), // Actual type depends on handler context
                    expr.span,
                ))
            }
            // More expression kinds - implement as needed
            _ => {
                // Placeholder for unimplemented expression kinds
                Ok(hir::Expr::new(
                    hir::ExprKind::Error,
                    Type::error(),
                    expr.span,
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
                    if let Some(sig) = self.fn_sigs.get(&def_id) {
                        let fn_ty = Type::function(sig.inputs.clone(), sig.output.clone());
                        Ok(hir::Expr::new(
                            hir::ExprKind::Def(def_id),
                            fn_ty,
                            span,
                        ))
                    } else {
                        // Could be a constant or other definition
                        Ok(hir::Expr::new(
                            hir::ExprKind::Def(def_id),
                            Type::error(),
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
            // More pattern kinds - simplified for Phase 1
            _ => hir::PatternKind::Wildcard,
        };

        Ok(hir::Pattern {
            kind,
            ty: expected_ty.clone(),
            span: pattern.span,
        })
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
                // For now, assume size is a literal integer
                // Const evaluation is Phase 2+
                let size_val = match &size.kind {
                    ast::ExprKind::Literal(ast::Literal { kind: ast::LiteralKind::Int { value, .. }, .. }) => {
                        *value as u64
                    }
                    _ => 0, // Placeholder
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
            _ => {
                // Other type kinds - Phase 2+
                Ok(Type::error())
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
                _ => continue,
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
