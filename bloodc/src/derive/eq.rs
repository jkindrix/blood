//! Eq/PartialEq derive implementation.
//!
//! Generates `fn eq(&self, other: &Self) -> bool` that compares all fields.

use std::collections::HashMap;

use crate::hir::{
    self, Type, LocalId, Body, Local, FnSig,
    Expr, ExprKind,
};
use crate::ast::BinOp;

use super::{DeriveExpander, DeriveRequest, local_expr, bool_literal};

/// Expand Eq derive for a struct.
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

    // Create signature: fn eq(&self, other: &Self) -> bool
    let sig = FnSig::new(vec![self_ref_ty.clone(), self_ref_ty.clone()], Type::bool());
    expander.fn_sigs.insert(method_def_id, sig);
    expander.method_self_types.insert(method_def_id, self_ty.clone());

    // Create body
    let self_local_id = LocalId::new(1);
    let other_local_id = LocalId::new(2);

    let self_local = Local {
        id: self_local_id,
        ty: self_ref_ty.clone(),
        mutable: false,
        name: Some("self".to_string()),
        span,
    };

    let other_local = Local {
        id: other_local_id,
        ty: self_ref_ty.clone(),
        mutable: false,
        name: Some("other".to_string()),
        span,
    };

    let return_local = Local {
        id: LocalId::new(0),
        ty: Type::bool(),
        mutable: false,
        name: None,
        span,
    };

    // Build comparison: self.field1 == other.field1 && self.field2 == other.field2 && ...
    let result_expr = if struct_info.fields.is_empty() {
        // Empty struct is always equal
        bool_literal(true, span)
    } else {
        let mut comparisons: Vec<Expr> = struct_info.fields.iter().map(|field| {
            // self.field
            let self_field = Expr::new(
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

            // other.field
            let other_field = Expr::new(
                ExprKind::Field {
                    base: Box::new(Expr::new(
                        ExprKind::Deref(Box::new(local_expr(other_local_id, self_ref_ty.clone(), span))),
                        self_ty.clone(),
                        span,
                    )),
                    field_idx: field.index,
                },
                field.ty.clone(),
                span,
            );

            // self.field == other.field
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(self_field),
                    right: Box::new(other_field),
                },
                Type::bool(),
                span,
            )
        }).collect();

        // Chain all comparisons with &&
        let first = comparisons.remove(0);
        comparisons.into_iter().fold(first, |acc, cmp| {
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Box::new(acc),
                    right: Box::new(cmp),
                },
                Type::bool(),
                span,
            )
        })
    };

    let body = Body {
        locals: vec![return_local, self_local, other_local],
        param_count: 2,
        expr: result_expr,
        span,
        tuple_destructures: HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block
    expander.create_impl_block(request, method_def_id, "eq", false);
}

