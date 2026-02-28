//! Expression type inference.
//!
//! This module contains all methods for inferring and checking expression types.

use std::collections::HashMap;

use crate::ast;
use crate::hir::{self, DefId, Type, TypeKind, PrimitiveTy, TyVarId};
use crate::hir::def::{IntTy, UintTy};
use crate::span::{Span, Spanned};

use super::TypeContext;
use super::super::const_eval;
use super::super::error::{TypeError, TypeErrorKind};
use super::super::exhaustiveness;
use super::super::resolve::{Binding, ScopeKind};

/// Handler information collected during try/with inference:
/// (effect_id, op_name, param_types, return_type, ast_handler, effect_type_args).
type HandlerInfo<'b> = Vec<(DefId, String, Vec<Type>, Type, &'b ast::TryWithHandler, Vec<Type>)>;

/// Resolved method information: (def_id, return_type, first_param_type, impl_generics, method_generics).
type MethodLookup = Option<(DefId, Type, Option<Type>, Vec<TyVarId>, Vec<TyVarId>)>;

/// Full method resolution result: (def_id, return_type, first_param_type, impl_generics, method_generics, needs_auto_ref).
type MethodResolution = (DefId, Type, Option<Type>, Vec<TyVarId>, Vec<TyVarId>, bool);

impl<'a> TypeContext<'a> {
    /// Check an expression against an expected type.
    pub(crate) fn check_expr(&mut self, expr: &ast::Expr, expected: &Type) -> Result<hir::Expr, Box<TypeError>> {
        use crate::hir::TypeKind;

        // Special case for literals: use expected type to guide numeric literal inference
        if let ast::ExprKind::Literal(lit) = &expr.kind {
            return self.check_literal(lit, expected, expr.span);
        }

        // Special case for unary negate of a literal (e.g., `-1`): propagate expected
        // type through the negation so unsuffixed integer literals get the right type.
        if let ast::ExprKind::Unary { op: ast::UnaryOp::Neg, operand } = &expr.kind {
            if let ast::ExprKind::Literal(lit) = &operand.kind {
                let inner = self.check_literal(lit, expected, operand.span)?;
                return Ok(hir::Expr::new(
                    hir::ExprKind::Unary { op: ast::UnaryOp::Neg, operand: Box::new(inner) },
                    expected.clone(),
                    expr.span,
                ));
            }
        }

        // Special case for if expressions: propagate expected type to branches
        if let ast::ExprKind::If { condition, then_branch, else_branch } = &expr.kind {
            return self.check_if(condition, then_branch, else_branch.as_ref(), expected, expr.span);
        }

        // Special case for match expressions: propagate expected type to arms
        if let ast::ExprKind::Match { scrutinee, arms } = &expr.kind {
            return self.check_match(scrutinee, arms, expected, expr.span);
        }

        // Special case for block expressions: propagate expected type
        if let ast::ExprKind::Block(block) = &expr.kind {
            return self.check_block(block, expected);
        }

        let inferred = self.infer_expr(expr)?;

        // Unify expected type with inferred - order matters for error messages
        self.unifier.unify(expected, &inferred.ty, expr.span)?;

        // Check if we need to insert an array-to-slice coercion.
        // This happens when expected is &[T] and inferred is &[T; N].
        let expected_resolved = self.unifier.resolve(expected);
        let inferred_resolved = self.unifier.resolve(&inferred.ty);

        if let (
            TypeKind::Ref { inner: expected_inner, mutable: m1 },
            TypeKind::Ref { inner: inferred_inner, mutable: m2 },
        ) = (expected_resolved.kind(), inferred_resolved.kind())
        {
            if m1 == m2 {
                if let (TypeKind::Slice { .. }, TypeKind::Array { size, .. }) =
                    (expected_inner.kind(), inferred_inner.kind())
                {
                    // Need to insert array-to-slice coercion
                    let coerced_ty = expected_resolved.clone();
                    let array_len = size.as_u64().unwrap_or(0);
                    return Ok(hir::Expr::new(
                        hir::ExprKind::ArrayToSlice {
                            expr: Box::new(inferred),
                            array_len,
                        },
                        coerced_ty,
                        expr.span,
                    ));
                }
            }
        }

        Ok(inferred)
    }

    /// Infer the type of an expression.
    pub(crate) fn infer_expr(&mut self, expr: &ast::Expr) -> Result<hir::Expr, Box<TypeError>> {
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
            ast::ExprKind::IfLet { pattern, scrutinee, then_branch, else_branch } => {
                self.infer_if_let(pattern, scrutinee, then_branch, else_branch.as_ref(), expr.span)
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
            ast::ExprKind::Loop { body, label } => {
                self.infer_loop(body, label.as_ref(), expr.span)
            }
            ast::ExprKind::While { condition, body, label } => {
                self.infer_while(condition, body, label.as_ref(), expr.span)
            }
            ast::ExprKind::WhileLet { pattern, scrutinee, body, label } => {
                self.infer_while_let(pattern, scrutinee, body, label.as_ref(), expr.span)
            }
            ast::ExprKind::For { pattern, iter, body, label } => {
                self.infer_for(pattern, iter, body, label.as_ref(), expr.span)
            }
            ast::ExprKind::Break { value, label } => {
                self.infer_break(value.as_deref(), label.as_ref(), expr.span)
            }
            ast::ExprKind::Continue { label } => {
                self.infer_continue(label.as_ref(), expr.span)
            }
            ast::ExprKind::Match { scrutinee, arms } => {
                self.infer_match(scrutinee, arms, expr.span)
            }
            ast::ExprKind::Record { path, fields, base } => {
                self.infer_record(path.as_ref(), fields, base.as_deref(), expr.span)
            }
            ast::ExprKind::Field { base: field_base, field } => {
                // DEF-001: Try to resolve module.item as a qualified path
                if let Some(result) = self.try_resolve_dot_as_qualified_path(field_base, field, expr.span) {
                    return result;
                }
                self.infer_field_access(field_base, field, expr.span)
            }
            ast::ExprKind::Closure { is_move, params, return_type, effects, body } => {
                self.infer_closure(*is_move, params, return_type.as_ref(), effects.as_ref(), body, expr.span)
            }
            ast::ExprKind::WithHandle { handler, body } => {
                self.infer_with_handle(handler, body, expr.span)
            }
            ast::ExprKind::TryWith { body, handlers } => {
                self.infer_try_with(body, handlers, expr.span)
            }
            ast::ExprKind::Perform { effect, operation, args } => {
                self.infer_perform(effect.as_ref(), operation, args, expr.span)
            }
            ast::ExprKind::Resume(value) => {
                self.infer_resume(value, expr.span)
            }
            ast::ExprKind::MethodCall { receiver, method, type_args: _, args } => {
                // DEF-001: Try to resolve module.func(args) as a qualified call
                if let Some(result) = self.try_resolve_dot_call_as_qualified(receiver, &method.node, args, expr.span) {
                    return result;
                }
                self.infer_method_call(receiver, &method.node, args, expr.span)
            }
            ast::ExprKind::Index { base, index } => {
                self.infer_index(base, index, expr.span)
            }
            ast::ExprKind::Array(array_expr) => {
                self.infer_array(array_expr, expr.span)
            }
            ast::ExprKind::Cast { expr: inner, ty } => {
                self.infer_cast(inner, ty, expr.span)
            }
            ast::ExprKind::AssignOp { op, target, value } => {
                self.infer_assign_op(*op, target, value, expr.span)
            }
            ast::ExprKind::Unsafe(block) => {
                self.infer_unsafe(block, expr.span)
            }
            ast::ExprKind::Region { name, body } => {
                let expected = self.unifier.fresh_var();
                let block_expr = self.check_block(body, &expected)?;
                // Unwrap the Block produced by check_block and re-wrap as Region
                let (stmts, tail) = match block_expr.kind {
                    hir::ExprKind::Block { stmts, expr: tail } => (stmts, tail),
                    _ => unreachable!("check_block always produces ExprKind::Block"),
                };
                let name_sym = name.as_ref().map(|n| n.node);
                Ok(hir::Expr {
                    kind: hir::ExprKind::Region {
                        name: name_sym,
                        stmts,
                        expr: tail,
                    },
                    ty: block_expr.ty,
                    span: expr.span,
                })
            }
            ast::ExprKind::Range { start, end, inclusive } => {
                self.infer_range(start.as_deref(), end.as_deref(), *inclusive, expr.span)
            }
            ast::ExprKind::Default => {
                self.infer_default(expr.span)
            }
            ast::ExprKind::MacroCall { path, kind } => {
                self.infer_macro_call(path, kind, expr.span)
            }
            ast::ExprKind::InlineHandle { body, effect, operations } => {
                self.infer_inline_handle(body, effect, operations, expr.span)
            }
            ast::ExprKind::InlineWithDo { effect, operations, body } => {
                // InlineWithDo is semantically equivalent to InlineHandle, just different syntax
                self.infer_inline_handle(body, effect, operations, expr.span)
            }
        }
    }

    /// Infer type for `default` expression.
    /// The type is inferred from context (where the value is used).
    fn infer_default(&mut self, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        // Create a fresh type variable - the type will be inferred from usage context
        let ty = self.unifier.fresh_var();
        Ok(hir::Expr {
            kind: hir::ExprKind::Default,
            ty,
            span,
        })
    }

