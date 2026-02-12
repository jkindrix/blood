//! Default derive implementation.
//!
//! Generates `fn default() -> Self` that returns the default value.

use std::collections::HashMap;

use crate::hir::{
    Type, TypeKind, LocalId, Body, Local, FnSig,
    Expr, ExprKind, FieldExpr, LiteralValue, PrimitiveTy,
};
use crate::span::Span;

use super::{DeriveExpander, DeriveRequest, literal_expr, bool_literal, string_literal};

/// Expand Default derive for a struct.
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

    // Create signature: fn default() -> Self (static method, no self parameter)
    let sig = FnSig::new(Vec::new(), self_ty.clone());
    expander.fn_sigs.insert(method_def_id, sig);
    expander.method_self_types.insert(method_def_id, self_ty.clone());

    // Create body
    let return_local = Local {
        id: LocalId::new(0),
        ty: self_ty.clone(),
        mutable: false,
        name: None,
        span,
    };

    // Build struct expression: Self { field1: Default::default(), ... }
    let fields: Vec<FieldExpr> = struct_info.fields.iter().map(|field| {
        FieldExpr {
            field_idx: field.index,
            value: default_value_for_type(&field.ty, span),
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
        locals: vec![return_local],
        param_count: 0,
        expr: struct_expr,
        span,
        tuple_destructures: HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block (static method)
    expander.create_impl_block(request, method_def_id, "default", true);
}

/// Expand Default derive for an enum.
///
/// For enums, Default generates a default for the first variant only if it's a unit variant.
/// Otherwise, it generates an error expression (enums need explicit default variant).
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

    // Create signature: fn default() -> Self
    let sig = FnSig::new(Vec::new(), self_ty.clone());
    expander.fn_sigs.insert(method_def_id, sig);
    expander.method_self_types.insert(method_def_id, self_ty.clone());

    // Create body
    let return_local = Local {
        id: LocalId::new(0),
        ty: self_ty.clone(),
        mutable: false,
        name: None,
        span,
    };

    // Find the first unit variant as default, or the first variant with all defaultable fields
    let default_variant = enum_info.variants.iter().find(|v| v.fields.is_empty())
        .or_else(|| enum_info.variants.first());

    let result_expr = match default_variant {
        Some(variant) => {
            if variant.fields.is_empty() {
                // Unit variant
                Expr::new(
                    ExprKind::Variant {
                        def_id: request.type_def_id,
                        variant_idx: variant.index,
                        fields: Vec::new(),
                    },
                    self_ty.clone(),
                    span,
                )
            } else {
                // Variant with fields - use defaults for each field
                let fields: Vec<Expr> = variant.fields.iter().map(|field| {
                    default_value_for_type(&field.ty, span)
                }).collect();

                Expr::new(
                    ExprKind::Variant {
                        def_id: request.type_def_id,
                        variant_idx: variant.index,
                        fields,
                    },
                    self_ty.clone(),
                    span,
                )
            }
        }
        None => {
            // No variants - this shouldn't happen for valid enums
            Expr::error(span)
        }
    };

    let body = Body {
        locals: vec![return_local],
        param_count: 0,
        expr: result_expr,
        span,
        tuple_destructures: HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block (static method)
    expander.create_impl_block(request, method_def_id, "default", true);
}

/// Generate a default value expression for a given type.
fn default_value_for_type(ty: &Type, span: Span) -> Expr {
    match ty.kind() {
        // Handle Never type first (canonical representation)
        TypeKind::Never => Expr::error(span),
        TypeKind::Primitive(prim) => match prim {
            PrimitiveTy::Bool => bool_literal(false, span),
            PrimitiveTy::Char => {
                let char_ty = Type::new(TypeKind::Primitive(PrimitiveTy::Char));
                literal_expr(LiteralValue::Char('\0'), char_ty, span)
            }
            PrimitiveTy::String | PrimitiveTy::Str => string_literal("", span),
            PrimitiveTy::Int(int_ty) => {
                let ty = Type::new(TypeKind::Primitive(PrimitiveTy::Int(*int_ty)));
                literal_expr(LiteralValue::Int(0), ty, span)
            }
            PrimitiveTy::Uint(uint_ty) => {
                let ty = Type::new(TypeKind::Primitive(PrimitiveTy::Uint(*uint_ty)));
                literal_expr(LiteralValue::Int(0), ty, span)
            }
            PrimitiveTy::Float(float_ty) => {
                let ty = Type::new(TypeKind::Primitive(PrimitiveTy::Float(*float_ty)));
                literal_expr(LiteralValue::Float(0.0), ty, span)
            }
            PrimitiveTy::Unit => Expr::new(ExprKind::Tuple(Vec::new()), Type::unit(), span),
            PrimitiveTy::Never => Expr::error(span), // Defensive: also handle via Primitive path
        },
        TypeKind::Tuple(elems) => {
            let defaults: Vec<Expr> = elems.iter()
                .map(|t| default_value_for_type(t, span))
                .collect();
            Expr::new(ExprKind::Tuple(defaults), ty.clone(), span)
        }
        TypeKind::Array { element, size } => {
            let default_elem = default_value_for_type(element, span);
            // Get concrete size, defaulting to 0 if it's a const param (shouldn't happen)
            let concrete_size = size.as_u64().unwrap_or(0);
            Expr::new(
                ExprKind::Repeat {
                    value: Box::new(default_elem),
                    count: concrete_size,
                },
                ty.clone(),
                span,
            )
        }
        _ => {
            // For other types (ADTs, etc.), use the special Default expression
            // This will be resolved by the type checker or codegen
            Expr::new(ExprKind::Default, ty.clone(), span)
        }
    }
}