/// Expand Eq derive for an enum.
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

    // Create signature: fn eq(&self, other: &Self) -> bool
    let sig = FnSig::new(vec![self_ref_ty.clone(), self_ref_ty.clone()], Type::bool());
    expander.fn_sigs.insert(method_def_id, sig);
    expander.method_self_types.insert(method_def_id, self_ty.clone());

    // Create body
    let self_local_id = LocalId::new(1);
    let other_local_id = LocalId::new(2);

    let self_local = Local {
        id: self_local_id,
        ty: self_ref_ty.clone(),
        mutable: false,
        name: Some("self".to_string()),
        span,
    };

    let other_local = Local {
        id: other_local_id,
        ty: self_ref_ty.clone(),
        mutable: false,
        name: Some("other".to_string()),
        span,
    };

    let return_local = Local {
        id: LocalId::new(0),
        ty: Type::bool(),
        mutable: false,
        name: None,
        span,
    };

    // Track locals for bindings
    let mut locals = vec![return_local, self_local, other_local];
    let mut next_local_id = 3u32;

    // Build match arms: match (self, other) { (Variant, Variant) => compare fields, _ => false }
    let mut arms = Vec::new();

    for variant in &enum_info.variants {
        if variant.fields.is_empty() {
            // Unit variant: (Self::Variant, Self::Variant) => true
            let self_pattern = hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: Vec::new(),
                },
                ty: self_ty.clone(),
                span,
            };

            let other_pattern = hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: Vec::new(),
                },
                ty: self_ty.clone(),
                span,
            };

            let tuple_pattern = hir::Pattern {
                kind: hir::PatternKind::Tuple(vec![self_pattern, other_pattern]),
                ty: Type::tuple(vec![self_ty.clone(), self_ty.clone()]),
                span,
            };

            arms.push(hir::MatchArm {
                pattern: tuple_pattern,
                guard: None,
                body: bool_literal(true, span),
            });
        } else {
            // Variant with fields: bind both and compare
            let mut self_field_patterns = Vec::new();
            let mut other_field_patterns = Vec::new();
            let mut field_comparisons = Vec::new();

            for field in &variant.fields {
                let self_field_id = LocalId::new(next_local_id);
                next_local_id += 1;
                let other_field_id = LocalId::new(next_local_id);
                next_local_id += 1;

                locals.push(Local {
                    id: self_field_id,
                    ty: field.ty.clone(),
                    mutable: false,
                    name: Some(format!("self_{}", field.name)),
                    span,
                });

                locals.push(Local {
                    id: other_field_id,
                    ty: field.ty.clone(),
                    mutable: false,
                    name: Some(format!("other_{}", field.name)),
                    span,
                });

                self_field_patterns.push(hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: self_field_id,
                        mutable: false,
                        subpattern: None,
                    },
                    ty: field.ty.clone(),
                    span,
                });

                other_field_patterns.push(hir::Pattern {
                    kind: hir::PatternKind::Binding {
                        local_id: other_field_id,
                        mutable: false,
                        subpattern: None,
                    },
                    ty: field.ty.clone(),
                    span,
                });

                // Compare: self_field == other_field
                field_comparisons.push(Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Eq,
                        left: Box::new(local_expr(self_field_id, field.ty.clone(), span)),
                        right: Box::new(local_expr(other_field_id, field.ty.clone(), span)),
                    },
                    Type::bool(),
                    span,
                ));
            }

            let self_pattern = hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: self_field_patterns,
                },
                ty: self_ty.clone(),
                span,
            };

            let other_pattern = hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: other_field_patterns,
                },
                ty: self_ty.clone(),
                span,
            };

            let tuple_pattern = hir::Pattern {
                kind: hir::PatternKind::Tuple(vec![self_pattern, other_pattern]),
                ty: Type::tuple(vec![self_ty.clone(), self_ty.clone()]),
                span,
            };

            // Combine field comparisons with &&
            let body_expr = if field_comparisons.is_empty() {
                bool_literal(true, span)
            } else {
                let first = field_comparisons.remove(0);
                field_comparisons.into_iter().fold(first, |acc, cmp| {
                    Expr::new(
                        ExprKind::Binary {
                            op: BinOp::And,
                            left: Box::new(acc),
                            right: Box::new(cmp),
                        },
                        Type::bool(),
                        span,
                    )
                })
            };

            arms.push(hir::MatchArm {
                pattern: tuple_pattern,
                guard: None,
                body: body_expr,
            });
        }
    }

    // Add wildcard arm for mismatched variants: _ => false
    arms.push(hir::MatchArm {
        pattern: hir::Pattern {
            kind: hir::PatternKind::Wildcard,
            ty: Type::tuple(vec![self_ty.clone(), self_ty.clone()]),
            span,
        },
        guard: None,
        body: bool_literal(false, span),
    });

    // Scrutinee: (*self, *other)
    let scrutinee = Expr::new(
        ExprKind::Tuple(vec![
            Expr::new(
                ExprKind::Deref(Box::new(local_expr(self_local_id, self_ref_ty.clone(), span))),
                self_ty.clone(),
                span,
            ),
            Expr::new(
                ExprKind::Deref(Box::new(local_expr(other_local_id, self_ref_ty.clone(), span))),
                self_ty.clone(),
                span,
            ),
        ]),
        Type::tuple(vec![self_ty.clone(), self_ty.clone()]),
        span,
    );

    let match_expr = Expr::new(
        ExprKind::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        },
        Type::bool(),
        span,
    );

    let body = Body {
        locals,
        param_count: 2,
        expr: match_expr,
        span,
        tuple_destructures: HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block
    expander.create_impl_block(request, method_def_id, "eq", false);
}
