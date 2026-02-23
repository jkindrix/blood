//! Hash derive implementation.
//!
//! Generates `fn hash(&self) -> u64` that combines field hashes.

use std::collections::HashMap;

use crate::hir::{
    self, Type, LocalId, Body, Local, FnSig,
    Expr, ExprKind, LiteralValue,
};
use crate::ast::BinOp;
use crate::span::Span;

use super::{DeriveExpander, DeriveRequest, local_expr, literal_expr};

/// Expand Hash derive for a struct.
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

    // Create signature: fn hash(&self) -> u64
    let sig = FnSig::new(vec![self_ref_ty.clone()], Type::u64());
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
        ty: Type::u64(),
        mutable: false,
        name: None,
        span,
    };

    // Build hash computation using FNV-1a-like algorithm:
    // hash = 14695981039346656037 (FNV offset basis for 64-bit)
    // for each field:
    //   hash = hash ^ field_hash
    //   hash = hash * 1099511628211 (FNV prime for 64-bit)
    let result_expr = if struct_info.fields.is_empty() {
        // Empty struct has constant hash
        literal_expr(LiteralValue::Int(0), Type::u64(), span)
    } else {
        // Start with FNV offset basis
        let mut hash_expr = literal_expr(
            LiteralValue::Int(14695981039346656037i128),
            Type::u64(),
            span,
        );

        for field in &struct_info.fields {
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

            // Get hash of field (cast to u64 for primitives, or call hash method)
            let field_hash = hash_value(field_access, &field.ty, span);

            // hash = hash ^ field_hash
            hash_expr = Expr::new(
                ExprKind::Binary {
                    op: BinOp::BitXor,
                    left: Box::new(hash_expr),
                    right: Box::new(field_hash),
                },
                Type::u64(),
                span,
            );

            // hash = hash * FNV_PRIME (wrapping multiply)
            let fnv_prime = literal_expr(
                LiteralValue::Int(1099511628211i128),
                Type::u64(),
                span,
            );
            hash_expr = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Box::new(hash_expr),
                    right: Box::new(fnv_prime),
                },
                Type::u64(),
                span,
            );
        }

        hash_expr
    };

    let body = Body {
        locals: vec![return_local, self_local],
        param_count: 1,
        expr: result_expr,
        span,
        tuple_destructures: HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block
    expander.create_impl_block(request, method_def_id, "hash", false);
}

/// Expand Hash derive for an enum.
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

    // Create signature: fn hash(&self) -> u64
    let sig = FnSig::new(vec![self_ref_ty.clone()], Type::u64());
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
        ty: Type::u64(),
        mutable: false,
        name: None,
        span,
    };

    // Track locals for bindings
    let mut locals = vec![return_local, self_local];
    let mut next_local_id = 2u32;

    // Build match arms for each variant
    let mut arms = Vec::new();

    for variant in &enum_info.variants {
        // Start with discriminant hash
        let discriminant_hash = literal_expr(
            LiteralValue::Int(variant.index as i128),
            Type::u64(),
            span,
        );

        if variant.fields.is_empty() {
            // Unit variant: just use discriminant
            let pattern = hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: Vec::new(),
                },
                ty: self_ty.clone(),
                span,
            };

            arms.push(hir::MatchArm {
                pattern,
                guard: None,
                body: discriminant_hash,
            });
        } else {
            // Variant with fields: hash discriminant + fields
            let mut field_patterns = Vec::new();
            let mut field_locals_info = Vec::new();

            for field in &variant.fields {
                let field_local_id = LocalId::new(next_local_id);
                next_local_id += 1;

                locals.push(Local {
                    id: field_local_id,
                    ty: field.ty.clone(),
                    mutable: false,
                    name: Some(field.name.clone()),
                    span,
                });

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

                field_locals_info.push((field_local_id, field.ty.clone()));
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

            // Compute hash: discriminant ^ field1_hash ^ field2_hash ^ ...
            let mut hash_expr = discriminant_hash;
            for (field_local_id, field_ty) in field_locals_info {
                let field_hash = hash_value(
                    local_expr(field_local_id, field_ty.clone(), span),
                    &field_ty,
                    span,
                );
                hash_expr = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::BitXor,
                        left: Box::new(hash_expr),
                        right: Box::new(field_hash),
                    },
                    Type::u64(),
                    span,
                );
            }

            arms.push(hir::MatchArm {
                pattern,
                guard: None,
                body: hash_expr,
            });
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
        Type::u64(),
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
    expander.create_impl_block(request, method_def_id, "hash", false);
}

/// Generate a hash value expression for a given value and type.
fn hash_value(value_expr: Expr, ty: &Type, span: Span) -> Expr {
    use crate::hir::TypeKind;

    match ty.kind() {
        // Handle Never type first (canonical representation)
        TypeKind::Never => {
            // Never type can't be instantiated, hash to 0
            literal_expr(LiteralValue::Int(0), Type::u64(), span)
        }
        TypeKind::Primitive(prim) => {
            use crate::hir::PrimitiveTy;
            match prim {
                PrimitiveTy::Bool |
                PrimitiveTy::Char |
                PrimitiveTy::Int(_) |
                PrimitiveTy::Uint(_) => {
                    // Cast to u64
                    Expr::new(
                        ExprKind::Cast {
                            expr: Box::new(value_expr),
                            target_ty: Type::u64(),
                        },
                        Type::u64(),
                        span,
                    )
                }
                PrimitiveTy::Float(_) => {
                    // Hash float bits
                    // For now, just cast (not ideal but works)
                    Expr::new(
                        ExprKind::Cast {
                            expr: Box::new(value_expr),
                            target_ty: Type::u64(),
                        },
                        Type::u64(),
                        span,
                    )
                }
                PrimitiveTy::String | PrimitiveTy::Str => {
                    // For strings, use a method call to hash
                    // For now, return a constant (proper impl would iterate chars)
                    literal_expr(LiteralValue::Int(0), Type::u64(), span)
                }
                PrimitiveTy::Unit => {
                    // Unit type always hashes to 0
                    literal_expr(LiteralValue::Int(0), Type::u64(), span)
                }
                PrimitiveTy::Never => {
                    // Defensive: also handle via Primitive path
                    literal_expr(LiteralValue::Int(0), Type::u64(), span)
                }
            }
        }
        _ => {
            // For other types, try to call .hash() method
            // For now, return 0 as fallback
            literal_expr(LiteralValue::Int(0), Type::u64(), span)
        }
    }
}
