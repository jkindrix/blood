//! Expression type inference.
//!
//! This module contains all methods for inferring and checking expression types.

use std::collections::HashMap;

use crate::ast;
use crate::hir::{self, DefId, Type, TypeKind, PrimitiveTy, TyVarId};
use crate::hir::def::{IntTy, UintTy};
use crate::span::Span;

use super::TypeContext;
use super::super::const_eval;
use super::super::error::{TypeError, TypeErrorKind};
use super::super::exhaustiveness;
use super::super::resolve::{Binding, ScopeKind};

impl<'a> TypeContext<'a> {
    /// Check an expression against an expected type.
    pub(crate) fn check_expr(&mut self, expr: &ast::Expr, expected: &Type) -> Result<hir::Expr, TypeError> {
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
            ast::ExprKind::Loop { body, .. } => {
                self.infer_loop(body, expr.span)
            }
            ast::ExprKind::While { condition, body, .. } => {
                self.infer_while(condition, body, expr.span)
            }
            ast::ExprKind::WhileLet { pattern, scrutinee, body, .. } => {
                self.infer_while_let(pattern, scrutinee, body, expr.span)
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
                self.infer_with_handle(handler, body, expr.span)
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
        }
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
    pub(crate) fn resolve_method(
        &mut self,
        receiver_ty: &Type,
        method_name: &str,
        _args: &[hir::Expr],
        span: Span,
    ) -> Result<(DefId, Type), TypeError> {
        // Try to find the method on the receiver type directly
        if let Some((def_id, ret_ty)) = self.find_method_for_type(receiver_ty, method_name) {
            return Ok((def_id, ret_ty));
        }

        // Try auto-deref: if receiver is &T or &mut T, try finding method on T
        if let TypeKind::Ref { inner, .. } = receiver_ty.kind() {
            if let Some((def_id, ret_ty)) = self.find_method_for_type(inner, method_name) {
                return Ok((def_id, ret_ty));
            }
        }

        // No method found
        Err(self.error_method_not_found(receiver_ty, method_name, span))
    }

    /// Find a method for a specific type by searching impl blocks.
    pub(crate) fn find_method_for_type(&self, ty: &Type, method_name: &str) -> Option<(DefId, Type)> {
        // First, look for inherent impl methods (impl blocks without trait_ref)
        for impl_block in &self.impl_blocks {
            if impl_block.trait_ref.is_some() {
                continue;
            }

            if !self.types_match_for_impl(&impl_block.self_ty, ty) {
                continue;
            }

            for method in &impl_block.methods {
                if method.name == method_name {
                    if let Some(sig) = self.fn_sigs.get(&method.def_id) {
                        return Some((method.def_id, sig.output.clone()));
                    }
                }
            }
        }

        // Second, look for trait impl methods
        for impl_block in &self.impl_blocks {
            let Some(_trait_id) = impl_block.trait_ref else {
                continue;
            };

            if !self.types_match_for_impl(&impl_block.self_ty, ty) {
                continue;
            }

            for method in &impl_block.methods {
                if method.name == method_name {
                    if let Some(sig) = self.fn_sigs.get(&method.def_id) {
                        return Some((method.def_id, sig.output.clone()));
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
                                    return Some((method.def_id, sig.output.clone()));
                                }
                            }
                        }
                    }
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
                        "u8" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8)))),
                        "u16" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16)))),
                        "u32" => return Ok(Type::u32()),
                        "u64" => return Ok(Type::u64()),
                        "u128" => return Ok(Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128)))),
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
                    // Two-segment path: Module::Type
                    let _module_name = self.symbol_to_string(path.segments[0].name.node);
                    let type_name = self.symbol_to_string(path.segments[1].name.node);

                    // Look up the type directly by the second segment
                    if let Some(def_id) = self.resolver.lookup_type(&type_name) {
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

                    Err(self.error_type_not_found(&type_name, ty.span))
                } else {
                    // More than two segments - not yet supported
                    let path_str = path.segments.iter()
                        .map(|s| self.symbol_to_string(s.name.node))
                        .collect::<Vec<_>>()
                        .join("::");
                    Err(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: format!("type paths with more than 2 segments: {}", path_str),
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
            ast::TypeKind::Ownership { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "ownership-qualified types".to_string(),
                    },
                    ty.span,
                ))
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
                    Some(ast::IntSuffix::U8) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U8))),
                    Some(ast::IntSuffix::U16) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U16))),
                    Some(ast::IntSuffix::U32) => Type::u32(),
                    Some(ast::IntSuffix::U64) => Type::u64(),
                    Some(ast::IntSuffix::U128) => Type::new(TypeKind::Primitive(PrimitiveTy::Uint(UintTy::U128))),
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
            ast::LiteralKind::String(s) => (hir::LiteralValue::String(s.clone()), Type::string()),
        };

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
                    } else if let Some(def_info) = self.resolver.def_info.get(&def_id) {
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
                            let field_types: Vec<Type> = variant_fields.iter()
                                .map(|f| f.ty.clone())
                                .collect();

                            let type_args: Vec<Type> = enum_info.generics.iter()
                                .map(|_| self.unifier.fresh_var())
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
                        return Err(TypeError::new(
                            TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                            span,
                        ));
                    }
                }
            }

            Err(TypeError::new(
                TypeErrorKind::NotFound { name: format!("{}::{}", first_name, second_name) },
                span,
            ))
        } else {
            let path_str = path.segments.iter()
                .map(|s| self.symbol_to_string(s.name.node))
                .collect::<Vec<_>>()
                .join("::");
            Err(TypeError::new(
                TypeErrorKind::UnsupportedFeature {
                    feature: format!("paths with more than 2 segments: {}", path_str),
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
        let right_expr = self.infer_expr(right)?;

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

        Ok(hir::Expr::new(
            hir::ExprKind::Call {
                callee: Box::new(callee_expr),
                args: hir_args,
            },
            return_type,
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
    pub(crate) fn infer_loop(&mut self, body: &ast::Block, span: Span) -> Result<hir::Expr, TypeError> {
        self.resolver.push_scope(ScopeKind::Loop, span);
        let body_expr = self.check_block(body, &Type::unit())?;
        self.resolver.pop_scope();

        Ok(hir::Expr::new(
            hir::ExprKind::Loop {
                body: Box::new(body_expr),
                label: None,
            },
            Type::never(),
            span,
        ))
    }

    /// Infer type of a while loop.
    pub(crate) fn infer_while(&mut self, condition: &ast::Expr, body: &ast::Block, span: Span) -> Result<hir::Expr, TypeError> {
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
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
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
                    label: None,
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
                label: None,
            },
            Type::unit(),
            span,
        ))
    }

    /// Infer type of a for loop.
    pub(crate) fn infer_for(
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
                        feature: "For loop currently only supports range expressions".into(),
                    },
                    iter.span,
                ));
            }
        };

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

        // Register user's loop variable
        let var_local_id = self.resolver.define_local(
            var_name.clone(),
            idx_ty.clone(),
            false,
            pattern.span,
        )?;

        self.locals.push(hir::Local {
            id: var_local_id,
            ty: idx_ty.clone(),
            mutable: false,
            name: Some(var_name),
            span: pattern.span,
        });

        let body_expr = self.check_block(body, &Type::unit())?;

        self.resolver.pop_scope();

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

        let bind_stmt = hir::Stmt::Let {
            local_id: var_local_id,
            init: Some(hir::Expr::new(
                hir::ExprKind::Local(idx_local_id),
                idx_ty.clone(),
                span,
            )),
        };

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

        let while_body = hir::Expr::new(
            hir::ExprKind::Block {
                stmts: vec![bind_stmt, hir::Stmt::Expr(body_expr), hir::Stmt::Expr(increment)],
                expr: None,
            },
            Type::unit(),
            span,
        );

        let while_loop = hir::Expr::new(
            hir::ExprKind::While {
                condition: Box::new(condition),
                body: Box::new(while_body),
                label: None,
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

    /// Infer type of a break.
    pub(crate) fn infer_break(&mut self, value: Option<&ast::Expr>, span: Span) -> Result<hir::Expr, TypeError> {
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

    /// Infer type of a record (struct) construction expression.
    pub(crate) fn infer_record(
        &mut self,
        path: Option<&ast::TypePath>,
        fields: &[ast::RecordExprField],
        _base: Option<&ast::Expr>,
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
            } else {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "multi-segment struct paths".to_string(),
                    },
                    span,
                ));
            }
        } else {
            return Err(TypeError::new(
                TypeErrorKind::UnsupportedFeature {
                    feature: "anonymous structs".to_string(),
                },
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

        // Type-check fields, substituting generics with fresh type vars
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
                // Infer the value type first, then unify with expected
                let inferred = self.infer_expr(value)?;
                self.unifier.unify(&inferred.ty, &expected_ty, value.span).map_err(|_| {
                    TypeError::new(
                        TypeErrorKind::Mismatch {
                            expected: self.unifier.resolve(&expected_ty),
                            found: self.unifier.resolve(&inferred.ty),
                        },
                        value.span,
                    )
                })?;
                inferred
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

        // Build result type with resolved type args
        let resolved_type_args: Vec<Type> = type_args.iter()
            .map(|ty| self.unifier.resolve(ty))
            .collect();
        let result_ty = Type::adt(def_id, resolved_type_args);

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

    /// Infer type of a field access expression.
    pub(crate) fn infer_field_access(
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

                if let TypeKind::Adt { def_id, .. } = base_ty.kind() {
                    if let Some(struct_info) = self.struct_defs.get(def_id) {
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
                            return Err(self.error_unknown_field(&base_ty, &field_name, span));
                        }
                    }
                }

                Err(TypeError::new(
                    TypeErrorKind::NotAStruct { ty: base_ty },
                    span,
                ))
            }
            ast::FieldAccess::Index(index, _span) => {
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
        let value_expr = self.infer_expr(value)?;

        self.unifier.unify(&target_expr.ty, &value_expr.ty, span)?;
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

}
