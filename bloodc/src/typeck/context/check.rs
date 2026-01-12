//! Function and handler body type checking.

use std::collections::HashMap;

use crate::ast;
use crate::effects::{infer_effects, verify_effects_subset, EffectRow, EffectRef as EffectsEffectRef};
use crate::hir::{self, DefId, LocalId, Type, TypeKind, TyVarId};
use crate::diagnostics::Diagnostic;
use crate::span::Span;

use super::TypeContext;
use super::super::error::{TypeError, TypeErrorKind};
use super::super::resolve::ScopeKind;
use super::EffectRef;

impl<'a> TypeContext<'a> {
    /// Type-check all queued bodies.
    pub fn check_all_bodies(&mut self) -> Result<(), Vec<Diagnostic>> {
        // Phase 1: Check coherence (no overlapping impls)
        self.check_coherence();

        // Type-check function bodies
        let pending = std::mem::take(&mut self.pending_bodies);
        for (def_id, func) in pending {
            if let Err(e) = self.check_function_body(def_id, &func) {
                self.errors.push(e);
            }
        }

        // Phase 3: Type-check handler operation bodies
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
    pub(crate) fn check_function_body(&mut self, def_id: DefId, func: &ast::FnDecl) -> Result<(), TypeError> {
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

        self.bodies.insert(body_id, hir_body.clone());
        self.fn_bodies.insert(def_id, body_id);

        // Effect inference and verification
        self.infer_and_verify_effects(def_id, &hir_body, body.span)?;

        // Clean up
        self.return_type = None;
        self.resolver.pop_scope();

        Ok(())
    }

    /// Type-check a handler's operation and return clause bodies.
    pub(crate) fn check_handler_body(&mut self, handler_def_id: DefId, handler: &ast::HandlerDecl) -> Result<(), TypeError> {
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

        // Build substitution map from effect's generic params to handler's type args.
        // This is needed because the effect's operation types (e.g., `put(s: S)`) use
        // the effect's TyVarIds, but the handler's state fields use the handler's TyVarIds.
        // We substitute effect's params with the handler's type args to unify them.
        let effect_subst: HashMap<TyVarId, Type> = effect_info.generics.iter()
            .zip(handler_info.effect_type_args.iter())
            .map(|(ty_var, ty_arg)| (*ty_var, ty_arg.clone()))
            .collect();

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
            // Substitute effect's type params with handler's type args
            let return_ty = self.substitute_type_vars(&effect_op.return_ty, &effect_subst);
            self.locals.push(hir::Local {
                id: LocalId::RETURN_PLACE,
                ty: return_ty,
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
            // Substitute effect's type params with handler's type args
            for (param_idx, param) in op_impl.params.iter().enumerate() {
                if let ast::PatternKind::Ident { name, mutable, .. } = &param.kind {
                    let param_name = self.symbol_to_string(name.node);
                    let param_ty = if param_idx < effect_op.params.len() {
                        // Apply substitution to get the handler's type args
                        self.substitute_type_vars(&effect_op.params[param_idx], &effect_subst)
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
            // Use the substituted return type for consistency with handler's type args
            let resume_return_ty = self.substitute_type_vars(&effect_op.return_ty, &effect_subst);
            let resume_ty = Type::function(vec![resume_return_ty.clone()], Type::unit());
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

            // Set the expected resume type for E0303 checking
            self.current_resume_type = Some(resume_return_ty);

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
            self.current_resume_type = None;
        }

        // Type-check return clause if present
        let return_clause_body_id = if let Some(ret_clause) = &handler.return_clause {
            self.resolver.push_scope(ScopeKind::Function, ret_clause.span);
            self.resolver.reset_local_ids();
            let _ = self.resolver.next_local_id(); // Skip LocalId(0)
            self.locals.clear();

            // Return clause input parameter type: the type of the handled computation's result.
            // Since handlers can be used with any compatible computation, we use a fresh type var.
            // This will be constrained when the handler is applied to a specific expression.
            let input_ty = self.unifier.fresh_var();

            // Add return place with inferred type (will be determined by body)
            let return_ty = self.unifier.fresh_var();
            self.locals.push(hir::Local {
                id: LocalId::RETURN_PLACE,
                ty: return_ty.clone(),
                mutable: false,
                name: None,
                span: ret_clause.span,
            });

            // Add the result parameter (the input to the return clause)
            let param_name = self.symbol_to_string(ret_clause.param.node);
            let local_id = self.resolver.define_local(
                param_name.clone(),
                input_ty.clone(),
                false,
                ret_clause.param.span,
            )?;
            self.locals.push(hir::Local {
                id: local_id,
                ty: input_ty,
                mutable: false,
                name: Some(param_name),
                span: ret_clause.param.span,
            });

            // Add handler state fields to scope (same as in operation bodies)
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

            // Infer the return clause body type (instead of checking against hardcoded type)
            // Use a fresh type var which will unify with the actual body type
            let expected_body_ty = self.unifier.fresh_var();
            let body_expr = self.check_block(&ret_clause.body, &expected_body_ty)?;

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
    pub(crate) fn check_block(&mut self, block: &ast::Block, expected: &Type) -> Result<hir::Expr, TypeError> {
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

    /// Check effect compatibility for a function call.
    ///
    /// When calling a function that uses effects, verify that either:
    /// 1. The effect is handled by an enclosing with...handle block
    /// 2. The calling function also declares the same effect
    pub(crate) fn check_effect_compatibility(&self, callee_def_id: DefId, span: Span) -> Result<(), TypeError> {
        // Get the callee's declared effects
        let callee_effects = match self.fn_effects.get(&callee_def_id) {
            Some(effects) => effects,
            None => return Ok(()), // Callee has no effects
        };

        // For each effect the callee uses, check if it's handled
        for effect_ref in callee_effects {
            let effect_id = effect_ref.def_id;

            // Check if handled by an enclosing handler
            if self.handled_effects.iter().any(|(id, _)| *id == effect_id) {
                continue;
            }

            // Check if the caller also declares this effect
            if let Some(caller_def_id) = self.current_fn {
                if let Some(caller_effects) = self.fn_effects.get(&caller_def_id) {
                    if caller_effects.iter().any(|er| er.def_id == effect_id) {
                        continue;
                    }
                }
            }

            // Effect is not handled - report error
            let effect_name = self.effect_defs.get(&effect_id)
                .map(|info| info.name.clone())
                .unwrap_or_else(|| format!("effect#{}", effect_id.index()));

            return Err(TypeError::new(
                TypeErrorKind::UnhandledEffect { effect: effect_name },
                span,
            ));
        }

        Ok(())
    }

    /// Infer effects from a function body and verify against declared effects.
    ///
    /// If the function has no explicit effect annotation, the inferred effects
    /// are stored as the function's effect signature. If the function has an
    /// explicit annotation, the inferred effects are verified to be a subset
    /// of the declared effects.
    pub(crate) fn infer_and_verify_effects(
        &mut self,
        def_id: DefId,
        body: &hir::Body,
        span: Span,
    ) -> Result<(), TypeError> {
        // Infer effects from the function body
        let inferred_row = infer_effects(body);

        // Check if the function has declared effects
        let has_declared_effects = self.fn_effects.contains_key(&def_id);

        if has_declared_effects {
            // Build declared effect row from fn_effects
            let declared_effects = self.fn_effects.get(&def_id).cloned().unwrap_or_default();
            let declared_row = self.build_effect_row_from_refs(&declared_effects);

            // Verify inferred effects are a subset of declared effects
            if let Err(undeclared) = verify_effects_subset(&inferred_row, &declared_row) {
                // Convert the undeclared effects to a readable error
                let effect_names: Vec<String> = undeclared
                    .iter()
                    .map(|e| {
                        self.effect_defs
                            .get(&e.def_id)
                            .map(|info| info.name.clone())
                            .unwrap_or_else(|| format!("effect#{}", e.def_id.index()))
                    })
                    .collect();

                return Err(TypeError::new(
                    TypeErrorKind::UndeclaredEffects {
                        effects: effect_names,
                    },
                    span,
                ));
            }
        } else {
            // No declared effects - use inferred effects as the function's effect signature
            // Convert EffectRow to Vec<EffectRef> for storage
            let effect_refs: Vec<EffectRef> = inferred_row
                .effects()
                .map(|e| EffectRef {
                    def_id: e.def_id,
                    type_args: e.type_args.clone(),
                })
                .collect();

            if !effect_refs.is_empty() {
                self.fn_effects.insert(def_id, effect_refs);
            }
        }

        Ok(())
    }

    /// Build an EffectRow from a list of EffectRef.
    fn build_effect_row_from_refs(&self, refs: &[EffectRef]) -> EffectRow {
        let mut row = EffectRow::pure();
        for effect_ref in refs {
            row.add_effect(EffectsEffectRef::with_args(
                effect_ref.def_id,
                effect_ref.type_args.clone(),
            ));
        }
        row
    }
}