    /// Infer type of a with...handle expression.
    fn infer_with_handle(&mut self, handler: &ast::Expr, body: &ast::Expr, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        // Type-check the handler expression first
        let handler_expr = self.infer_expr(handler)?;

        // Extract handler def_id and effect info from the type (handlers are ADTs)
        let handled_effect_info = match handler_expr.ty.kind() {
            hir::TypeKind::Adt { def_id: handler_def_id, args } => {
                if let Some(handler_info) = self.handler_defs.get(handler_def_id) {
                    // Get the effect type args by substituting the handler's resolved type args
                    // into the effect's type parameter positions.
                    //
                    // Example: handler LocalState<S> for State<S>
                    // - handler_info.generics = [TyVarId for S]
                    // - handler_info.effect_type_args = [Type::Param(S)]
                    // - args (from instantiation) = [i32]  (resolved from LocalState { state: 0 })
                    // - We need to substitute S -> i32 to get effect_type_args = [i32]

                    let resolved_args: Vec<Type> = args.iter()
                        .map(|ty| self.unifier.resolve(ty))
                        .collect();

                    // Create substitution from handler's generic params to resolved args
                    let handler_subst: std::collections::HashMap<TyVarId, Type> =
                        handler_info.generics.iter()
                            .zip(resolved_args.iter())
                            .map(|(&ty_var, ty)| (ty_var, ty.clone()))
                            .collect();

                    // Substitute to get concrete effect type args
                    let effect_type_args: Vec<Type> = handler_info.effect_type_args.iter()
                        .map(|ty| self.substitute_type_vars(ty, &handler_subst))
                        .collect();

                    Some((handler_info.effect_id, effect_type_args))
                } else {
                    None
                }
            }
            _ => None,
        };

        // Push the handled effect with its type args onto the stack
        if let Some((effect_id, effect_type_args)) = &handled_effect_info {
            self.handled_effects.push((*effect_id, effect_type_args.clone()));
        }

        let body_block = match &body.kind {
            ast::ExprKind::Block(block) => block,
            _ => {
                // Pop effect if we pushed it
                if handled_effect_info.is_some() {
                    self.handled_effects.pop();
                }
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "Handle body must be a block".into(),
                    },
                    body.span,
                )));
            }
        };

        // Push a handler scope and register effect operations
        self.resolver.push_scope(ScopeKind::Handler, body.span);

        // Register the handled effect's operations in this scope
        if let Some((effect_id, _)) = &handled_effect_info {
            if let Some(effect_info) = self.effect_defs.get(effect_id).cloned() {
                for op_info in &effect_info.operations {
                    self.resolver.current_scope_mut()
                        .bindings
                        .insert(op_info.name.clone(), Binding::Def(op_info.def_id));
                }
            }
        }

        // Type-check the block
        let expected = self.unifier.fresh_var();
        let result = self.check_block(body_block, &expected);

        // Pop the handler scope
        self.resolver.pop_scope();

        // Pop the handled effect
        if handled_effect_info.is_some() {
            self.handled_effects.pop();
        }

        // Extract handler_id from handler expression type
        let handler_id = match handler_expr.ty.kind() {
            hir::TypeKind::Adt { def_id, .. } => *def_id,
            _ => {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "Handler must be an ADT type".into(),
                    },
                    span,
                )));
            }
        };

        // Wrap the body in a Handle expression
        let body_expr = result?;

        // Unify handler's stored inference variables with concrete types from this handle site.
        // The handler body was type-checked in isolation; now we connect the inference variables
        // created there with the actual types from this use site.
        if let Some(handler_info) = self.handler_defs.get(&handler_id) {
            let body_ty = self.unifier.resolve(&expected);

            // continuation_result_ty: what resume() returns in deep handlers.
            // At the handle site, this must equal the body's result type.
            if let Some(ref cont_ty) = handler_info.continuation_result_ty {
                self.unifier.unify(cont_ty, &body_ty, span)?;
            }

            // return_clause_input_ty: what the handled computation produces.
            // At the handle site, this must equal the body's result type.
            if let Some(ref input_ty) = handler_info.return_clause_input_ty {
                self.unifier.unify(input_ty, &body_ty, span)?;
            }

            // return_clause_output_ty: what the return clause body produces.
            // At the handle site, this must equal the overall handle expression's type.
            if let Some(ref output_ty) = handler_info.return_clause_output_ty {
                self.unifier.unify(output_ty, &body_ty, span)?;
            }
        }

        // Resolve the expected type to get the concrete type (not an inference variable)
        let resolved_ty = self.unifier.resolve(&expected);
        Ok(hir::Expr {
            kind: hir::ExprKind::Handle {
                body: Box::new(body_expr),
                handler_id,
                handler_instance: Box::new(handler_expr),
            },
            ty: resolved_ty,
            span,
        })
    }

    /// Infer type of a try-with inline handler expression.
    ///
    /// `try { body } with { Effect::op(x) => { handler_body } }`
    fn infer_try_with(
        &mut self,
        body: &ast::Block,
        handlers: &[ast::TryWithHandler],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Collect effect info for each handler clause
        // (effect_id, op_name, param_types, return_type, ast_handler, effect_type_args)
        let mut handler_infos: HandlerInfo<'_> = Vec::new();

        for handler in handlers {
            // Resolve the effect type path
            let effect_name = if let Some(first_seg) = handler.effect.segments.first() {
                self.symbol_to_string(first_seg.name.node)
            } else {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "Effect path must have at least one segment".into(),
                    },
                    handler.span,
                )));
            };

            // Look up the effect definition first so we know how many generics it has
            let effect_id = self.effect_defs.iter()
                .find(|(_, info)| info.name == effect_name)
                .map(|(def_id, _)| *def_id);

            let effect_id = match effect_id {
                Some(id) => id,
                None => {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotAnEffect { name: effect_name },
                        handler.span,
                    )));
                }
            };

            let effect_info = match self.effect_defs.get(&effect_id).cloned() {
                Some(info) => info,
                None => {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotAnEffect { name: effect_name },
                        handler.span,
                    )));
                }
            };

            // Extract type arguments from the effect, or create fresh inference variables
            // if none provided and the effect has generic parameters
            let effect_type_args: Vec<Type> = if let Some(first_seg) = handler.effect.segments.first() {
                if let Some(ref args) = first_seg.args {
                    let explicit_args: Vec<Type> = args.args.iter()
                        .filter_map(|arg| {
                            if let ast::TypeArg::Type(ty) = arg {
                                self.ast_type_to_hir_type(ty).ok()
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !explicit_args.is_empty() {
                        explicit_args
                    } else if !effect_info.generics.is_empty() {
                        // No explicit args but effect has generics - create fresh inference vars
                        effect_info.generics.iter()
                            .map(|_| self.unifier.fresh_var())
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else if !effect_info.generics.is_empty() {
                    // No type args syntax but effect has generics - create fresh inference vars
                    effect_info.generics.iter()
                        .map(|_| self.unifier.fresh_var())
                        .collect()
                } else {
                    Vec::new()
                }
            } else if !effect_info.generics.is_empty() {
                // No first segment but effect has generics - create fresh inference vars
                effect_info.generics.iter()
                    .map(|_| self.unifier.fresh_var())
                    .collect()
            } else {
                Vec::new()
            };

            // Look up the operation in this effect
            let op_name = self.symbol_to_string(handler.operation.node);

            let op_info = match effect_info.operations.iter().find(|op| op.name == op_name) {
                Some(info) => info.clone(),
                None => {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: format!("Unknown operation `{}` on effect `{}`", op_name, effect_name),
                        },
                        handler.operation.span,
                    )));
                }
            };

            // Substitute type parameters in the operation's parameter types
            // Always use effect_type_args (which may be fresh inference vars)
            let param_types: Vec<Type> = op_info.params.iter()
                .map(|ty| {
                    if !effect_type_args.is_empty() {
                        self.substitute_effect_type_args(ty, &effect_info.generics, &effect_type_args)
                    } else {
                        ty.clone()
                    }
                })
                .collect();

            let return_type = if !effect_type_args.is_empty() {
                self.substitute_effect_type_args(&op_info.return_ty, &effect_info.generics, &effect_type_args)
            } else {
                op_info.return_ty.clone()
            };

            handler_infos.push((effect_id, op_name, param_types, return_type, handler, effect_type_args.clone()));
        }

        // Push all handled effects onto the stack (using the effect_type_args we computed/inferred)
        for (effect_id, _, _, _, _, effect_type_args) in &handler_infos {
            self.handled_effects.push((*effect_id, effect_type_args.clone()));
        }

        // Push a handler scope
        self.resolver.push_scope(ScopeKind::Handler, span);

        // Type-check the body
        let expected = self.unifier.fresh_var();
        let body_result = self.check_block(body, &expected);

        // Pop handler scope
        self.resolver.pop_scope();

        // Pop handled effects
        for _ in &handler_infos {
            self.handled_effects.pop();
        }

        let body_expr = body_result?;

        // Type-check each handler's body and build HIR handlers
        let mut hir_handlers = Vec::new();

        for (effect_id, op_name, param_types, return_type, handler, _effect_type_args) in handler_infos {
            // Create a handler scope for the handler clause (must be Handler, not Block, so resume is valid)
            self.resolver.push_scope(ScopeKind::Handler, handler.span);

            // Bind the operation parameters
            let mut param_locals = Vec::new();
            let mut param_type_vec = Vec::new();

            // Check parameter count matches
            if handler.params.len() != param_types.len() {
                self.resolver.pop_scope();
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::WrongArity {
                        expected: param_types.len(),
                        found: handler.params.len(),
                    },
                    handler.span,
                )));
            }

            for (idx, (pattern, param_ty)) in handler.params.iter().zip(param_types.iter()).enumerate() {
                // Support identifier, wildcard, tuple, and parenthesized patterns
                match &pattern.kind {
                    ast::PatternKind::Ident { name, .. } => {
                        let local_name = self.symbol_to_string(name.node);
                        let local_id = self.resolver.next_local_id();
                        self.locals.push(hir::Local {
                            id: local_id,
                            name: Some(local_name.clone()),
                            ty: param_ty.clone(),
                            mutable: false,
                            span: pattern.span,
                        });
                        self.resolver.current_scope_mut()
                            .bindings
                            .insert(local_name.clone(), Binding::Local {
                                local_id,
                                ty: param_ty.clone(),
                                mutable: false,
                                span: pattern.span,
                            });
                        param_locals.push(local_id);
                        param_type_vec.push(param_ty.clone());
                    }
                    ast::PatternKind::Wildcard => {
                        let local_id = self.resolver.next_local_id();
                        self.locals.push(hir::Local {
                            id: local_id,
                            name: Some(format!("_param_{}", idx)),
                            ty: param_ty.clone(),
                            mutable: false,
                            span: pattern.span,
                        });
                        param_locals.push(local_id);
                        param_type_vec.push(param_ty.clone());
                    }
                    ast::PatternKind::Tuple { fields, .. } => {
                        // Tuple destructuring pattern in handler parameter
                        let elem_types = match param_ty.kind() {
                            hir::TypeKind::Tuple(elems) => elems.clone(),
                            _ => {
                                self.resolver.pop_scope();
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::NotATuple { ty: param_ty.clone() },
                                    pattern.span,
                                )));
                            }
                        };

                        if fields.len() != elem_types.len() {
                            self.resolver.pop_scope();
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::WrongArity {
                                    expected: elem_types.len(),
                                    found: fields.len(),
                                },
                                pattern.span,
                            )));
                        }

                        // Define each element of the tuple pattern
                        for (field_pat, elem_ty) in fields.iter().zip(elem_types.iter()) {
                            self.define_pattern(field_pat, elem_ty.clone())?;
                        }

                        // Create a hidden tuple local for the parameter
                        let hidden_name = format!("__handler_param_{}_{}", idx, pattern.span.start);
                        let tuple_local_id = self.resolver.next_local_id();
                        self.locals.push(hir::Local {
                            id: tuple_local_id,
                            name: Some(hidden_name),
                            ty: param_ty.clone(),
                            mutable: false,
                            span: pattern.span,
                        });

                        // Record tuple destructuring for MIR lowering
                        let element_locals: Vec<_> = (0..fields.len())
                            .map(|i| hir::LocalId::new(tuple_local_id.index - fields.len() as u32 + i as u32))
                            .collect();
                        self.tuple_destructures.insert(tuple_local_id, element_locals);

                        param_locals.push(tuple_local_id);
                        param_type_vec.push(param_ty.clone());
                    }
                    ast::PatternKind::Paren(inner) => {
                        // Unwrap parenthesized pattern and process the inner pattern
                        // We need to handle this recursively, but since we're in a loop
                        // processing individual params, we handle paren specially here
                        match &inner.kind {
                            ast::PatternKind::Ident { name, .. } => {
                                let local_name = self.symbol_to_string(name.node);
                                let local_id = self.resolver.next_local_id();
                                self.locals.push(hir::Local {
                                    id: local_id,
                                    name: Some(local_name.clone()),
                                    ty: param_ty.clone(),
                                    mutable: false,
                                    span: inner.span,
                                });
                                self.resolver.current_scope_mut()
                                    .bindings
                                    .insert(local_name.clone(), Binding::Local {
                                        local_id,
                                        ty: param_ty.clone(),
                                        mutable: false,
                                        span: inner.span,
                                    });
                                param_locals.push(local_id);
                                param_type_vec.push(param_ty.clone());
                            }
                            ast::PatternKind::Wildcard => {
                                let local_id = self.resolver.next_local_id();
                                self.locals.push(hir::Local {
                                    id: local_id,
                                    name: Some(format!("_param_{}", idx)),
                                    ty: param_ty.clone(),
                                    mutable: false,
                                    span: inner.span,
                                });
                                param_locals.push(local_id);
                                param_type_vec.push(param_ty.clone());
                            }
                            _ => {
                                // For other patterns inside parens, use define_pattern
                                let local_id = self.define_pattern(inner, param_ty.clone())?;
                                param_locals.push(local_id);
                                param_type_vec.push(param_ty.clone());
                            }
                        }
                    }
                    _ => {
                        self.resolver.pop_scope();
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: format!(
                                    "pattern kind {:?} in inline handler parameters \
                                     (only identifiers, wildcards, and tuples are supported)",
                                    std::mem::discriminant(&pattern.kind)
                                ),
                            },
                            pattern.span,
                        )));
                    }
                }
            }

            // Set up resume type and resume result type for this handler
            let prev_resume_type = self.current_resume_type.take();
            let prev_resume_result = self.current_resume_result_type.take();
            self.current_resume_type = Some(return_type.clone());
            self.current_resume_result_type = Some(expected.clone());

            // Type-check the handler body (should return unit)
            let handler_body_result = self.check_block(&handler.body, &Type::unit());

            // Restore resume type and resume result type
            self.current_resume_type = prev_resume_type;
            self.current_resume_result_type = prev_resume_result;

            // Pop handler clause scope
            self.resolver.pop_scope();

            let handler_body = handler_body_result?;

            hir_handlers.push(hir::InlineOpHandler {
                effect_id,
                op_name,
                params: param_locals,
                param_types: param_type_vec,
                return_type,
                body: handler_body,
            });
        }

        let resolved_ty = self.unifier.resolve(&expected);

        Ok(hir::Expr {
            kind: hir::ExprKind::InlineHandle {
                body: Box::new(body_expr),
                handlers: hir_handlers,
            },
            ty: resolved_ty,
            span,
        })
    }

    /// Infer type of an inline handle expression.
    ///
    /// Two syntaxes are supported:
    /// - `handle { body } with Effect { op name(params) -> T { handler_body } }`
    /// - `with Effect { op name(params) -> T { handler_body } } do { body }`
    fn infer_inline_handle(
        &mut self,
        body: &ast::Expr,
        effect_path: &ast::TypePath,
        operations: &[ast::InlineHandlerOp],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Resolve the effect from the path
        let effect_name = if let Some(first_seg) = effect_path.segments.first() {
            self.symbol_to_string(first_seg.name.node)
        } else {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::UnsupportedFeature {
                    feature: "Effect path must have at least one segment".into(),
                },
                span,
            )));
        };

        // Look up the effect definition
        let effect_id = self.effect_defs.iter()
            .find(|(_, info)| info.name == effect_name)
            .map(|(def_id, _)| *def_id);

        let effect_id = match effect_id {
            Some(id) => id,
            None => {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::NotAnEffect { name: effect_name },
                    span,
                )));
            }
        };

        let effect_info = match self.effect_defs.get(&effect_id).cloned() {
            Some(info) => info,
            None => {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::NotAnEffect { name: effect_name },
                    span,
                )));
            }
        };

        // Extract type arguments from the effect, or create fresh inference variables
        let effect_type_args: Vec<Type> = if let Some(first_seg) = effect_path.segments.first() {
            if let Some(ref args) = first_seg.args {
                let explicit_args: Vec<Type> = args.args.iter()
                    .filter_map(|arg| {
                        if let ast::TypeArg::Type(ty) = arg {
                            self.ast_type_to_hir_type(ty).ok()
                        } else {
                            None
                        }
                    })
                    .collect();
                if !explicit_args.is_empty() {
                    explicit_args
                } else if !effect_info.generics.is_empty() {
                    effect_info.generics.iter()
                        .map(|_| self.unifier.fresh_var())
                        .collect()
                } else {
                    Vec::new()
                }
            } else if !effect_info.generics.is_empty() {
                effect_info.generics.iter()
                    .map(|_| self.unifier.fresh_var())
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Push the handled effect onto the stack
        self.handled_effects.push((effect_id, effect_type_args.clone()));

        // Record the current local ID counter before entering the handler scope.
        // Any local with ID >= this value was defined inside the handler.
        // Locals with ID < this value are "captured" from outer scopes.
        let outer_local_boundary = self.resolver.current_local_id_counter();

        // Push a handler scope for the body
        self.resolver.push_scope(ScopeKind::Handler, span);

        // Register the handled effect's operations in this scope
        for op_info in &effect_info.operations {
            self.resolver.current_scope_mut()
                .bindings
                .insert(op_info.name.clone(), Binding::Def(op_info.def_id));
        }

        // Type-check the body
        let expected = self.unifier.fresh_var();
        let body_block = match &body.kind {
            ast::ExprKind::Block(block) => block,
            _ => {
                self.resolver.pop_scope();
                self.handled_effects.pop();
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "Handle body must be a block".into(),
                    },
                    body.span,
                )));
            }
        };
        let body_result = self.check_block(body_block, &expected);

        // Pop handler scope
        self.resolver.pop_scope();

        // Pop handled effect
        self.handled_effects.pop();

        let body_expr = body_result?;

        // Check for captured linear/affine variables in the handler body.
        // Inline handlers are deep (multi-shot) by default, meaning the continuation
        // can be resumed multiple times. Linear/affine values cannot be safely
        // captured because they could be used multiple times across resumptions.
        self.check_captured_linearity(&body_expr, outer_local_boundary, &effect_name, span)?;

        // Type-check each inline operation handler
        let mut hir_handlers = Vec::new();

        for op in operations {
            let op_name_str = self.symbol_to_string(op.name.node);

            // Find this operation in the effect
            let op_info = match effect_info.operations.iter().find(|o| o.name == op_name_str) {
                Some(info) => info.clone(),
                None => {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: format!("Unknown operation `{}` on effect `{}`", op_name_str, effect_name),
                        },
                        op.name.span,
                    )));
                }
            };

            // Substitute type parameters in the operation's types
            let param_types: Vec<Type> = op_info.params.iter()
                .map(|ty| {
                    if !effect_type_args.is_empty() {
                        self.substitute_effect_type_args(ty, &effect_info.generics, &effect_type_args)
                    } else {
                        ty.clone()
                    }
                })
                .collect();

            // The effect operation's return type (used for resume's parameter)
            let effect_op_return_type = if !effect_type_args.is_empty() {
                self.substitute_effect_type_args(&op_info.return_ty, &effect_info.generics, &effect_type_args)
            } else {
                op_info.return_ty.clone()
            };

            // The inline handler can declare an explicit return type for documentation/checking
            // but this doesn't override the effect operation's return type for resume purposes
            let _handler_declared_return_type = if let Some(ref ret_ty) = op.return_type {
                Some(self.ast_type_to_hir_type(ret_ty)?)
            } else {
                None
            };

            // Create scope for the handler
            self.resolver.push_scope(ScopeKind::Handler, op.span);

            // Bind operation parameters
            let mut param_locals = Vec::new();
            let mut param_type_vec = Vec::new();

            for (idx, pattern) in op.params.iter().enumerate() {
                match &pattern.kind {
                    ast::PatternKind::Ident { name, .. } => {
                        let local_name = self.symbol_to_string(name.node);

                        // Check if this is the `resume` parameter
                        if local_name == "resume" {
                            // `resume` is special - it's a continuation, not a regular parameter
                            // We bind it in scope for resolution but don't add to param_locals
                            // The resume type is based on the effect operation's return type
                            let resume_id = self.resolver.next_local_id();
                            let resume_ty = Type::new(hir::TypeKind::Fn {
                                params: vec![effect_op_return_type.clone()],
                                ret: expected.clone(),
                                effects: Vec::new(),
                                const_args: Vec::new(),
                            });
                            self.locals.push(hir::Local {
                                id: resume_id,
                                name: Some(local_name.clone()),
                                ty: resume_ty.clone(),
                                mutable: false,
                                span: pattern.span,
                            });
                            self.resolver.current_scope_mut()
                                .bindings
                                .insert(local_name, Binding::Local {
                                    local_id: resume_id,
                                    ty: resume_ty,
                                    mutable: false,
                                    span: pattern.span,
                                });
                        } else {
                            // Regular parameter
                            let param_ty = param_types.get(idx).cloned()
                                .unwrap_or_else(|| self.unifier.fresh_var());
                            let local_id = self.resolver.next_local_id();
                            self.locals.push(hir::Local {
                                id: local_id,
                                name: Some(local_name.clone()),
                                ty: param_ty.clone(),
                                mutable: false,
                                span: pattern.span,
                            });
                            self.resolver.current_scope_mut()
                                .bindings
                                .insert(local_name, Binding::Local {
                                    local_id,
                                    ty: param_ty.clone(),
                                    mutable: false,
                                    span: pattern.span,
                                });
                            param_locals.push(local_id);
                            param_type_vec.push(param_ty);
                        }
                    }
                    ast::PatternKind::Wildcard => {
                        let param_ty = param_types.get(idx).cloned()
                            .unwrap_or_else(|| self.unifier.fresh_var());
                        let local_id = self.resolver.next_local_id();
                        self.locals.push(hir::Local {
                            id: local_id,
                            name: Some(format!("_param_{}", idx)),
                            ty: param_ty.clone(),
                            mutable: false,
                            span: pattern.span,
                        });
                        param_locals.push(local_id);
                        param_type_vec.push(param_ty);
                    }
                    _ => {
                        self.resolver.pop_scope();
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: format!(
                                    "pattern kind {:?} in inline handler parameters",
                                    std::mem::discriminant(&pattern.kind)
                                ),
                            },
                            pattern.span,
                        )));
                    }
                }
            }

            // Set up resume type for this handler (from effect operation's return type)
            let prev_resume_type = self.current_resume_type.take();
            self.current_resume_type = Some(effect_op_return_type.clone());

            // Type-check the handler body
            let handler_body_result = self.check_block(&op.body, &expected);

            // Restore resume type
            self.current_resume_type = prev_resume_type;

            // Pop handler scope
            self.resolver.pop_scope();

            let handler_body = handler_body_result?;

            hir_handlers.push(hir::InlineOpHandler {
                effect_id,
                op_name: op_name_str,
                params: param_locals,
                param_types: param_type_vec,
                return_type: effect_op_return_type,
                body: handler_body,
            });
        }

        let resolved_ty = self.unifier.resolve(&expected);

        Ok(hir::Expr {
            kind: hir::ExprKind::InlineHandle {
                body: Box::new(body_expr),
                handlers: hir_handlers,
            },
            ty: resolved_ty,
            span,
        })
    }

    /// Substitute effect type parameters with concrete types.
    fn substitute_effect_type_args(&self, ty: &Type, type_params: &[TyVarId], type_args: &[Type]) -> Type {
        let subst: std::collections::HashMap<TyVarId, Type> = type_params.iter()
            .zip(type_args.iter())
            .map(|(&var, ty)| (var, ty.clone()))
            .collect();
        self.substitute_type_vars(ty, &subst)
    }

    /// Check for captured linear/affine variables in a handler body.
    ///
    /// Deep (multi-shot) handlers can resume multiple times, which means any
    /// captured linear or affine values would be used multiple times, violating
    /// their ownership semantics.
    ///
    /// # Arguments
    /// * `body` - The HIR expression for the handler body
    /// * `outer_local_boundary` - Local IDs below this are "captured" from outer scopes
    /// * `effect_name` - Name of the effect (for error messages)
    /// * `span` - Span for error reporting
    fn check_captured_linearity(
        &self,
        body: &hir::Expr,
        outer_local_boundary: u32,
        effect_name: &str,
        _span: Span,
    ) -> Result<(), Box<TypeError>> {
        // Collect all local variable references in the body
        let mut captured_locals = Vec::new();
        self.collect_local_refs(body, &mut captured_locals);

        // Check each captured local
        for (local_id, local_ty, local_span) in captured_locals {
            // Only check locals that were defined before the handler (captured)
            if local_id.index() < outer_local_boundary {
                let resolved_ty = self.unifier.resolve(&local_ty);

                if self.is_type_linear(&resolved_ty) {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::LinearValueInMultiShotHandler {
                            operation: effect_name.to_string(),
                            field_name: format!("captured variable (local {})", local_id.index()),
                            field_type: resolved_ty.to_string(),
                        },
                        local_span,
                    ).with_help(
                        "linear values must be used exactly once, but deep handlers can \
                         resume multiple times. Consider restructuring to avoid capturing \
                         linear values, or use a shallow handler."
                    )));
                }

                if self.is_type_affine(&resolved_ty) {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::AffineValueInMultiShotHandler {
                            operation: effect_name.to_string(),
                            field_name: format!("captured variable (local {})", local_id.index()),
                            field_type: resolved_ty.to_string(),
                        },
                        local_span,
                    ).with_help(
                        "affine values may be used at most once, but deep handlers can \
                         resume multiple times. Consider restructuring to avoid capturing \
                         affine values, or use a shallow handler."
                    )));
                }
            }
        }

        Ok(())
    }

    /// Collect all local variable references in an HIR expression.
    /// Returns a list of (LocalId, Type, Span) for each reference.
    fn collect_local_refs(&self, expr: &hir::Expr, locals: &mut Vec<(hir::LocalId, Type, Span)>) {
        use hir::ExprKind;

        match &expr.kind {
            ExprKind::Local(local_id) => {
                locals.push((*local_id, expr.ty.clone(), expr.span));
            }

            // Compound expressions - recurse into children
            ExprKind::Block { stmts, expr: tail } => {
                for stmt in stmts {
                    match stmt {
                        hir::Stmt::Let { init: Some(init), .. } => self.collect_local_refs(init, locals),
                        hir::Stmt::Let { init: None, .. } => {}
                        hir::Stmt::Expr(e) => self.collect_local_refs(e, locals),
                        hir::Stmt::Item(_) => {}
                    }
                }
                if let Some(tail) = tail {
                    self.collect_local_refs(tail, locals);
                }
            }

            ExprKind::If { condition, then_branch, else_branch } => {
                self.collect_local_refs(condition, locals);
                self.collect_local_refs(then_branch, locals);
                if let Some(else_br) = else_branch {
                    self.collect_local_refs(else_br, locals);
                }
            }

            ExprKind::Match { scrutinee, arms } => {
                self.collect_local_refs(scrutinee, locals);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.collect_local_refs(guard, locals);
                    }
                    self.collect_local_refs(&arm.body, locals);
                }
            }

            ExprKind::Loop { body, .. } => {
                self.collect_local_refs(body, locals);
            }

            ExprKind::While { condition, body, .. } => {
                self.collect_local_refs(condition, locals);
                self.collect_local_refs(body, locals);
            }

            ExprKind::Call { callee, args } => {
                self.collect_local_refs(callee, locals);
                for arg in args {
                    self.collect_local_refs(arg, locals);
                }
            }

            ExprKind::MethodCall { receiver, args, .. } => {
                self.collect_local_refs(receiver, locals);
                for arg in args {
                    self.collect_local_refs(arg, locals);
                }
            }

            ExprKind::Binary { left, right, .. } => {
                self.collect_local_refs(left, locals);
                self.collect_local_refs(right, locals);
            }

            ExprKind::Unary { operand, .. } => {
                self.collect_local_refs(operand, locals);
            }

            ExprKind::Borrow { expr: inner, .. }
            | ExprKind::AddrOf { expr: inner, .. }
            | ExprKind::Deref(inner) => {
                self.collect_local_refs(inner, locals);
            }

            ExprKind::Field { base, .. } => {
                self.collect_local_refs(base, locals);
            }

            ExprKind::Index { base, index } => {
                self.collect_local_refs(base, locals);
                self.collect_local_refs(index, locals);
            }

            ExprKind::Cast { expr: inner, .. } => {
                self.collect_local_refs(inner, locals);
            }

            ExprKind::Tuple(exprs) | ExprKind::Array(exprs) | ExprKind::VecLiteral(exprs) => {
                for e in exprs {
                    self.collect_local_refs(e, locals);
                }
            }

            ExprKind::Repeat { value, .. } => {
                self.collect_local_refs(value, locals);
            }

            ExprKind::VecRepeat { value, count } => {
                self.collect_local_refs(value, locals);
                self.collect_local_refs(count, locals);
            }

            ExprKind::Struct { fields, base, .. } => {
                for field_expr in fields {
                    self.collect_local_refs(&field_expr.value, locals);
                }
                if let Some(b) = base {
                    self.collect_local_refs(b, locals);
                }
            }

            ExprKind::Record { fields } => {
                for field in fields {
                    self.collect_local_refs(&field.value, locals);
                }
            }

            ExprKind::Variant { fields, .. } => {
                for e in fields {
                    self.collect_local_refs(e, locals);
                }
            }

            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.collect_local_refs(s, locals);
                }
                if let Some(e) = end {
                    self.collect_local_refs(e, locals);
                }
            }

            ExprKind::Return(inner) => {
                if let Some(e) = inner {
                    self.collect_local_refs(e, locals);
                }
            }

            ExprKind::Break { value, .. } => {
                if let Some(v) = value {
                    self.collect_local_refs(v, locals);
                }
            }

            ExprKind::Assign { target, value } => {
                self.collect_local_refs(target, locals);
                self.collect_local_refs(value, locals);
            }

            ExprKind::Let { init, .. } => {
                self.collect_local_refs(init, locals);
            }

            // Closures capture variables - but we can't inspect the body directly
            // since it's stored separately via body_id. For now, skip closure bodies.
            ExprKind::Closure { .. } => {}

            ExprKind::Perform { args, .. } => {
                for arg in args {
                    self.collect_local_refs(arg, locals);
                }
            }

            ExprKind::Resume { value } => {
                if let Some(v) = value {
                    self.collect_local_refs(v, locals);
                }
            }

            ExprKind::Handle { body, handler_instance, .. } => {
                self.collect_local_refs(body, locals);
                self.collect_local_refs(handler_instance, locals);
            }

            ExprKind::InlineHandle { body, handlers } => {
                self.collect_local_refs(body, locals);
                for handler in handlers {
                    self.collect_local_refs(&handler.body, locals);
                }
            }

            ExprKind::Region { stmts, expr, .. } => {
                for stmt in stmts {
                    match stmt {
                        hir::Stmt::Let { init: Some(init), .. } => self.collect_local_refs(init, locals),
                        hir::Stmt::Let { init: None, .. } => {}
                        hir::Stmt::Expr(e) => self.collect_local_refs(e, locals),
                        hir::Stmt::Item(_) => {}
                    }
                }
                if let Some(e) = expr {
                    self.collect_local_refs(e, locals);
                }
            }

            ExprKind::Unsafe(inner) | ExprKind::Dbg(inner) => {
                self.collect_local_refs(inner, locals);
            }

            ExprKind::Assert { condition, message } => {
                self.collect_local_refs(condition, locals);
                if let Some(msg) = message {
                    self.collect_local_refs(msg, locals);
                }
            }

            ExprKind::MacroExpansion { args, named_args, .. } => {
                for arg in args {
                    self.collect_local_refs(arg, locals);
                }
                for (_, arg) in named_args {
                    self.collect_local_refs(arg, locals);
                }
            }

            ExprKind::SliceLen(inner) | ExprKind::VecLen(inner) => {
                self.collect_local_refs(inner, locals);
            }

            ExprKind::ArrayToSlice { expr: inner, .. } => {
                self.collect_local_refs(inner, locals);
            }

            // Leaf expressions - no recursion needed
            ExprKind::Literal(_)
            | ExprKind::Def(_)
            | ExprKind::ConstParam(_)
            | ExprKind::Continue { .. }
            | ExprKind::Default
            | ExprKind::MethodFamily { .. }
            | ExprKind::Error => {}
        }
    }

    /// Infer type of a perform expression.
    fn infer_perform(
        &mut self,
        effect: Option<&ast::TypePath>,
        operation: &crate::span::Spanned<ast::Symbol>,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        use crate::ice;

        let op_name = self.symbol_to_string(operation.node);

        // Try to find the operation - either from explicit effect path or from scope
        let (effect_id, op_index, op_def_id, type_args) = if let Some(effect_path) = effect {
            // Explicit effect specified: `perform Effect<T>.op(args)`
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
                    // If no explicit type args were provided, try to get them from:
                    // 1. Enclosing with...handle blocks (handled_effects stack)
                    // 2. Current function's effect declaration (fn_effects)
                    let type_args = if type_args.is_empty() {
                        // First try handled_effects (from enclosing with...handle blocks)
                        let from_handled = self.handled_effects.iter().rev()
                            .find(|(effect_id, _)| *effect_id == eff_id)
                            .map(|(_, args)| args.clone());

                        if let Some(args) = from_handled {
                            args
                        } else if let Some(fn_id) = self.current_fn {
                            // Fall back to function's effect declaration
                            if let Some(fn_effects) = self.fn_effects.get(&fn_id) {
                                fn_effects.iter()
                                    .find(|er| er.def_id == eff_id)
                                    .map(|er| er.type_args.clone())
                                    .unwrap_or_default()
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        }
                    } else {
                        type_args
                    };

                    let effect_info = self.effect_defs.get(&eff_id).cloned();
                    match effect_info {
                        Some(info) => {
                            // Find the operation by name
                            let op_result = info.operations.iter().enumerate()
                                .find(|(_, op)| op.name == op_name);
                            match op_result {
                                Some((idx, op)) => (eff_id, idx as u32, op.def_id, type_args),
                                None => {
                                    self.errors.push(TypeError::new(
                                        TypeErrorKind::NotFound { name: format!("{}.{}", effect_name, op_name) },
                                        operation.span,
                                    ));
                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Error,
                                        Type::error(),
                                        span,
                                    ));
                                }
                            }
                        }
                        None => {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::TypeNotFound { name: effect_name },
                                span,
                            ));
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Error,
                                Type::error(),
                                span,
                            ));
                        }
                    }
                }
                None => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeNotFound { name: effect_name },
                        span,
                    ));
                    return Ok(hir::Expr::new(
                        hir::ExprKind::Error,
                        Type::error(),
                        span,
                    ));
                }
            }
        } else {
            // No explicit effect - look up operation in scope
            let binding = self.resolver.lookup(&op_name);
            match binding {
                Some(Binding::Def(op_def_id)) => {
                    // Found the operation - now find its parent effect
                    let def_info = self.resolver.def_info.get(&op_def_id);
                    match def_info.and_then(|info| info.parent) {
                        Some(effect_def_id) => {
                            // Find operation index in the effect
                            let effect_info = self.effect_defs.get(&effect_def_id);

                            // Get type args - first from handled_effects, then from fn_effects
                            let from_handled = self.handled_effects.iter().rev()
                                .find(|(effect_id, _)| *effect_id == effect_def_id)
                                .map(|(_, args)| args.clone());

                            let type_args = if let Some(args) = from_handled {
                                args
                            } else if let Some(fn_id) = self.current_fn {
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
                                            self.errors.push(TypeError::new(
                                                TypeErrorKind::NotFound { name: op_name },
                                                operation.span,
                                            ));
                                            return Ok(hir::Expr::new(
                                                hir::ExprKind::Error,
                                                Type::error(),
                                                span,
                                            ));
                                        }
                                    }
                                }
                                None => {
                                    self.errors.push(TypeError::new(
                                        TypeErrorKind::NotFound { name: op_name },
                                        operation.span,
                                    ));
                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Error,
                                        Type::error(),
                                        span,
                                    ));
                                }
                            }
                        }
                        None => {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::NotFound { name: op_name },
                                operation.span,
                            ));
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Error,
                                Type::error(),
                                span,
                            ));
                        }
                    }
                }
                _ => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::NotFound { name: op_name },
                        operation.span,
                    ));
                    return Ok(hir::Expr::new(
                        hir::ExprKind::Error,
                        Type::error(),
                        span,
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
                        ice!("operation signature not found during type checking";
                             "effect_id" => effect_id,
                             "op_index" => op_index,
                             "note" => "effect resolution succeeded but operation lookup failed");
                        return Ok(hir::Expr::new(
                            hir::ExprKind::Error,
                            Type::error(),
                            span,
                        ));
                    }
                }
            }
        };

        // Type-check arguments
        if args.len() != param_types.len() {
            self.errors.push(TypeError::new(
                TypeErrorKind::WrongArity {
                    expected: param_types.len(),
                    found: args.len(),
                },
                span,
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

        // Validate that the effect is either handled by an enclosing handler or declared in the function signature
        let is_handled = self.handled_effects.iter().any(|(eff_id, _)| *eff_id == effect_id);
        let is_declared = if let Some(fn_id) = self.current_fn {
            self.fn_effects.get(&fn_id)
                .map(|effects| effects.iter().any(|er| er.def_id == effect_id))
                .unwrap_or(false)
        } else {
            false
        };

        if !is_handled && !is_declared {
            // Get effect name for error message
            let effect_name = self.effect_defs.get(&effect_id)
                .map(|info| info.name.clone())
                .unwrap_or_else(|| format!("effect@{:?}", effect_id));
            self.errors.push(TypeError::new(
                TypeErrorKind::UndeclaredEffects {
                    effects: vec![effect_name],
                },
                span,
            ));
        }

        Ok(hir::Expr::new(
            hir::ExprKind::Perform {
                effect_id,
                op_index,
                args: hir_args,
                type_args,
            },
            return_ty,
            span,
        ))
    }

    /// Infer type of a resume expression.
    fn infer_resume(&mut self, value: &ast::Expr, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        // Validate we're inside a handler scope
        if !self.resolver.in_handler() {
            self.errors.push(TypeError::new(
                TypeErrorKind::InvalidHandler {
                    reason: "`resume` can only be used inside an effect handler".to_string(),
                },
                span,
            ));
            return Ok(hir::Expr::new(
                hir::ExprKind::Error,
                Type::error(),
                span,
            ));
        }

        // Increment resume count for linearity checking (shallow handlers)
        self.resume_count_in_current_op += 1;

        let value_expr = self.infer_expr(value)?;

        // Check that the value type matches the expected resume type (E0303)
        if let Some(ref expected_ty) = self.current_resume_type {
            let value_ty = self.unifier.resolve(&value_expr.ty);
            let expected = self.unifier.resolve(expected_ty);
            if self.unifier.unify(&value_ty, &expected, span).is_err() {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::ResumeTypeMismatch {
                        expected: format!("{}", expected),
                        found: format!("{}", value_ty),
                    },
                    span,
                )));
            }
        }

        // The type of the resume expression depends on the continuation's return type.
        // For deep handlers, this is the continuation result type set by the handler.
        // For shallow handlers (or if not tracked), default to a fresh variable.
        let resume_ty = self.current_resume_result_type
            .clone()
            .unwrap_or_else(|| self.unifier.fresh_var());

        Ok(hir::Expr::new(
            hir::ExprKind::Resume {
                value: Some(Box::new(value_expr)),
            },
            resume_ty,
            span,
        ))
    }

    /// Try to desugar a closure-accepting method call to a match expression.
    ///
    /// For example, `opt.map(f)` becomes:
    /// ```ignore
    /// match opt {
    ///     Some(_tmp) => Some(f(_tmp)),
    ///     None => None
    /// }
    /// ```
    ///
    /// Returns `Some(expr)` if desugaring was performed, `None` otherwise.
    fn try_desugar_closure_method(
        &mut self,
        receiver_expr: &hir::Expr,
        method_name: &str,
        args: &[hir::Expr],
        span: Span,
    ) -> Result<Option<hir::Expr>, Box<TypeError>> {
        // Get the underlying receiver type (auto-deref if needed)
        let receiver_ty = match receiver_expr.ty.kind() {
            TypeKind::Ref { inner, .. } => inner.clone(),
            _ => receiver_expr.ty.clone(),
        };

        // Check if receiver is Option<T>
        if let TypeKind::Adt { def_id, args: type_args, .. } = receiver_ty.kind() {
            if Some(*def_id) == self.option_def_id {
                // Get T from Option<T>
                let t_ty = type_args.first().cloned().unwrap_or_else(Type::unit);

                match method_name {
                    "map" if args.len() == 1 => {
                        return self.desugar_option_map(receiver_expr, &args[0], &t_ty, span).map(Some);
                    }
                    "and_then" if args.len() == 1 => {
                        return self.desugar_option_and_then(receiver_expr, &args[0], &t_ty, span).map(Some);
                    }
                    "filter" if args.len() == 1 => {
                        return self.desugar_option_filter(receiver_expr, &args[0], &t_ty, span).map(Some);
                    }
                    "map_or" if args.len() == 2 => {
                        return self.desugar_option_map_or(receiver_expr, &args[0], &args[1], &t_ty, span).map(Some);
                    }
                    "map_or_else" if args.len() == 2 => {
                        return self.desugar_option_map_or_else(receiver_expr, &args[0], &args[1], &t_ty, span).map(Some);
                    }
                    "or_else" if args.len() == 1 => {
                        return self.desugar_option_or_else(receiver_expr, &args[0], &t_ty, span).map(Some);
                    }
                    "unwrap_or_else" if args.len() == 1 => {
                        return self.desugar_option_unwrap_or_else(receiver_expr, &args[0], &t_ty, span).map(Some);
                    }
                    "unwrap_or_default" if args.is_empty() => {
                        return self.desugar_option_unwrap_or_default(receiver_expr, &t_ty, span).map(Some);
                    }
                    _ => {}
                }
            }

            // Check if receiver is Result<T, E>
            if Some(*def_id) == self.result_def_id {
                let t_ty = type_args.first().cloned().unwrap_or_else(Type::unit);
                let e_ty = type_args.get(1).cloned().unwrap_or_else(Type::unit);

                match method_name {
                    "map" if args.len() == 1 => {
                        return self.desugar_result_map(receiver_expr, &args[0], &t_ty, &e_ty, span).map(Some);
                    }
                    "map_err" if args.len() == 1 => {
                        return self.desugar_result_map_err(receiver_expr, &args[0], &t_ty, &e_ty, span).map(Some);
                    }
                    "and_then" if args.len() == 1 => {
                        return self.desugar_result_and_then(receiver_expr, &args[0], &t_ty, &e_ty, span).map(Some);
                    }
                    "or_else" if args.len() == 1 => {
                        return self.desugar_result_or_else(receiver_expr, &args[0], &t_ty, &e_ty, span).map(Some);
                    }
                    "unwrap_or_else" if args.len() == 1 => {
                        return self.desugar_result_unwrap_or_else(receiver_expr, &args[0], &t_ty, &e_ty, span).map(Some);
                    }
                    "unwrap_or_default" if args.is_empty() => {
                        return self.desugar_result_unwrap_or_default(receiver_expr, &t_ty, &e_ty, span).map(Some);
                    }
                    _ => {}
                }
            }
        }

        Ok(None)
    }

    /// Desugar Option.map(f): match opt { Some(x) => Some(f(x)), None => None }
    fn desugar_option_map(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let option_def_id = self.option_def_id.expect("option_def_id must be set");

        // Get U from closure type: fn(T) -> U
        let u_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => self.unifier.fresh_var(),
        };

        // Result type: Option<U>
        let result_ty = Type::adt(option_def_id, vec![u_ty.clone()]);

        // Get Some and None variant def_ids
        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        // Create a fresh local for the captured value
        let tmp_local_id = self.resolver.next_local_id();

        // Pattern for Some(x)
        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        // Body for Some arm: Some(f(x))
        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![local_ref],
            },
            u_ty.clone(),
            span,
        );
        let some_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![closure_call],
            },
            result_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: None,
            body: some_result,
        };

        // Pattern and body for None arm
        let none_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            ty: receiver.ty.clone(),
            span,
        };
        let none_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            result_ty.clone(),
            span,
        );

        let none_arm = hir::MatchArm {
            pattern: none_pattern,
            guard: None,
            body: none_result,
        };

        // Build the match expression
        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, none_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Option.and_then(f): match opt { Some(x) => f(x), None => None }
    fn desugar_option_and_then(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let option_def_id = self.option_def_id.expect("option_def_id must be set");

        // Get Option<U> from closure return type
        let result_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => self.unifier.fresh_var(),
        };

        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        // Some(x) pattern
        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        // Body: f(x) (returns Option<U>)
        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![local_ref],
            },
            result_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: None,
            body: closure_call,
        };

        // None arm returns None of the result type
        let none_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            ty: receiver.ty.clone(),
            span,
        };

        // Get U from Option<U> to create None of correct type
        let u_ty = match result_ty.kind() {
            TypeKind::Adt { args, .. } => args.first().cloned().unwrap_or_else(Type::unit),
            _ => Type::unit(),
        };
        let none_result_ty = Type::adt(option_def_id, vec![u_ty]);
        let none_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            none_result_ty,
            span,
        );

        let none_arm = hir::MatchArm {
            pattern: none_pattern,
            guard: None,
            body: none_result,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, none_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Option.filter(pred): match opt { Some(x) if pred(&x) => Some(x), _ => None }
    fn desugar_option_filter(
        &mut self,
        receiver: &hir::Expr,
        predicate: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = receiver.ty.clone();

        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        // Some(x) pattern
        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        // Guard: pred(&x)
        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );
        let local_borrow = hir::Expr::new(
            hir::ExprKind::Borrow {
                mutable: false,
                expr: Box::new(local_ref.clone()),
            },
            Type::reference(t_ty.clone(), false),
            span,
        );
        let guard_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(predicate.clone()),
                args: vec![local_borrow],
            },
            Type::bool(),
            span,
        );

        // Body: Some(x)
        let some_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![local_ref],
            },
            result_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: Some(guard_call),
            body: some_result,
        };

        // Wildcard arm returns None
        let wildcard_pattern = hir::Pattern {
            kind: hir::PatternKind::Wildcard,
            ty: receiver.ty.clone(),
            span,
        };
        let none_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            result_ty.clone(),
            span,
        );

        let wildcard_arm = hir::MatchArm {
            pattern: wildcard_pattern,
            guard: None,
            body: none_result,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, wildcard_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Option.map_or(default, f): match opt { Some(x) => f(x), None => default }
    fn desugar_option_map_or(
        &mut self,
        receiver: &hir::Expr,
        default: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Result type is U (return type of closure)
        let result_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => default.ty.clone(),
        };

        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![local_ref],
            },
            result_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: None,
            body: closure_call,
        };

        let none_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let none_arm = hir::MatchArm {
            pattern: none_pattern,
            guard: None,
            body: default.clone(),
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, none_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Option.map_or_else(default_fn, f): match opt { Some(x) => f(x), None => default_fn() }
    fn desugar_option_map_or_else(
        &mut self,
        receiver: &hir::Expr,
        default_fn: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => self.unifier.fresh_var(),
        };

        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![local_ref],
            },
            result_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: None,
            body: closure_call,
        };

        let none_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            ty: receiver.ty.clone(),
            span,
        };
        let default_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(default_fn.clone()),
                args: vec![],
            },
            result_ty.clone(),
            span,
        );

        let none_arm = hir::MatchArm {
            pattern: none_pattern,
            guard: None,
            body: default_call,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, none_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Option.or_else(f): match opt { Some(x) => Some(x), None => f() }
    fn desugar_option_or_else(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = receiver.ty.clone();

        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );
        let some_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![local_ref],
            },
            result_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: None,
            body: some_result,
        };

        let none_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            ty: receiver.ty.clone(),
            span,
        };
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![],
            },
            result_ty.clone(),
            span,
        );

        let none_arm = hir::MatchArm {
            pattern: none_pattern,
            guard: None,
            body: closure_call,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, none_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Option.unwrap_or_else(f): match opt { Some(x) => x, None => f() }
    fn desugar_option_unwrap_or_else(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = t_ty.clone();

        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: None,
            body: local_ref,
        };

        let none_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            ty: receiver.ty.clone(),
            span,
        };
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![],
            },
            result_ty.clone(),
            span,
        );

        let none_arm = hir::MatchArm {
            pattern: none_pattern,
            guard: None,
            body: closure_call,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, none_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Option.unwrap_or_default(): match opt { Some(x) => x, None => Default::default() }
    fn desugar_option_unwrap_or_default(
        &mut self,
        receiver: &hir::Expr,
        t_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = t_ty.clone();

        let (some_def_id, none_def_id) = self.get_option_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        let some_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: some_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );

        let some_arm = hir::MatchArm {
            pattern: some_pattern,
            guard: None,
            body: local_ref,
        };

        let none_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: none_def_id,
                variant_idx: 1,
                fields: vec![],
            },
            ty: receiver.ty.clone(),
            span,
        };

        // Generate default value for the type
        let default_expr = self.generate_default_value(t_ty, span)?;

        let none_arm = hir::MatchArm {
            pattern: none_pattern,
            guard: None,
            body: default_expr,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![some_arm, none_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Helper to get Option's Some and None variant DefIds
    fn get_option_variant_def_ids(&self) -> Result<(DefId, DefId), Box<TypeError>> {
        let option_def_id = self.option_def_id.expect("option_def_id must be set");

        // Look up the enum info to get variant def IDs
        if let Some(enum_info) = self.enum_defs.get(&option_def_id) {
            let some_def_id = enum_info.variants.iter()
                .find(|v| v.name == "Some")
                .map(|v| v.def_id)
                .expect("Option must have Some variant");
            let none_def_id = enum_info.variants.iter()
                .find(|v| v.name == "None")
                .map(|v| v.def_id)
                .expect("Option must have None variant");
            Ok((some_def_id, none_def_id))
        } else {
            // Fallback: use synthetic def IDs (this shouldn't happen in practice)
            Ok((DefId::new(option_def_id.index() + 1), DefId::new(option_def_id.index() + 2)))
        }
    }

    /// Helper to get Result's Ok and Err variant DefIds
    fn get_result_variant_def_ids(&self) -> Result<(DefId, DefId), Box<TypeError>> {
        let result_def_id = self.result_def_id.expect("result_def_id must be set");

        if let Some(enum_info) = self.enum_defs.get(&result_def_id) {
            let ok_def_id = enum_info.variants.iter()
                .find(|v| v.name == "Ok")
                .map(|v| v.def_id)
                .expect("Result must have Ok variant");
            let err_def_id = enum_info.variants.iter()
                .find(|v| v.name == "Err")
                .map(|v| v.def_id)
                .expect("Result must have Err variant");
            Ok((ok_def_id, err_def_id))
        } else {
            Ok((DefId::new(result_def_id.index() + 1), DefId::new(result_def_id.index() + 2)))
        }
    }

    /// Generate a default value for a type
    fn generate_default_value(&self, ty: &Type, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        match ty.kind() {
            TypeKind::Primitive(prim) => {
                let lit = match prim {
                    hir::PrimitiveTy::Bool => hir::LiteralValue::Bool(false),
                    hir::PrimitiveTy::Char => hir::LiteralValue::Char('\0'),
                    hir::PrimitiveTy::Int(_) | hir::PrimitiveTy::Uint(_) => hir::LiteralValue::Int(0),
                    hir::PrimitiveTy::Float(_) => hir::LiteralValue::Float(0.0),
                    hir::PrimitiveTy::Unit => return Ok(hir::Expr::new(
                        hir::ExprKind::Tuple(vec![]),
                        Type::unit(),
                        span,
                    )),
                    hir::PrimitiveTy::String => hir::LiteralValue::String("".to_string()),
                    hir::PrimitiveTy::Str => hir::LiteralValue::String("".to_string()),
                    hir::PrimitiveTy::Never => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "Never type has no default value".to_string(),
                            },
                            span,
                        )));
                    }
                };
                Ok(hir::Expr::new(
                    hir::ExprKind::Literal(lit),
                    ty.clone(),
                    span,
                ))
            }
            TypeKind::Tuple(elems) if elems.is_empty() => {
                Ok(hir::Expr::new(
                    hir::ExprKind::Tuple(vec![]),
                    Type::unit(),
                    span,
                ))
            }
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.option_def_id => {
                // Option defaults to None
                let (_, none_def_id) = self.get_option_variant_def_ids()?;
                Ok(hir::Expr::new(
                    hir::ExprKind::Variant {
                        def_id: none_def_id,
                        variant_idx: 1,
                        fields: vec![],
                    },
                    ty.clone(),
                    span,
                ))
            }
            _ => {
                // For types without a known default, use Default expression
                // This will be handled at codegen time
                Ok(hir::Expr::new(
                    hir::ExprKind::Default,
                    ty.clone(),
                    span,
                ))
            }
        }
    }

    // Result desugarings (similar pattern to Option)

    /// Desugar Result.map(f): match res { Ok(x) => Ok(f(x)), Err(e) => Err(e) }
    fn desugar_result_map(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        e_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_def_id = self.result_def_id.expect("result_def_id must be set");

        let u_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => self.unifier.fresh_var(),
        };

        let result_ty = Type::adt(result_def_id, vec![u_ty.clone(), e_ty.clone()]);

        let (ok_def_id, err_def_id) = self.get_result_variant_def_ids()?;

        let tmp_local_id = self.resolver.next_local_id();

        // Ok(x) arm
        let ok_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: tmp_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let local_ref = hir::Expr::new(
            hir::ExprKind::Local(tmp_local_id),
            t_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![local_ref],
            },
            u_ty.clone(),
            span,
        );
        let ok_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![closure_call],
            },
            result_ty.clone(),
            span,
        );

        let ok_arm = hir::MatchArm {
            pattern: ok_pattern,
            guard: None,
            body: ok_result,
        };

        // Err(e) arm - propagate error
        let err_local_id = self.resolver.next_local_id();

        let err_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: err_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: e_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let err_ref = hir::Expr::new(
            hir::ExprKind::Local(err_local_id),
            e_ty.clone(),
            span,
        );
        let err_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![err_ref],
            },
            result_ty.clone(),
            span,
        );

        let err_arm = hir::MatchArm {
            pattern: err_pattern,
            guard: None,
            body: err_result,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![ok_arm, err_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Result.map_err(f): match res { Ok(x) => Ok(x), Err(e) => Err(f(e)) }
    fn desugar_result_map_err(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        e_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_def_id = self.result_def_id.expect("result_def_id must be set");

        let f_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => self.unifier.fresh_var(),
        };

        let result_ty = Type::adt(result_def_id, vec![t_ty.clone(), f_ty.clone()]);

        let (ok_def_id, err_def_id) = self.get_result_variant_def_ids()?;

        // Ok(x) arm - just propagate
        let ok_local_id = self.resolver.next_local_id();

        let ok_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: ok_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let ok_ref = hir::Expr::new(
            hir::ExprKind::Local(ok_local_id),
            t_ty.clone(),
            span,
        );
        let ok_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![ok_ref],
            },
            result_ty.clone(),
            span,
        );

        let ok_arm = hir::MatchArm {
            pattern: ok_pattern,
            guard: None,
            body: ok_result,
        };

        // Err(e) arm - apply closure
        let err_local_id = self.resolver.next_local_id();

        let err_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: err_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: e_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let err_ref = hir::Expr::new(
            hir::ExprKind::Local(err_local_id),
            e_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![err_ref],
            },
            f_ty.clone(),
            span,
        );
        let err_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![closure_call],
            },
            result_ty.clone(),
            span,
        );

        let err_arm = hir::MatchArm {
            pattern: err_pattern,
            guard: None,
            body: err_result,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![ok_arm, err_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Result.and_then(f): match res { Ok(x) => f(x), Err(e) => Err(e) }
    fn desugar_result_and_then(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        e_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => self.unifier.fresh_var(),
        };

        let (ok_def_id, err_def_id) = self.get_result_variant_def_ids()?;

        // Ok(x) arm - call closure
        let ok_local_id = self.resolver.next_local_id();

        let ok_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: ok_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let ok_ref = hir::Expr::new(
            hir::ExprKind::Local(ok_local_id),
            t_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![ok_ref],
            },
            result_ty.clone(),
            span,
        );

        let ok_arm = hir::MatchArm {
            pattern: ok_pattern,
            guard: None,
            body: closure_call,
        };

        // Err(e) arm - propagate with correct result type
        let err_local_id = self.resolver.next_local_id();

        let err_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: err_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: e_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let err_ref = hir::Expr::new(
            hir::ExprKind::Local(err_local_id),
            e_ty.clone(),
            span,
        );
        let err_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![err_ref],
            },
            result_ty.clone(),
            span,
        );

        let err_arm = hir::MatchArm {
            pattern: err_pattern,
            guard: None,
            body: err_result,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![ok_arm, err_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Result.or_else(f): match res { Ok(x) => Ok(x), Err(e) => f(e) }
    fn desugar_result_or_else(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        e_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = match closure.ty.kind() {
            TypeKind::Fn { ret, .. } => ret.clone(),
            TypeKind::Closure { ret, .. } => ret.clone(),
            _ => self.unifier.fresh_var(),
        };

        let (ok_def_id, err_def_id) = self.get_result_variant_def_ids()?;

        // Ok(x) arm - propagate with correct result type
        let ok_local_id = self.resolver.next_local_id();

        let ok_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: ok_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let ok_ref = hir::Expr::new(
            hir::ExprKind::Local(ok_local_id),
            t_ty.clone(),
            span,
        );
        let ok_result = hir::Expr::new(
            hir::ExprKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![ok_ref],
            },
            result_ty.clone(),
            span,
        );

        let ok_arm = hir::MatchArm {
            pattern: ok_pattern,
            guard: None,
            body: ok_result,
        };

        // Err(e) arm - call closure
        let err_local_id = self.resolver.next_local_id();

        let err_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: err_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: e_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let err_ref = hir::Expr::new(
            hir::ExprKind::Local(err_local_id),
            e_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![err_ref],
            },
            result_ty.clone(),
            span,
        );

        let err_arm = hir::MatchArm {
            pattern: err_pattern,
            guard: None,
            body: closure_call,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![ok_arm, err_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Result.unwrap_or_else(f): match res { Ok(x) => x, Err(e) => f(e) }
    fn desugar_result_unwrap_or_else(
        &mut self,
        receiver: &hir::Expr,
        closure: &hir::Expr,
        t_ty: &Type,
        e_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = t_ty.clone();

        let (ok_def_id, err_def_id) = self.get_result_variant_def_ids()?;

        // Ok(x) arm - return x
        let ok_local_id = self.resolver.next_local_id();

        let ok_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: ok_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let ok_ref = hir::Expr::new(
            hir::ExprKind::Local(ok_local_id),
            t_ty.clone(),
            span,
        );

        let ok_arm = hir::MatchArm {
            pattern: ok_pattern,
            guard: None,
            body: ok_ref,
        };

        // Err(e) arm - call closure
        let err_local_id = self.resolver.next_local_id();

        let err_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: err_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: e_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let err_ref = hir::Expr::new(
            hir::ExprKind::Local(err_local_id),
            e_ty.clone(),
            span,
        );
        let closure_call = hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(closure.clone()),
                args: vec![err_ref],
            },
            result_ty.clone(),
            span,
        );

        let err_arm = hir::MatchArm {
            pattern: err_pattern,
            guard: None,
            body: closure_call,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![ok_arm, err_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Desugar Result.unwrap_or_default(): match res { Ok(x) => x, Err(_) => Default::default() }
    fn desugar_result_unwrap_or_default(
        &mut self,
        receiver: &hir::Expr,
        t_ty: &Type,
        _e_ty: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let result_ty = t_ty.clone();

        let (ok_def_id, err_def_id) = self.get_result_variant_def_ids()?;

        // Ok(x) arm
        let ok_local_id = self.resolver.next_local_id();

        let ok_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: ok_def_id,
                variant_idx: 0,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: ok_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: t_ty.clone(),
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let ok_ref = hir::Expr::new(
            hir::ExprKind::Local(ok_local_id),
            t_ty.clone(),
            span,
        );

        let ok_arm = hir::MatchArm {
            pattern: ok_pattern,
            guard: None,
            body: ok_ref,
        };

        // Err(_) arm - return default
        let err_pattern = hir::Pattern {
            kind: hir::PatternKind::Variant {
                def_id: err_def_id,
                variant_idx: 1,
                fields: vec![hir::Pattern {
                    kind: hir::PatternKind::Wildcard,
                    ty: Type::unit(), // Wildcard, type doesn't matter
                    span,
                }],
            },
            ty: receiver.ty.clone(),
            span,
        };

        let default_expr = self.generate_default_value(t_ty, span)?;

        let err_arm = hir::MatchArm {
            pattern: err_pattern,
            guard: None,
            body: default_expr,
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(receiver.clone()),
                arms: vec![ok_arm, err_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Infer type of a method call.
    ///
    /// This desugars `receiver.method(args)` into `method(receiver, args)`.
    pub(crate) fn infer_method_call(
        &mut self,
        receiver: &ast::Expr,
        method: &ast::Symbol,
        args: &[ast::CallArg],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let receiver_expr = self.infer_expr(receiver)?;
        let method_name = self.symbol_to_string(*method);

        // First do a preliminary inference of arguments for special handling
        // (closure desugaring, etc.) - these will be re-checked with proper types later
        let mut preliminary_args = Vec::with_capacity(args.len());
        for arg in args {
            let arg_expr = self.infer_expr(&arg.value)?;
            preliminary_args.push(arg_expr);
        }

        // Try to desugar closure-accepting methods (needs preliminary args)
        if let Some(desugared) = self.try_desugar_closure_method(&receiver_expr, &method_name, &preliminary_args, span)? {
            return Ok(desugared);
        }

        // Special handling for .len() on arrays and slices
        if method_name == "len" && preliminary_args.is_empty() {
            // Get the underlying type (auto-deref if needed)
            let underlying_ty = match receiver_expr.ty.kind() {
                TypeKind::Ref { inner, .. } => inner,
                _ => &receiver_expr.ty,
            };

            match underlying_ty.kind() {
                // Array: return constant length
                TypeKind::Array { size, .. } => {
                    let concrete_size = size.as_u64().unwrap_or(0) as i128;
                    return Ok(hir::Expr::new(
                        hir::ExprKind::Literal(hir::LiteralValue::Int(concrete_size)),
                        Type::usize(),
                        span,
                    ));
                }
                // Slice: return SliceLen expression
                TypeKind::Slice { .. } => {
                    return Ok(hir::Expr::new(
                        hir::ExprKind::SliceLen(Box::new(receiver_expr)),
                        Type::usize(),
                        span,
                    ));
                }
                _ => {}
            }
        }

        // Try to find method signature
        let (method_def_id, return_ty, first_param, impl_generics, method_generics, needs_auto_ref) =
            self.resolve_method(&receiver_expr.ty, &method_name, &preliminary_args, span)?;

        // Build substitution from impl generics to concrete types from receiver
        let mut substitution: HashMap<TyVarId, Type> = HashMap::new();

        // Extract concrete type args from receiver to build substitution
        // Handle both direct ADT types (Vec<i32>) and references to them (&mut Vec<i32>)
        // Also handle slice types ([T])
        if !impl_generics.is_empty() {
            // Get the underlying type (unwrap references if needed)
            let underlying_ty = match receiver_expr.ty.kind() {
                TypeKind::Ref { inner, .. } => inner,
                _ => &receiver_expr.ty,
            };

            match underlying_ty.kind() {
                TypeKind::Adt { args: receiver_args, .. } => {
                    for (tyvar, concrete_ty) in impl_generics.iter().zip(receiver_args.iter()) {
                        substitution.insert(*tyvar, concrete_ty.clone());
                    }
                }
                TypeKind::Slice { element } => {
                    // For slice types, the element type is the single type argument
                    if let Some(&tyvar) = impl_generics.first() {
                        substitution.insert(tyvar, (*element).clone());
                    }
                }
                _ => {}
            }
        }

        // Add fresh type vars for method-level generics
        for &tyvar in &method_generics {
            substitution.insert(tyvar, self.unifier.fresh_var());
        }

        // Now that we know the method signature, re-check arguments with expected parameter types.
        // This allows integer literals to be inferred as the correct type (e.g., u8 instead of i32).
        let hir_args = if let Some(sig) = self.fn_sigs.get(&method_def_id).cloned() {
            // Apply substitution to get concrete parameter types
            let subst_inputs: Vec<Type> = sig.inputs.iter()
                .map(|ty| self.substitute_type_vars(ty, &substitution))
                .collect();

            // Re-check each argument with the expected parameter type
            // subst_inputs[0] is the receiver, subst_inputs[1..] are the method parameters
            let mut checked_args = Vec::with_capacity(args.len());
            for (i, arg) in args.iter().enumerate() {
                if let Some(param_ty) = subst_inputs.get(i + 1) {
                    // Use check_expr to infer literals with the expected type
                    let checked_arg = self.check_expr(&arg.value, param_ty)?;
                    checked_args.push(checked_arg);
                } else {
                    // Fallback to inference if no param type (shouldn't happen)
                    let inferred_arg = self.infer_expr(&arg.value)?;
                    checked_args.push(inferred_arg);
                }
            }
            checked_args
        } else {
            // No signature available, use preliminary args
            preliminary_args
        };

        // Build the callee type by substituting in the stored signature
        // Also compute the final return type with method generics substituted
        let (callee_ty, final_return_ty) = if let Some(sig) = self.fn_sigs.get(&method_def_id).cloned() {
            // Apply substitution to inputs and output
            let subst_inputs: Vec<Type> = sig.inputs.iter()
                .map(|ty| self.substitute_type_vars(ty, &substitution))
                .collect();
            let subst_output = self.substitute_type_vars(&sig.output, &substitution);

            // Unify substituted parameter types with actual argument types
            // This infers method-level type parameters from arguments
            // Skip the first (receiver) parameter, unify remaining with hir_args
            for (i, arg) in hir_args.iter().enumerate() {
                // subst_inputs[0] is receiver, subst_inputs[1..] are the rest
                if let Some(param_ty) = subst_inputs.get(i + 1) {
                    // Unify arg type with param type to infer type vars
                    // If unification fails, report a type mismatch error
                    self.unifier.unify(param_ty, &arg.ty, arg.span).map_err(|_| {
                        TypeError::new(
                            TypeErrorKind::Mismatch {
                                expected: self.unifier.resolve(param_ty),
                                found: arg.ty.clone(),
                            },
                            arg.span,
                        )
                    })?;
                }
            }

            // Also substitute the return type from resolve_method
            let subst_return_ty = self.substitute_type_vars(&return_ty, &substitution);
            // Resolve to replace any unified type vars with their concrete types
            let resolved_return_ty = self.unifier.resolve(&subst_return_ty);
            (Type::function(subst_inputs, subst_output), resolved_return_ty)
        } else {
            // Fallback to inferred function type
            let receiver_ty = if needs_auto_ref {
                Type::reference(receiver_expr.ty.clone(), false)
            } else {
                receiver_expr.ty.clone()
            };
            let mut param_types = vec![receiver_ty];
            param_types.extend(hir_args.iter().map(|a| a.ty.clone()));
            (Type::function(param_types, return_ty.clone()), return_ty.clone())
        };

        let callee = hir::Expr::new(
            hir::ExprKind::Def(method_def_id),
            callee_ty,
            span,
        );

        // Build receiver expression, auto-borrowing or auto-derefing if needed
        let final_receiver = if needs_auto_ref {
            // Get mutability from the first param type
            let mutable = if let Some(ref param_ty) = first_param {
                if let TypeKind::Ref { mutable, .. } = param_ty.kind() {
                    *mutable
                } else {
                    false
                }
            } else {
                false
            };
            let ref_ty = Type::reference(receiver_expr.ty.clone(), mutable);
            hir::Expr::new(
                hir::ExprKind::Borrow {
                    mutable,
                    expr: Box::new(receiver_expr),
                },
                ref_ty,
                span,
            )
        } else {
            // Check if we need auto-deref: receiver is &T but method expects T (not &T)
            // This happens when a method was found via auto-deref in resolve_method
            let needs_auto_deref = if let TypeKind::Ref { inner: _, .. } = receiver_expr.ty.kind() {
                if let Some(ref param_ty) = first_param {
                    // Need auto-deref if first param is NOT a reference and matches the inner type
                    !matches!(param_ty.kind(), TypeKind::Ref { .. })
                } else {
                    false
                }
            } else {
                false
            };

            if needs_auto_deref {
                // Insert a deref to convert &T to T
                // Clone inner type before moving receiver_expr
                let inner_ty = if let TypeKind::Ref { inner, .. } = receiver_expr.ty.kind() {
                    Some(inner.clone())
                } else {
                    None
                };
                if let Some(derefed_ty) = inner_ty {
                    hir::Expr::new(
                        hir::ExprKind::Deref(Box::new(receiver_expr)),
                        derefed_ty,
                        span,
                    )
                } else {
                    receiver_expr
                }
            } else {
                receiver_expr
            }
        };

        // Build args with receiver as first argument
        let mut all_args = Vec::with_capacity(1 + hir_args.len());
        all_args.push(final_receiver);
        all_args.extend(hir_args);

        Ok(hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(callee),
                args: all_args,
            },
            final_return_ty,
            span,
        ))
    }

    /// Resolve a method on a type.
    /// Returns (method_def_id, return_type, first_param_type, impl_generics, method_generics, needs_auto_ref).
    pub(crate) fn resolve_method(
        &mut self,
        receiver_ty: &Type,
        method_name: &str,
        args: &[hir::Expr],
        span: Span,
    ) -> Result<MethodResolution, Box<TypeError>> {
        // Try to find the method on the receiver type directly
        if let Some((def_id, ret_ty, first_param, impl_generics, method_generics)) = self.find_method_for_type(receiver_ty, method_name, args) {
            // Check if we need to auto-ref the receiver
            let needs_auto_ref = if let Some(ref param_ty) = first_param {
                // If first param is a reference and receiver is not, we need auto-ref
                matches!(param_ty.kind(), TypeKind::Ref { .. }) && !matches!(receiver_ty.kind(), TypeKind::Ref { .. })
            } else {
                false
            };
            return Ok((def_id, ret_ty, first_param, impl_generics, method_generics, needs_auto_ref));
        }

        // Try iterative auto-deref: peel references (&T, &&T, etc.) to find method
        let mut deref_ty = receiver_ty.clone();
        while let TypeKind::Ref { inner, .. } = deref_ty.kind() {
            let inner_resolved = self.unifier.resolve(inner);
            if let Some((def_id, ret_ty, first_param, impl_generics, method_generics)) = self.find_method_for_type(&inner_resolved, method_name, args) {
                // When auto-deref is used, we don't need auto-ref
                return Ok((def_id, ret_ty, first_param, impl_generics, method_generics, false));
            }
            deref_ty = inner_resolved;
        }

        // No method found
        Err(self.error_method_not_found(receiver_ty, method_name, span))
    }

    /// Find a method for a specific type by searching impl blocks.
    /// Returns (method_def_id, substituted return type, substituted first param type, impl generics, method generics).
    /// Supports method overloading by matching argument types when multiple methods share the same name.
    pub(crate) fn find_method_for_type(&self, ty: &Type, method_name: &str, args: &[hir::Expr]) -> MethodLookup {
        // First, look for inherent impl methods (impl blocks without trait_ref)
        for impl_block in &self.impl_blocks {
            if impl_block.trait_ref.is_some() {
                continue;
            }

            // Try to extract substitution from the impl block
            let subst = if impl_block.generics.is_empty() {
                if !self.types_match_for_impl(&impl_block.self_ty, ty) {
                    continue;
                }
                HashMap::new()
            } else {
                match self.extract_impl_substitution(&impl_block.generics, &impl_block.self_ty, ty) {
                    Some(s) => s,
                    None => continue,
                }
            };

            // Collect all methods with matching name for overload resolution
            let matching_methods: Vec<_> = impl_block.methods.iter()
                .filter(|m| m.name == method_name)
                .collect();

            // If only one method, return it (no overload resolution needed)
            if matching_methods.len() == 1 {
                let method = matching_methods[0];
                if let Some(sig) = self.fn_sigs.get(&method.def_id) {
                    let subst_output = self.substitute_type_vars(&sig.output, &subst);
                    let first_param = sig.inputs.first().map(|p| self.substitute_type_vars(p, &subst));
                    return Some((
                        method.def_id,
                        subst_output,
                        first_param,
                        impl_block.generics.clone(),
                        sig.generics.clone(),
                    ));
                }
            }

            // Multiple methods with same name - perform overload resolution
            for method in &matching_methods {
                if let Some(sig) = self.fn_sigs.get(&method.def_id) {
                    // Apply substitution to parameter types
                    let subst_inputs: Vec<Type> = sig.inputs.iter()
                        .map(|t| self.substitute_type_vars(t, &subst))
                        .collect();

                    // Check if argument count matches (params[1..] are the non-receiver args)
                    let param_count = subst_inputs.len().saturating_sub(1); // exclude receiver
                    if args.len() != param_count {
                        continue;
                    }

                    // Check if argument types are compatible
                    let mut matches = true;
                    for (i, arg) in args.iter().enumerate() {
                        if let Some(param_ty) = subst_inputs.get(i + 1) {
                            // For overload resolution, check structural compatibility
                            // This handles cases like i32 vs bool without full unification
                            if !self.types_compatible_for_overload(&arg.ty, param_ty) {
                                matches = false;
                                break;
                            }
                        }
                    }

                    if matches {
                        let subst_output = self.substitute_type_vars(&sig.output, &subst);
                        let first_param = sig.inputs.first().map(|p| self.substitute_type_vars(p, &subst));
                        return Some((
                            method.def_id,
                            subst_output,
                            first_param,
                            impl_block.generics.clone(),
                            sig.generics.clone(),
                        ));
                    }
                }
            }
        }

        // Second, look for trait impl methods (including default methods from trait)
        for impl_block in &self.impl_blocks {
            let Some(trait_id) = impl_block.trait_ref else {
                continue;
            };

            // Try to extract substitution from the impl block
            let subst = if impl_block.generics.is_empty() {
                if !self.types_match_for_impl(&impl_block.self_ty, ty) {
                    continue;
                }
                HashMap::new()
            } else {
                match self.extract_impl_substitution(&impl_block.generics, &impl_block.self_ty, ty) {
                    Some(s) => s,
                    None => continue,
                }
            };

            // First check methods defined in the impl block
            for method in &impl_block.methods {
                if method.name == method_name {
                    if let Some(sig) = self.fn_sigs.get(&method.def_id) {
                        let subst_output = self.substitute_type_vars(&sig.output, &subst);
                        let first_param = sig.inputs.first().map(|p| self.substitute_type_vars(p, &subst));
                        return Some((
                            method.def_id,
                            subst_output,
                            first_param,
                            impl_block.generics.clone(),
                            sig.generics.clone(),
                        ));
                    }
                }
            }

            // If not found in impl, check for default method in trait definition
            if let Some(trait_info) = self.trait_defs.get(&trait_id) {
                for trait_method in &trait_info.methods {
                    if trait_method.name == method_name && trait_method.has_default {
                        if let Some(sig) = self.fn_sigs.get(&trait_method.def_id) {
                            let subst_output = self.substitute_type_vars(&sig.output, &subst);
                            let first_param = sig.inputs.first().map(|p| self.substitute_type_vars(p, &subst));
                            return Some((
                                trait_method.def_id,
                                subst_output,
                                first_param,
                                impl_block.generics.clone(),
                                sig.generics.clone(),
                            ));
                        }
                    }
                }
            }
        }

        // Third, look for methods from trait bounds on type parameters
        if let TypeKind::Param(ty_var_id) = ty.kind() {
            if let Some(bounds) = self.type_param_bounds.get(ty_var_id) {
                for &trait_def_id in bounds {
                    if let Some(trait_info) = self.trait_defs.get(&trait_def_id) {
                        for method in &trait_info.methods {
                            if method.name == method_name {
                                if let Some(sig) = self.fn_sigs.get(&method.def_id) {
                                    let first_param = sig.inputs.first().cloned();
                                    return Some((
                                        method.def_id,
                                        sig.output.clone(),
                                        first_param,
                                        Vec::new(),
                                        sig.generics.clone(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Fourth, look for builtin methods on primitive/builtin types
        if let Some(result) = self.find_builtin_method(ty, method_name) {
            return Some(result);
        }

        None
    }

    /// Find a builtin method for a type.
    /// Returns (method_def_id, return type, first param type, impl generics, method generics).
    fn find_builtin_method(&self, ty: &Type, method_name: &str) -> MethodLookup {
        use super::BuiltinMethodType;
        use crate::hir::ty::PrimitiveTy;

        // Determine which builtin type category this type matches
        let type_match = match ty.kind() {
            TypeKind::Primitive(PrimitiveTy::Str) => Some(BuiltinMethodType::Str),
            TypeKind::Primitive(PrimitiveTy::Char) => Some(BuiltinMethodType::Char),
            TypeKind::Primitive(PrimitiveTy::String) => Some(BuiltinMethodType::String),
            TypeKind::Slice { .. } => Some(BuiltinMethodType::Slice),
            TypeKind::Ref { inner, .. } => {
                match inner.kind() {
                    TypeKind::Primitive(PrimitiveTy::Str) => Some(BuiltinMethodType::StrRef),
                    TypeKind::Primitive(PrimitiveTy::String) => Some(BuiltinMethodType::String),
                    TypeKind::Slice { .. } => Some(BuiltinMethodType::Slice),
                    _ => None,
                }
            }
            TypeKind::Adt { def_id, .. } => {
                // Check if it's Option, Vec, Box, or Result
                if Some(*def_id) == self.option_def_id {
                    Some(BuiltinMethodType::Option)
                } else if Some(*def_id) == self.vec_def_id {
                    Some(BuiltinMethodType::Vec)
                } else if Some(*def_id) == self.box_def_id {
                    Some(BuiltinMethodType::Box)
                } else if Some(*def_id) == self.result_def_id {
                    Some(BuiltinMethodType::Result)
                } else {
                    None
                }
            }
            _ => None,
        };

        let type_match = type_match?;

        // Search for matching builtin method
        for builtin_method in &self.builtin_methods {
            if builtin_method.type_match == type_match && builtin_method.name == method_name {
                if let Some(sig) = self.fn_sigs.get(&builtin_method.def_id) {
                    // For generic types (Option<T>, Vec<T>), we need to substitute
                    // the type argument into the return type
                    let return_type = match &type_match {
                        BuiltinMethodType::Option => {
                            // Extract the type argument from Option<T>
                            if let TypeKind::Adt { args, .. } = ty.kind() {
                                if !args.is_empty() {
                                    let element_ty = args[0].clone();
                                    // For Option<T>.unwrap() and Option<T>.try_(), return type is T
                                    if method_name == "unwrap" || method_name == "try_" || method_name == "expect" {
                                        element_ty
                                    } else if method_name == "unwrap_or" {
                                        // Option<T>.unwrap_or(default: T) -> T
                                        element_ty
                                    } else if method_name == "or" || method_name == "xor" {
                                        // Option<T>.or/xor(other: Option<T>) -> Option<T>
                                        ty.clone()
                                    } else if method_name == "take" || method_name == "replace" {
                                        // Option<T>.take/replace() -> Option<T>
                                        ty.clone()
                                    } else if method_name == "as_ref" {
                                        // Option<T>.as_ref() -> Option<&T>
                                        let ref_elem = Type::reference(element_ty, false);
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![ref_elem],
                                        )
                                    } else if method_name == "as_mut" {
                                        // Option<T>.as_mut() -> Option<&mut T>
                                        let ref_mut_elem = Type::reference(element_ty, true);
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![ref_mut_elem],
                                        )
                                    } else {
                                        // Default: return registered type
                                        sig.output.clone()
                                    }
                                } else {
                                    sig.output.clone()
                                }
                            } else {
                                sig.output.clone()
                            }
                        }
                        BuiltinMethodType::Vec | BuiltinMethodType::Box => {
                            // Extract the type argument from the ADT
                            if let TypeKind::Adt { args, .. } = ty.kind() {
                                if !args.is_empty() {
                                    let element_ty = args[0].clone();
                                    if method_name == "get" || method_name == "first" || method_name == "last" {
                                        // Vec<T>.get/first/last() returns Option<&T>
                                        let ref_elem = Type::reference(element_ty, false);
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![ref_elem],
                                        )
                                    } else if method_name == "get_mut" || method_name == "first_mut" || method_name == "last_mut" {
                                        // Vec<T>.get_mut/first_mut/last_mut() returns Option<&mut T>
                                        let ref_mut_elem = Type::reference(element_ty, true);
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![ref_mut_elem],
                                        )
                                    } else if method_name == "pop" {
                                        // Vec<T>.pop() returns Option<T>
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![element_ty],
                                        )
                                    } else if method_name == "remove" || method_name == "swap_remove" {
                                        // Vec<T>.remove/swap_remove() returns T
                                        element_ty
                                    } else if method_name == "as_slice" {
                                        // Vec<T>.as_slice() returns &[T]
                                        Type::reference(Type::slice(element_ty), false)
                                    } else if method_name == "as_mut_slice" {
                                        // Vec<T>.as_mut_slice() returns &mut [T]
                                        Type::reference(Type::slice(element_ty), true)
                                    } else if method_name == "as_ref" {
                                        // Box<T>.as_ref() returns &T
                                        Type::reference(element_ty, false)
                                    } else if method_name == "as_mut" {
                                        // Box<T>.as_mut() returns &mut T
                                        Type::reference(element_ty, true)
                                    } else if method_name == "into_inner" {
                                        // Box<T>.into_inner() returns T
                                        element_ty
                                    } else {
                                        sig.output.clone()
                                    }
                                } else {
                                    sig.output.clone()
                                }
                            } else {
                                sig.output.clone()
                            }
                        }
                        BuiltinMethodType::Result => {
                            // Result<T, E> has two type arguments
                            if let TypeKind::Adt { args, def_id } = ty.kind() {
                                if args.len() >= 2 {
                                    let ok_ty = args[0].clone();
                                    let err_ty = args[1].clone();
                                    // unwrap(), try_(), expect() return T
                                    if method_name == "unwrap" || method_name == "try_" || method_name == "expect" {
                                        ok_ty
                                    // unwrap_err(), expect_err() return E
                                    } else if method_name == "unwrap_err" || method_name == "expect_err" {
                                        err_ty
                                    // unwrap_or(default: T) -> T
                                    } else if method_name == "unwrap_or" {
                                        ok_ty
                                    // ok() -> Option<T>
                                    } else if method_name == "ok" {
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![ok_ty],
                                        )
                                    // err() -> Option<E>
                                    } else if method_name == "err" {
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![err_ty],
                                        )
                                    // or(other: Result<T, F>) -> Result<T, F>
                                    // Note: The second Result might have different error type
                                    // For simplicity, return the receiver's Result type
                                    } else if method_name == "or" {
                                        // Result<T, F> where T comes from self, F from other
                                        // Since we can't easily extract F here, fall back to sig
                                        ty.clone()
                                    // as_ref() -> Result<&T, &E>
                                    } else if method_name == "as_ref" {
                                        let ref_ok = Type::reference(ok_ty, false);
                                        let ref_err = Type::reference(err_ty, false);
                                        Type::adt(*def_id, vec![ref_ok, ref_err])
                                    // as_mut() -> Result<&mut T, &mut E>
                                    } else if method_name == "as_mut" {
                                        let ref_mut_ok = Type::reference(ok_ty, true);
                                        let ref_mut_err = Type::reference(err_ty, true);
                                        Type::adt(*def_id, vec![ref_mut_ok, ref_mut_err])
                                    } else {
                                        sig.output.clone()
                                    }
                                } else {
                                    sig.output.clone()
                                }
                            } else {
                                sig.output.clone()
                            }
                        }
                        BuiltinMethodType::Slice => {
                            // Extract element type from slice
                            // Type can be [T] directly or &[T]
                            let element_ty = match ty.kind() {
                                TypeKind::Slice { element } => element.clone(),
                                TypeKind::Ref { inner, .. } => {
                                    if let TypeKind::Slice { element } = inner.kind() {
                                        element.clone()
                                    } else {
                                        return None;
                                    }
                                }
                                _ => return None,
                            };

                            // first(), last(), get() return Option<&T>
                            if method_name == "first" || method_name == "last" || method_name == "get" {
                                let ref_elem = Type::reference(element_ty, false);
                                Type::adt(
                                    self.option_def_id.expect("BUG: option_def_id not set"),
                                    vec![ref_elem],
                                )
                            } else if method_name == "get_mut" {
                                // get_mut() returns Option<&mut T>
                                let ref_mut_elem = Type::reference(element_ty, true);
                                Type::adt(
                                    self.option_def_id.expect("BUG: option_def_id not set"),
                                    vec![ref_mut_elem],
                                )
                            } else {
                                sig.output.clone()
                            }
                        }
                        _ => sig.output.clone(),
                    };

                    let first_param = sig.inputs.first().cloned();

                    // For generic builtin types (Option<T>, Vec<T>, Box<T>, Result<T, E>, [T]),
                    // we need to return the impl_generics so that type arguments from the
                    // receiver type can be substituted into the method signature.
                    // TyVarId(9000) is used as placeholder for T, TyVarId(9001) for E.
                    let impl_generics = match &type_match {
                        BuiltinMethodType::Option
                        | BuiltinMethodType::Vec
                        | BuiltinMethodType::Box
                        | BuiltinMethodType::Slice => vec![TyVarId(9000)],
                        BuiltinMethodType::Result => vec![TyVarId(9000), TyVarId(9001)],
                        _ => Vec::new(),
                    };

                    // Return method generics from signature so they get instantiated with fresh vars
                    return Some((
                        builtin_method.def_id,
                        return_type,
                        first_param,
                        impl_generics,
                        sig.generics.clone(),
                    ));
                }
            }
        }

        None
    }

    /// Find a builtin static method (e.g., String::new(), Vec::new()).
    /// Returns (method_def_id, function type).
    fn find_builtin_static_method(&mut self, type_name: &str, method_name: &str) -> Option<(DefId, Type)> {
        use super::BuiltinMethodType;

        // Map type name to BuiltinMethodType
        let type_match = match type_name {
            "String" => Some(BuiltinMethodType::String),
            "Vec" => Some(BuiltinMethodType::Vec),
            "Option" => Some(BuiltinMethodType::Option),
            "Box" => Some(BuiltinMethodType::Box),
            "Result" => Some(BuiltinMethodType::Result),
            _ => None,
        };

        let type_match = type_match?;

        // Search for matching static builtin method
        for builtin_method in &self.builtin_methods {
            if builtin_method.type_match == type_match
                && builtin_method.name == method_name
                && builtin_method.is_static
            {
                if let Some(sig) = self.fn_sigs.get(&builtin_method.def_id).cloned() {
                    // For generic static methods like Vec::new(), Box::new(), instantiate with fresh vars
                    // The synthetic TyVarId(9000) placeholder needs to be replaced with fresh vars
                    let needs_fresh_vars = matches!(&type_match, BuiltinMethodType::Vec | BuiltinMethodType::Option | BuiltinMethodType::Box | BuiltinMethodType::Result);

                    let fn_ty = if needs_fresh_vars {
                        // Create a fresh type variable to substitute for the placeholder
                        let fresh_var = self.unifier.fresh_var();
                        let placeholder_id = TyVarId(9000);

                        // Create substitution map
                        let mut subst = HashMap::new();
                        subst.insert(placeholder_id, fresh_var);

                        // Substitute in inputs and output
                        let subst_inputs: Vec<Type> = sig.inputs.iter()
                            .map(|ty| self.substitute_type_vars(ty, &subst))
                            .collect();
                        let subst_output = self.substitute_type_vars(&sig.output, &subst);

                        Type::function(subst_inputs, subst_output)
                    } else {
                        Type::function(sig.inputs.clone(), sig.output.clone())
                    };

                    return Some((builtin_method.def_id, fn_ty));
                }
            }
        }

        None
    }

    /// Instantiate a generic function signature with fresh type variables.
    pub(crate) fn instantiate_fn_sig(&mut self, sig: &hir::FnSig) -> Type {
        self.instantiate_fn_sig_with_effects(sig, Vec::new())
    }

    /// Instantiate a generic function signature with fresh type variables, including effects.
    pub(crate) fn instantiate_fn_sig_with_effects(
        &mut self,
        sig: &hir::FnSig,
        effects: Vec<hir::FnEffect>,
    ) -> Type {
        if sig.generics.is_empty() {
            return Type::function_with_effects(sig.inputs.clone(), sig.output.clone(), effects);
        }

        // Create a mapping from old type vars to fresh ones
        let mut substitution: HashMap<TyVarId, Type> = HashMap::new();
        for &old_var in &sig.generics {
            let fresh_var = self.unifier.fresh_var();
            substitution.insert(old_var, fresh_var);
        }

        // Substitute in parameter types
        let subst_inputs: Vec<Type> = sig.inputs.iter()
            .map(|ty| self.substitute_type_vars(ty, &substitution))
            .collect();

        // Substitute in return type
        let subst_output = self.substitute_type_vars(&sig.output, &substitution);

        // Substitute type variables in effect annotations
        let subst_effects: Vec<hir::FnEffect> = effects.iter()
            .map(|eff| hir::FnEffect::new(
                eff.def_id,
                eff.type_args.iter()
                    .map(|arg| self.substitute_type_vars(arg, &substitution))
                    .collect(),
            ))
            .collect();

        Type::function_with_effects(subst_inputs, subst_output, subst_effects)
    }

    /// Substitute type variables in a type using the given mapping.
    pub(crate) fn substitute_type_vars(&self, ty: &Type, subst: &HashMap<TyVarId, Type>) -> Type {
        match ty.kind() {
            TypeKind::Param(var_id) => {
                subst.get(var_id).cloned().unwrap_or_else(|| ty.clone())
            }
            TypeKind::Adt { def_id, args } => {
                let subst_args: Vec<Type> = args.iter()
                    .map(|arg| self.substitute_type_vars(arg, subst))
                    .collect();
                Type::adt(*def_id, subst_args)
            }
            TypeKind::Fn { params, ret, effects, const_args } => {
                let subst_params: Vec<Type> = params.iter()
                    .map(|p| self.substitute_type_vars(p, subst))
                    .collect();
                let subst_ret = self.substitute_type_vars(ret, subst);
                // Also substitute type variables in effect annotations
                let subst_effects: Vec<hir::FnEffect> = effects.iter()
                    .map(|eff| hir::FnEffect::new(
                        eff.def_id,
                        eff.type_args.iter()
                            .map(|arg| self.substitute_type_vars(arg, subst))
                            .collect(),
                    ))
                    .collect();
                Type::function_with_const_args(subst_params, subst_ret, subst_effects, const_args.clone())
            }
            TypeKind::Tuple(elems) => {
                let subst_elems: Vec<Type> = elems.iter()
                    .map(|e| self.substitute_type_vars(e, subst))
                    .collect();
                Type::tuple(subst_elems)
            }
            TypeKind::Ref { mutable, inner } => {
                let subst_inner = self.substitute_type_vars(inner, subst);
                Type::reference(subst_inner, *mutable)
            }
            TypeKind::Ptr { mutable, inner } => {
                let subst_inner = self.substitute_type_vars(inner, subst);
                Type::new(TypeKind::Ptr { mutable: *mutable, inner: subst_inner })
            }
            TypeKind::Array { element, size } => {
                let subst_elem = self.substitute_type_vars(element, subst);
                Type::array_with_const(subst_elem, size.clone())
            }
            TypeKind::Slice { element } => {
                let subst_elem = self.substitute_type_vars(element, subst);
                Type::slice(subst_elem)
            }
            // Primitives, Never, Error, Infer don't contain type variables to substitute
            _ => ty.clone(),
        }
    }

    /// Convert an AST type to an HIR type.
    pub(crate) fn ast_type_to_hir_type(&mut self, ty: &ast::Type) -> Result<Type, Box<TypeError>> {
        match &ty.kind {
            ast::TypeKind::Path(path) => {
                if path.segments.is_empty() {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::TypeNotFound { name: "empty path".to_string() },
                        ty.span,
                    )));
                }

                // Handle simple single-segment paths
                if path.segments.len() == 1 {
                    let name = self.symbol_to_string(path.segments[0].name.node);

                    // Check for built-in types first
                    match name.as_str() {
                        "i8" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8)))),
                        "i16" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16)))),
                        "i32" => return Ok(Type::i32()),
                        "i64" => return Ok(Type::i64()),
                        "i128" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128)))),
                        "isize" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::Isize)))),
                        "u8" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8)))),
                        "u16" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16)))),
                        "u32" => return Ok(Type::u32()),
                        "u64" => return Ok(Type::u64()),
                        "u128" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128)))),
                        "usize" => return Ok(Type::usize()),
                        "f32" => return Ok(Type::f32()),
                        "f64" => return Ok(Type::f64()),
                        "bool" => return Ok(Type::bool()),
                        "char" => return Ok(Type::char()),
                        "str" => return Ok(Type::str()),
                        "String" => return Ok(Type::string()),
                        "()" => return Ok(Type::unit()),
                        "Self" => {
                            // Look up self type in method context (type checking phase)
                            if let Some(fn_id) = self.current_fn {
                                if let Some(self_ty) = self.method_self_types.get(&fn_id) {
                                    return Ok(self_ty.clone());
                                }
                            }
                            // During collection phase, use current_impl_self_ty
                            if let Some(ref self_ty) = self.current_impl_self_ty {
                                return Ok(self_ty.clone());
                            }
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::TypeNotFound { name: "Self".to_string() },
                                ty.span,
                            )));
                        }
                        // Non-primitive type names: fall through to user-defined type lookup
                        _ => {}
                    }

                    // Check forall-bound type parameters first (innermost scope)
                    let segment_symbol = path.segments[0].name.node;
                    for (param_name, ty_var_id) in &self.forall_param_env {
                        if *param_name == segment_symbol {
                            return Ok(Type::new(TypeKind::Param(*ty_var_id)));
                        }
                    }

                    // Check generic type parameters in scope
                    if let Some(&ty_var_id) = self.generic_params.get(&name) {
                        return Ok(Type::new(TypeKind::Param(ty_var_id)));
                    }

                    // Look up user-defined types (structs, enums, type aliases)
                    if let Some(def_id) = self.resolver.lookup_type(&name) {
                        // Extract type arguments first
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

                        // Check if it's a type alias and expand it
                        if let Some(alias_info) = self.type_aliases.get(&def_id).cloned() {
                            // If the alias has generic parameters, substitute them
                            if !alias_info.generics.is_empty() && !type_args.is_empty() {
                                let subst: HashMap<TyVarId, Type> = alias_info.generics.iter()
                                    .zip(type_args.iter())
                                    .map(|(&var, ty)| (var, ty.clone()))
                                    .collect();
                                return Ok(self.substitute_type_vars(&alias_info.ty, &subst));
                            }
                            return Ok(alias_info.ty);
                        }

                        return Ok(Type::adt(def_id, type_args));
                    }

                    Err(self.error_type_not_found(&name, ty.span))
                } else if path.segments.len() == 2 {
                    // Two-segment path: Self::AssocType, Module::Type or Bridge::Type
                    let first_segment = self.symbol_to_string(path.segments[0].name.node);
                    let second_segment = self.symbol_to_string(path.segments[1].name.node);

                    // Handle Self::AssocType (associated type projection)
                    if first_segment == "Self" {
                        // Check current trait context first (for Self::Item in trait bodies)
                        for assoc_ty in &self.current_trait_assoc_types {
                            if assoc_ty.name == second_segment {
                                // In a trait body, Self::Item is abstract.
                                // If there's a default, use it; otherwise create a fresh type variable.
                                if let Some(ref default_ty) = assoc_ty.default {
                                    return Ok(default_ty.clone());
                                } else {
                                    let var_id = TyVarId::new(self.next_type_param_id);
                                    self.next_type_param_id += 1;
                                    return Ok(Type::param(var_id));
                                }
                            }
                        }

                        // Then check current_impl_assoc_types (for associated types in the
                        // impl block currently being collected)
                        for assoc_ty in &self.current_impl_assoc_types {
                            if assoc_ty.name == second_segment {
                                return Ok(assoc_ty.ty.clone());
                            }
                        }

                        // Look up the associated type from current impl context
                        // First, find the current impl block's trait
                        if let Some(ref self_ty) = self.current_impl_self_ty {
                            // Find the impl block for this self type that has this associated type
                            for impl_block in &self.impl_blocks {
                                if impl_block.self_ty == *self_ty {
                                    // Check if this impl has the associated type
                                    for assoc_ty in &impl_block.assoc_types {
                                        if assoc_ty.name == second_segment {
                                            return Ok(assoc_ty.ty.clone());
                                        }
                                    }
                                }
                            }
                        }
                        // If not in impl context, try method context
                        if let Some(fn_id) = self.current_fn {
                            if let Some(self_ty) = self.method_self_types.get(&fn_id).cloned() {
                                for impl_block in &self.impl_blocks {
                                    if impl_block.self_ty == self_ty {
                                        for assoc_ty in &impl_block.assoc_types {
                                            if assoc_ty.name == second_segment {
                                                return Ok(assoc_ty.ty.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::TypeNotFound {
                                name: format!("Self::{}", second_segment),
                            },
                            ty.span,
                        )));
                    }

                    // Module::Type or Bridge::Type
                    let module_name = first_segment;
                    let type_name = second_segment;

                    // First try to find the type's DefId in bridge definitions with matching namespace
                    // Store found def_id and whether it's opaque (no type args)
                    let mut found_def_id: Option<DefId> = None;
                    let mut is_opaque = false;
                    let mut alias_underlying_ty: Option<Type> = None;

                    for bridge_info in &self.bridge_defs {
                        if bridge_info.name == module_name {
                            // Check opaque types (no type args)
                            for opaque in &bridge_info.opaque_types {
                                if opaque.name == type_name {
                                    found_def_id = Some(opaque.def_id);
                                    is_opaque = true;
                                    break;
                                }
                            }
                            if found_def_id.is_some() { break; }

                            // Check type aliases - store the underlying type for expansion
                            for alias in &bridge_info.type_aliases {
                                if alias.name == type_name {
                                    found_def_id = Some(alias.def_id);
                                    alias_underlying_ty = Some(alias.ty.clone());
                                    break;
                                }
                            }
                            if found_def_id.is_some() { break; }

                            // Check structs
                            for struct_info in &bridge_info.structs {
                                if struct_info.name == type_name {
                                    found_def_id = Some(struct_info.def_id);
                                    break;
                                }
                            }
                            if found_def_id.is_some() { break; }

                            // Check enums
                            for enum_info in &bridge_info.enums {
                                if enum_info.name == type_name {
                                    found_def_id = Some(enum_info.def_id);
                                    break;
                                }
                            }
                            if found_def_id.is_some() { break; }

                            // Check unions
                            for union_info in &bridge_info.unions {
                                if union_info.name == type_name {
                                    found_def_id = Some(union_info.def_id);
                                    break;
                                }
                            }
                        }
                    }

                    // If found in bridge, return with type args
                    if let Some(def_id) = found_def_id {
                        if is_opaque {
                            // Opaque types don't have type arguments
                            return Ok(Type::adt(def_id, Vec::new()));
                        }
                        // If this is a type alias, return the underlying type
                        if let Some(underlying_ty) = alias_underlying_ty {
                            return Ok(underlying_ty);
                        }
                        // Extract type arguments if any (for structs/enums)
                        let type_args = if let Some(ref args) = path.segments[1].args {
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
                        return Ok(Type::adt(def_id, type_args));
                    }

                    // Check module_defs for regular modules (e.g., `mod helper;`)
                    for mod_info in self.module_defs.values() {
                        if mod_info.name == module_name {
                            // Search for type within this module's items
                            for &item_def_id in &mod_info.items {
                                if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                                    if def_info.name == type_name {
                                        match def_info.kind {
                                            hir::DefKind::Struct | hir::DefKind::Enum => {
                                                // Extract type arguments if any
                                                let type_args = if let Some(ref args) = path.segments[1].args {
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
                                                return Ok(Type::adt(item_def_id, type_args));
                                            }
                                            hir::DefKind::TypeAlias => {
                                                // Look up the alias and return its underlying type
                                                if let Some(alias_info) = self.type_aliases.get(&item_def_id).cloned() {
                                                    // Parse type arguments if provided (e.g., `helper::Pair<i32>`)
                                                    let type_args = if let Some(ref args) = path.segments[1].args {
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

                                                    // If the alias has generic parameters, substitute them
                                                    if !alias_info.generics.is_empty() && !type_args.is_empty() {
                                                        let subst: HashMap<TyVarId, Type> = alias_info.generics.iter()
                                                            .zip(type_args.iter())
                                                            .map(|(&var, ty)| (var, ty.clone()))
                                                            .collect();
                                                        return Ok(self.substitute_type_vars(&alias_info.ty, &subst));
                                                    }
                                                    return Ok(alias_info.ty.clone());
                                                }
                                                // Fallback to ADT if alias info not found (shouldn't happen)
                                                return Ok(Type::adt(item_def_id, Vec::new()));
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }

                        }
                    }

                    // Check re-exported items separately (from `pub use`)
                    // We need to extract the reexport info first to avoid borrow issues
                    let reexport_match: Option<(DefId, hir::DefKind, Option<super::TypeAliasInfo>)> = {
                        let mut result = None;
                        for mod_info in self.module_defs.values() {
                            if mod_info.name == module_name {
                                for (reexport_name, reexport_def_id, _vis) in &mod_info.reexports {
                                    if reexport_name == &type_name {
                                        if let Some(def_info) = self.resolver.def_info.get(reexport_def_id) {
                                            let alias_info = if def_info.kind == hir::DefKind::TypeAlias {
                                                self.type_aliases.get(reexport_def_id).cloned()
                                            } else {
                                                None
                                            };
                                            result = Some((*reexport_def_id, def_info.kind, alias_info));
                                            break;
                                        }
                                    }
                                }
                                break;
                            }
                        }
                        result
                    };

                    if let Some((reexport_def_id, def_kind, alias_info_opt)) = reexport_match {
                        match def_kind {
                            hir::DefKind::Struct | hir::DefKind::Enum => {
                                // Extract type arguments if any
                                let type_args = if let Some(ref args) = path.segments[1].args {
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
                                return Ok(Type::adt(reexport_def_id, type_args));
                            }
                            hir::DefKind::TypeAlias => {
                                if let Some(alias_info) = alias_info_opt {
                                    let type_args = if let Some(ref args) = path.segments[1].args {
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

                                    if !alias_info.generics.is_empty() && !type_args.is_empty() {
                                        let subst: HashMap<TyVarId, Type> = alias_info.generics.iter()
                                            .zip(type_args.iter())
                                            .map(|(&var, ty)| (var, ty.clone()))
                                            .collect();
                                        return Ok(self.substitute_type_vars(&alias_info.ty, &subst));
                                    }
                                    return Ok(alias_info.ty.clone());
                                }
                                return Ok(Type::adt(reexport_def_id, Vec::new()));
                            }
                            _ => {}
                        }
                    }

                    // Fall back to looking up type directly (for non-bridge modules)
                    if let Some(def_id) = self.resolver.lookup_type(&type_name) {
                        let type_args = if let Some(ref args) = path.segments[1].args {
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
                        return Ok(Type::adt(def_id, type_args));
                    }

                    Err(self.error_type_not_found(&format!("{}::{}", module_name, type_name), ty.span))
                } else {
                    // Multi-segment path: resolve through module hierarchy
                    // e.g., std::collections::Vec or foo::bar::Baz
                    self.resolve_multi_segment_type_path(path, ty.span)
                }
            }
            ast::TypeKind::Reference { mutable, lifetime: _, inner } => {
                let inner_ty = self.ast_type_to_hir_type(inner)?;
                Ok(Type::reference(inner_ty, *mutable))
            }
            ast::TypeKind::Pointer { mutable, inner } => {
                let inner_ty = self.ast_type_to_hir_type(inner)?;
                Ok(Type::new(TypeKind::Ptr { mutable: *mutable, inner: inner_ty }))
            }
            ast::TypeKind::Array { element, size } => {
                let elem_ty = self.ast_type_to_hir_type(element)?;

                // First, check if the size is a const generic parameter
                if let ast::ExprKind::Path(path) = &size.kind {
                    if path.segments.len() == 1 {
                        let symbol = path.segments[0].name.node;
                        if let Some(name) = self.interner.resolve(symbol) {
                            // Check if it's a const generic parameter
                            if let Some(&const_param_id) = self.const_params.get(name) {
                                // Return array type with const param
                                return Ok(Type::array_with_const(elem_ty, hir::ConstValue::Param(const_param_id)));
                            }
                        }
                    }
                }

                // If not a const param, try to evaluate as a const expression
                let const_values = &self.const_values;
                let resolver = &self.resolver;
                let interner = &self.interner;
                let lookup = |path: &ast::ExprPath| -> Option<const_eval::ConstResult> {
                    // Only handle single-segment paths (simple const names)
                    if path.segments.len() == 1 {
                        let symbol = path.segments[0].name.node;
                        if let Some(name) = interner.resolve(symbol) {
                            // Look up the DefId for this name
                            if let Some(Binding::Def(def_id)) = resolver.lookup(name) {
                                // Check if we have an evaluated value for this const
                                return const_values.get(&def_id).copied();
                            }
                        }
                    }
                    None
                };
                let size = const_eval::eval_const_expr_with_lookup(size, &lookup)?
                    .as_u64()
                    .ok_or_else(|| Box::new(TypeError::new(
                        TypeErrorKind::ConstEvalError {
                            reason: "array size must be a non-negative integer".to_string(),
                        },
                        ty.span,
                    )))?;
                Ok(Type::array(elem_ty, size))
            }
            ast::TypeKind::Slice { element } => {
                let elem_ty = self.ast_type_to_hir_type(element)?;
                Ok(Type::slice(elem_ty))
            }
            ast::TypeKind::Tuple(elements) => {
                let elem_types: Vec<Type> = elements.iter()
                    .map(|e| self.ast_type_to_hir_type(e))
                    .collect::<Result<_, _>>()?;
                Ok(Type::tuple(elem_types))
            }
            ast::TypeKind::Function { params, return_type, effects } => {
                let param_types: Vec<Type> = params.iter()
                    .map(|p| self.ast_type_to_hir_type(p))
                    .collect::<Result<_, _>>()?;
                let ret_ty = self.ast_type_to_hir_type(return_type)?;
                // Parse effect annotations and convert to FnEffect
                let fn_effects = if let Some(effect_row) = effects {
                    let (effect_refs, _row_var) = self.parse_effect_row(effect_row)?;
                    effect_refs.into_iter()
                        .map(|er| hir::FnEffect::new(er.def_id, er.type_args))
                        .collect()
                } else {
                    Vec::new()
                };
                Ok(Type::function_with_effects(param_types, ret_ty, fn_effects))
            }
            ast::TypeKind::Never => Ok(Type::never()),
            ast::TypeKind::Infer => Ok(self.unifier.fresh_var()),
            ast::TypeKind::Paren(inner) => self.ast_type_to_hir_type(inner),
            ast::TypeKind::Record { fields, rest } => {
                // Convert fields from AST to HIR
                let hir_fields: Vec<hir::RecordField> = fields.iter()
                    .map(|f| {
                        let field_ty = self.ast_type_to_hir_type(&f.ty)?;
                        Ok(hir::RecordField {
                            name: f.name.node,
                            ty: field_ty,
                        })
                    })
                    .collect::<Result<_, Box<TypeError>>>()?;

                // If there's a rest (row variable), create a fresh row variable
                let row_var = rest.as_ref().map(|_| self.unifier.fresh_row_var());

                Ok(Type::record(hir_fields, row_var))
            }
            ast::TypeKind::Forall { params, body } => {
                // Create fresh type variable IDs for each forall parameter
                // These are special: they represent universally quantified variables
                let hir_params: Vec<hir::TyVarId> = params
                    .iter()
                    .map(|_| self.unifier.fresh_forall_var())
                    .collect();

                // Store mapping from parameter names to their TyVarIds for body conversion
                // We need to temporarily extend the type environment
                let mut param_env: Vec<(ast::Symbol, hir::TyVarId)> = Vec::new();
                for (name, id) in params.iter().zip(hir_params.iter()) {
                    param_env.push((name.node, *id));
                }

                // Save old forall params and set new ones
                let old_forall_params = std::mem::take(&mut self.forall_param_env);
                self.forall_param_env = param_env;

                // Convert the body type with forall params in scope
                let body_ty = self.ast_type_to_hir_type(body)?;

                // Restore old forall params
                self.forall_param_env = old_forall_params;

                Ok(Type::forall(hir_params, body_ty))
            }
            ast::TypeKind::Ownership { qualifier, inner } => {
                // Convert AST ownership qualifier to HIR
                let hir_qualifier = match qualifier {
                    ast::OwnershipQualifier::Linear => hir::ty::OwnershipQualifier::Linear,
                    ast::OwnershipQualifier::Affine => hir::ty::OwnershipQualifier::Affine,
                };
                // Recursively convert the inner type
                let inner_ty = self.ast_type_to_hir_type(inner)?;
                Ok(Type::ownership(hir_qualifier, inner_ty))
            }
        }
    }

    /// Resolve a multi-segment type path through the module hierarchy.
    ///
    /// For paths like `std::collections::Vec`, this:
    /// 1. Looks up `std` as a module
    /// 2. Within that module, looks up `collections` as a submodule
    /// 3. Within that, looks up `Vec` as a type
    fn resolve_multi_segment_type_path(
        &mut self,
        path: &ast::TypePath,
        span: Span,
    ) -> Result<Type, Box<TypeError>> {
        let segments: Vec<String> = path.segments.iter()
            .map(|s| self.symbol_to_string(s.name.node))
            .collect();

        // We need at least 3 segments for this function (1 and 2 are handled elsewhere)
        if segments.len() < 3 {
            return Err(self.error_type_not_found(&segments.join("::"), span));
        }

        // Start by looking up the first segment as a module
        let first_name = &segments[0];
        let mut current_module_def_id = self.resolver.lookup_type(first_name)
            .or_else(|| {
                // Also check if it's a module by name
                self.module_defs.iter()
                    .find(|(_, info)| info.name == *first_name)
                    .map(|(def_id, _)| *def_id)
            })
            .ok_or_else(|| Box::new(TypeError::new(
                TypeErrorKind::ModuleNotFound {
                    name: first_name.clone(),
                    searched_paths: vec![first_name.clone()],
                },
                span,
            )))?;

        // Navigate through intermediate segments (all should be modules)
        for segment_name in &segments[1..segments.len()-1] {
            let module_info = self.module_defs.get(&current_module_def_id).cloned()
                .ok_or_else(|| Box::new(TypeError::new(
                    TypeErrorKind::TypeNotFound {
                        name: format!("{} is not a module", segment_name),
                    },
                    span,
                )))?;

            // Find the next segment within this module's items
            let mut found = false;
            for &item_def_id in &module_info.items {
                if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                    if def_info.name == *segment_name {
                        // Check if this is a module (for intermediate segments)
                        if matches!(def_info.kind, hir::DefKind::Mod) {
                            current_module_def_id = item_def_id;
                            found = true;
                            break;
                        }
                    }
                }
            }

            if !found {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::TypeNotFound {
                        name: format!("{}::{}", module_info.name, segment_name),
                    },
                    span,
                )));
            }
        }

        // Now look up the final segment as a type within the last module
        // SAFETY: segments.len() >= 3 guaranteed by check at line 3817
        let type_name = segments.last().expect("BUG: segments guaranteed non-empty by guard at function start");
        let module_info = self.module_defs.get(&current_module_def_id).cloned()
            .ok_or_else(|| Box::new(TypeError::new(
                TypeErrorKind::TypeNotFound {
                    name: segments.join("::"),
                },
                span,
            )))?;

        // Find the type in the module's items
        for &item_def_id in &module_info.items {
            if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                if def_info.name == *type_name {
                    // Check if it's a type (struct, enum, type alias)
                    match def_info.kind {
                        hir::DefKind::Struct | hir::DefKind::Enum => {
                            // Extract type arguments from the last segment
                            let type_args = if let Some(args) = path.segments.last().and_then(|s| s.args.as_ref()) {
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
                            return Ok(Type::adt(item_def_id, type_args));
                        }
                        hir::DefKind::TypeAlias => {
                            // Look up and return the aliased type
                            if let Some(alias_info) = self.type_aliases.get(&item_def_id).cloned() {
                                // Extract type arguments from the last segment
                                let type_args = if let Some(args) = path.segments.last().and_then(|s| s.args.as_ref()) {
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

                                // If the alias has generic parameters, substitute them
                                if !alias_info.generics.is_empty() && !type_args.is_empty() {
                                    let subst: HashMap<TyVarId, Type> = alias_info.generics.iter()
                                        .zip(type_args.iter())
                                        .map(|(&var, ty)| (var, ty.clone()))
                                        .collect();
                                    return Ok(self.substitute_type_vars(&alias_info.ty, &subst));
                                }
                                return Ok(alias_info.ty);
                            }
                            return Ok(Type::adt(item_def_id, Vec::new()));
                        }
                        _ => {}
                    }
                }
            }
        }

        Err(Box::new(TypeError::new(
            TypeErrorKind::TypeNotFound {
                name: segments.join("::"),
            },
            span,
        )))
    }

    /// Infer type of an index expression.
    pub(crate) fn infer_index(
        &mut self,
        base: &ast::Expr,
        index: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let base_expr = self.infer_expr(base)?;
        let index_expr = self.infer_expr(index)?;

        // Check that index is an integer type
        match index_expr.ty.kind() {
            TypeKind::Primitive(PrimitiveTy::Int(_) | PrimitiveTy::Uint(_)) => {}
            TypeKind::Infer(_) => {
                self.unifier.unify(&index_expr.ty, &Type::i32(), span)?;
            }
            _ => {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::NotIndexable { ty: index_expr.ty.clone() },
                    span,
                )));
            }
        }

        // Determine element type based on base type
        let elem_ty = match base_expr.ty.kind() {
            TypeKind::Array { element, .. } => element.clone(),
            TypeKind::Slice { element } => element.clone(),
            TypeKind::Primitive(PrimitiveTy::Str) => {
                // String indexing returns char (by character index)
                // Note: This uses character-based indexing (O(n)), not byte-based
                Type::char()
            }
            TypeKind::Primitive(PrimitiveTy::String) => {
                // String indexing returns char
                Type::char()
            }
            TypeKind::Adt { def_id, args } => {
                // Vec<T> indexing returns T
                if Some(*def_id) == self.vec_def_id {
                    if !args.is_empty() {
                        args[0].clone()
                    } else {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                            span,
                        )));
                    }
                } else {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                        span,
                    )));
                }
            }
            TypeKind::Ref { inner, .. } => {
                match inner.kind() {
                    TypeKind::Array { element, .. } => element.clone(),
                    TypeKind::Slice { element } => element.clone(),
                    TypeKind::Primitive(PrimitiveTy::Str) => Type::char(),
                    TypeKind::Primitive(PrimitiveTy::String) => Type::char(),
                    TypeKind::Adt { def_id, args } => {
                        // &Vec<T> indexing returns T
                        if Some(*def_id) == self.vec_def_id {
                            if !args.is_empty() {
                                args[0].clone()
                            } else {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                                    span,
                                )));
                            }
                        } else {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                                span,
                            )));
                        }
                    }
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                            span,
                        )));
                    }
                }
            }
            _ => {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::NotIndexable { ty: base_expr.ty.clone() },
                    span,
                )));
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
    pub(crate) fn infer_array(
        &mut self,
        array_expr: &ast::ArrayExpr,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        match array_expr {
            ast::ArrayExpr::List(elements) => {
                if elements.is_empty() {
                    let elem_ty = self.unifier.fresh_var();
                    return Ok(hir::Expr::new(
                        hir::ExprKind::Array(vec![]),
                        Type::array(elem_ty, 0),
                        span,
                    ));
                }

                let mut hir_elements = Vec::with_capacity(elements.len());
                let first_elem = self.infer_expr(&elements[0])?;
                let elem_ty = first_elem.ty.clone();
                hir_elements.push(first_elem);

                for elem in &elements[1..] {
                    let elem_expr = self.infer_expr(elem)?;
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
                let value_expr = self.infer_expr(value)?;
                let count_expr = self.infer_expr(count)?;

                self.unifier.unify(&count_expr.ty, &Type::i32(), count.span)?;

                let size = match const_eval::eval_const_expr(count) {
                    Ok(result) => result.as_u64().ok_or_else(|| {
                        TypeError::new(
                            TypeErrorKind::ConstEvalError {
                                reason: "array size must be a non-negative integer that fits in u64".to_string(),
                            },
                            count.span,
                        )
                    })?,
                    Err(e) => return Err(e),
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
    pub(crate) fn infer_literal(&mut self, lit: &ast::Literal, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        let (value, ty) = match &lit.kind {
            ast::LiteralKind::Int { value, suffix } => {
                let ty = match suffix {
                    Some(ast::IntSuffix::I8) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8))),
                    Some(ast::IntSuffix::I16) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16))),
                    Some(ast::IntSuffix::I32) => Type::i32(),
                    Some(ast::IntSuffix::I64) => Type::i64(),
                    Some(ast::IntSuffix::I128) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128))),
                    Some(ast::IntSuffix::Isize) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::Isize))),
                    Some(ast::IntSuffix::U8) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8))),
                    Some(ast::IntSuffix::U16) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16))),
                    Some(ast::IntSuffix::U32) => Type::u32(),
                    Some(ast::IntSuffix::U64) => Type::u64(),
                    Some(ast::IntSuffix::U128) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128))),
                    Some(ast::IntSuffix::Usize) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::Usize))),
                    None => Type::i32(),
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
            ast::LiteralKind::String(s) => {
                // String literals are &str (reference to str), like in Rust
                (hir::LiteralValue::String(s.clone()), Type::reference(Type::str(), false))
            }
            ast::LiteralKind::ByteString(bytes) => {
                // Byte string literals are &[u8] (reference to u8 slice), like in Rust
                let u8_ty = Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8)));
                let slice_ty = Type::slice(u8_ty);
                (hir::LiteralValue::ByteString(bytes.clone()), Type::reference(slice_ty, false))
            }
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Literal(value),
            ty,
            span,
        ))
    }

    /// Check a literal against an expected type.
    /// This allows unsuffixed numeric literals to be coerced to the expected type.
    pub(crate) fn check_literal(&mut self, lit: &ast::Literal, expected: &Type, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        use crate::hir::def::{FloatTy, IntTy, UintTy};

        let resolved_expected = self.unifier.resolve(expected);
        let (value, ty) = match &lit.kind {
            // For unsuffixed integer literals, use expected type if it's an integer type
            ast::LiteralKind::Int { value, suffix: None } => {
                let ty = match resolved_expected.kind() {
                    TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8)) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I8))),
                    TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16)) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I16))),
                    TypeKind::Primitive(PrimitiveTy::Int(IntTy::I32)) => Type::i32(),
                    TypeKind::Primitive(PrimitiveTy::Int(IntTy::I64)) => Type::i64(),
                    TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128)) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I128))),
                    TypeKind::Primitive(PrimitiveTy::Int(IntTy::Isize)) => Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::Isize))),
                    TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8)) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8))),
                    TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16)) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16))),
                    TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U32)) => Type::u32(),
                    TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U64)) => Type::u64(),
                    TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128)) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128))),
                    TypeKind::Primitive(PrimitiveTy::Uint(UintTy::Usize)) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::Usize))),
                    // Default to i32 if expected type is not an integer type
                    _ => Type::i32(),
                };
                (hir::LiteralValue::Int(*value as i128), ty)
            }
            // For unsuffixed float literals, use expected type if it's a float type
            ast::LiteralKind::Float { value, suffix: None } => {
                let ty = match resolved_expected.kind() {
                    TypeKind::Primitive(PrimitiveTy::Float(FloatTy::F32)) => Type::f32(),
                    TypeKind::Primitive(PrimitiveTy::Float(FloatTy::F64)) => Type::f64(),
                    // Default to f64 if expected type is not a float type
                    _ => Type::f64(),
                };
                (hir::LiteralValue::Float(value.0), ty)
            }
            // For suffixed literals or other types, use regular inference
            // but still unify with expected type to bind inference variables
            _ => {
                let inferred = self.infer_literal(lit, span)?;
                self.unifier.unify(expected, &inferred.ty, span)?;
                return Ok(inferred);
            }
        };

        // The type is now compatible with expected, but unify anyway to catch edge cases
        self.unifier.unify(expected, &ty, span)?;

        Ok(hir::Expr::new(
            hir::ExprKind::Literal(value),
            ty,
            span,
        ))
    }

    /// Infer type of a path (variable/function reference).
    pub(crate) fn infer_path(&mut self, path: &ast::ExprPath, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        if path.segments.len() == 1 {
            let name = self.symbol_to_string(path.segments[0].name.node);

            // Check const generic parameters first (e.g., `N` in `fn foo<const N: usize>`)
            if let Some(&const_param_id) = self.const_params.get(&name) {
                return Ok(hir::Expr::new(
                    hir::ExprKind::ConstParam(const_param_id),
                    Type::usize(),
                    span,
                ));
            }

            match self.resolver.lookup(&name) {
                Some(Binding::Local { local_id, ty, .. }) => {
                    Ok(hir::Expr::new(
                        hir::ExprKind::Local(local_id),
                        ty,
                        span,
                    ))
                }
                Some(Binding::Def(def_id)) => {
                    if let Some(sig) = self.fn_sigs.get(&def_id).cloned() {
                        // Get effect annotations for this function
                        let fn_effects: Vec<hir::FnEffect> = self.fn_effects.get(&def_id)
                            .map(|effect_refs| effect_refs.iter()
                                .map(|er| hir::FnEffect::new(er.def_id, er.type_args.clone()))
                                .collect())
                            .unwrap_or_default();

                        // Check for explicit type arguments (turbofish syntax)
                        let explicit_const_args: Vec<(hir::ConstParamId, hir::ConstValue)> =
                            if let Some(ref args) = path.segments[0].args {
                                let mut const_args = Vec::new();
                                let mut const_idx = 0usize;
                                for arg in &args.args {
                                    if let ast::TypeArg::Const(expr) = arg {
                                        // Get the const param id from the signature
                                        if const_idx < sig.const_generics.len() {
                                            let const_param_id = sig.const_generics[const_idx];
                                            // Evaluate the const expression
                                            if let Some(val) = self.evaluate_const_expr(expr) {
                                                const_args.push((const_param_id, val));
                                            }
                                            const_idx += 1;
                                        }
                                    }
                                }
                                const_args
                            } else {
                                Vec::new()
                            };

                        let fn_ty = self.instantiate_fn_sig_with_effects(&sig, fn_effects);

                        // If we have explicit const args, add them to the function type
                        let fn_ty = if !explicit_const_args.is_empty() {
                            match fn_ty.kind() {
                                hir::TypeKind::Fn { params, ret, effects, .. } => {
                                    Type::function_with_const_args(
                                        params.clone(),
                                        ret.clone(),
                                        effects.clone(),
                                        explicit_const_args,
                                    )
                                }
                                _ => fn_ty,
                            }
                        } else {
                            fn_ty
                        };

                        Ok(hir::Expr::new(
                            hir::ExprKind::Def(def_id),
                            fn_ty,
                            span,
                        ))
                    } else if let Some(def_info) = self.resolver.def_info.get(&def_id).cloned() {
                        // Check if this is a variant constructor (e.g., Some, None, Ok, Err)
                        if def_info.kind == hir::DefKind::Variant {
                            if let Some(parent_def_id) = def_info.parent {
                                if let Some(enum_info) = self.enum_defs.get(&parent_def_id).cloned() {
                                    // Find the variant info
                                    if let Some(variant) = enum_info.variants.iter().find(|v| v.def_id == def_id) {
                                        let variant_idx = variant.index;
                                        let variant_fields = variant.fields.clone();

                                        if variant_fields.is_empty() {
                                            // Unit variant (like None) - returns the enum type directly
                                            let type_args: Vec<Type> = enum_info.generics.iter()
                                                .map(|_| self.unifier.fresh_var())
                                                .collect();
                                            let enum_ty = Type::adt(parent_def_id, type_args);

                                            return Ok(hir::Expr::new(
                                                hir::ExprKind::Variant {
                                                    def_id: parent_def_id,
                                                    variant_idx,
                                                    fields: vec![],
                                                },
                                                enum_ty,
                                                span,
                                            ));
                                        } else {
                                            // Tuple variant (like Some(T)) - returns a function type
                                            let type_args: Vec<Type> = enum_info.generics.iter()
                                                .map(|_| self.unifier.fresh_var())
                                                .collect();

                                            // Build substitution map from generic params to fresh vars
                                            let subst: std::collections::HashMap<TyVarId, Type> = enum_info.generics.iter()
                                                .zip(type_args.iter())
                                                .map(|(&tyvar, ty)| (tyvar, ty.clone()))
                                                .collect();

                                            // Substitute type parameters in field types
                                            let field_types: Vec<Type> = variant_fields.iter()
                                                .map(|f| self.substitute_type_vars(&f.ty, &subst))
                                                .collect();

                                            let enum_ty = Type::adt(parent_def_id, type_args);
                                            let fn_ty = Type::function(field_types, enum_ty);

                                            return Ok(hir::Expr::new(
                                                hir::ExprKind::Def(def_id),
                                                fn_ty,
                                                span,
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        // Default handling for non-variant defs
                        let ty = if let Some(ty) = &def_info.ty {
                            ty.clone()
                        } else {
                            self.unifier.fresh_var()
                        };
                        Ok(hir::Expr::new(
                            hir::ExprKind::Def(def_id),
                            ty,
                            span,
                        ))
                    } else {
                        Err(self.error_name_not_found(&name, span))
                    }
                }
                Some(Binding::Methods(method_def_ids)) => {
                    // Multiple dispatch: method family with several overloads
                    // Create a MethodFamily expression that will be resolved at call site
                    // For now, return the first method's type as a placeholder
                    // The actual dispatch happens in check_call_expr
                    if let Some(&first_def_id) = method_def_ids.first() {
                        if let Some(sig) = self.fn_sigs.get(&first_def_id).cloned() {
                            let fn_ty = if sig.generics.is_empty() {
                                Type::function(sig.inputs.clone(), sig.output.clone())
                            } else {
                                self.instantiate_fn_sig(&sig)
                            };
                            // Store the method family for later dispatch resolution
                            Ok(hir::Expr::new(
                                hir::ExprKind::MethodFamily {
                                    name: name.clone(),
                                    candidates: method_def_ids.clone(),
                                },
                                fn_ty,
                                span,
                            ))
                        } else {
                            Err(self.error_name_not_found(&name, span))
                        }
                    } else {
                        Err(self.error_name_not_found(&name, span))
                    }
                }
                None => {
                    Err(self.error_name_not_found(&name, span))
                }
            }
        } else if path.segments.len() == 2 {
            let first_name = self.symbol_to_string(path.segments[0].name.node);
            let second_name = self.symbol_to_string(path.segments[1].name.node);

            if let Some(type_def_id) = self.resolver.lookup_type(&first_name) {
                if let Some(enum_info) = self.enum_defs.get(&type_def_id).cloned() {
                    if let Some(variant) = enum_info.variants.iter().find(|v| v.name == second_name) {
                        let variant_idx = variant.index;
                        let variant_def_id = variant.def_id;
                        let variant_fields = variant.fields.clone();

                        if variant_fields.is_empty() {
                            let type_args: Vec<Type> = enum_info.generics.iter()
                                .map(|_| self.unifier.fresh_var())
                                .collect();
                            let enum_ty = Type::adt(type_def_id, type_args);

                            return Ok(hir::Expr::new(
                                hir::ExprKind::Variant {
                                    def_id: type_def_id,
                                    variant_idx,
                                    fields: vec![],
                                },
                                enum_ty,
                                span,
                            ));
                        } else {
                            // Create fresh type variables for each generic parameter
                            let type_args: Vec<Type> = enum_info.generics.iter()
                                .map(|_| self.unifier.fresh_var())
                                .collect();

                            // Build substitution map from generic params to fresh vars
                            let subst: std::collections::HashMap<TyVarId, Type> = enum_info.generics.iter()
                                .zip(type_args.iter())
                                .map(|(&tyvar, ty)| (tyvar, ty.clone()))
                                .collect();

                            // Substitute type parameters in field types
                            let field_types: Vec<Type> = variant_fields.iter()
                                .map(|f| self.substitute_type_vars(&f.ty, &subst))
                                .collect();

                            let enum_ty = Type::adt(type_def_id, type_args);

                            let fn_ty = Type::function(field_types, enum_ty);

                            return Ok(hir::Expr::new(
                                hir::ExprKind::Def(variant_def_id),
                                fn_ty,
                                span,
                            ));
                        }
                    } else {
                        // Not an enum variant - check for associated function in impl blocks
                        let self_ty = Type::adt(type_def_id, Vec::new());

                        // Find the method def_id, signature, and impl block generics
                        let mut found_method: Option<(DefId, hir::FnSig, Vec<TyVarId>)> = None;
                        for impl_block in &self.impl_blocks {
                            // Only check inherent impls (not trait impls)
                            if impl_block.trait_ref.is_some() {
                                continue;
                            }
                            // Check if impl block applies to this type
                            if !self.types_match_for_impl(&impl_block.self_ty, &self_ty) {
                                continue;
                            }
                            // Look for a method with matching name
                            for method in &impl_block.methods {
                                if method.name == second_name {
                                    // Found the associated function!
                                    if let Some(sig) = self.fn_sigs.get(&method.def_id).cloned() {
                                        found_method = Some((method.def_id, sig, impl_block.generics.clone()));
                                        break;
                                    }
                                }
                            }
                            if found_method.is_some() {
                                break;
                            }
                        }

                        // Process the found method outside the borrow
                        if let Some((method_def_id, sig, impl_generics)) = found_method {
                            // Combine impl-level and method-level generics for instantiation
                            let all_generics: Vec<TyVarId> = impl_generics.iter()
                                .chain(sig.generics.iter())
                                .copied()
                                .collect();

                            let fn_ty = if all_generics.is_empty() {
                                Type::function(sig.inputs.clone(), sig.output.clone())
                            } else {
                                // Create fresh type vars for all generics (impl + method)
                                let mut substitution: HashMap<TyVarId, Type> = HashMap::new();
                                for &old_var in &all_generics {
                                    let fresh_var = self.unifier.fresh_var();
                                    substitution.insert(old_var, fresh_var);
                                }

                                // Substitute in parameter types
                                let subst_inputs: Vec<Type> = sig.inputs.iter()
                                    .map(|ty| self.substitute_type_vars(ty, &substitution))
                                    .collect();

                                // Substitute in return type
                                let subst_output = self.substitute_type_vars(&sig.output, &substitution);

                                Type::function(subst_inputs, subst_output)
                            };
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Def(method_def_id),
                                fn_ty,
                                span,
                            ));
                        }

                        // Fall through to error if no method found
                    }
                } else if self.struct_defs.contains_key(&type_def_id) {
                    // It's a struct - check for associated functions in impl blocks
                    let self_ty = Type::adt(type_def_id, Vec::new());

                    // Find the method def_id, signature, and impl block generics (to avoid borrow issues)
                    let mut found_method: Option<(DefId, hir::FnSig, Vec<TyVarId>)> = None;
                    for impl_block in &self.impl_blocks {
                        // Only check inherent impls (not trait impls)
                        if impl_block.trait_ref.is_some() {
                            continue;
                        }
                        // Check if impl block applies to this type
                        if !self.types_match_for_impl(&impl_block.self_ty, &self_ty) {
                            continue;
                        }
                        // Look for a method with matching name
                        for method in &impl_block.methods {
                            if method.name == second_name {
                                // Found the associated function!
                                if let Some(sig) = self.fn_sigs.get(&method.def_id).cloned() {
                                    found_method = Some((method.def_id, sig, impl_block.generics.clone()));
                                    break;
                                }
                            }
                        }
                        if found_method.is_some() {
                            break;
                        }
                    }

                    // Process the found method outside the borrow
                    if let Some((method_def_id, sig, impl_generics)) = found_method {
                        // Combine impl-level and method-level generics for instantiation
                        let all_generics: Vec<TyVarId> = impl_generics.iter()
                            .chain(sig.generics.iter())
                            .copied()
                            .collect();

                        let fn_ty = if all_generics.is_empty() {
                            Type::function(sig.inputs.clone(), sig.output.clone())
                        } else {
                            // Create fresh type vars for all generics (impl + method)
                            let mut substitution: HashMap<TyVarId, Type> = HashMap::new();
                            for &old_var in &all_generics {
                                let fresh_var = self.unifier.fresh_var();
                                substitution.insert(old_var, fresh_var);
                            }

                            // Substitute in parameter types
                            let subst_inputs: Vec<Type> = sig.inputs.iter()
                                .map(|ty| self.substitute_type_vars(ty, &substitution))
                                .collect();

                            // Substitute in return type
                            let subst_output = self.substitute_type_vars(&sig.output, &substitution);

                            Type::function(subst_inputs, subst_output)
                        };
                        return Ok(hir::Expr::new(
                            hir::ExprKind::Def(method_def_id),
                            fn_ty,
                            span,
                        ));
                    }

                    // Type found but no impl block method - check builtin static methods
                    if let Some((method_def_id, fn_ty)) = self.find_builtin_static_method(&first_name, &second_name) {
                        return Ok(hir::Expr::new(
                            hir::ExprKind::Def(method_def_id),
                            fn_ty,
                            span,
                        ));
                    }

                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                        span,
                    )));
                }
            }

            // Check for builtin type static methods (e.g., String::new(), Vec::new())
            if let Some((method_def_id, fn_ty)) = self.find_builtin_static_method(&first_name, &second_name) {
                return Ok(hir::Expr::new(
                    hir::ExprKind::Def(method_def_id),
                    fn_ty,
                    span,
                ));
            }

            // Check for module namespace paths (module_name::item_name)
            // First look up the module by name
            if let Some(Binding::Def(module_def_id)) = self.resolver.lookup(&first_name) {
                // Check if this is a module definition
                if let Some(module_info) = self.module_defs.get(&module_def_id).cloned() {
                    // Look for a function with this name in the module's items
                    for &item_def_id in &module_info.items {
                        if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                            let item_name = def_info.name.clone();
                            if item_name == second_name {
                                // Found the item - check what kind it is
                                if let Some(sig) = self.fn_sigs.get(&item_def_id).cloned() {
                                    // It's a function
                                    let fn_ty = if sig.generics.is_empty() {
                                        Type::function(sig.inputs.clone(), sig.output.clone())
                                    } else {
                                        self.instantiate_fn_sig(&sig)
                                    };
                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Def(item_def_id),
                                        fn_ty,
                                        span,
                                    ));
                                } else if let Some(ty) = &def_info.ty {
                                    // It's some other kind of definition with a type
                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Def(item_def_id),
                                        ty.clone(),
                                        span,
                                    ));
                                }
                            }
                        }
                    }
                    // Module found but item not found
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                        span,
                    )));
                }
            }

            // Check for FFI bridge namespace paths (bridge_name::function_name)
            for bridge_info in &self.bridge_defs {
                if bridge_info.name == first_name {
                    // Look for a function with this name in the bridge
                    for fn_info in &bridge_info.extern_fns {
                        if fn_info.name == second_name {
                            // Found the FFI function - return it as a function expression
                            let fn_ty = Type::function(
                                fn_info.params.clone(),
                                fn_info.return_ty.clone(),
                            );
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Def(fn_info.def_id),
                                fn_ty,
                                span,
                            ));
                        }
                    }
                    // Look for constants in the bridge
                    for const_info in &bridge_info.consts {
                        if const_info.name == second_name {
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Def(const_info.def_id),
                                const_info.ty.clone(),
                                span,
                            ));
                        }
                    }
                    // Bridge found but item not found
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                        span,
                    )));
                }
            }

            Err(Box::new(TypeError::new(
                TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                span,
            )))
        } else if path.segments.len() == 3 {
            // Three-segment path: module::Type::method, module::Enum::Variant, or module::submodule::item
            let module_name = self.symbol_to_string(path.segments[0].name.node);
            let type_name = self.symbol_to_string(path.segments[1].name.node);
            let third_name = self.symbol_to_string(path.segments[2].name.node);

            // First, find the module
            let mut module_def_id: Option<DefId> = None;
            for (def_id, info) in &self.module_defs {
                if info.name == module_name {
                    module_def_id = Some(*def_id);
                    break;
                }
            }

            if let Some(mod_def_id) = module_def_id {
                // Check if second segment is a nested submodule
                // This handles module::submodule::function pattern
                let mut submodule_def_id: Option<DefId> = None;
                if let Some(mod_info) = self.module_defs.get(&mod_def_id) {
                    for &item_def_id in &mod_info.items {
                        if let Some(submod_info) = self.module_defs.get(&item_def_id) {
                            if submod_info.name == type_name {
                                submodule_def_id = Some(item_def_id);
                                break;
                            }
                        }
                    }
                }

                if let Some(submod_def_id) = submodule_def_id {
                    // Found a submodule, look for the item in it
                    if let Some(submod_info) = self.module_defs.get(&submod_def_id) {
                        // Look for a function with this name
                        for &item_def_id in &submod_info.items {
                            if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                                if def_info.name == third_name {
                                    if let Some(fn_sig) = self.fn_sigs.get(&item_def_id).cloned() {
                                        // Found the function!
                                        let fn_ty = if fn_sig.generics.is_empty() {
                                            Type::function(fn_sig.inputs.clone(), fn_sig.output.clone())
                                        } else {
                                            // Create fresh type vars for generics
                                            let mut substitution: HashMap<TyVarId, Type> = HashMap::new();
                                            for &old_var in &fn_sig.generics {
                                                let fresh_var = self.unifier.fresh_var();
                                                substitution.insert(old_var, fresh_var);
                                            }

                                            let subst_inputs: Vec<Type> = fn_sig.inputs.iter()
                                                .map(|ty| self.substitute_type_vars(ty, &substitution))
                                                .collect();
                                            let subst_output = self.substitute_type_vars(&fn_sig.output, &substitution);

                                            Type::function(subst_inputs, subst_output)
                                        };

                                        return Ok(hir::Expr::new(
                                            hir::ExprKind::Def(item_def_id),
                                            fn_ty,
                                            span,
                                        ));
                                    }
                                }
                            }
                        }

                        // Look for a struct type
                        for &item_def_id in &submod_info.items {
                            if let Some(struct_info) = self.struct_defs.get(&item_def_id) {
                                if struct_info.name == third_name {
                                    // Return the struct type (for use in expressions like type annotation)
                                    let type_args: Vec<Type> = struct_info.generics.iter()
                                        .map(|_| self.unifier.fresh_var())
                                        .collect();
                                    let struct_ty = Type::adt(item_def_id, type_args);

                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Def(item_def_id),
                                        struct_ty,
                                        span,
                                    ));
                                }
                            }
                        }

                        // Look for a constant
                        for &item_def_id in &submod_info.items {
                            if let Some(const_info) = self.const_defs.get(&item_def_id) {
                                if const_info.name == third_name {
                                    return Ok(hir::Expr::new(
                                        hir::ExprKind::Def(item_def_id),
                                        const_info.ty.clone(),
                                        span,
                                    ));
                                }
                            }
                        }

                        // Item not found in submodule
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotFound { name: format!("{}::{}::{}", module_name, type_name, third_name) },
                            span,
                        )));
                    }
                }

                // Not a submodule, check if the second segment is an enum and third is a variant
                // This handles module::Enum::Variant pattern
                // First, collect enum info from both items and reexports to avoid borrow issues
                let enum_match: Option<(DefId, super::EnumInfo)> = {
                    let mut result = None;
                    if let Some(mod_info) = self.module_defs.get(&mod_def_id) {
                        // Check direct items
                        for &item_def_id in &mod_info.items {
                            if let Some(enum_info) = self.enum_defs.get(&item_def_id) {
                                if enum_info.name == type_name {
                                    result = Some((item_def_id, enum_info.clone()));
                                    break;
                                }
                            }
                        }
                        // If not found in items, check re-exports (from `pub use`)
                        if result.is_none() {
                            for (reexport_name, reexport_def_id, _vis) in &mod_info.reexports {
                                if reexport_name == &type_name {
                                    if let Some(enum_info) = self.enum_defs.get(reexport_def_id) {
                                        result = Some((*reexport_def_id, enum_info.clone()));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    result
                };

                if let Some((enum_def_id, enum_info)) = enum_match {
                    // Found the enum, now look for the variant
                    if let Some(variant) = enum_info.variants.iter().find(|v| v.name == third_name) {
                        let variant_idx = variant.index;
                        let variant_def_id = variant.def_id;
                        let variant_fields = variant.fields.clone();

                        if variant_fields.is_empty() {
                            let type_args: Vec<Type> = enum_info.generics.iter()
                                .map(|_| self.unifier.fresh_var())
                                .collect();
                            let enum_ty = Type::adt(enum_def_id, type_args);

                            return Ok(hir::Expr::new(
                                hir::ExprKind::Variant {
                                    def_id: enum_def_id,
                                    variant_idx,
                                    fields: vec![],
                                },
                                enum_ty,
                                span,
                            ));
                        } else {
                            // Enum variant with fields - return as constructor function
                            let type_args: Vec<Type> = enum_info.generics.iter()
                                .map(|_| self.unifier.fresh_var())
                                .collect();

                            let subst: std::collections::HashMap<TyVarId, Type> = enum_info.generics.iter()
                                .zip(type_args.iter())
                                .map(|(&tyvar, ty)| (tyvar, ty.clone()))
                                .collect();

                            let field_types: Vec<Type> = variant_fields.iter()
                                .map(|f| self.substitute_type_vars(&f.ty, &subst))
                                .collect();

                            let enum_ty = Type::adt(enum_def_id, type_args);
                            let fn_ty = Type::function(field_types, enum_ty);

                            return Ok(hir::Expr::new(
                                hir::ExprKind::Def(variant_def_id),
                                fn_ty,
                                span,
                            ));
                        }
                    }
                }

                // Not an enum variant, try to find a struct or enum with an associated function
                // Find the type (struct or enum) within the module's items (properly scoped)
                let mut type_def_id: Option<DefId> = None;
                if let Some(mod_info) = self.module_defs.get(&mod_def_id) {
                    // Check direct items
                    for &item_def_id in &mod_info.items {
                        // Check structs
                        if let Some(struct_info) = self.struct_defs.get(&item_def_id) {
                            if struct_info.name == type_name {
                                type_def_id = Some(item_def_id);
                                break;
                            }
                        }
                        // Check enums (for associated functions, not variants)
                        if let Some(enum_info) = self.enum_defs.get(&item_def_id) {
                            if enum_info.name == type_name {
                                type_def_id = Some(item_def_id);
                                break;
                            }
                        }
                    }
                    // If not found in items, check re-exports (from `pub use`)
                    if type_def_id.is_none() {
                        for (reexport_name, reexport_def_id, _vis) in &mod_info.reexports {
                            if reexport_name == &type_name {
                                // Check if it's a struct
                                if self.struct_defs.contains_key(reexport_def_id) {
                                    type_def_id = Some(*reexport_def_id);
                                    break;
                                }
                                // Check if it's an enum
                                if self.enum_defs.contains_key(reexport_def_id) {
                                    type_def_id = Some(*reexport_def_id);
                                    break;
                                }
                            }
                        }
                    }
                }

                if let Some(type_def_id) = type_def_id {
                    let method_name = third_name;
                    // Now look for the associated function on this type
                    let self_ty = Type::adt(type_def_id, Vec::new());
                    let mut found_method: Option<(DefId, hir::FnSig, Vec<TyVarId>)> = None;

                    for impl_block in &self.impl_blocks {
                        // Only check inherent impls (not trait impls)
                        if impl_block.trait_ref.is_some() {
                            continue;
                        }
                        // Check if impl block applies to this type
                        if !self.types_match_for_impl(&impl_block.self_ty, &self_ty) {
                            continue;
                        }
                        // Look for a method with matching name
                        for method in &impl_block.methods {
                            if method.name == method_name {
                                if let Some(sig) = self.fn_sigs.get(&method.def_id).cloned() {
                                    found_method = Some((method.def_id, sig, impl_block.generics.clone()));
                                    break;
                                }
                            }
                        }
                        if found_method.is_some() {
                            break;
                        }
                    }

                    if let Some((method_def_id, sig, impl_generics)) = found_method {
                        // Combine impl-level and method-level generics for instantiation
                        let all_generics: Vec<TyVarId> = impl_generics.iter()
                            .chain(sig.generics.iter())
                            .copied()
                            .collect();

                        let fn_ty = if all_generics.is_empty() {
                            Type::function(sig.inputs.clone(), sig.output.clone())
                        } else {
                            // Create fresh type vars for all generics (impl + method)
                            let mut substitution: HashMap<TyVarId, Type> = HashMap::new();
                            for &old_var in &all_generics {
                                let fresh_var = self.unifier.fresh_var();
                                substitution.insert(old_var, fresh_var);
                            }

                            let subst_inputs: Vec<Type> = sig.inputs.iter()
                                .map(|ty| self.substitute_type_vars(ty, &substitution))
                                .collect();
                            let subst_output = self.substitute_type_vars(&sig.output, &substitution);

                            Type::function(subst_inputs, subst_output)
                        };
                        return Ok(hir::Expr::new(
                            hir::ExprKind::Def(method_def_id),
                            fn_ty,
                            span,
                        ));
                    }

                    // Type found but no matching method
                    Err(Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}::{}", module_name, type_name, method_name) },
                        span,
                    )))
                } else {
                    // Type not found in module
                    Err(self.error_type_not_found(
                        &format!("{}::{}", module_name, type_name),
                        span,
                    ))
                }
            } else {
                // Module not found
                Err(Box::new(TypeError::new(
                    TypeErrorKind::NotFound { name: format!("module '{}'", module_name) },
                    span,
                )))
            }
        } else {
            let path_str = path.segments.iter()
                .map(|s| self.symbol_to_string(s.name.node))
                .collect::<Vec<_>>()
                .join("::");
            Err(Box::new(TypeError::new(
                TypeErrorKind::UnsupportedFeature {
                    feature: format!("paths with more than 3 segments: {}", path_str),
                },
                span,
            )))
        }
    }

    /// Infer type of a binary expression.
    pub(crate) fn infer_binary(
        &mut self,
        op: ast::BinOp,
        left: &ast::Expr,
        right: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let left_expr = self.infer_expr(left)?;
        // Use check_expr for right side to propagate left type for better literal inference
        let right_expr = match self.check_expr(right, &left_expr.ty) {
            Ok(expr) => expr,
            // Fall back to infer if check fails (types may legitimately differ)
            Err(_) => self.infer_expr(right)?,
        };

        let result_ty = match op {
            ast::BinOp::Add | ast::BinOp::Sub | ast::BinOp::Mul | ast::BinOp::Div | ast::BinOp::Rem => {
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
                // Verify the resolved type is numeric (not bool, str, etc.)
                let resolved = self.unifier.resolve(&left_expr.ty);
                match resolved.kind() {
                    // Numeric primitives are allowed
                    TypeKind::Primitive(PrimitiveTy::Int(_) | PrimitiveTy::Uint(_) | PrimitiveTy::Float(_)) => {}
                    // Inference variables and type params — allow (checked later or by monomorphization)
                    TypeKind::Infer(_) | TypeKind::Param(_) => {}
                    // Error type — don't cascade errors
                    TypeKind::Error => {}
                    // Everything else is invalid
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::InvalidBinaryOp {
                                op: op.as_str().to_string(),
                                left: self.unifier.resolve(&left_expr.ty),
                                right: self.unifier.resolve(&right_expr.ty),
                            },
                            span,
                        )));
                    }
                }
                left_expr.ty.clone()
            }
            ast::BinOp::Eq | ast::BinOp::Ne | ast::BinOp::Lt | ast::BinOp::Le | ast::BinOp::Gt | ast::BinOp::Ge => {
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
                Type::bool()
            }
            ast::BinOp::And | ast::BinOp::Or => {
                self.unifier.unify(&left_expr.ty, &Type::bool(), span)?;
                self.unifier.unify(&right_expr.ty, &Type::bool(), span)?;
                Type::bool()
            }
            ast::BinOp::BitAnd | ast::BinOp::BitOr | ast::BinOp::BitXor => {
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
                left_expr.ty.clone()
            }
            ast::BinOp::Shl | ast::BinOp::Shr => {
                // Shift operators: the shift amount (right operand) does NOT need to match
                // the shifted value (left operand) type. Both must be integer types,
                // but u64 >> u32 is perfectly valid.
                let left_resolved = self.unifier.resolve(&left_expr.ty);
                let right_resolved = self.unifier.resolve(&right_expr.ty);

                // Verify left operand is an integer type
                match left_resolved.kind() {
                    TypeKind::Primitive(PrimitiveTy::Int(_) | PrimitiveTy::Uint(_)) => {}
                    TypeKind::Infer(_) | TypeKind::Param(_) | TypeKind::Error => {}
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::InvalidBinaryOp {
                                op: op.as_str().to_string(),
                                left: left_resolved,
                                right: right_resolved.clone(),
                            },
                            span,
                        )));
                    }
                }

                // Verify right operand (shift amount) is an integer type
                match right_resolved.kind() {
                    TypeKind::Primitive(PrimitiveTy::Int(_) | PrimitiveTy::Uint(_)) => {}
                    TypeKind::Infer(_) | TypeKind::Param(_) | TypeKind::Error => {}
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::InvalidBinaryOp {
                                op: op.as_str().to_string(),
                                left: left_resolved,
                                right: right_resolved,
                            },
                            span,
                        )));
                    }
                }

                // Result type is the type of the value being shifted (left operand)
                left_expr.ty.clone()
            }
            ast::BinOp::Pipe => {
                match right_expr.ty.kind() {
                    TypeKind::Fn { params, ret, .. } => {
                        if params.is_empty() {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::WrongArity {
                                    expected: 1,
                                    found: 0,
                                },
                                span,
                            )));
                        }
                        self.unifier.unify(&left_expr.ty, &params[0], span)?;
                        ret.clone()
                    }
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotAFunction { ty: right_expr.ty.clone() },
                            span,
                        )));
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
    pub(crate) fn infer_unary(
        &mut self,
        op: ast::UnaryOp,
        operand: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let operand_expr = self.infer_expr(operand)?;

        let result_ty = match op {
            ast::UnaryOp::Neg => operand_expr.ty.clone(),
            ast::UnaryOp::Not => operand_expr.ty.clone(),
            ast::UnaryOp::Deref => {
                match operand_expr.ty.kind() {
                    TypeKind::Ref { inner, .. } => inner.clone(),
                    TypeKind::Ptr { inner, .. } => inner.clone(),
                    // Box<T> can be dereferenced to T
                    TypeKind::Adt { def_id, args, .. } => {
                        if let Some(info) = self.resolver.def_info.get(def_id) {
                            if info.name == "Box" && args.len() == 1 {
                                args[0].clone()
                            } else {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::CannotDeref { ty: operand_expr.ty.clone() },
                                    span,
                                )));
                            }
                        } else {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::CannotDeref { ty: operand_expr.ty.clone() },
                                span,
                            )));
                        }
                    }
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::CannotDeref { ty: operand_expr.ty.clone() },
                            span,
                        )));
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
    pub(crate) fn infer_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::CallArg],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let callee_expr = self.infer_expr(callee)?;

        // Handle multiple dispatch: if callee is a MethodFamily, resolve to specific method
        let callee_expr = if let hir::ExprKind::MethodFamily { name, candidates } = &callee_expr.kind {
            self.resolve_method_family_dispatch(name, candidates, args, span)?
        } else {
            callee_expr
        };

        // Resolve the callee type to handle inference variables
        let resolved_callee_ty = self.unifier.resolve(&callee_expr.ty);

        // Handle forall types by instantiating them with fresh type variables
        // Keep track of type param -> fresh var mapping for trait bound checking later
        let type_param_fresh_vars: Option<Vec<(hir::TyVarId, Type)>> = match resolved_callee_ty.kind() {
            TypeKind::Forall { params, .. } => {
                Some(params.iter().cloned()
                    .map(|p| (p, self.unifier.fresh_var()))
                    .collect())
            }
            _ => None,
        };

        let instantiated_ty = match resolved_callee_ty.kind() {
            TypeKind::Forall { body, .. } => {
                // Build substitution map from the captured mapping
                // SAFETY: type_param_fresh_vars is Some when resolved_callee_ty is Forall (set at line 5071-5078)
                let subst: std::collections::HashMap<hir::TyVarId, Type> =
                    type_param_fresh_vars.as_ref()
                        .expect("BUG: type_param_fresh_vars must be Some when callee is Forall type")
                        .iter()
                        .map(|(p, v)| (*p, v.clone()))
                        .collect();

                // Substitute params in the body
                self.substitute_type_vars(body, &subst)
            }
            _ => resolved_callee_ty.clone(),
        };

        // For generic function calls, create a mapping of type params to fresh vars
        // to check trait bounds after unification. This handles the case where
        // instantiate_fn_sig_with_effects was already called (no Forall type).
        let generic_param_mapping: Option<Vec<(hir::TyVarId, Type)>> = if type_param_fresh_vars.is_none() {
            if let hir::ExprKind::Def(def_id) = &callee_expr.kind {
                if let Some(sig) = self.fn_sigs.get(def_id) {
                    if !sig.generics.is_empty() {
                        // The callee type has already been instantiated with fresh vars.
                        // We need to extract those vars by re-instantiating and unifying.
                        let mapping: Vec<(hir::TyVarId, Type)> = sig.generics.iter()
                            .map(|&ty_var_id| {
                                let fresh_var = self.unifier.fresh_var();
                                (ty_var_id, fresh_var)
                            })
                            .collect();

                        // Build substitution and create the expected function type
                        let subst: std::collections::HashMap<hir::TyVarId, Type> = mapping.iter()
                            .map(|(p, v)| (*p, v.clone()))
                            .collect();
                        let expected_inputs: Vec<Type> = sig.inputs.iter()
                            .map(|ty| self.substitute_type_vars(ty, &subst))
                            .collect();
                        let expected_output = self.substitute_type_vars(&sig.output, &subst);
                        let expected_fn_ty = Type::function(expected_inputs, expected_output);

                        // Unify with the callee's actual type to bind our fresh vars
                        // to the same concrete types the callee was instantiated with
                        let _ = self.unifier.unify(&expected_fn_ty, &resolved_callee_ty, span);

                        Some(mapping)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Combine both sources of type param mappings
        let effective_param_mapping = type_param_fresh_vars.or(generic_param_mapping);

        let (param_types, return_type) = match instantiated_ty.kind() {
            TypeKind::Fn { params, ret, .. } => (params.clone(), ret.clone()),
            _ => {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::NotAFunction { ty: callee_expr.ty.clone() },
                    span,
                )));
            }
        };

        if args.len() != param_types.len() {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::WrongArity {
                    expected: param_types.len(),
                    found: args.len(),
                },
                span,
            )));
        }

        // Check effect compatibility
        if let hir::ExprKind::Def(callee_def_id) = &callee_expr.kind {
            self.check_effect_compatibility(*callee_def_id, span)?;
        } else {
            // For function-typed expressions (e.g., calling a function parameter),
            // check effect compatibility using the effects from the function type
            if let TypeKind::Fn { effects, .. } = instantiated_ty.kind() {
                if !effects.is_empty() {
                    self.check_effect_compatibility_from_type(effects, span)?;
                }
            }
        }

        let mut hir_args = Vec::new();
        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
            let arg_expr = self.check_expr(&arg.value, param_ty)?;
            hir_args.push(arg_expr);
        }

        // Check trait bounds on type parameters after unification
        // At this point, the fresh type variables have been unified with concrete types
        if let Some(ref param_mapping) = effective_param_mapping {
            for (ty_var_id, fresh_var) in param_mapping {
                // Resolve the fresh variable to its concrete type
                let resolved_ty = self.unifier.resolve(fresh_var);

                // Look up the bounds for this type parameter
                if let Some(bounds) = self.type_param_bounds.get(ty_var_id) {
                    // Check that the resolved type satisfies all bounds
                    self.check_trait_bounds(&resolved_ty, bounds, span)?;
                }
            }
        }

        // Check where clause bounds from the function definition
        // This handles bounds like `where T: Trait, U: OtherTrait`
        if let hir::ExprKind::Def(def_id) = &callee_expr.kind {
            if let Some(predicates) = self.fn_where_bounds.get(def_id).cloned() {
                for predicate in &predicates {
                    // Resolve the subject type using the current substitution
                    let resolved_subject = if let Some(ref param_mapping) = effective_param_mapping {
                        // Build substitution from param mapping
                        let subst: std::collections::HashMap<hir::TyVarId, Type> = param_mapping.iter()
                            .map(|(p, v)| (*p, self.unifier.resolve(v)))
                            .collect();
                        self.substitute_type_vars(&predicate.subject_ty, &subst)
                    } else {
                        self.unifier.resolve(&predicate.subject_ty)
                    };

                    // Check that the resolved subject type satisfies all trait bounds
                    self.check_trait_bounds(&resolved_subject, &predicate.trait_bounds, span)?;
                }
            }
        }

        // Check if this is a call to an enum variant constructor
        // If so, convert it to a Variant expression instead of a Call
        if let hir::ExprKind::Def(def_id) = &callee_expr.kind {
            if let Some(info) = self.resolver.def_info.get(def_id) {
                if info.kind == hir::DefKind::Variant {
                    // Find the enum and variant index
                    if let Some(parent_id) = info.parent {
                        if let Some(enum_info) = self.enum_defs.get(&parent_id) {
                            if let Some(variant) = enum_info.variants.iter().find(|v| v.def_id == *def_id) {
                                return Ok(hir::Expr::new(
                                    hir::ExprKind::Variant {
                                        def_id: parent_id,
                                        variant_idx: variant.index,
                                        fields: hir_args,
                                    },
                                    return_type,
                                    span,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Resolve the callee's type to ensure inference variables are substituted.
        // This is critical for generic function calls where the callee type contains
        // type parameters that were unified during argument type checking.
        let resolved_callee_ty = self.unifier.resolve(&callee_expr.ty);

        let resolved_callee_expr = hir::Expr::new(
            callee_expr.kind.clone(),
            resolved_callee_ty,
            callee_expr.span,
        );

        // Also resolve the return type
        let resolved_return_type = self.unifier.resolve(&return_type);

        Ok(hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(resolved_callee_expr),
                args: hir_args,
            },
            resolved_return_type,
            span,
        ))
    }

    /// Resolve a method family to a specific method based on argument types.
    ///
    /// This implements the core of multiple dispatch: given a set of candidate methods
    /// and actual argument types, find the most specific matching method.
    fn resolve_method_family_dispatch(
        &mut self,
        name: &str,
        candidates: &[DefId],
        args: &[ast::CallArg],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // First, infer the types of all arguments to guide dispatch
        let mut inferred_arg_types = Vec::new();
        for arg in args {
            let arg_expr = self.infer_expr(&arg.value)?;
            inferred_arg_types.push(self.unifier.resolve(&arg_expr.ty));
        }

        // Find applicable candidates: methods whose parameter types match the arguments
        let mut applicable: Vec<(DefId, &hir::FnSig)> = Vec::new();

        for &def_id in candidates {
            if let Some(sig) = self.fn_sigs.get(&def_id) {
                // Check arity match
                if sig.inputs.len() != inferred_arg_types.len() {
                    continue;
                }

                // Check if argument types are compatible with parameter types
                let mut compatible = true;
                for (arg_ty, param_ty) in inferred_arg_types.iter().zip(sig.inputs.iter()) {
                    // For dispatch, we check if the argument type can unify with or is a subtype of param
                    // We use a temporary unifier to avoid polluting the main one
                    let mut test_unifier = self.unifier.clone();
                    if test_unifier.unify(arg_ty, param_ty, span).is_err() {
                        compatible = false;
                        break;
                    }
                }

                if compatible {
                    applicable.push((def_id, sig));
                }
            }
        }

        // If no applicable methods, report error
        if applicable.is_empty() {
            let arg_types: Vec<String> = inferred_arg_types.iter()
                .map(|t| format!("{:?}", t))
                .collect();
            return Err(Box::new(TypeError::new(
                TypeErrorKind::NoApplicableMethod {
                    name: name.to_string(),
                    arg_types,
                },
                span,
            )));
        }

        // If exactly one applicable method, use it
        if applicable.len() == 1 {
            let (def_id, sig) = applicable[0];
            let fn_ty = Type::function(sig.inputs.clone(), sig.output.clone());
            return Ok(hir::Expr::new(
                hir::ExprKind::Def(def_id),
                fn_ty,
                span,
            ));
        }

        // Multiple applicable methods: find the most specific one
        // Most specific = one where all parameter types are subtypes of (or equal to) other candidates
        // For now, use a simple strategy: prefer exact type matches over coercions
        let mut best: Option<(DefId, &hir::FnSig, usize)> = None; // (def_id, sig, specificity_score)

        for (def_id, sig) in &applicable {
            let mut score = 0;
            for (arg_ty, param_ty) in inferred_arg_types.iter().zip(sig.inputs.iter()) {
                // Exact match gets higher score
                if arg_ty == param_ty {
                    score += 2;
                } else {
                    score += 1;
                }
            }

            if let Some((_, _, best_score)) = &best {
                if score > *best_score {
                    best = Some((*def_id, *sig, score));
                }
            } else {
                best = Some((*def_id, *sig, score));
            }
        }

        if let Some((def_id, sig, _)) = best {
            let fn_ty = Type::function(sig.inputs.clone(), sig.output.clone());
            return Ok(hir::Expr::new(
                hir::ExprKind::Def(def_id),
                fn_ty,
                span,
            ));
        }

        // Ambiguous dispatch - report error
        let candidate_sigs: Vec<String> = applicable.iter()
            .map(|(_, sig)| {
                let params: Vec<String> = sig.inputs.iter()
                    .map(|t| format!("{:?}", t))
                    .collect();
                format!("({})", params.join(", "))
            })
            .collect();
        Err(Box::new(TypeError::new(
            TypeErrorKind::AmbiguousDispatch {
                name: name.to_string(),
                candidates: candidate_sigs,
            },
            span,
        )))
    }

    /// Infer type of an if expression.
    pub(crate) fn infer_if(
        &mut self,
        condition: &ast::Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ast::ElseBranch>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let cond_expr = self.check_expr(condition, &Type::bool())?;

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

    /// Check an if expression against an expected type.
    /// This propagates the expected type to all branches for better type inference.
    pub(crate) fn check_if(
        &mut self,
        condition: &ast::Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ast::ElseBranch>,
        expected: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let cond_expr = self.check_expr(condition, &Type::bool())?;

        // Use expected type directly instead of fresh variable
        let then_expr = self.check_block(then_branch, expected)?;

        let else_expr = if let Some(else_branch) = else_branch {
            match else_branch {
                ast::ElseBranch::Block(block) => {
                    Some(Box::new(self.check_block(block, expected)?))
                }
                ast::ElseBranch::If(if_expr) => {
                    Some(Box::new(self.check_expr(if_expr, expected)?))
                }
            }
        } else {
            self.unifier.unify(expected, &Type::unit(), span)?;
            None
        };

        let result_ty = self.unifier.resolve(expected);

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

    /// Infer type of an if-let expression.
    ///
    /// Desugars `if let PATTERN = SCRUTINEE { THEN } else { ELSE }` into:
    /// ```text
    /// match SCRUTINEE {
    ///     PATTERN => THEN,
    ///     _ => ELSE,  // or unit if no else
    /// }
    /// ```
    pub(crate) fn infer_if_let(
        &mut self,
        pattern: &ast::Pattern,
        scrutinee: &ast::Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ast::ElseBranch>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Infer the scrutinee type
        let scrutinee_expr = self.infer_expr(scrutinee)?;
        let scrutinee_ty = scrutinee_expr.ty.clone();

        // Expected result type for both branches
        let expected = self.unifier.fresh_var();

        // Enter a scope for the pattern bindings in the then branch
        self.resolver.push_scope(ScopeKind::Block, span);

        // Lower the pattern with the scrutinee type
        let hir_pattern = self.lower_pattern(pattern, &scrutinee_ty)?;

        // Type check the then branch
        let then_expr = self.check_block(then_branch, &expected)?;

        self.resolver.pop_scope(); // pattern scope

        // Create the matching arm
        let match_arm = hir::MatchArm {
            pattern: hir_pattern,
            guard: None,
            body: then_expr,
        };

        // Create the wildcard (else) arm
        let else_body = if let Some(else_branch) = else_branch {
            match else_branch {
                ast::ElseBranch::Block(block) => {
                    self.check_block(block, &expected)?
                }
                ast::ElseBranch::If(if_expr) => {
                    self.check_expr(if_expr, &expected)?
                }
            }
        } else {
            // No else branch - result must be unit
            self.unifier.unify(&expected, &Type::unit(), span)?;
            hir::Expr::new(
                hir::ExprKind::Tuple(vec![]),
                Type::unit(),
                span,
            )
        };

        let wildcard_arm = hir::MatchArm {
            pattern: hir::Pattern {
                kind: hir::PatternKind::Wildcard,
                ty: scrutinee_ty.clone(),
                span,
            },
            guard: None,
            body: else_body,
        };

        let result_ty = self.unifier.resolve(&expected);

        // Build the match expression
        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(scrutinee_expr),
                arms: vec![match_arm, wildcard_arm],
            },
            result_ty,
            span,
        ))
    }

    /// Infer type of a return expression.
    pub(crate) fn infer_return(&mut self, value: Option<&ast::Expr>, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        let return_type = self.return_type.clone().ok_or_else(|| {
            TypeError::new(TypeErrorKind::ReturnOutsideFunction, span)
        })?;

        let value_expr = if let Some(value) = value {
            Some(Box::new(self.check_expr(value, &return_type)?))
        } else {
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
    pub(crate) fn infer_tuple(&mut self, exprs: &[ast::Expr], span: Span) -> Result<hir::Expr, Box<TypeError>> {
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
    pub(crate) fn infer_assign(&mut self, target: &ast::Expr, value: &ast::Expr, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        // Check mutability for simple variable assignments
        if let ast::ExprKind::Path(path) = &target.kind {
            if path.segments.len() == 1 {
                let name = self.symbol_to_string(path.segments[0].name.node);
                if let Some(Binding::Local { mutable, .. }) = self.resolver.lookup(&name) {
                    if !mutable {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::ImmutableAssign { name: name.clone() },
                            span,
                        ).with_help(format!("consider making `{name}` mutable: `let mut {name}`"))));
                    }
                }
            }
        }

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
    pub(crate) fn infer_loop(&mut self, body: &ast::Block, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);
        let body_expr = self.check_block(body, &Type::unit())?;
        self.resolver.pop_scope();

        // exit_loop computes the loop's result type based on break types
        let loop_ty = self.exit_loop(label_str.as_deref());

        Ok(hir::Expr::new(
            hir::ExprKind::Loop {
                body: Box::new(body_expr),
                label: Some(loop_id),
            },
            loop_ty,
            span,
        ))
    }

    /// Infer type of a while loop.
    pub(crate) fn infer_while(&mut self, condition: &ast::Expr, body: &ast::Block, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);

        let cond_expr = self.check_expr(condition, &Type::bool())?;
        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        self.exit_loop(label_str.as_deref());

        Ok(hir::Expr::new(
            hir::ExprKind::While {
                condition: Box::new(cond_expr),
                body: Box::new(body_expr),
                label: Some(loop_id),
            },
            Type::unit(),
            span,
        ))
    }

    /// Infer type of a while-let loop.
    ///
    /// Desugars `while let PATTERN = SCRUTINEE { BODY }` into:
    /// ```text
    /// loop {
    ///     match SCRUTINEE {
    ///         PATTERN => BODY,
    ///         _ => break,
    ///     }
    /// }
    /// ```
    pub(crate) fn infer_while_let(
        &mut self,
        pattern: &ast::Pattern,
        scrutinee: &ast::Expr,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);

        // Infer the scrutinee type
        let scrutinee_expr = self.infer_expr(scrutinee)?;
        let scrutinee_ty = scrutinee_expr.ty.clone();

        // Enter a scope for the pattern bindings
        self.resolver.push_scope(ScopeKind::Block, span);

        // Lower the pattern with the scrutinee type
        let hir_pattern = self.lower_pattern(pattern, &scrutinee_ty)?;

        // Type check the body (unit type, side effects only)
        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope(); // pattern scope
        self.resolver.pop_scope(); // loop scope

        self.exit_loop(label_str.as_deref());

        // Build the match arms:
        // 1. PATTERN => BODY
        // 2. _ => break
        let match_arm = hir::MatchArm {
            pattern: hir_pattern,
            guard: None,
            body: body_expr,
        };

        let wildcard_arm = hir::MatchArm {
            pattern: hir::Pattern {
                kind: hir::PatternKind::Wildcard,
                ty: scrutinee_ty.clone(),
                span,
            },
            guard: None,
            body: hir::Expr::new(
                hir::ExprKind::Break {
                    label: Some(loop_id),
                    value: None,
                },
                Type::never(),
                span,
            ),
        };

        // Build the match expression
        let match_expr = hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(scrutinee_expr),
                arms: vec![match_arm, wildcard_arm],
            },
            Type::unit(),
            span,
        );

        // Wrap in a loop
        Ok(hir::Expr::new(
            hir::ExprKind::Loop {
                body: Box::new(match_expr),
                label: Some(loop_id),
            },
            Type::unit(),
            span,
        ))
    }

    /// Infer type of a for loop.
    ///
    /// Supports:
    /// - Range iteration: `for i in 0..10 { ... }`
    /// - Array iteration: `for item in array { ... }`
    pub(crate) fn infer_for(
        &mut self,
        pattern: &ast::Pattern,
        iter: &ast::Expr,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Try to match range expression first
        if let ast::ExprKind::Range { start, end, inclusive } = &iter.kind {
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
            return self.infer_for_range(pattern, start, end, *inclusive, body, label, span);
        }

        // Try to infer the iterator expression and check if it's iterable
        let iter_expr = self.infer_expr(iter)?;
        let iter_ty = self.unifier.resolve(&iter_expr.ty);

        // Check for array types (fixed-size or dynamically-determined)
        match iter_ty.kind() {
            TypeKind::Array { element, size } => {
                let array_size = size.as_u64().unwrap_or(0);
                return self.infer_for_array(pattern, iter_expr, element.clone(), array_size, body, label, span);
            }
            TypeKind::Ref { inner, mutable } => {
                // Handle references to arrays: &[T; N] and &[T]
                let inner_ty = self.unifier.resolve(inner);
                match inner_ty.kind() {
                    TypeKind::Array { element, size } => {
                        let array_size = size.as_u64().unwrap_or(0);
                        return self.infer_for_array(pattern, iter_expr, element.clone(), array_size, body, label, span);
                    }
                    TypeKind::Slice { element } => {
                        // Slice iteration with runtime length check
                        return self.infer_for_slice(pattern, iter_expr, element.clone(), body, label, span);
                    }
                    TypeKind::Adt { def_id, args } => {
                        // Handle &Vec<T> and &mut Vec<T>
                        if Some(*def_id) == self.vec_def_id {
                            if let Some(element_ty) = args.first() {
                                return self.infer_for_vec(pattern, iter_expr, element_ty.clone(), *mutable, body, label, span);
                            }
                        }
                    }
                    _ => {}
                }
            }
            TypeKind::Slice { element } => {
                // Slice iteration with runtime length check
                return self.infer_for_slice(pattern, iter_expr, element.clone(), body, label, span);
            }
            TypeKind::Adt { def_id, args } => {
                // Handle Vec<T> directly (consuming iteration)
                if Some(*def_id) == self.vec_def_id {
                    if let Some(element_ty) = args.first() {
                        // For owned Vec, we iterate by value (consuming)
                        return self.infer_for_vec_owned(pattern, iter_expr, element_ty.clone(), body, label, span);
                    }
                }
            }
            _ => {}
        }

        // Not a supported iterable type
        Err(Box::new(TypeError::new(
            TypeErrorKind::UnsupportedFeature {
                feature: format!(
                    "For loop over type `{}` not supported. Use range expressions (0..n), arrays, or Vec.",
                    iter_ty
                ),
            },
            iter.span,
        )))
    }

    /// Helper: Infer for loop over a range expression.
    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
    fn infer_for_range(
        &mut self,
        pattern: &ast::Pattern,
        start: &ast::Expr,
        end: &ast::Expr,
        inclusive: bool,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Extract variable name, or None for wildcard pattern
        let var_name = match &pattern.kind {
            ast::PatternKind::Ident { name, .. } => Some(self.symbol_to_string(name.node)),
            ast::PatternKind::Wildcard => None,
            _ => {
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "For loop currently only supports simple identifier or wildcard patterns".into(),
                    },
                    pattern.span,
                )));
            }
        };

        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);

        let start_expr = self.infer_expr(start)?;
        let idx_ty = start_expr.ty.clone();

        let end_expr = self.check_expr(end, &idx_ty)?;

        // Create internal loop index
        let idx_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: idx_local_id,
            ty: idx_ty.clone(),
            mutable: true,
            name: Some("_loop_idx".to_string()),
            span,
        });

        // Register user's loop variable (only if not a wildcard pattern)
        let var_local_id = if let Some(ref name) = var_name {
            let local_id = self.resolver.define_local(
                name.clone(),
                idx_ty.clone(),
                false,
                pattern.span,
            )?;

            self.locals.push(hir::Local {
                id: local_id,
                ty: idx_ty.clone(),
                mutable: false,
                name: Some(name.clone()),
                span: pattern.span,
            });
            Some(local_id)
        } else {
            None
        };

        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        self.exit_loop(label_str.as_deref());

        // Build desugared while loop
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

        // Only create bind statement if we have a named loop variable (not wildcard)
        let bind_stmt = var_local_id.map(|local_id| hir::Stmt::Let {
            local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::Local(idx_local_id),
                idx_ty.clone(),
                span,
            )),
        });

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

        // Build loop body statements - include bind_stmt only for named patterns
        // NOTE: increment MUST come before body so that `continue` in the body
        // (which jumps to cond_block) doesn't skip the index advancement.
        // The user variable is already bound from _idx before the increment.
        let mut body_stmts = Vec::new();
        if let Some(stmt) = bind_stmt {
            body_stmts.push(stmt);
        }
        body_stmts.push(hir::Stmt::Expr(increment));
        body_stmts.push(hir::Stmt::Expr(body_expr));

        let while_body = hir::Expr::new(
            hir::ExprKind::Block {
                stmts: body_stmts,
                expr: None,
            },
            Type::unit(),
            span,
        );

        let while_loop = hir::Expr::new(
            hir::ExprKind::While {
                condition: Box::new(condition),
                body: Box::new(while_body),
                label: Some(loop_id),
            },
            Type::unit(),
            span,
        );

        let init_stmt = hir::Stmt::Let {
            local_id: idx_local_id,
            init: Some(start_expr),
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Block {
                stmts: vec![init_stmt],
                expr: Some(Box::new(while_loop)),
            },
            Type::unit(),
            span,
        ))
    }

    /// Helper: Infer for loop over a fixed-size array.
    ///
    /// Desugars `for item in array` to:
    /// ```text
    /// let mut _idx = 0;
    /// while _idx < ARRAY_SIZE {
    ///     let item = array[_idx];
    ///     body;
    ///     _idx = _idx + 1;
    /// }
    /// ```
    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
    fn infer_for_array(
        &mut self,
        pattern: &ast::Pattern,
        array_expr: hir::Expr,
        element_ty: Type,
        array_size: u64,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);

        let idx_ty = Type::i64(); // Use i64 for array indices

        // Store the array in a temporary to avoid re-evaluating
        let array_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: array_local_id,
            ty: array_expr.ty.clone(),
            mutable: false,
            name: Some("_array".to_string()),
            span,
        });

        // Create internal loop index
        let idx_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: idx_local_id,
            ty: idx_ty.clone(),
            mutable: true,
            name: Some("_loop_idx".to_string()),
            span,
        });

        // Use define_pattern to support tuple and struct destructuring
        // This handles simple identifiers, wildcards, tuples, and struct patterns
        let var_local_id = self.define_pattern(pattern, element_ty.clone())?;

        // Check if it's a wildcard pattern - we track this for generating the loop body
        let is_wildcard = matches!(pattern.kind, ast::PatternKind::Wildcard);
        let var_local_id = if is_wildcard { None } else { Some(var_local_id) };

        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        self.exit_loop(label_str.as_deref());

        // Build condition: _idx < array_size
        let condition = hir::Expr::new(
            hir::ExprKind::Binary {
                op: ast::BinOp::Lt,
                left: Box::new(hir::Expr::new(
                    hir::ExprKind::Local(idx_local_id),
                    idx_ty.clone(),
                    span,
                )),
                right: Box::new(hir::Expr::new(
                    hir::ExprKind::Literal(hir::LiteralValue::Int(array_size as i128)),
                    idx_ty.clone(),
                    span,
                )),
            },
            Type::bool(),
            span,
        );

        // Helper to build array access: array[_idx]
        let make_array_access = || {
            hir::Expr::new(
                hir::ExprKind::Index {
                    base: Box::new(hir::Expr::new(
                        hir::ExprKind::Local(array_local_id),
                        array_expr.ty.clone(),
                        span,
                    )),
                    index: Box::new(hir::Expr::new(
                        hir::ExprKind::Local(idx_local_id),
                        idx_ty.clone(),
                        span,
                    )),
                },
                element_ty.clone(),
                span,
            )
        };

        // Build bind statement if we have a named variable
        let bind_stmt = var_local_id.map(|local_id| hir::Stmt::Let {
            local_id,
            init: Some(make_array_access()),
        });

        // Build increment: _idx = _idx + 1
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

        // Build loop body statements
        // NOTE: increment MUST come before body so that `continue` in the body
        // (which jumps to cond_block) doesn't skip the index advancement.
        let mut body_stmts = Vec::new();
        if let Some(stmt) = bind_stmt {
            body_stmts.push(stmt);
        } else {
            // Even for wildcard, we still need to do the array access (for side effects)
            body_stmts.push(hir::Stmt::Expr(make_array_access()));
        }
        body_stmts.push(hir::Stmt::Expr(increment));
        body_stmts.push(hir::Stmt::Expr(body_expr));

        let while_body = hir::Expr::new(
            hir::ExprKind::Block {
                stmts: body_stmts,
                expr: None,
            },
            Type::unit(),
            span,
        );

        let while_loop = hir::Expr::new(
            hir::ExprKind::While {
                condition: Box::new(condition),
                body: Box::new(while_body),
                label: Some(loop_id),
            },
            Type::unit(),
            span,
        );

        // Initialize array local and index
        let array_init_stmt = hir::Stmt::Let {
            local_id: array_local_id,
            init: Some(array_expr),
        };

        let idx_init_stmt = hir::Stmt::Let {
            local_id: idx_local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::Literal(hir::LiteralValue::Int(0)),
                idx_ty,
                span,
            )),
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Block {
                stmts: vec![array_init_stmt, idx_init_stmt],
                expr: Some(Box::new(while_loop)),
            },
            Type::unit(),
            span,
        ))
    }

    /// Helper: Infer for loop over a slice (runtime length).
    ///
    /// Desugars `for item in slice` to:
    /// ```text
    /// let _slice = slice;
    /// let mut _idx = 0;
    /// while _idx < _slice.len() {
    ///     let item = _slice[_idx];
    ///     body;
    ///     _idx = _idx + 1;
    /// }
    /// ```
    fn infer_for_slice(
        &mut self,
        pattern: &ast::Pattern,
        slice_expr: hir::Expr,
        element_ty: Type,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);

        let idx_ty = Type::i64(); // Use i64 for slice indices (same as arrays)

        // Store the slice in a temporary to avoid re-evaluating
        let slice_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: slice_local_id,
            ty: slice_expr.ty.clone(),
            mutable: false,
            name: Some("_slice".to_string()),
            span,
        });

        // Create internal loop index
        let idx_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: idx_local_id,
            ty: idx_ty.clone(),
            mutable: true,
            name: Some("_loop_idx".to_string()),
            span,
        });

        // Create a local for the length (we'll compute it once before the loop)
        let len_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: len_local_id,
            ty: Type::u64(), // SliceLen returns u64
            mutable: false,
            name: Some("_slice_len".to_string()),
            span,
        });

        // For slice iteration, the loop variable is a reference to the element
        let ref_element_ty = Type::reference(element_ty.clone(), false);

        // Use define_pattern to support tuple and struct destructuring
        // This handles simple identifiers, wildcards, tuples, and struct patterns
        let var_local_id = self.define_pattern(pattern, ref_element_ty.clone())?;

        // Check if it's a wildcard pattern - we track this for generating the loop body
        let is_wildcard = matches!(pattern.kind, ast::PatternKind::Wildcard);
        let var_local_id = if is_wildcard { None } else { Some(var_local_id) };

        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        self.exit_loop(label_str.as_deref());

        // Build condition: _idx < _slice_len (cast _slice_len to i64 for comparison)
        let condition = hir::Expr::new(
            hir::ExprKind::Binary {
                op: ast::BinOp::Lt,
                left: Box::new(hir::Expr::new(
                    hir::ExprKind::Local(idx_local_id),
                    idx_ty.clone(),
                    span,
                )),
                right: Box::new(hir::Expr::new(
                    hir::ExprKind::Cast {
                        expr: Box::new(hir::Expr::new(
                            hir::ExprKind::Local(len_local_id),
                            Type::u64(),
                            span,
                        )),
                        target_ty: idx_ty.clone(),
                    },
                    idx_ty.clone(),
                    span,
                )),
            },
            Type::bool(),
            span,
        );

        // Helper to build slice access: slice[_idx] returns &element_ty
        let slice_ty = slice_expr.ty.clone();
        let make_slice_access = || {
            hir::Expr::new(
                hir::ExprKind::Index {
                    base: Box::new(hir::Expr::new(
                        hir::ExprKind::Local(slice_local_id),
                        slice_ty.clone(),
                        span,
                    )),
                    index: Box::new(hir::Expr::new(
                        hir::ExprKind::Local(idx_local_id),
                        idx_ty.clone(),
                        span,
                    )),
                },
                ref_element_ty.clone(),
                span,
            )
        };

        // Build bind statement if we have a named variable
        let bind_stmt = var_local_id.map(|local_id| hir::Stmt::Let {
            local_id,
            init: Some(make_slice_access()),
        });

        // Build increment: _idx = _idx + 1
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

        // Build loop body statements
        // NOTE: increment MUST come before body so that `continue` in the body
        // (which jumps to cond_block) doesn't skip the index advancement.
        let mut body_stmts = Vec::new();
        if let Some(stmt) = bind_stmt {
            body_stmts.push(stmt);
        } else {
            // Even for wildcard, we still need to do the slice access (for side effects)
            body_stmts.push(hir::Stmt::Expr(make_slice_access()));
        }
        body_stmts.push(hir::Stmt::Expr(increment));
        body_stmts.push(hir::Stmt::Expr(body_expr));

        let while_body = hir::Expr::new(
            hir::ExprKind::Block {
                stmts: body_stmts,
                expr: None,
            },
            Type::unit(),
            span,
        );

        let while_loop = hir::Expr::new(
            hir::ExprKind::While {
                condition: Box::new(condition),
                body: Box::new(while_body),
                label: Some(loop_id),
            },
            Type::unit(),
            span,
        );

        // Initialize slice local
        let slice_init_stmt = hir::Stmt::Let {
            local_id: slice_local_id,
            init: Some(slice_expr.clone()),
        };

        // Initialize length local using SliceLen
        let len_init_stmt = hir::Stmt::Let {
            local_id: len_local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::SliceLen(Box::new(hir::Expr::new(
                    hir::ExprKind::Local(slice_local_id),
                    slice_ty,
                    span,
                ))),
                Type::u64(),
                span,
            )),
        };

        // Initialize index local
        let idx_init_stmt = hir::Stmt::Let {
            local_id: idx_local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::Literal(hir::LiteralValue::Int(0)),
                idx_ty,
                span,
            )),
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Block {
                stmts: vec![slice_init_stmt, len_init_stmt, idx_init_stmt],
                expr: Some(Box::new(while_loop)),
            },
            Type::unit(),
            span,
        ))
    }

    /// Helper: Infer for loop over a borrowed Vec (&Vec<T> or &mut Vec<T>).
    ///
    /// Generates index-based iteration: `for i in 0..vec.len() { elem = &vec[i]; ... }`
    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
    fn infer_for_vec(
        &mut self,
        pattern: &ast::Pattern,
        vec_expr: hir::Expr,
        element_ty: Type,
        mutable: bool,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let _loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);

        let idx_ty = Type::usize();

        // Store the vec reference in a temporary to avoid re-evaluating
        let vec_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: vec_local_id,
            ty: vec_expr.ty.clone(),
            mutable: false,
            name: Some("_vec".to_string()),
            span,
        });

        // Create internal loop index
        let idx_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: idx_local_id,
            ty: idx_ty.clone(),
            mutable: true,
            name: Some("_loop_idx".to_string()),
            span,
        });

        // Create a local for the length (we'll compute it once before the loop)
        let len_local_id = self.resolver.next_local_id();
        self.locals.push(hir::Local {
            id: len_local_id,
            ty: Type::usize(),
            mutable: false,
            name: Some("_vec_len".to_string()),
            span,
        });

        // For Vec iteration, the loop variable is a reference to the element
        let ref_element_ty = Type::reference(element_ty.clone(), mutable);

        // Use define_pattern to support tuple and struct destructuring
        // This handles simple identifiers, wildcards, tuples, and struct patterns
        let var_local_id = self.define_pattern(pattern, ref_element_ty.clone())?;

        // Check if it's a wildcard pattern - we track this for generating the loop body
        let is_wildcard = matches!(pattern.kind, ast::PatternKind::Wildcard);
        let var_local_id = if is_wildcard { None } else { Some(var_local_id) };

        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

        self.exit_loop(label_str.as_deref());

        // Build condition: _idx < _vec_len
        let condition = hir::Expr::new(
            hir::ExprKind::Binary {
                op: ast::BinOp::Lt,
                left: Box::new(hir::Expr::new(
                    hir::ExprKind::Local(idx_local_id),
                    idx_ty.clone(),
                    span,
                )),
                right: Box::new(hir::Expr::new(
                    hir::ExprKind::Local(len_local_id),
                    Type::usize(),
                    span,
                )),
            },
            Type::bool(),
            span,
        );

        // Build vec access using index: vec[_idx]
        let vec_ty = vec_expr.ty.clone();
        let make_vec_access = || {
            hir::Expr::new(
                hir::ExprKind::Index {
                    base: Box::new(hir::Expr::new(
                        hir::ExprKind::Local(vec_local_id),
                        vec_ty.clone(),
                        span,
                    )),
                    index: Box::new(hir::Expr::new(
                        hir::ExprKind::Local(idx_local_id),
                        idx_ty.clone(),
                        span,
                    )),
                },
                ref_element_ty.clone(),
                span,
            )
        };

        // Build loop body with element binding and increment
        let bind_stmt = var_local_id.map(|local_id| hir::Stmt::Let {
            local_id,
            init: Some(make_vec_access()),
        });

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

        // Build the loop body: bind; increment; user_body;
        // NOTE: increment MUST come before body so that `continue` in the body
        // (which jumps to the loop header) doesn't skip the index advancement.
        let mut loop_body_stmts = Vec::new();
        if let Some(stmt) = bind_stmt {
            loop_body_stmts.push(stmt);
        }
        loop_body_stmts.push(hir::Stmt::Expr(increment));
        loop_body_stmts.push(hir::Stmt::Expr(body_expr));

        let loop_body = hir::Expr::new(
            hir::ExprKind::Block {
                stmts: loop_body_stmts,
                expr: None,
            },
            Type::unit(),
            span,
        );

        let while_loop = hir::Expr::new(
            hir::ExprKind::Loop {
                body: Box::new(hir::Expr::new(
                    hir::ExprKind::If {
                        condition: Box::new(condition),
                        then_branch: Box::new(loop_body),
                        else_branch: Some(Box::new(hir::Expr::new(
                            hir::ExprKind::Break { label: None, value: None },
                            Type::never(),
                            span,
                        ))),
                    },
                    Type::unit(),
                    span,
                )),
                label: None,
            },
            Type::unit(),
            span,
        );

        // Build the outer block with vec, len, and idx initialization
        let vec_init_stmt = hir::Stmt::Let {
            local_id: vec_local_id,
            init: Some(vec_expr),
        };

        // Get Vec length using VecLen operation
        let vec_ref_for_len = hir::Expr::new(
            hir::ExprKind::Local(vec_local_id),
            vec_ty.clone(),
            span,
        );
        let len_init_stmt = hir::Stmt::Let {
            local_id: len_local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::VecLen(Box::new(vec_ref_for_len)),
                Type::usize(),
                span,
            )),
        };

        let idx_init_stmt = hir::Stmt::Let {
            local_id: idx_local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::Literal(hir::LiteralValue::Int(0)),
                idx_ty,
                span,
            )),
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Block {
                stmts: vec![vec_init_stmt, len_init_stmt, idx_init_stmt],
                expr: Some(Box::new(while_loop)),
            },
            Type::unit(),
            span,
        ))
    }

    /// Helper: Infer for loop over an owned Vec (consuming iteration).
    ///
    /// For now, this just errors - iterating over an owned Vec would copy the
    /// entire Vec under MVS. Users should use `&vec` or `&mut vec` instead.
    fn infer_for_vec_owned(
        &mut self,
        _pattern: &ast::Pattern,
        _vec_expr: hir::Expr,
        _element_ty: Type,
        _body: &ast::Block,
        _label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        Err(Box::new(TypeError::new(
            TypeErrorKind::UnsupportedFeature {
                feature: "Consuming iteration over Vec not supported. Use `&vec` or `&mut vec` instead.".into(),
            },
            span,
        )))
    }

    /// Infer type of a break.
    pub(crate) fn infer_break(&mut self, value: Option<&ast::Expr>, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        if !self.resolver.in_loop() {
            return Err(Box::new(TypeError::new(TypeErrorKind::BreakOutsideLoop, span)));
        }

        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.find_loop(label_str.as_deref());

        // Check that the label exists
        if label.is_some() && loop_id.is_none() {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::UnresolvedName { name: label_str.unwrap_or_default() },
                span,
            )));
        }

        let (value_expr, break_ty) = if let Some(value) = value {
            let expr = self.infer_expr(value)?;
            let ty = expr.ty.clone();
            (Some(Box::new(expr)), ty)
        } else {
            (None, Type::unit())
        };

        // Record the break type for computing the loop's result type
        if let Some(loop_id) = loop_id {
            self.record_break_type(loop_id, break_ty);
        }

        Ok(hir::Expr::new(
            hir::ExprKind::Break {
                label: loop_id,
                value: value_expr,
            },
            Type::never(),
            span,
        ))
    }

    /// Infer type of a continue.
    pub(crate) fn infer_continue(&mut self, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        if !self.resolver.in_loop() {
            return Err(Box::new(TypeError::new(TypeErrorKind::ContinueOutsideLoop, span)));
        }

        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.find_loop(label_str.as_deref());

        // Check that the label exists
        if label.is_some() && loop_id.is_none() {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::UnresolvedName { name: label_str.unwrap_or_default() },
                span,
            )));
        }

        Ok(hir::Expr::new(
            hir::ExprKind::Continue { label: loop_id },
            Type::never(),
            span,
        ))
    }

    /// Infer type of a match expression.
    pub(crate) fn infer_match(
        &mut self,
        scrutinee: &ast::Expr,
        arms: &[ast::MatchArm],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
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

        // Check for exhaustiveness
        let enum_info = self.get_enum_variant_info(&scrutinee_expr.ty);
        let result = exhaustiveness::check_exhaustiveness(
            &hir_arms,
            &scrutinee_expr.ty,
            enum_info.as_ref(),
        );

        if !result.is_exhaustive {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::NonExhaustivePatterns {
                    missing: result.missing_patterns,
                },
                span,
            )));
        }

        // Report unreachable patterns
        for idx in result.unreachable_arms {
            if let Some(arm) = arms.get(idx) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::UnreachablePattern,
                    arm.span,
                ));
            }
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

    /// Check a match expression against an expected type.
    /// This propagates the expected type to all arm bodies for better type inference.
    pub(crate) fn check_match(
        &mut self,
        scrutinee: &ast::Expr,
        arms: &[ast::MatchArm],
        expected: &Type,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
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

        // Use expected type directly instead of fresh variable
        let mut hir_arms = Vec::new();

        for arm in arms {
            self.resolver.push_scope(ScopeKind::MatchArm, arm.span);

            let pattern = self.lower_pattern(&arm.pattern, &scrutinee_expr.ty)?;

            let guard = if let Some(ref guard) = arm.guard {
                Some(self.check_expr(guard, &Type::bool())?)
            } else {
                None
            };

            let body = self.check_expr(&arm.body, expected)?;

            self.resolver.pop_scope();

            hir_arms.push(hir::MatchArm {
                pattern,
                guard,
                body,
            });
        }

        // Check for exhaustiveness
        let enum_info = self.get_enum_variant_info(&scrutinee_expr.ty);
        let result = exhaustiveness::check_exhaustiveness(
            &hir_arms,
            &scrutinee_expr.ty,
            enum_info.as_ref(),
        );

        if !result.is_exhaustive {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::NonExhaustivePatterns {
                    missing: result.missing_patterns,
                },
                span,
            )));
        }

        // Report unreachable patterns
        for idx in result.unreachable_arms {
            if let Some(arm) = arms.get(idx) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::UnreachablePattern,
                    arm.span,
                ));
            }
        }

        let final_ty = self.unifier.resolve(expected);

        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(scrutinee_expr),
                arms: hir_arms,
            },
            final_ty,
            span,
        ))
    }

    /// Infer type of a record (struct) construction expression.
    ///
    /// Supports struct spread syntax: `MyStruct { field1: val1, ..base }`
    /// where `base` is an expression that provides values for fields not explicitly listed.
    pub(crate) fn infer_record(
        &mut self,
        path: Option<&ast::TypePath>,
        fields: &[ast::RecordExprField],
        base: Option<&ast::Expr>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let (def_id, struct_info, result_ty) = if let Some(path) = path {
            if path.segments.len() == 1 {
                let name = self.symbol_to_string(path.segments[0].name.node);

                if let Some(Binding::Def(def_id)) = self.resolver.lookup(&name) {
                    if let Some(struct_info) = self.struct_defs.get(&def_id) {
                        let result_ty = Type::adt(def_id, Vec::new());
                        (def_id, struct_info.clone(), result_ty)
                    } else if let Some(handler_info) = self.handler_defs.get(&def_id) {
                        // Handlers with state fields are instantiable like structs
                        let struct_info = super::StructInfo {
                            name: handler_info.name.clone(),
                            fields: handler_info.fields.clone(),
                            generics: handler_info.generics.clone(),
                        };
                        let result_ty = Type::adt(def_id, Vec::new());
                        (def_id, struct_info, result_ty)
                    } else {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: Type::adt(def_id, Vec::new()) },
                            span,
                        )));
                    }
                } else {
                    return Err(self.error_type_not_found(&name, span));
                }
            } else if path.segments.len() == 2 {
                // Two-segment path: Module::Struct
                let module_name = self.symbol_to_string(path.segments[0].name.node);
                let type_name = self.symbol_to_string(path.segments[1].name.node);

                // First check bridge definitions
                let mut found_def_id: Option<DefId> = None;
                for bridge_info in &self.bridge_defs {
                    if bridge_info.name == module_name {
                        for struct_info in &bridge_info.structs {
                            if struct_info.name == type_name {
                                found_def_id = Some(struct_info.def_id);
                                break;
                            }
                        }
                    }
                    if found_def_id.is_some() { break; }
                }

                if let Some(def_id) = found_def_id {
                    if let Some(struct_info) = self.struct_defs.get(&def_id) {
                        let result_ty = Type::adt(def_id, Vec::new());
                        (def_id, struct_info.clone(), result_ty)
                    } else {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: Type::adt(def_id, Vec::new()) },
                            span,
                        )));
                    }
                } else {
                    // Check if first segment is an enum type and second is a variant
                    // This handles EnumName::Variant { field: value } syntax
                    if let Some(enum_def_id) = self.resolver.lookup_type(&module_name) {
                        if let Some(enum_info) = self.enum_defs.get(&enum_def_id).cloned() {
                            if let Some(variant) = enum_info.variants.iter().find(|v| v.name == type_name) {
                                // Found an enum variant - handle struct-like construction
                                let variant_idx = variant.index;
                                let variant_fields = variant.fields.clone();

                                // Create fresh type variables for enum generics
                                let type_args: Vec<Type> = enum_info.generics.iter()
                                    .map(|_| self.unifier.fresh_var())
                                    .collect();

                                // Build substitution map from generic params to fresh vars
                                let subst: std::collections::HashMap<TyVarId, Type> = enum_info.generics.iter()
                                    .zip(type_args.iter())
                                    .map(|(&tyvar, ty)| (tyvar, ty.clone()))
                                    .collect();

                                // Type-check each field from the expression
                                let mut hir_field_exprs = Vec::new();
                                for field in fields {
                                    let field_name = self.symbol_to_string(field.name.node);

                                    // Find the corresponding variant field
                                    let variant_field = match variant_fields.iter().find(|f| f.name == field_name) {
                                        Some(f) => f,
                                        None => {
                                            return Err(Box::new(TypeError::new(
                                                TypeErrorKind::UnknownField {
                                                    ty: Type::adt(enum_def_id, type_args.clone()),
                                                    field: field_name,
                                                },
                                                field.span,
                                            )));
                                        }
                                    };

                                    // Apply substitution to field type
                                    let expected_ty = self.substitute_type_vars(&variant_field.ty, &subst);

                                    // Handle shorthand syntax: `{ x }` means `{ x: x }`
                                    let value_expr = if let Some(value) = &field.value {
                                        // Use check_expr to propagate expected type for better literal inference
                                        self.check_expr(value, &expected_ty).map_err(|_| {
                                            // Type::error() is acceptable here: a diagnostic is emitted
                                            // below. The found type is only used for the error message.
                                            let found = self.infer_expr(value).map(|e| e.ty).unwrap_or_else(|_| Type::error());
                                            TypeError::new(
                                                TypeErrorKind::Mismatch {
                                                    expected: self.unifier.resolve(&expected_ty),
                                                    found: self.unifier.resolve(&found),
                                                },
                                                value.span,
                                            )
                                        })?
                                    } else {
                                        // Shorthand: look up the field name as a variable
                                        let path = ast::ExprPath {
                                            segments: vec![ast::ExprPathSegment {
                                                name: field.name.clone(),
                                                args: None,
                                            }],
                                            span: field.span,
                                        };
                                        let expr = ast::Expr {
                                            kind: ast::ExprKind::Path(path),
                                            span: field.span,
                                        };
                                        let inferred = self.infer_expr(&expr)?;
                                        self.unifier.unify(&inferred.ty, &expected_ty, field.span).map_err(|_| {
                                            TypeError::new(
                                                TypeErrorKind::Mismatch {
                                                    expected: self.unifier.resolve(&expected_ty),
                                                    found: self.unifier.resolve(&inferred.ty),
                                                },
                                                field.span,
                                            )
                                        })?;
                                        inferred
                                    };

                                    hir_field_exprs.push((variant_field.index, value_expr));
                                }

                                // Sort fields by index to ensure correct order in Variant
                                hir_field_exprs.sort_by_key(|(idx, _)| *idx);
                                let ordered_fields: Vec<hir::Expr> = hir_field_exprs.into_iter()
                                    .map(|(_, expr)| expr)
                                    .collect();

                                // Build result type with resolved type args
                                let resolved_type_args: Vec<Type> = type_args.iter()
                                    .map(|ty| self.unifier.resolve(ty))
                                    .collect();
                                let enum_ty = Type::adt(enum_def_id, resolved_type_args);

                                return Ok(hir::Expr::new(
                                    hir::ExprKind::Variant {
                                        def_id: enum_def_id,
                                        variant_idx,
                                        fields: ordered_fields,
                                    },
                                    enum_ty,
                                    span,
                                ));
                            }
                        }
                    }

                    // Try to find in module_defs
                    let mut module_def_id: Option<DefId> = None;
                    for (def_id, info) in &self.module_defs {
                        if info.name == module_name {
                            module_def_id = Some(*def_id);
                            break;
                        }
                    }

                    if let Some(mod_def_id) = module_def_id {
                        // Find the struct within the module's items (properly scoped)
                        let mut found_struct: Option<(DefId, super::StructInfo)> = None;

                        if let Some(mod_info) = self.module_defs.get(&mod_def_id) {
                            // Check direct items first
                            for &item_def_id in &mod_info.items {
                                if let Some(struct_info) = self.struct_defs.get(&item_def_id) {
                                    if struct_info.name == type_name {
                                        found_struct = Some((item_def_id, struct_info.clone()));
                                        break;
                                    }
                                }
                            }

                            // If not found in items, check re-exports (from `pub use`)
                            if found_struct.is_none() {
                                for (reexport_name, reexport_def_id, _vis) in &mod_info.reexports {
                                    if reexport_name == &type_name {
                                        if let Some(struct_info) = self.struct_defs.get(reexport_def_id) {
                                            found_struct = Some((*reexport_def_id, struct_info.clone()));
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if let Some((def_id, struct_info)) = found_struct {
                            let result_ty = Type::adt(def_id, Vec::new());
                            (def_id, struct_info, result_ty)
                        } else {
                            return Err(self.error_type_not_found(
                                &format!("{}::{}", module_name, type_name),
                                span,
                            ));
                        }
                    } else {
                        return Err(self.error_type_not_found(
                            &format!("{}::{}", module_name, type_name),
                            span,
                        ));
                    }
                }
            } else {
                // Multi-segment path: resolve through module hierarchy
                // e.g., std::collections::HashMap { ... } or module::Enum::Variant { ... }
                let segments: Vec<String> = path.segments.iter()
                    .map(|s| self.symbol_to_string(s.name.node))
                    .collect();

                // Check for 3-segment pattern: module::Enum::Variant { fields }
                if segments.len() == 3 {
                    let module_name = &segments[0];
                    let enum_name = &segments[1];
                    let variant_name = &segments[2];

                    // Try to find the module
                    let mut module_def_id: Option<DefId> = None;
                    for (def_id, info) in &self.module_defs {
                        if &info.name == module_name {
                            module_def_id = Some(*def_id);
                            break;
                        }
                    }

                    if let Some(mod_def_id) = module_def_id {
                        // Find the enum within the module's items
                        let mut found_enum: Option<(DefId, super::EnumInfo)> = None;

                        if let Some(mod_info) = self.module_defs.get(&mod_def_id) {
                            for &item_def_id in &mod_info.items {
                                if let Some(enum_info) = self.enum_defs.get(&item_def_id) {
                                    if &enum_info.name == enum_name {
                                        found_enum = Some((item_def_id, enum_info.clone()));
                                        break;
                                    }
                                }
                            }
                        }

                        if let Some((enum_def_id, enum_info)) = found_enum {
                            // Find the variant
                            if let Some(variant) = enum_info.variants.iter().find(|v| &v.name == variant_name) {
                                // Found an enum variant - handle struct-like construction
                                let variant_idx = variant.index;
                                let variant_fields = variant.fields.clone();

                                // Extract type arguments from the path if provided (e.g., module::Enum::<i32>::Variant)
                                // For now, check the enum segment for type args
                                let path_type_args: Vec<Type> = if let Some(args) = &path.segments[1].args {
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

                                // Create fresh type variables for enum generics (if no explicit args provided)
                                let type_args: Vec<Type> = if !path_type_args.is_empty() {
                                    path_type_args
                                } else {
                                    enum_info.generics.iter()
                                        .map(|_| self.unifier.fresh_var())
                                        .collect()
                                };

                                // Build substitution map from generic params to type args
                                let subst: std::collections::HashMap<TyVarId, Type> = enum_info.generics.iter()
                                    .copied()
                                    .zip(type_args.iter().cloned())
                                    .collect();

                                // Type-check each field from the expression
                                let mut hir_field_exprs = Vec::new();
                                for field in fields {
                                    let field_name = self.symbol_to_string(field.name.node);

                                    // Find the corresponding variant field
                                    let variant_field = match variant_fields.iter().find(|f| f.name == field_name) {
                                        Some(f) => f,
                                        None => {
                                            return Err(Box::new(TypeError::new(
                                                TypeErrorKind::UnknownField {
                                                    ty: Type::adt(enum_def_id, type_args.clone()),
                                                    field: field_name,
                                                },
                                                field.span,
                                            )));
                                        }
                                    };

                                    // Apply substitution to field type
                                    let expected_ty = self.substitute_type_vars(&variant_field.ty, &subst);

                                    // Handle shorthand syntax: `{ x }` means `{ x: x }`
                                    let value_expr = if let Some(value) = &field.value {
                                        // Use check_expr to propagate expected type for better literal inference
                                        self.check_expr(value, &expected_ty).map_err(|_| {
                                            // Type::error() is acceptable here: a diagnostic is emitted
                                            // below. The found type is only used for the error message.
                                            let found = self.infer_expr(value).map(|e| e.ty).unwrap_or_else(|_| Type::error());
                                            TypeError::new(
                                                TypeErrorKind::Mismatch {
                                                    expected: self.unifier.resolve(&expected_ty),
                                                    found: self.unifier.resolve(&found),
                                                },
                                                value.span,
                                            )
                                        })?
                                    } else {
                                        // Shorthand: look up the field name as a variable
                                        let var_path = ast::ExprPath {
                                            segments: vec![ast::ExprPathSegment {
                                                name: field.name.clone(),
                                                args: None,
                                            }],
                                            span: field.span,
                                        };
                                        let expr = ast::Expr {
                                            kind: ast::ExprKind::Path(var_path),
                                            span: field.span,
                                        };
                                        let inferred = self.infer_expr(&expr)?;
                                        self.unifier.unify(&inferred.ty, &expected_ty, field.span).map_err(|_| {
                                            TypeError::new(
                                                TypeErrorKind::Mismatch {
                                                    expected: self.unifier.resolve(&expected_ty),
                                                    found: self.unifier.resolve(&inferred.ty),
                                                },
                                                field.span,
                                            )
                                        })?;
                                        inferred
                                    };

                                    hir_field_exprs.push((variant_field.index, value_expr));
                                }

                                // Sort fields by index to ensure correct order in Variant
                                hir_field_exprs.sort_by_key(|(idx, _)| *idx);
                                let ordered_fields: Vec<hir::Expr> = hir_field_exprs.into_iter()
                                    .map(|(_, expr)| expr)
                                    .collect();

                                // Build result type with resolved type args
                                let resolved_type_args: Vec<Type> = type_args.iter()
                                    .map(|ty| self.unifier.resolve(ty))
                                    .collect();
                                let enum_ty = Type::adt(enum_def_id, resolved_type_args);

                                return Ok(hir::Expr::new(
                                    hir::ExprKind::Variant {
                                        def_id: enum_def_id,
                                        variant_idx,
                                        fields: ordered_fields,
                                    },
                                    enum_ty,
                                    span,
                                ));
                            }
                        }
                    }
                }

                // Fall through to struct resolution for non-enum cases
                // Create an AST type path and resolve it
                let ast_type = ast::Type {
                    kind: ast::TypeKind::Path(path.clone()),
                    span,
                };

                let struct_ty = self.ast_type_to_hir_type(&ast_type)?;

                // Extract the DefId from the resolved type
                if let TypeKind::Adt { def_id, args: type_args } = struct_ty.kind() {
                    // Check if it's a struct
                    if let Some(struct_info) = self.struct_defs.get(def_id).cloned() {
                        let mut hir_fields = Vec::new();

                        // Build substitution map for generics
                        let subst: std::collections::HashMap<hir::ty::TyVarId, Type> = struct_info.generics.iter()
                            .cloned()
                            .zip(type_args.iter().cloned())
                            .collect();

                        // Type-check each field
                        for field in fields {
                            let field_name = self.symbol_to_string(field.name.node);
                            let expected_ty = struct_info.fields.iter()
                                .find(|f| f.name == field_name)
                                .map(|f| self.substitute_type_vars(&f.ty, &subst))
                                .ok_or_else(|| Box::new(TypeError::new(
                                    TypeErrorKind::Mismatch {
                                        expected: struct_ty.clone(),
                                        found: Type::error(),
                                    },
                                    field.span,
                                )))?;

                            let value_expr = if let Some(value) = &field.value {
                                self.check_expr(value, &expected_ty)?
                            } else {
                                // Shorthand: look up variable with same name
                                let var_path = ast::ExprPath {
                                    segments: vec![ast::ExprPathSegment {
                                        name: field.name.clone(),
                                        args: None,
                                    }],
                                    span: field.span,
                                };
                                let expr = ast::Expr {
                                    kind: ast::ExprKind::Path(var_path),
                                    span: field.span,
                                };
                                self.check_expr(&expr, &expected_ty)?
                            };

                            let field_idx = struct_info.fields.iter()
                                .position(|f| f.name == field_name)
                                .unwrap_or(0) as u32;

                            hir_fields.push(hir::FieldExpr {
                                field_idx,
                                value: value_expr,
                            });
                        }

                        return Ok(hir::Expr::new(
                            hir::ExprKind::Struct {
                                def_id: *def_id,
                                fields: hir_fields,
                                base: None,
                            },
                            struct_ty,
                            span,
                        ));
                    }
                }

                return Err(Box::new(TypeError::new(
                    TypeErrorKind::TypeNotFound {
                        name: segments.join("::"),
                    },
                    span,
                )));
            }
        } else {
            // Anonymous record literal: { x: 1, y: 2 }
            // Type-check each field and build a Record type
            let mut hir_fields = Vec::new();
            let mut record_fields = Vec::new();

            for field in fields {
                let field_name = field.name.node;

                // Infer the value type (or look up variable for shorthand)
                let value_expr = if let Some(value) = &field.value {
                    self.infer_expr(value)?
                } else {
                    // Shorthand: { x } means { x: x }
                    let path = ast::ExprPath {
                        segments: vec![ast::ExprPathSegment {
                            name: field.name.clone(),
                            args: None,
                        }],
                        span: field.span,
                    };
                    let expr = ast::Expr {
                        kind: ast::ExprKind::Path(path),
                        span: field.span,
                    };
                    self.infer_expr(&expr)?
                };

                // Add to record type fields
                record_fields.push(hir::RecordField {
                    name: field_name,
                    ty: value_expr.ty.clone(),
                });

                // Add to HIR expression fields
                hir_fields.push(hir::RecordFieldExpr {
                    name: field_name,
                    value: value_expr,
                });
            }

            // Create the record type
            let result_ty = Type::record(record_fields, None);

            return Ok(hir::Expr::new(
                hir::ExprKind::Record { fields: hir_fields },
                result_ty,
                span,
            ));
        };

        // For structs/handlers with generics, create fresh type vars for the generics
        // and collect them so unification can determine concrete types from field values
        let type_args: Vec<Type> = if !struct_info.generics.is_empty() {
            struct_info.generics.iter().map(|_| self.unifier.fresh_var()).collect()
        } else {
            Vec::new()
        };

        // Create a substitution map from generic type param ids to fresh type vars
        let subst: std::collections::HashMap<TyVarId, Type> = struct_info.generics.iter()
            .zip(type_args.iter())
            .map(|(ty_var_id, ty)| (*ty_var_id, ty.clone()))
            .collect();

        // Collect explicitly provided field names for struct spread validation
        let provided_field_names: std::collections::HashSet<String> = fields.iter()
            .map(|f| self.symbol_to_string(f.name.node))
            .collect();

        // Type-check the base expression for struct spread syntax (..base)
        let hir_base = if let Some(base_expr) = base {
            let base_hir = self.infer_expr(base_expr)?;
            let base_ty = self.unifier.resolve(&base_hir.ty);

            // Verify the base expression has the same struct type
            match base_ty.kind() {
                TypeKind::Adt { def_id: base_def_id, args: base_args } => {
                    if *base_def_id != def_id {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::Mismatch {
                                expected: result_ty.clone(),
                                found: base_ty,
                            },
                            base_expr.span,
                        )));
                    }

                    // Unify type arguments from base with our fresh type vars
                    for (base_arg, type_arg) in base_args.iter().zip(type_args.iter()) {
                        self.unifier.unify(base_arg, type_arg, base_expr.span).map_err(|_| {
                            TypeError::new(
                                TypeErrorKind::Mismatch {
                                    expected: type_arg.clone(),
                                    found: base_arg.clone(),
                                },
                                base_expr.span,
                            )
                        })?;
                    }
                }
                _ => {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotAStruct { ty: base_ty },
                        base_expr.span,
                    )));
                }
            }

            Some(Box::new(base_hir))
        } else {
            None
        };

        // Type-check explicitly provided fields, substituting generics with fresh type vars
        let mut hir_fields = Vec::new();
        for field in fields {
            let field_name = self.symbol_to_string(field.name.node);

            let field_info = match struct_info.fields.iter().find(|f| f.name == field_name) {
                Some(info) => info,
                None => return Err(self.error_unknown_field(&result_ty, &field_name, field.span)),
            };

            // Apply substitution to field type (replace generics with fresh vars)
            let expected_ty = self.substitute_type_vars(&field_info.ty, &subst);

            // Handle shorthand syntax: `{ x }` is equivalent to `{ x: x }`
            let value_expr = if let Some(value) = &field.value {
                // Use check_expr to propagate expected type for better literal inference
                self.check_expr(value, &expected_ty).map_err(|_| {
                    // Type::error() is acceptable here: a diagnostic is emitted below.
                    // The found type is only used for the error message.
                    let found = self.infer_expr(value).map(|e| e.ty).unwrap_or_else(|_| Type::error());
                    TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: self.unifier.resolve(&expected_ty),
                            found: self.unifier.resolve(&found),
                        },
                        value.span,
                    )
                })?
            } else {
                // Shorthand: look up the field name as a variable
                let path = ast::ExprPath {
                    segments: vec![ast::ExprPathSegment {
                        name: field.name.clone(),
                        args: None,
                    }],
                    span: field.span,
                };
                let expr = ast::Expr {
                    kind: ast::ExprKind::Path(path),
                    span: field.span,
                };
                let inferred = self.infer_expr(&expr)?;
                self.unifier.unify(&inferred.ty, &expected_ty, field.span).map_err(|_| {
                    TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: self.unifier.resolve(&expected_ty),
                            found: self.unifier.resolve(&inferred.ty),
                        },
                        field.span,
                    )
                })?;
                inferred
            };

            hir_fields.push(hir::FieldExpr {
                field_idx: field_info.index,
                value: value_expr,
            });
        }

        // If no base is provided, verify all fields are present
        if hir_base.is_none() {
            for field_info in &struct_info.fields {
                if !provided_field_names.contains(&field_info.name) {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::MissingField {
                            ty: result_ty.clone(),
                            field: field_info.name.clone(),
                        },
                        span,
                    )));
                }
            }
        }

        // Build result type with resolved type args
        let resolved_type_args: Vec<Type> = type_args.iter()
            .map(|ty| self.unifier.resolve(ty))
            .collect();
        let result_ty = Type::adt(def_id, resolved_type_args);

        Ok(hir::Expr::new(
            hir::ExprKind::Struct {
                def_id,
                fields: hir_fields,
                base: hir_base,
            },
            result_ty,
            span,
        ))
    }

    /// DEF-001: Extract a chain of module path segments from an expression.
    /// Returns None if the expression isn't a chain of field accesses on a path,
    /// or if the base resolves to a local variable.
    fn try_extract_module_chain(&self, expr: &ast::Expr) -> Option<Vec<String>> {
        match &expr.kind {
            ast::ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    let name = self.symbol_to_string(path.segments[0].name.node);
                    // Guard: skip if name resolves to a local variable
                    if let Some(Binding::Local { .. }) = self.resolver.lookup(&name) {
                        return None;
                    }
                    Some(vec![name])
                } else {
                    None
                }
            }
            ast::ExprKind::Field { base, field } => {
                if let ast::FieldAccess::Named(field_name) = field {
                    if let Some(mut segments) = self.try_extract_module_chain(base) {
                        let fname = self.symbol_to_string(field_name.node);
                        segments.push(fname);
                        Some(segments)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// DEF-001: Try to resolve a qualified path from module chain segments.
    /// Returns Some(hir::Expr) if the chain resolves to a module item.
    fn try_resolve_qualified_chain(&mut self, segments: &[String], span: Span) -> Option<Result<hir::Expr, Box<TypeError>>> {
        if segments.len() < 2 {
            return None;
        }

        // Walk the module chain
        let first_name = &segments[0];

        // Look up first segment - must be a module
        let module_def_id = match self.resolver.lookup(first_name) {
            Some(Binding::Def(def_id)) => def_id,
            Some(_) => return None,
            None => return None,
        };

        let module_info = match self.module_defs.get(&module_def_id).cloned() {
            Some(info) => info,
            None => return None,
        };

        // For two-segment paths: module.item
        if segments.len() == 2 {
            let item_name = &segments[1];
            for &item_def_id in &module_info.items {
                if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                    if def_info.name == *item_name {
                        if let Some(sig) = self.fn_sigs.get(&item_def_id).cloned() {
                            let fn_ty = if sig.generics.is_empty() {
                                Type::function(sig.inputs.clone(), sig.output.clone())
                            } else {
                                self.instantiate_fn_sig(&sig)
                            };
                            return Some(Ok(hir::Expr::new(
                                hir::ExprKind::Def(item_def_id),
                                fn_ty,
                                span,
                            )));
                        } else if let Some(ty) = &def_info.ty {
                            return Some(Ok(hir::Expr::new(
                                hir::ExprKind::Def(item_def_id),
                                ty.clone(),
                                span,
                            )));
                        }
                    }
                }
            }
            // Check if it's a nested module
            for &item_def_id in &module_info.items {
                if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                    if def_info.name == *item_name && self.module_defs.contains_key(&item_def_id) {
                        // It's a sub-module - return as a Def
                        return Some(Ok(hir::Expr::new(
                            hir::ExprKind::Def(item_def_id),
                            self.unifier.fresh_var(),
                            span,
                        )));
                    }
                }
            }
            return None;
        }

        // For longer chains: module.submodule...item
        // Walk through intermediate modules
        let mut current_items = module_info.items.clone();
        for i in 1..segments.len() - 1 {
            let seg_name = &segments[i];
            let mut found_submod = false;
            for &item_def_id in &current_items {
                if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                    if def_info.name == *seg_name {
                        if let Some(sub_info) = self.module_defs.get(&item_def_id).cloned() {
                            current_items = sub_info.items;
                            found_submod = true;
                            break;
                        }
                    }
                }
            }
            if !found_submod {
                return None;
            }
        }

        // Resolve the last segment in the final module
        let last_name = &segments[segments.len() - 1];
        for &item_def_id in &current_items {
            if let Some(def_info) = self.resolver.def_info.get(&item_def_id) {
                if def_info.name == *last_name {
                    if let Some(sig) = self.fn_sigs.get(&item_def_id).cloned() {
                        let fn_ty = if sig.generics.is_empty() {
                            Type::function(sig.inputs.clone(), sig.output.clone())
                        } else {
                            self.instantiate_fn_sig(&sig)
                        };
                        return Some(Ok(hir::Expr::new(
                            hir::ExprKind::Def(item_def_id),
                            fn_ty,
                            span,
                        )));
                    } else if let Some(ty) = &def_info.ty {
                        return Some(Ok(hir::Expr::new(
                            hir::ExprKind::Def(item_def_id),
                            ty.clone(),
                            span,
                        )));
                    }
                }
            }
        }

        None
    }

    /// DEF-001: Try to resolve module.field as a qualified path.
    fn try_resolve_dot_as_qualified_path(
        &mut self,
        base: &ast::Expr,
        field: &ast::FieldAccess,
        span: Span,
    ) -> Option<Result<hir::Expr, Box<TypeError>>> {
        if let ast::FieldAccess::Named(_) = field {
            if let Some(segments) = self.try_extract_module_chain(base) {
                let mut full_segments = segments;
                if let ast::FieldAccess::Named(field_name) = field {
                    let fname = self.symbol_to_string(field_name.node);
                    full_segments.push(fname);
                }
                return self.try_resolve_qualified_chain(&full_segments, span);
            }
        }
        None
    }

    /// DEF-001: Try to resolve module.func(args) as a qualified call.
    fn try_resolve_dot_call_as_qualified(
        &mut self,
        receiver: &ast::Expr,
        method: &ast::Symbol,
        args: &[ast::CallArg],
        span: Span,
    ) -> Option<Result<hir::Expr, Box<TypeError>>> {
        let chain = self.try_extract_module_chain(receiver);
        if let Some(mut segments) = chain {
            let method_name = self.symbol_to_string(*method);
            segments.push(method_name);
            if let Some(callee_result) = self.try_resolve_qualified_chain(&segments, span) {
                // If we resolved the callee, type-check args and build a Call expression
                match callee_result {
                    Ok(callee_expr) => {
                        // Extract param and return types from the callee's function type
                        let callee_ty = self.unifier.resolve(&callee_expr.ty);
                        let (param_types, return_type) = match callee_ty.kind() {
                            TypeKind::Fn { params, ret, .. } => (params.clone(), ret.clone()),
                            _ => {
                                // Callee resolved but isn't a function — fall through to method call
                                return None;
                            }
                        };

                        // Infer argument expressions and unify with parameter types
                        let mut hir_args = Vec::with_capacity(args.len());
                        for (i, arg) in args.iter().enumerate() {
                            match self.infer_expr(&arg.value) {
                                Ok(arg_expr) => {
                                    if i < param_types.len() {
                                        let _ = self.unifier.unify(&arg_expr.ty, &param_types[i], span);
                                    }
                                    hir_args.push(arg_expr);
                                }
                                Err(e) => return Some(Err(e)),
                            }
                        }

                        return Some(Ok(hir::Expr::new(
                            hir::ExprKind::Call {
                                callee: Box::new(callee_expr),
                                args: hir_args,
                            },
                            return_type,
                            span,
                        )));
                    }
                    Err(e) => return Some(Err(e)),
                }
            }
        }
        None
    }

    /// Infer type of a field access expression.
    pub(crate) fn infer_field_access(
        &mut self,
        base: &ast::Expr,
        field: &ast::FieldAccess,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let base_expr = self.infer_expr(base)?;
        let base_ty = self.unifier.resolve(&base_expr.ty);

        // Auto-deref references for field access (iterative, like Rust).
        // Handles arbitrarily nested references: &&T, &&&T, etc.
        let (deref_expr, inner_ty) = {
            let mut current_expr = base_expr;
            let mut current_ty = base_ty.clone();
            while let TypeKind::Ref { inner, .. } = current_ty.kind() {
                let deref_ty = self.unifier.resolve(inner);
                current_expr = hir::Expr::new(
                    hir::ExprKind::Deref(Box::new(current_expr)),
                    deref_ty.clone(),
                    span,
                );
                current_ty = deref_ty;
            }
            (current_expr, current_ty)
        };

        // Unwrap ownership types for field access (linear/affine are compile-time only)
        let inner_ty = if let TypeKind::Ownership { inner, .. } = inner_ty.kind() {
            self.unifier.resolve(inner)
        } else {
            inner_ty
        };

        match field {
            ast::FieldAccess::Named(name) => {
                let field_name = self.symbol_to_string(name.node);

                // Check ADT (struct) types
                if let TypeKind::Adt { def_id, args } = inner_ty.kind() {
                    if let Some(struct_info) = self.struct_defs.get(def_id).cloned() {
                        if let Some(field_info) = struct_info.fields.iter().find(|f| f.name == field_name) {
                            // Build substitution map from struct's generic params to concrete args
                            // For example, if struct Pair<T> has field first: T, and we have Pair<i32>,
                            // we need to substitute T -> i32 in the field type.
                            let subst: std::collections::HashMap<crate::hir::ty::TyVarId, Type> =
                                struct_info.generics.iter()
                                    .zip(args.iter())
                                    .map(|(&tyvar, arg)| (tyvar, arg.clone()))
                                    .collect();

                            // Substitute type parameters in the field type
                            let field_ty = self.substitute_type_vars(&field_info.ty, &subst);

                            return Ok(hir::Expr::new(
                                hir::ExprKind::Field {
                                    base: Box::new(deref_expr),
                                    field_idx: field_info.index,
                                },
                                field_ty,
                                span,
                            ));
                        } else {
                            return Err(self.error_unknown_field(&inner_ty, &field_name, span));
                        }
                    } else {
                        // Type is an ADT but struct definition not found - this indicates
                        // a cross-module struct whose field information wasn't registered.
                        return Err(self.error_unknown_field(&inner_ty, &field_name, span));
                    }
                }

                // Check anonymous record types
                if let TypeKind::Record { fields, .. } = inner_ty.kind() {
                    for (idx, record_field) in fields.iter().enumerate() {
                        let record_field_name = self.symbol_to_string(record_field.name);
                        if record_field_name == field_name {
                            return Ok(hir::Expr::new(
                                hir::ExprKind::Field {
                                    base: Box::new(deref_expr),
                                    field_idx: idx as u32,
                                },
                                record_field.ty.clone(),
                                span,
                            ));
                        }
                    }
                    return Err(self.error_unknown_field(&inner_ty, &field_name, span));
                }

                Err(Box::new(TypeError::new(
                    TypeErrorKind::NotAStruct { ty: inner_ty },
                    span,
                )))
            }
            ast::FieldAccess::Index(index, _span) => {
                if let TypeKind::Tuple(types) = inner_ty.kind() {
                    if (*index as usize) < types.len() {
                        let field_ty = types[*index as usize].clone();
                        return Ok(hir::Expr::new(
                            hir::ExprKind::Field {
                                base: Box::new(deref_expr),
                                field_idx: *index,
                            },
                            field_ty,
                            span,
                        ));
                    }
                }

                Err(Box::new(TypeError::new(
                    TypeErrorKind::NotATuple { ty: inner_ty },
                    span,
                )))
            }
        }
    }

    /// Infer type of a cast expression.
    fn infer_cast(&mut self, inner: &ast::Expr, ty: &ast::Type, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        let inner_expr = self.infer_expr(inner)?;
        let target_ty = self.ast_type_to_hir_type(ty)?;

        // Validate the cast is legal
        let source_ty = self.unifier.resolve(&inner_expr.ty);
        let resolved_target = self.unifier.resolve(&target_ty);
        if !Self::is_cast_compatible(&source_ty, &resolved_target) {
            return Err(Box::new(TypeError::new(
                TypeErrorKind::InvalidCast {
                    source: source_ty,
                    target: resolved_target,
                },
                span,
            )));
        }

        Ok(hir::Expr::new(
            hir::ExprKind::Cast {
                expr: Box::new(inner_expr),
                target_ty: target_ty.clone(),
            },
            target_ty,
            span,
        ))
    }

    /// Check whether casting from `source` to `target` is valid.
    fn is_cast_compatible(source: &Type, target: &Type) -> bool {
        use crate::hir::{PrimitiveTy, TypeKind};

        match (source.kind(), target.kind()) {
            // Unresolved inference variables — defer to later resolution
            (TypeKind::Infer(_), _) | (_, TypeKind::Infer(_)) => true,
            // Error types can be cast to/from anything
            (TypeKind::Error, _) | (_, TypeKind::Error) => true,

            // Primitive → Primitive
            (TypeKind::Primitive(src), TypeKind::Primitive(dst)) => {
                let is_numeric = |p: &PrimitiveTy| matches!(p,
                    PrimitiveTy::Int(_) | PrimitiveTy::Uint(_) | PrimitiveTy::Float(_));
                // Numeric ↔ numeric (widening, narrowing, int↔float, sign reinterpret)
                if is_numeric(src) && is_numeric(dst) { return true; }
                // Bool → numeric
                if matches!(src, PrimitiveTy::Bool) && is_numeric(dst) { return true; }
                // Char ↔ numeric
                if matches!(src, PrimitiveTy::Char) && is_numeric(dst) { return true; }
                if is_numeric(src) && matches!(dst, PrimitiveTy::Char) { return true; }
                false
            }

            // Ptr → usize/isize
            (TypeKind::Ptr { .. }, TypeKind::Primitive(dst)) => {
                matches!(dst, PrimitiveTy::Uint(UintTy::Usize) | PrimitiveTy::Int(IntTy::Isize))
            }
            // usize/isize → Ptr
            (TypeKind::Primitive(src), TypeKind::Ptr { .. }) => {
                matches!(src, PrimitiveTy::Uint(UintTy::Usize) | PrimitiveTy::Int(IntTy::Isize))
            }

            // Ref → Ptr (&T → *const T, &mut T → *mut T, &mut T → *const T)
            (TypeKind::Ref { mutable: src_mut, .. }, TypeKind::Ptr { mutable: dst_mut, .. }) => {
                !dst_mut || *src_mut
            }
            // Ref → numeric (pointer packing, used in self-hosted compiler)
            (TypeKind::Ref { .. }, TypeKind::Primitive(dst)) => {
                matches!(dst,
                    PrimitiveTy::Uint(_) | PrimitiveTy::Int(_))
            }
            // Numeric → Ref (pointer unpacking, used in self-hosted compiler)
            (TypeKind::Primitive(src), TypeKind::Ref { .. }) => {
                matches!(src,
                    PrimitiveTy::Uint(_) | PrimitiveTy::Int(_))
            }

            // Ptr → Ptr (pointer coercion, respects mutability)
            (TypeKind::Ptr { mutable: src_mut, .. }, TypeKind::Ptr { mutable: dst_mut, .. }) => {
                !dst_mut || *src_mut
            }

            // Fn → numeric (function pointer packing)
            (TypeKind::Fn { .. }, TypeKind::Primitive(dst)) => {
                matches!(dst,
                    PrimitiveTy::Uint(_) | PrimitiveTy::Int(_))
            }

            // Ownership wrappers — check inner type
            (TypeKind::Ownership { inner, .. }, _) => Self::is_cast_compatible(inner, target),
            (_, TypeKind::Ownership { inner, .. }) => Self::is_cast_compatible(source, inner),

            // Everything else is invalid
            _ => false,
        }
    }

    /// Infer type of a compound assignment.
    fn infer_assign_op(
        &mut self,
        op: ast::BinOp,
        target: &ast::Expr,
        value: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Check mutability for simple variable compound assignments
        if let ast::ExprKind::Path(path) = &target.kind {
            if path.segments.len() == 1 {
                let name = self.symbol_to_string(path.segments[0].name.node);
                if let Some(Binding::Local { mutable, .. }) = self.resolver.lookup(&name) {
                    if !mutable {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::ImmutableAssign { name: name.clone() },
                            span,
                        ).with_help(format!("consider making `{name}` mutable: `let mut {name}`"))));
                    }
                }
            }
        }

        let target_expr = self.infer_expr(target)?;
        // Use check_expr to propagate target type for better literal inference
        let value_expr = self.check_expr(value, &target_expr.ty)?;
        let result_ty = target_expr.ty.clone();

        Ok(hir::Expr::new(
            hir::ExprKind::Assign {
                target: Box::new(target_expr.clone()),
                value: Box::new(hir::Expr::new(
                    hir::ExprKind::Binary {
                        op,
                        left: Box::new(target_expr),
                        right: Box::new(value_expr),
                    },
                    result_ty,
                    span,
                )),
            },
            Type::unit(),
            span,
        ))
    }

    /// Infer type of an unsafe block.
    fn infer_unsafe(&mut self, block: &ast::Block, span: Span) -> Result<hir::Expr, Box<TypeError>> {
        let expected = self.unifier.fresh_var();
        let block_expr = self.check_block(block, &expected)?;
        let result_ty = block_expr.ty.clone();

        Ok(hir::Expr::new(
            hir::ExprKind::Unsafe(Box::new(block_expr)),
            result_ty,
            span,
        ))
    }

    /// Infer type of a range expression.
    fn infer_range(
        &mut self,
        start: Option<&ast::Expr>,
        end: Option<&ast::Expr>,
        inclusive: bool,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let (start_expr, end_expr, element_ty) = match (start, end) {
            (Some(s), Some(e)) => {
                let start_expr = self.infer_expr(s)?;
                let element_ty = start_expr.ty.clone();
                let end_expr = self.check_expr(e, &element_ty)?;
                (Some(Box::new(start_expr)), Some(Box::new(end_expr)), element_ty)
            }
            (Some(s), None) => {
                let start_expr = self.infer_expr(s)?;
                let element_ty = start_expr.ty.clone();
                (Some(Box::new(start_expr)), None, element_ty)
            }
            (None, Some(e)) => {
                let end_expr = self.infer_expr(e)?;
                let element_ty = end_expr.ty.clone();
                (None, Some(Box::new(end_expr)), element_ty)
            }
            (None, None) => {
                let element_ty = Type::unit();
                (None, None, element_ty)
            }
        };

        let range_ty = Type::new(hir::TypeKind::Range {
            element: element_ty,
            inclusive,
        });

        Ok(hir::Expr::new(
            hir::ExprKind::Range {
                start: start_expr,
                end: end_expr,
                inclusive,
            },
            range_ty,
            span,
        ))
    }

    /// Infer type of a macro call expression.
    /// Built-in macros are expanded during type checking.
    fn infer_macro_call(
        &mut self,
        path: &ast::ExprPath,
        kind: &ast::MacroCallKind,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Get the macro name for dispatch
        let macro_name = path.segments.last()
            .map(|seg| self.interner.resolve(seg.name.node).unwrap_or("").to_string())
            .unwrap_or_default();

        match kind {
            // Format-style macros
            ast::MacroCallKind::Format { format_str, args, named_args } => {
                self.expand_format_macro(&macro_name, format_str, args, named_args, span)
            }

            // vec! macro
            ast::MacroCallKind::Vec(vec_args) => {
                self.expand_vec_macro(vec_args, span)
            }

            // assert! macro
            ast::MacroCallKind::Assert { condition, message } => {
                self.expand_assert_macro(condition, message.as_deref(), span)
            }

            // dbg! macro
            ast::MacroCallKind::Dbg(expr) => {
                self.expand_dbg_macro(expr, span)
            }

            // matches! macro
            ast::MacroCallKind::Matches { expr, pattern } => {
                self.expand_matches_macro(expr, pattern, span)
            }

            // Custom/user-defined macros - not yet supported
            ast::MacroCallKind::Custom { .. } => {
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: format!("user-defined macro `{}!`", macro_name),
                    },
                    span,
                )))
            }
        }
    }

    /// Expand format-style macros (format!, println!, print!, etc.)
    fn expand_format_macro(
        &mut self,
        macro_name: &str,
        format_str: &crate::span::Spanned<String>,
        args: &[ast::Expr],
        named_args: &[(String, ast::Expr)],
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Type-check all positional arguments
        let mut checked_args = Vec::new();
        for arg in args {
            let arg_expr = self.infer_expr(arg)?;
            checked_args.push(arg_expr);
        }

        // Type-check all named arguments
        let mut checked_named_args = Vec::new();
        for (name, arg) in named_args {
            let arg_expr = self.infer_expr(arg)?;
            checked_named_args.push((name.clone(), arg_expr));
        }

        // Determine return type based on macro
        let return_ty = match macro_name {
            "format" => {
                // format! returns &str (a string slice)
                // Note: The underlying conversion functions return BloodStr {ptr, len}
                // which is the runtime representation of &str.
                Type::reference(Type::str(), false)
            }
            "println" | "print" | "eprintln" | "eprint" => {
                // print macros return unit
                Type::unit()
            }
            "panic" | "unreachable" | "todo" | "unimplemented" => {
                // panic!, unreachable!, todo!, unimplemented! all return Never type
                // (they all terminate the program or branch)
                Type::new(hir::TypeKind::Never)
            }
            "write" | "writeln" => {
                // write macros return Result<(), Error>
                // For now, simplify to unit
                Type::unit()
            }
            _ => Type::unit(),
        };

        // For now, generate a placeholder Call expression
        // In a full implementation, we'd generate the actual string building code
        Ok(hir::Expr::new(
            hir::ExprKind::MacroExpansion {
                macro_name: macro_name.to_string(),
                format_str: format_str.node.clone(),
                args: checked_args,
                named_args: checked_named_args,
            },
            return_ty,
            span,
        ))
    }

    /// Expand vec! macro to array construction
    fn expand_vec_macro(
        &mut self,
        vec_args: &ast::VecMacroArgs,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        match vec_args {
            ast::VecMacroArgs::List(elements) => {
                // Type-check all elements
                let mut checked_elements = Vec::new();
                let element_ty = if elements.is_empty() {
                    self.unifier.fresh_var()
                } else {
                    let first = self.infer_expr(&elements[0])?;
                    let first_ty = first.ty.clone();
                    checked_elements.push(first);

                    for elem in &elements[1..] {
                        let elem_expr = self.infer_expr(elem)?;
                        // Unify element types
                        self.unifier.unify(&first_ty, &elem_expr.ty, elem.span)?;
                        checked_elements.push(elem_expr);
                    }
                    first_ty
                };

                // Return array type [T; N] where N is the element count
                let array_ty = Type::array(element_ty.clone(), checked_elements.len() as u64);

                Ok(hir::Expr::new(
                    hir::ExprKind::VecLiteral(checked_elements),
                    array_ty,
                    span,
                ))
            }
            ast::VecMacroArgs::Repeat { value, count } => {
                let value_expr = self.infer_expr(value)?;
                let count_expr = self.infer_expr(count)?;

                // Count should be an integer type (we accept any integer and extract the value)
                // Since we only support constant literals, we don't strictly require usize type
                let count_is_int = matches!(
                    count_expr.ty.kind(),
                    hir::TypeKind::Primitive(
                        hir::PrimitiveTy::Int(_) | hir::PrimitiveTy::Uint(_)
                    )
                );
                if !count_is_int {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::usize(),
                            found: count_expr.ty.clone(),
                        },
                        count.span,
                    )));
                }

                let element_ty = value_expr.ty.clone();

                // Try to extract the count as a constant for array type sizing
                let array_size = self.extract_usize_const(&count_expr);

                match array_size {
                    Some(size) => {
                        // Count is constant, create array type [T; N]
                        let array_ty = Type::array(element_ty, size);
                        Ok(hir::Expr::new(
                            hir::ExprKind::VecRepeat {
                                value: Box::new(value_expr),
                                count: Box::new(count_expr),
                            },
                            array_ty,
                            span,
                        ))
                    }
                    None => {
                        // Count is not a constant - for now, error
                        Err(Box::new(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "vec! repeat count must be a constant integer literal".to_string(),
                            },
                            span,
                        ).with_help("use a literal like `vec![0; 10]` instead of a variable")))
                    }
                }
            }
        }
    }

    /// Expand assert! macro to conditional panic
    fn expand_assert_macro(
        &mut self,
        condition: &ast::Expr,
        message: Option<&ast::Expr>,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let cond_expr = self.infer_expr(condition)?;

        // Condition must be bool
        let bool_ty = Type::new(hir::TypeKind::Primitive(hir::PrimitiveTy::Bool));
        self.unifier.unify(&bool_ty, &cond_expr.ty, condition.span)?;

        let msg_expr = if let Some(msg) = message {
            Some(Box::new(self.infer_expr(msg)?))
        } else {
            None
        };

        Ok(hir::Expr::new(
            hir::ExprKind::Assert {
                condition: Box::new(cond_expr),
                message: msg_expr,
            },
            Type::unit(),
            span,
        ))
    }

    /// Expand dbg! macro - evaluates and prints expression, returns the value
    fn expand_dbg_macro(
        &mut self,
        expr: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        let inner_expr = self.infer_expr(expr)?;
        let ty = inner_expr.ty.clone();

        Ok(hir::Expr::new(
            hir::ExprKind::Dbg(Box::new(inner_expr)),
            ty,
            span,
        ))
    }

    /// Expand matches! macro - returns true if expression matches pattern
    ///
    /// Transforms `matches!(expr, pattern)` into:
    /// ```text
    /// match expr { pattern => true, _ => false }
    /// ```
    fn expand_matches_macro(
        &mut self,
        expr: &ast::Expr,
        pattern: &ast::Pattern,
        span: Span,
    ) -> Result<hir::Expr, Box<TypeError>> {
        // Type-check the scrutinee expression
        let scrutinee = self.infer_expr(expr)?;
        let scrutinee_ty = scrutinee.ty.clone();

        // Lower the pattern and check against scrutinee type
        let hir_pattern = self.lower_pattern(pattern, &scrutinee_ty)?;

        // Create true arm: pattern => true
        let true_expr = hir::Expr::new(
            hir::ExprKind::Literal(hir::LiteralValue::Bool(true)),
            hir::Type::bool(),
            span,
        );
        let true_arm = hir::MatchArm {
            pattern: hir_pattern,
            guard: None,
            body: true_expr,
        };

        // Create false arm: _ => false
        let wildcard_pattern = hir::Pattern {
            kind: hir::PatternKind::Wildcard,
            ty: scrutinee_ty.clone(),
            span,
        };
        let false_expr = hir::Expr::new(
            hir::ExprKind::Literal(hir::LiteralValue::Bool(false)),
            hir::Type::bool(),
            span,
        );
        let false_arm = hir::MatchArm {
            pattern: wildcard_pattern,
            guard: None,
            body: false_expr,
        };

        // Create the match expression
        Ok(hir::Expr::new(
            hir::ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![true_arm, false_arm],
            },
            hir::Type::bool(),
            span,
        ))
    }

    /// Try to extract a constant usize value from an expression.
    /// Returns Some(value) if the expression is an integer literal, None otherwise.
    fn extract_usize_const(&self, expr: &hir::Expr) -> Option<u64> {
        match &expr.kind {
            hir::ExprKind::Literal(hir::LiteralValue::Int(n)) => {
                // Convert i128 to u64 if it fits
                if *n >= 0 && *n <= u64::MAX as i128 {
                    Some(*n as u64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Evaluate an AST expression as a compile-time constant.
    /// Returns Some(ConstValue) if the expression can be evaluated at compile time,
    /// or None if it cannot. Used for explicit const generic arguments in turbofish syntax.
    fn evaluate_const_expr(&self, expr: &ast::Expr) -> Option<hir::ConstValue> {
        match const_eval::eval_const_expr(expr) {
            Ok(const_eval::ConstResult::Int(v)) => Some(hir::ConstValue::Int(v)),
            Ok(const_eval::ConstResult::Uint(v)) => Some(hir::ConstValue::Uint(v)),
            Ok(const_eval::ConstResult::Bool(v)) => Some(hir::ConstValue::Bool(v)),
            Err(_) => None,
        }
    }
}
