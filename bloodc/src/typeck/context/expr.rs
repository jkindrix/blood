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

impl<'a> TypeContext<'a> {
    /// Check an expression against an expected type.
    pub(crate) fn check_expr(&mut self, expr: &ast::Expr, expected: &Type) -> Result<hir::Expr, TypeError> {
        // Special case for literals: use expected type to guide numeric literal inference
        if let ast::ExprKind::Literal(lit) = &expr.kind {
            return self.check_literal(lit, expected, expr.span);
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

        Ok(inferred)
    }

    /// Infer the type of an expression.
    pub(crate) fn infer_expr(&mut self, expr: &ast::Expr) -> Result<hir::Expr, TypeError> {
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
                self.infer_field_access(field_base, field, expr.span)
            }
            ast::ExprKind::Closure { is_move, params, return_type, effects: _, body } => {
                self.infer_closure(*is_move, params, return_type.as_ref(), body, expr.span)
            }
            ast::ExprKind::WithHandle { handler, body } => {
                self.infer_with_handle(handler, body, expr.span)
            }
            ast::ExprKind::TryWith { body: _, handlers: _ } => {
                // TODO: Implement try-with inline handlers
                // For now, return an error indicating this feature is not yet implemented
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "try-with inline handler expressions are not yet implemented. \
                                  Please use `deep handler` declarations with `with Handler handle { }` syntax instead.".into(),
                    },
                    expr.span,
                ))
            }
            ast::ExprKind::Perform { effect, operation, args } => {
                self.infer_perform(effect.as_ref(), operation, args, expr.span)
            }
            ast::ExprKind::Resume(value) => {
                self.infer_resume(value, expr.span)
            }
            ast::ExprKind::MethodCall { receiver, method, type_args: _, args } => {
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
            ast::ExprKind::Region { body, .. } => {
                let expected = self.unifier.fresh_var();
                self.check_block(body, &expected)
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
        }
    }

    /// Infer type for `default` expression.
    /// The type is inferred from context (where the value is used).
    fn infer_default(&mut self, span: Span) -> Result<hir::Expr, TypeError> {
        // Create a fresh type variable - the type will be inferred from usage context
        let ty = self.unifier.fresh_var();
        Ok(hir::Expr {
            kind: hir::ExprKind::Default,
            ty,
            span,
        })
    }

    /// Infer type of a with...handle expression.
    fn infer_with_handle(&mut self, handler: &ast::Expr, body: &ast::Expr, span: Span) -> Result<hir::Expr, TypeError> {
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
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "Handler must be an ADT type".into(),
                    },
                    span,
                ));
            }
        };

        // Wrap the body in a Handle expression
        let body_expr = result?;
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

    /// Infer type of a perform expression.
    fn infer_perform(
        &mut self,
        effect: Option<&ast::TypePath>,
        operation: &crate::span::Spanned<ast::Symbol>,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
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

        Ok(hir::Expr::new(
            hir::ExprKind::Perform {
                effect_id,
                op_index,
                args: hir_args,
            },
            return_ty,
            span,
        ))
    }

    /// Infer type of a resume expression.
    fn infer_resume(&mut self, value: &ast::Expr, span: Span) -> Result<hir::Expr, TypeError> {
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
                return Err(TypeError::new(
                    TypeErrorKind::ResumeTypeMismatch {
                        expected: format!("{}", expected),
                        found: format!("{}", value_ty),
                    },
                    span,
                ));
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

    /// Infer type of a method call.
    ///
    /// This desugars `receiver.method(args)` into `method(receiver, args)`.
    pub(crate) fn infer_method_call(
        &mut self,
        receiver: &ast::Expr,
        method: &ast::Symbol,
        args: &[ast::CallArg],
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let receiver_expr = self.infer_expr(receiver)?;
        let method_name = self.symbol_to_string(*method);

        // Type-check arguments
        let mut hir_args = Vec::with_capacity(args.len());
        for arg in args {
            let arg_expr = self.infer_expr(&arg.value)?;
            hir_args.push(arg_expr);
        }

        // Try to find method signature
        let (method_def_id, return_ty, first_param, impl_generics, method_generics, needs_auto_ref) =
            self.resolve_method(&receiver_expr.ty, &method_name, &hir_args, span)?;

        // Build substitution from impl generics to concrete types from receiver
        let mut substitution: HashMap<TyVarId, Type> = HashMap::new();

        // Extract concrete type args from receiver to build substitution
        if !impl_generics.is_empty() {
            if let TypeKind::Adt { args: receiver_args, .. } = receiver_expr.ty.kind() {
                for (tyvar, concrete_ty) in impl_generics.iter().zip(receiver_args.iter()) {
                    substitution.insert(*tyvar, concrete_ty.clone());
                }
            }
        }

        // Add fresh type vars for method-level generics
        for &tyvar in &method_generics {
            substitution.insert(tyvar, self.unifier.fresh_var());
        }

        // Build the callee type by substituting in the stored signature
        let callee_ty = if let Some(sig) = self.fn_sigs.get(&method_def_id).cloned() {
            // Apply substitution to inputs and output
            let subst_inputs: Vec<Type> = sig.inputs.iter()
                .map(|ty| self.substitute_type_vars(ty, &substitution))
                .collect();
            let subst_output = self.substitute_type_vars(&sig.output, &substitution);
            Type::function(subst_inputs, subst_output)
        } else {
            // Fallback to inferred function type
            let receiver_ty = if needs_auto_ref {
                Type::reference(receiver_expr.ty.clone(), false)
            } else {
                receiver_expr.ty.clone()
            };
            let mut param_types = vec![receiver_ty];
            param_types.extend(hir_args.iter().map(|a| a.ty.clone()));
            Type::function(param_types, return_ty.clone())
        };

        let callee = hir::Expr::new(
            hir::ExprKind::Def(method_def_id),
            callee_ty,
            span,
        );

        // Build receiver expression, auto-borrowing if needed
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
            receiver_expr
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
            return_ty,
            span,
        ))
    }

    /// Resolve a method on a type.
    /// Returns (method_def_id, return_type, first_param_type, impl_generics, method_generics, needs_auto_ref).
    pub(crate) fn resolve_method(
        &mut self,
        receiver_ty: &Type,
        method_name: &str,
        _args: &[hir::Expr],
        span: Span,
    ) -> Result<(DefId, Type, Option<Type>, Vec<TyVarId>, Vec<TyVarId>, bool), TypeError> {
        // Try to find the method on the receiver type directly
        if let Some((def_id, ret_ty, first_param, impl_generics, method_generics)) = self.find_method_for_type(receiver_ty, method_name) {
            // Check if we need to auto-ref the receiver
            let needs_auto_ref = if let Some(ref param_ty) = first_param {
                // If first param is a reference and receiver is not, we need auto-ref
                matches!(param_ty.kind(), TypeKind::Ref { .. }) && !matches!(receiver_ty.kind(), TypeKind::Ref { .. })
            } else {
                false
            };
            return Ok((def_id, ret_ty, first_param, impl_generics, method_generics, needs_auto_ref));
        }

        // Try auto-deref: if receiver is &T or &mut T, try finding method on T
        if let TypeKind::Ref { inner, .. } = receiver_ty.kind() {
            if let Some((def_id, ret_ty, first_param, impl_generics, method_generics)) = self.find_method_for_type(inner, method_name) {
                // When auto-deref is used, we don't need auto-ref
                return Ok((def_id, ret_ty, first_param, impl_generics, method_generics, false));
            }
        }

        // No method found
        Err(self.error_method_not_found(receiver_ty, method_name, span))
    }

    /// Find a method for a specific type by searching impl blocks.
    /// Returns (method_def_id, substituted return type, substituted first param type, impl generics, method generics).
    pub(crate) fn find_method_for_type(&self, ty: &Type, method_name: &str) -> Option<(DefId, Type, Option<Type>, Vec<TyVarId>, Vec<TyVarId>)> {
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

            for method in &impl_block.methods {
                if method.name == method_name {
                    if let Some(sig) = self.fn_sigs.get(&method.def_id) {
                        // Apply substitution to return type
                        let subst_output = self.substitute_type_vars(&sig.output, &subst);
                        // Get the first parameter type (receiver) and apply substitution
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

        // Second, look for trait impl methods
        for impl_block in &self.impl_blocks {
            let Some(_trait_id) = impl_block.trait_ref else {
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
    fn find_builtin_method(&self, ty: &Type, method_name: &str) -> Option<(DefId, Type, Option<Type>, Vec<TyVarId>, Vec<TyVarId>)> {
        use super::BuiltinMethodType;
        use crate::hir::ty::PrimitiveTy;

        // Determine which builtin type category this type matches
        let type_match = match ty.kind() {
            TypeKind::Primitive(PrimitiveTy::Str) => Some(BuiltinMethodType::Str),
            TypeKind::Primitive(PrimitiveTy::Char) => Some(BuiltinMethodType::Char),
            TypeKind::Primitive(PrimitiveTy::String) => Some(BuiltinMethodType::String),
            TypeKind::Ref { inner, .. } => {
                match inner.kind() {
                    TypeKind::Primitive(PrimitiveTy::Str) => Some(BuiltinMethodType::StrRef),
                    TypeKind::Primitive(PrimitiveTy::String) => Some(BuiltinMethodType::String),
                    _ => None,
                }
            }
            TypeKind::Adt { def_id, .. } => {
                // Check if it's Option or Vec
                if Some(*def_id) == self.option_def_id {
                    Some(BuiltinMethodType::Option)
                } else if Some(*def_id) == self.vec_def_id {
                    Some(BuiltinMethodType::Vec)
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
                        BuiltinMethodType::Option | BuiltinMethodType::Vec => {
                            // Extract the type argument from the ADT
                            if let TypeKind::Adt { args, .. } = ty.kind() {
                                if !args.is_empty() {
                                    let element_ty = args[0].clone();
                                    // For Option<T>.unwrap() and Option<T>.try_(), return type is T
                                    if method_name == "unwrap" || method_name == "try_" {
                                        element_ty
                                    } else if method_name == "get" {
                                        // Vec<T>.get() returns Option<&T>
                                        // Substitute T with the actual element type
                                        let ref_elem = Type::reference(element_ty, false);
                                        Type::adt(
                                            self.option_def_id.expect("BUG: option_def_id not set"),
                                            vec![ref_elem],
                                        )
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
                        _ => sig.output.clone(),
                    };

                    let first_param = sig.inputs.first().cloned();
                    return Some((
                        builtin_method.def_id,
                        return_type,
                        first_param,
                        Vec::new(),
                        Vec::new(),
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
                    // For generic static methods like Vec::new(), instantiate with fresh vars
                    // The synthetic TyVarId(9000) placeholder needs to be replaced with fresh vars
                    let needs_fresh_vars = matches!(&type_match, BuiltinMethodType::Vec | BuiltinMethodType::Option);

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
        if sig.generics.is_empty() {
            return Type::function(sig.inputs.clone(), sig.output.clone());
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

        Type::function(subst_inputs, subst_output)
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
            TypeKind::Fn { params, ret } => {
                let subst_params: Vec<Type> = params.iter()
                    .map(|p| self.substitute_type_vars(p, subst))
                    .collect();
                let subst_ret = self.substitute_type_vars(ret, subst);
                Type::function(subst_params, subst_ret)
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
                Type::array(subst_elem, *size)
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
    pub(crate) fn ast_type_to_hir_type(&mut self, ty: &ast::Type) -> Result<Type, TypeError> {
        match &ty.kind {
            ast::TypeKind::Path(path) => {
                if path.segments.is_empty() {
                    return Err(TypeError::new(
                        TypeErrorKind::TypeNotFound { name: "empty path".to_string() },
                        ty.span,
                    ));
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
                            return Err(TypeError::new(
                                TypeErrorKind::TypeNotFound { name: "Self".to_string() },
                                ty.span,
                            ));
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
                        // Check if it's a type alias and expand it
                        if let Some(alias_info) = self.type_aliases.get(&def_id).cloned() {
                            // For now, simple type aliases without generic args
                            return Ok(alias_info.ty);
                        }

                        // Extract type arguments if any
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

                        return Ok(Type::adt(def_id, type_args));
                    }

                    Err(self.error_type_not_found(&name, ty.span))
                } else if path.segments.len() == 2 {
                    // Two-segment path: Module::Type or Bridge::Type
                    let module_name = self.symbol_to_string(path.segments[0].name.node);
                    let type_name = self.symbol_to_string(path.segments[1].name.node);

                    // First try to find the type's DefId in bridge definitions with matching namespace
                    // Store found def_id and whether it's opaque (no type args)
                    let mut found_def_id: Option<DefId> = None;
                    let mut is_opaque = false;

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

                            // Check type aliases
                            for alias in &bridge_info.type_aliases {
                                if alias.name == type_name {
                                    found_def_id = Some(alias.def_id);
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
                        return Ok(Type::adt(def_id, type_args));
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
                    // More than two segments requires full module system (not yet implemented)
                    let path_str = path.segments.iter()
                        .map(|s| self.symbol_to_string(s.name.node))
                        .collect::<Vec<_>>()
                        .join("::");
                    Err(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: format!(
                                "type paths with more than 2 segments (`{}`). \
                                 Blood currently supports single-segment types and two-segment \
                                 paths (Module::Type). Full module hierarchies require the module \
                                 system which is planned for a future release.",
                                path_str
                            ),
                        },
                        ty.span,
                    ))
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
                let size = const_eval::eval_const_expr(size)?
                    .as_u64()
                    .ok_or_else(|| TypeError::new(
                        TypeErrorKind::ConstEvalError {
                            reason: "array size must be a non-negative integer".to_string(),
                        },
                        ty.span,
                    ))?;
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
            ast::TypeKind::Function { params, return_type, effects: _ } => {
                let param_types: Vec<Type> = params.iter()
                    .map(|p| self.ast_type_to_hir_type(p))
                    .collect::<Result<_, _>>()?;
                let ret_ty = self.ast_type_to_hir_type(return_type)?;
                Ok(Type::function(param_types, ret_ty))
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
                    .collect::<Result<_, TypeError>>()?;

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

    /// Infer type of an index expression.
    pub(crate) fn infer_index(
        &mut self,
        base: &ast::Expr,
        index: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let base_expr = self.infer_expr(base)?;
        let index_expr = self.infer_expr(index)?;

        // Check that index is an integer type
        match index_expr.ty.kind() {
            TypeKind::Primitive(PrimitiveTy::Int(_) | PrimitiveTy::Uint(_)) => {}
            TypeKind::Infer(_) => {
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
    pub(crate) fn infer_array(
        &mut self,
        array_expr: &ast::ArrayExpr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
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
    pub(crate) fn infer_literal(&mut self, lit: &ast::Literal, span: Span) -> Result<hir::Expr, TypeError> {
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
    pub(crate) fn check_literal(&mut self, lit: &ast::Literal, expected: &Type, span: Span) -> Result<hir::Expr, TypeError> {
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
    pub(crate) fn infer_path(&mut self, path: &ast::ExprPath, span: Span) -> Result<hir::Expr, TypeError> {
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
                    if let Some(sig) = self.fn_sigs.get(&def_id).cloned() {
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

                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                        span,
                    ));
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
                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                        span,
                    ));
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
                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                        span,
                    ));
                }
            }

            Err(TypeError::new(
                TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                span,
            ))
        } else if path.segments.len() == 3 {
            // Three-segment path: module::Type::method
            let module_name = self.symbol_to_string(path.segments[0].name.node);
            let type_name = self.symbol_to_string(path.segments[1].name.node);
            let method_name = self.symbol_to_string(path.segments[2].name.node);

            // First, find the module
            let mut module_def_id: Option<DefId> = None;
            for (def_id, info) in &self.module_defs {
                if info.name == module_name {
                    module_def_id = Some(*def_id);
                    break;
                }
            }

            if let Some(_mod_def_id) = module_def_id {
                // Find the type (struct) by name
                let mut type_def_id: Option<DefId> = None;
                for (def_id, struct_info) in &self.struct_defs {
                    if struct_info.name == type_name {
                        type_def_id = Some(*def_id);
                        break;
                    }
                }

                if let Some(type_def_id) = type_def_id {
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
                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}::{}", module_name, type_name, method_name) },
                        span,
                    ));
                } else {
                    // Type not found in module
                    return Err(self.error_type_not_found(
                        &format!("{}::{}", module_name, type_name),
                        span,
                    ));
                }
            } else {
                // Module not found
                return Err(TypeError::new(
                    TypeErrorKind::NotFound { name: format!("module '{}'", module_name) },
                    span,
                ));
            }
        } else {
            let path_str = path.segments.iter()
                .map(|s| self.symbol_to_string(s.name.node))
                .collect::<Vec<_>>()
                .join("::");
            Err(TypeError::new(
                TypeErrorKind::UnsupportedFeature {
                    feature: format!("paths with more than 3 segments: {}", path_str),
                },
                span,
            ))
        }
    }

    /// Infer type of a binary expression.
    pub(crate) fn infer_binary(
        &mut self,
        op: ast::BinOp,
        left: &ast::Expr,
        right: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let left_expr = self.infer_expr(left)?;
        // Use check_expr for right side to propagate left type for better literal inference
        let right_expr = self.check_expr(right, &left_expr.ty).unwrap_or_else(|_| {
            // Fall back to infer if check fails (types may legitimately differ)
            self.infer_expr(right).unwrap_or_else(|_| hir::Expr::new(
                hir::ExprKind::Literal(hir::LiteralValue::Int(0)),
                Type::error(),
                right.span,
            ))
        });

        let result_ty = match op {
            ast::BinOp::Add | ast::BinOp::Sub | ast::BinOp::Mul | ast::BinOp::Div | ast::BinOp::Rem => {
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
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
            ast::BinOp::BitAnd | ast::BinOp::BitOr | ast::BinOp::BitXor | ast::BinOp::Shl | ast::BinOp::Shr => {
                self.unifier.unify(&left_expr.ty, &right_expr.ty, span)?;
                left_expr.ty.clone()
            }
            ast::BinOp::Pipe => {
                match right_expr.ty.kind() {
                    TypeKind::Fn { params, ret } => {
                        if params.is_empty() {
                            return Err(TypeError::new(
                                TypeErrorKind::WrongArity {
                                    expected: 1,
                                    found: 0,
                                },
                                span,
                            ));
                        }
                        self.unifier.unify(&left_expr.ty, &params[0], span)?;
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
    pub(crate) fn infer_unary(
        &mut self,
        op: ast::UnaryOp,
        operand: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let operand_expr = self.infer_expr(operand)?;

        let result_ty = match op {
            ast::UnaryOp::Neg => operand_expr.ty.clone(),
            ast::UnaryOp::Not => operand_expr.ty.clone(),
            ast::UnaryOp::Deref => {
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
    pub(crate) fn infer_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::CallArg],
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
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
        let instantiated_ty = match resolved_callee_ty.kind() {
            TypeKind::Forall { params, body } => {
                // Create fresh inference variables for each forall parameter
                let fresh_vars: Vec<Type> = (0..params.len())
                    .map(|_| self.unifier.fresh_var())
                    .collect();

                // Build substitution map
                let subst: std::collections::HashMap<hir::TyVarId, Type> = params.iter()
                    .cloned()
                    .zip(fresh_vars.into_iter())
                    .collect();

                // Substitute params in the body
                self.substitute_type_vars(body, &subst)
            }
            _ => resolved_callee_ty.clone(),
        };

        let (param_types, return_type) = match instantiated_ty.kind() {
            TypeKind::Fn { params, ret } => (params.clone(), ret.clone()),
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::NotAFunction { ty: callee_expr.ty.clone() },
                    span,
                ));
            }
        };

        if args.len() != param_types.len() {
            return Err(TypeError::new(
                TypeErrorKind::WrongArity {
                    expected: param_types.len(),
                    found: args.len(),
                },
                span,
            ));
        }

        // Check effect compatibility
        if let hir::ExprKind::Def(callee_def_id) = &callee_expr.kind {
            self.check_effect_compatibility(callee_def_id.clone(), span)?;
        }

        let mut hir_args = Vec::new();
        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
            let arg_expr = self.check_expr(&arg.value, param_ty)?;
            hir_args.push(arg_expr);
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
        let resolved_callee_expr = hir::Expr::new(
            callee_expr.kind.clone(),
            self.unifier.resolve(&callee_expr.ty),
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
    ) -> Result<hir::Expr, TypeError> {
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
            return Err(TypeError::new(
                TypeErrorKind::NoApplicableMethod {
                    name: name.to_string(),
                    arg_types,
                },
                span,
            ));
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
        Err(TypeError::new(
            TypeErrorKind::AmbiguousDispatch {
                name: name.to_string(),
                candidates: candidate_sigs,
            },
            span,
        ))
    }

    /// Infer type of an if expression.
    pub(crate) fn infer_if(
        &mut self,
        condition: &ast::Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ast::ElseBranch>,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
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
    pub(crate) fn infer_return(&mut self, value: Option<&ast::Expr>, span: Span) -> Result<hir::Expr, TypeError> {
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
    pub(crate) fn infer_tuple(&mut self, exprs: &[ast::Expr], span: Span) -> Result<hir::Expr, TypeError> {
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
    pub(crate) fn infer_assign(&mut self, target: &ast::Expr, value: &ast::Expr, span: Span) -> Result<hir::Expr, TypeError> {
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
    pub(crate) fn infer_loop(&mut self, body: &ast::Block, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, TypeError> {
        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.enter_loop(label_str.as_deref());

        self.resolver.push_scope(ScopeKind::Loop, span);
        let body_expr = self.check_block(body, &Type::unit())?;
        self.resolver.pop_scope();

        self.exit_loop(label_str.as_deref());

        Ok(hir::Expr::new(
            hir::ExprKind::Loop {
                body: Box::new(body_expr),
                label: Some(loop_id),
            },
            Type::never(),
            span,
        ))
    }

    /// Infer type of a while loop.
    pub(crate) fn infer_while(&mut self, condition: &ast::Expr, body: &ast::Block, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
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
                return self.infer_for_array(pattern, iter_expr, element.clone(), *size, body, label, span);
            }
            TypeKind::Ref { inner, .. } => {
                // Handle references to arrays: &[T; N] and &[T]
                let inner_ty = self.unifier.resolve(inner);
                match inner_ty.kind() {
                    TypeKind::Array { element, size } => {
                        return self.infer_for_array(pattern, iter_expr, element.clone(), *size, body, label, span);
                    }
                    TypeKind::Slice { element } => {
                        // Slices require runtime length - not yet supported
                        return Err(TypeError::new(
                            TypeErrorKind::UnsupportedFeature {
                                feature: "For loop over slices requires runtime length check (use while loop)".into(),
                            },
                            iter.span,
                        ));
                    }
                    _ => {}
                }
            }
            TypeKind::Slice { element } => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "For loop over slices requires runtime length check (use while loop)".into(),
                    },
                    iter.span,
                ));
            }
            _ => {}
        }

        // Not a supported iterable type
        Err(TypeError::new(
            TypeErrorKind::UnsupportedFeature {
                feature: format!(
                    "For loop over type `{}` not supported. Use range expressions (0..n) or arrays.",
                    iter_ty
                ),
            },
            iter.span,
        ))
    }

    /// Helper: Infer for loop over a range expression.
    fn infer_for_range(
        &mut self,
        pattern: &ast::Pattern,
        start: &ast::Expr,
        end: &ast::Expr,
        inclusive: bool,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Extract variable name, or None for wildcard pattern
        let var_name = match &pattern.kind {
            ast::PatternKind::Ident { name, .. } => Some(self.symbol_to_string(name.node)),
            ast::PatternKind::Wildcard => None,
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "For loop currently only supports simple identifier or wildcard patterns".into(),
                    },
                    pattern.span,
                ));
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
        let mut body_stmts = Vec::new();
        if let Some(stmt) = bind_stmt {
            body_stmts.push(stmt);
        }
        body_stmts.push(hir::Stmt::Expr(body_expr));
        body_stmts.push(hir::Stmt::Expr(increment));

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
    fn infer_for_array(
        &mut self,
        pattern: &ast::Pattern,
        array_expr: hir::Expr,
        element_ty: Type,
        array_size: u64,
        body: &ast::Block,
        label: Option<&Spanned<ast::Symbol>>,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Extract variable name, or None for wildcard pattern
        let var_name = match &pattern.kind {
            ast::PatternKind::Ident { name, .. } => Some(self.symbol_to_string(name.node)),
            ast::PatternKind::Wildcard => None,
            _ => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "For loop currently only supports simple identifier or wildcard patterns".into(),
                    },
                    pattern.span,
                ));
            }
        };

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

        // Register user's loop variable (only if not a wildcard pattern)
        let var_local_id = if let Some(ref name) = var_name {
            let local_id = self.resolver.define_local(
                name.clone(),
                element_ty.clone(),
                false,
                pattern.span,
            )?;

            self.locals.push(hir::Local {
                id: local_id,
                ty: element_ty.clone(),
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
        let mut body_stmts = Vec::new();
        if let Some(stmt) = bind_stmt {
            body_stmts.push(stmt);
        } else {
            // Even for wildcard, we still need to do the array access (for side effects)
            body_stmts.push(hir::Stmt::Expr(make_array_access()));
        }
        body_stmts.push(hir::Stmt::Expr(body_expr));
        body_stmts.push(hir::Stmt::Expr(increment));

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

    /// Infer type of a break.
    pub(crate) fn infer_break(&mut self, value: Option<&ast::Expr>, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, TypeError> {
        if !self.resolver.in_loop() {
            return Err(TypeError::new(TypeErrorKind::BreakOutsideLoop, span));
        }

        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.find_loop(label_str.as_deref());

        // Check that the label exists
        if label.is_some() && loop_id.is_none() {
            return Err(TypeError::new(
                TypeErrorKind::UnresolvedName { name: label_str.unwrap_or_default() },
                span,
            ));
        }

        let value_expr = if let Some(value) = value {
            Some(Box::new(self.infer_expr(value)?))
        } else {
            None
        };

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
    pub(crate) fn infer_continue(&mut self, label: Option<&Spanned<ast::Symbol>>, span: Span) -> Result<hir::Expr, TypeError> {
        if !self.resolver.in_loop() {
            return Err(TypeError::new(TypeErrorKind::ContinueOutsideLoop, span));
        }

        let label_str = label.map(|l| self.symbol_to_string(l.node));
        let loop_id = self.find_loop(label_str.as_deref());

        // Check that the label exists
        if label.is_some() && loop_id.is_none() {
            return Err(TypeError::new(
                TypeErrorKind::UnresolvedName { name: label_str.unwrap_or_default() },
                span,
            ));
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
            return Err(TypeError::new(
                TypeErrorKind::NonExhaustivePatterns {
                    missing: result.missing_patterns,
                },
                span,
            ));
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
            return Err(TypeError::new(
                TypeErrorKind::NonExhaustivePatterns {
                    missing: result.missing_patterns,
                },
                span,
            ));
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
    ) -> Result<hir::Expr, TypeError> {
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
                        return Err(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: Type::adt(def_id, Vec::new()) },
                            span,
                        ));
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
                        return Err(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: Type::adt(def_id, Vec::new()) },
                            span,
                        ));
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
                                            return Err(TypeError::new(
                                                TypeErrorKind::UnknownField {
                                                    ty: Type::adt(enum_def_id, type_args.clone()),
                                                    field: field_name,
                                                },
                                                field.span,
                                            ));
                                        }
                                    };

                                    // Apply substitution to field type
                                    let expected_ty = self.substitute_type_vars(&variant_field.ty, &subst);

                                    // Handle shorthand syntax: `{ x }` means `{ x: x }`
                                    let value_expr = if let Some(value) = &field.value {
                                        // Use check_expr to propagate expected type for better literal inference
                                        self.check_expr(value, &expected_ty).map_err(|_| {
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

                    if let Some(_mod_def_id) = module_def_id {
                        // Find the type in the module's items by looking up the struct definition
                        let mut found_struct: Option<(DefId, super::StructInfo)> = None;
                        for (def_id, struct_info) in &self.struct_defs {
                            if struct_info.name == type_name {
                                found_struct = Some((*def_id, struct_info.clone()));
                                break;
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
                // More than 2 segments not yet supported
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "struct paths with more than 2 segments".to_string(),
                    },
                    span,
                ));
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
                        return Err(TypeError::new(
                            TypeErrorKind::Mismatch {
                                expected: result_ty.clone(),
                                found: base_ty,
                            },
                            base_expr.span,
                        ));
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
                    return Err(TypeError::new(
                        TypeErrorKind::NotAStruct { ty: base_ty },
                        base_expr.span,
                    ));
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
                    // Re-infer to get the actual found type for error message
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
                    return Err(TypeError::new(
                        TypeErrorKind::MissingField {
                            ty: result_ty.clone(),
                            field: field_info.name.clone(),
                        },
                        span,
                    ));
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

    /// Infer type of a field access expression.
    pub(crate) fn infer_field_access(
        &mut self,
        base: &ast::Expr,
        field: &ast::FieldAccess,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        let base_expr = self.infer_expr(base)?;
        let base_ty = self.unifier.resolve(&base_expr.ty);

        // Auto-deref references for field access
        let (deref_expr, inner_ty) = if let TypeKind::Ref { inner, .. } = base_ty.kind() {
            // Dereference the reference
            let deref_ty = self.unifier.resolve(inner);
            let deref_hir = hir::Expr::new(
                hir::ExprKind::Deref(Box::new(base_expr)),
                deref_ty.clone(),
                span,
            );
            (deref_hir, deref_ty)
        } else {
            (base_expr, base_ty.clone())
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

                Err(TypeError::new(
                    TypeErrorKind::NotAStruct { ty: inner_ty },
                    span,
                ))
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

                Err(TypeError::new(
                    TypeErrorKind::NotATuple { ty: inner_ty },
                    span,
                ))
            }
        }
    }

    /// Infer type of a cast expression.
    fn infer_cast(&mut self, inner: &ast::Expr, ty: &ast::Type, span: Span) -> Result<hir::Expr, TypeError> {
        let inner_expr = self.infer_expr(inner)?;
        let target_ty = self.ast_type_to_hir_type(ty)?;

        Ok(hir::Expr::new(
            hir::ExprKind::Cast {
                expr: Box::new(inner_expr),
                target_ty: target_ty.clone(),
            },
            target_ty,
            span,
        ))
    }

    /// Infer type of a compound assignment.
    fn infer_assign_op(
        &mut self,
        op: ast::BinOp,
        target: &ast::Expr,
        value: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
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
    fn infer_unsafe(&mut self, block: &ast::Block, span: Span) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
        // Get the macro name for dispatch
        let macro_name = path.segments.last()
            .map(|seg| self.interner.resolve(seg.name.node).unwrap_or("").to_string())
            .unwrap_or_default();

        match kind {
            // Format-style macros
            ast::MacroCallKind::Format { format_str, args } => {
                self.expand_format_macro(&macro_name, format_str, args, span)
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
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: format!("user-defined macro `{}!`", macro_name),
                    },
                    span,
                ))
            }
        }
    }

    /// Expand format-style macros (format!, println!, print!, etc.)
    fn expand_format_macro(
        &mut self,
        macro_name: &str,
        format_str: &crate::span::Spanned<String>,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Type-check all arguments
        let mut checked_args = Vec::new();
        for arg in args {
            let arg_expr = self.infer_expr(arg)?;
            checked_args.push(arg_expr);
        }

        // Determine return type based on macro
        let return_ty = match macro_name {
            "format" => {
                // format! returns String (an owned string)
                Type::new(hir::TypeKind::Primitive(hir::PrimitiveTy::String))
            }
            "println" | "print" | "eprintln" | "eprint" => {
                // print macros return unit
                Type::unit()
            }
            "panic" => {
                // panic! returns Never type
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
    ) -> Result<hir::Expr, TypeError> {
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
                    return Err(TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: Type::usize(),
                            found: count_expr.ty.clone(),
                        },
                        count.span,
                    ));
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
                        Err(TypeError {
                            kind: TypeErrorKind::UnsupportedFeature {
                                feature: "vec! repeat count must be a constant integer literal".to_string(),
                            },
                            span,
                            help: Some("use a literal like `vec![0; 10]` instead of a variable".to_string()),
                        })
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
    ) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
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
    ) -> Result<hir::Expr, TypeError> {
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
}
