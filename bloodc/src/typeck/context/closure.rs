//! Closure type inference and capture analysis.
//!
//! This module handles closure expressions including parameter type inference,
//! body type checking, and captured variable analysis.

use std::collections::HashSet;

use crate::ast;
use crate::hir::{self, LocalId, Type};
use crate::span::Span;

use super::TypeContext;
use super::super::error::{TypeError, TypeErrorKind};
use super::super::resolve::ScopeKind;

impl<'a> TypeContext<'a> {
    /// Infer the type of a closure expression.
    ///
    /// Closures are type-checked as follows:
    /// 1. Create a new closure scope
    /// 2. Determine parameter types (from annotations or inference)
    /// 3. Type-check the body expression
    /// 4. Determine return type (from annotation or body type)
    /// 5. Analyze captured variables
    /// 6. Create HIR closure with function type
    pub(crate) fn infer_closure(
        &mut self,
        is_move: bool,
        params: &[ast::ClosureParam],
        return_type: Option<&ast::Type>,
        effects: Option<&ast::EffectRow>,
        body: &ast::Expr,
        span: Span,
    ) -> Result<hir::Expr, TypeError> {
        // Save current locals and create fresh ones for closure
        let outer_locals = std::mem::take(&mut self.locals);
        let outer_return_type = self.return_type.take();

        // Parse and register closure's effect annotation if present.
        // This ensures that `perform Effect.op()` inside the closure uses the closure's
        // effect type arguments, not the enclosing function's.
        let closure_effects_count = if let Some(effect_row) = effects {
            let (effect_refs, _row_var) = self.parse_effect_row(effect_row)?;
            for effect_ref in &effect_refs {
                self.handled_effects.push((effect_ref.def_id, effect_ref.type_args.clone()));
            }
            effect_refs.len()
        } else {
            0
        };

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
        for (i, param) in params.iter().enumerate() {
            let param_ty = if let Some(ty) = &param.ty {
                self.ast_type_to_hir_type(ty)?
            } else {
                // Create inference variable for parameter without annotation
                self.unifier.fresh_var()
            };

            // Handle parameter pattern
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
                        ty: param_ty.clone(),
                        mutable: *mutable,
                        name: Some(param_name),
                        span: param.span,
                    });
                }
                ast::PatternKind::Wildcard => {
                    let param_name = format!("_param{i}");
                    let local_id = self.resolver.define_local(
                        param_name.clone(),
                        param_ty.clone(),
                        false,
                        param.span,
                    )?;

                    self.locals.push(hir::Local {
                        id: local_id,
                        ty: param_ty.clone(),
                        mutable: false,
                        name: Some(param_name),
                        span: param.span,
                    });
                }
                ast::PatternKind::Tuple { fields, .. } => {
                    // Tuple destructuring in closure parameters: |(x, y)|
                    // Create a hidden parameter local, then define the pattern bindings
                    let hidden_name = format!("__cparam{i}");
                    let _hidden_local_id = self.resolver.define_local(
                        hidden_name.clone(),
                        param_ty.clone(),
                        false,
                        param.span,
                    )?;

                    // Now define the pattern bindings from the tuple type
                    let elem_types = match param_ty.kind() {
                        hir::TypeKind::Tuple(elems) => elems.clone(),
                        hir::TypeKind::Infer(_) => {
                            // Type not yet known - create fresh variables for each element
                            let vars: Vec<Type> = (0..fields.len())
                                .map(|_| self.unifier.fresh_var())
                                .collect();
                            // Unify with the param type to establish the tuple structure
                            let tuple_ty = Type::tuple(vars.clone());
                            self.unifier.unify(&param_ty, &tuple_ty, param.span)?;
                            vars
                        }
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
                        ty: param_ty.clone(),
                        mutable: false,
                        name: Some(hidden_name),
                        span: param.span,
                    });
                }
                ast::PatternKind::Paren(inner) => {
                    // Parenthesized pattern - recurse through it
                    self.define_pattern(inner, param_ty.clone())?;
                }
                _ => {
                    // Other complex patterns - use define_pattern which will error appropriately
                    self.define_pattern(&param.pattern, param_ty.clone())?;
                }
            }

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
            tuple_destructures: std::mem::take(&mut self.tuple_destructures),
        };

        self.bodies.insert(body_id, hir_body);

        // Pop closure scope
        self.resolver.pop_scope();

        // Pop the closure's effects from handled_effects stack
        for _ in 0..closure_effects_count {
            self.handled_effects.pop();
        }

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
    pub(crate) fn analyze_closure_captures(&self, body: &hir::Expr, is_move: bool) -> Vec<hir::Capture> {
        let mut captures = Vec::new();
        let mut seen = HashSet::new();
        self.collect_captures(body, is_move, &mut captures, &mut seen);
        captures
    }

    /// Recursively collect captured variables from an expression.
    fn collect_captures(
        &self,
        expr: &hir::Expr,
        is_move: bool,
        captures: &mut Vec<hir::Capture>,
        seen: &mut HashSet<LocalId>,
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
                        hir::Stmt::Let { init: None, .. } => {
                            // Let without initializer - no captures to collect
                        }
                        hir::Stmt::Expr(e) => {
                            self.collect_captures(e, is_move, captures, seen);
                        }
                        hir::Stmt::Item(_) => {
                            // Item declarations don't directly capture variables
                            // (nested functions have their own capture analysis)
                        }
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
            hir::ExprKind::InlineHandle { body, handlers } => {
                self.collect_captures(body, is_move, captures, seen);
                for handler in handlers {
                    self.collect_captures(&handler.body, is_move, captures, seen);
                }
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
            hir::ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.collect_captures(s, is_move, captures, seen);
                }
                if let Some(e) = end {
                    self.collect_captures(e, is_move, captures, seen);
                }
            }
            hir::ExprKind::Record { fields } => {
                for field in fields {
                    self.collect_captures(&field.value, is_move, captures, seen);
                }
            }
            // Macro expansion nodes - collect from subexpressions
            hir::ExprKind::MacroExpansion { args, .. } => {
                for arg in args {
                    self.collect_captures(arg, is_move, captures, seen);
                }
            }
            hir::ExprKind::VecLiteral(exprs) => {
                for e in exprs {
                    self.collect_captures(e, is_move, captures, seen);
                }
            }
            hir::ExprKind::VecRepeat { value, count } => {
                self.collect_captures(value, is_move, captures, seen);
                self.collect_captures(count, is_move, captures, seen);
            }
            hir::ExprKind::Assert { condition, message } => {
                self.collect_captures(condition, is_move, captures, seen);
                if let Some(msg) = message {
                    self.collect_captures(msg, is_move, captures, seen);
                }
            }
            hir::ExprKind::Dbg(inner) => {
                self.collect_captures(inner, is_move, captures, seen);
            }
            // These don't contain local references directly
            hir::ExprKind::Literal(_)
            | hir::ExprKind::Def(_)
            | hir::ExprKind::Continue { .. }
            | hir::ExprKind::MethodFamily { .. }
            | hir::ExprKind::Default
            | hir::ExprKind::Error => {}
        }
    }
}
