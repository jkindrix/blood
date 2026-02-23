//! Debug derive implementation.
//!
//! Generates `fn debug(&self) -> String` that formats the type's fields.

use crate::hir::{
    self, Type, LocalId, Body, Local, FnSig,
    Expr, ExprKind,
};

use super::{DeriveExpander, DeriveRequest, string_literal, local_expr};

/// Expand Debug derive for a struct.
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

    // Create signature: fn debug(&self) -> str
    // Note: Returns str slice rather than heap-allocated String
    let sig = FnSig::new(vec![self_ref_ty.clone()], Type::str());
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

    // Return local (index 0)
    let return_local = Local {
        id: LocalId::new(0),
        ty: Type::str(),
        mutable: false,
        name: None,
        span,
    };

    // Build the debug string: "TypeName { field1, field2, ... }"
    // For now, just return the struct name with field names (not values)
    // This simplification avoids complex string concatenation and format! expansion
    let field_names: Vec<String> = struct_info.fields.iter()
        .map(|f| f.name.clone())
        .collect();
    let debug_str = if field_names.is_empty() {
        struct_info.name.clone()
    } else {
        format!("{} {{ {} }}", struct_info.name, field_names.join(", "))
    };
    let result_expr = string_literal(&debug_str, span);

    let body = Body {
        locals: vec![return_local, self_local],
        param_count: 1,
        expr: result_expr,
        span,
        tuple_destructures: std::collections::HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block
    expander.create_impl_block(request, method_def_id, "debug", false);
}

/// Expand Debug derive for an enum.
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

    // Create signature: fn debug(&self) -> str
    let sig = FnSig::new(vec![self_ref_ty.clone()], Type::str());
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
        ty: Type::str(),
        mutable: false,
        name: None,
        span,
    };

    // Build match arms for each variant
    let mut arms = Vec::new();
    for variant in &enum_info.variants {
        let pattern = if variant.fields.is_empty() {
            // Unit variant: Enum::Variant
            hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: Vec::new(),
                },
                ty: self_ty.clone(),
                span,
            }
        } else {
            // Variant with fields: Enum::Variant { ... } - use wildcard for now
            hir::Pattern {
                kind: hir::PatternKind::Variant {
                    def_id: request.type_def_id,
                    variant_idx: variant.index,
                    fields: variant.fields.iter().map(|_| {
                        hir::Pattern {
                            kind: hir::PatternKind::Wildcard,
                            ty: Type::error(), // Type doesn't matter for wildcard
                            span,
                        }
                    }).collect(),
                },
                ty: self_ty.clone(),
                span,
            }
        };

        // Body: return the variant name as string
        let body_expr = if variant.fields.is_empty() {
            string_literal(&format!("{}::{}", enum_info.name, variant.name), span)
        } else {
            // For variants with fields, show "Enum::Variant { ... }"
            string_literal(&format!("{}::{} {{ ... }}", enum_info.name, variant.name), span)
        };

        arms.push(hir::MatchArm {
            pattern,
            guard: None,
            body: body_expr,
        });
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
        Type::str(),
        span,
    );

    let body = Body {
        locals: vec![return_local, self_local],
        param_count: 1,
        expr: match_expr,
        span,
        tuple_destructures: std::collections::HashMap::new(),
    };

    expander.bodies.insert(body_id, body);
    expander.fn_bodies.insert(method_def_id, body_id);

    // Create impl block
    expander.create_impl_block(request, method_def_id, "debug", false);
}

// NOTE: More sophisticated Debug implementation with field values would require
// string concatenation support in codegen and format! macro integration.
// For now, the simplified version just returns the type structure.
