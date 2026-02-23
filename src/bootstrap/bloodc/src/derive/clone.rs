//! Clone derive implementation.
//!
//! Generates `fn clone(&self) -> Self` that creates a copy of the value.

use std::collections::HashMap;

use crate::hir::{
    self, Type, LocalId, Body, Local, FnSig,
    Expr, ExprKind, FieldExpr,
};

use super::{DeriveExpander, DeriveRequest, local_expr};

/// Expand Clone derive for a struct.
pub fn expand_struct(expander: &mut DeriveExpander, request: &DeriveRequest) {
    let struct_info = match expander.struct_defs.get(&request.type_def_id) {
        Some(info) => info.clone(),
        None => return,
    };

    let method_def_id = expander.alloc_def_id();
    let body_id = expander.alloc_body_id();
    let span = request.span;

    // Self type
    let self_ty = expander.get_self_type(request);
    let self_ref_ty = Type::reference(self_ty.clone(), false);

    // Create signature: fn clone(&self) -> Self
    let sig = FnSig::new(vec![self_ref_ty.clone()], self_ty.clone());
    expander.fn_sigs.insert(method_def_id, sig);
    expander.method_self_types.insert(method_def_id, self_ty.clone());

    // Create body
    let self_local_id = LocalId::new(1);
    let self_local = Local {
        id: self_local_id,
        ty: self_ref_ty.clone(),
        mutable: false,
        name: Some("self".to_string()),
        span,
    };

    let return_local = Local {
        id: LocalId::new(0),
        ty: self_ty.clone(),
        mutable: false,
        name: None,
        span,
    };

    // Build struct expression: Self { field1: self.field1.clone(), ... }
    let fields: Vec<FieldExpr> = struct_info.fields.iter().map(|field| {
        // Access field: (*self).field
        let field_access = Expr::new(
            ExprKind::Field {
                base: Box::new(Expr::new(
                    ExprKind::Deref(Box::new(local_expr(self_local_id, self_ref_ty.clone(), span))),
                    self_ty.clone(),
                    span,
                )),
                field_idx: field.index,
            },
            field.ty.clone(),
            span,
        );

        // For primitives, just copy the value
        // For other types, ideally call .clone() - for now, copy
        FieldExpr {
            field_idx: field.index,
            value: field_access,
        }
    }).collect();

    let struct_expr = Expr::new(
        ExprKind::Struct {
            def_id: request.type_def_id,
            fields,
            base: None,
        },
        self_ty.clone(),
        span,
    );

    let body = Body {
        locals: vec![return_local, self_local],
        param_count: 1,
        expr: struct_expr,
        span,
        tuple_destructures: HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block
    expander.create_impl_block(request, method_def_id, "clone", false);
}

/// Expand Clone derive for an enum.
pub fn expand_enum(expander: &mut DeriveExpander, request: &DeriveRequest) {
    let enum_info = match expander.enum_defs.get(&request.type_def_id) {
        Some(info) => info.clone(),
        None => return,
    };

    let method_def_id = expander.alloc_def_id();
    let body_id = expander.alloc_body_id();
    let span = request.span;

    // Self type
    let self_ty = expander.get_self_type(request);
    let self_ref_ty = Type::reference(self_ty.clone(), false);

    // Create signature: fn clone(&self) -> Self
    let sig = FnSig::new(vec![self_ref_ty.clone()], self_ty.clone());
    expander.fn_sigs.insert(method_def_id, sig);
    expander.method_self_types.insert(method_def_id, self_ty.clone());

    // Create body with match expression
    let self_local_id = LocalId::new(1);
    let self_local = Local {
        id: self_local_id,
        ty: self_ref_ty.clone(),
        mutable: false,
        name: Some("self".to_string()),
        span,
    };

    let return_local = Local {
        id: LocalId::new(0),
        ty: self_ty.clone(),
        mutable: false,
        name: None,
        span,
    };

    // Build match arms for each variant
    let mut arms = Vec::new();
    let mut next_local_id = 2u32;

    // Track locals for bindings
    let mut locals = vec![return_local, self_local];

    for variant in &enum_info.variants {
        if variant.fields.is_empty() {
            // Unit variant: Self::Variant => Self::Variant
            let pattern = hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: Vec::new(),
                },
                ty: self_ty.clone(),
                span,
            };

            let body_expr = Expr::new(
                ExprKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: Vec::new(),
                },
                self_ty.clone(),
                span,
            );

            arms.push(hir::MatchArm {
                pattern,
                guard: None,
                body: body_expr,
            });
        } else {
            // Variant with fields: Self::Variant { f1, f2 } => Self::Variant { f1: f1.clone(), ... }
            let mut field_patterns = Vec::new();
            let mut field_exprs = Vec::new();
            let mut field_locals = Vec::new();

            for field in &variant.fields {
                let field_local_id = LocalId::new(next_local_id);
                next_local_id += 1;

                // Create local for the binding
                let local = Local {
                    id: field_local_id,
                    ty: field.ty.clone(),
                    mutable: false,
                    name: Some(field.name.clone()),
                    span,
                };
                field_locals.push(local);

                // Pattern binding for this field
                field_patterns.push(hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: field_local_id,
                        mutable: false,
                        by_ref: false,
                        subpattern: None,
                    },
                    ty: field.ty.clone(),
                    span,
                });

                // Expression: local reference (clone for non-primitives would go here)
                field_exprs.push(local_expr(field_local_id, field.ty.clone(), span));
            }

            let pattern = hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: field_patterns,
                },
                ty: self_ty.clone(),
                span,
            };

            let body_expr = Expr::new(
                ExprKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: field_exprs,
                },
                self_ty.clone(),
                span,
            );

            arms.push(hir::MatchArm {
                pattern,
                guard: None,
                body: body_expr,
            });

            locals.extend(field_locals);
        }
    }

    // Scrutinee: *self
    let scrutinee = Expr::new(
        ExprKind::Deref(Box::new(local_expr(self_local_id, self_ref_ty.clone(), span))),
        self_ty.clone(),
        span,
    );

    let match_expr = Expr::new(
        ExprKind::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        },
        self_ty.clone(),
        span,
    );

    let body = Body {
        locals,
        param_count: 1,
        expr: match_expr,
        span,
        tuple_destructures: HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block
    expander.create_impl_block(request, method_def_id, "clone", false);
}
