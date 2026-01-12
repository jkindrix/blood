//! Pattern matching type checking.
//!
//! This module contains methods for defining and lowering patterns.

use std::collections::HashSet;

use crate::ast;
use crate::hir::{self, LocalId, Type, TypeKind};

use super::TypeContext;
use super::super::error::{TypeError, TypeErrorKind};
use super::super::resolve::Binding;

impl<'a> TypeContext<'a> {
    /// Define a pattern, returning the local ID for simple patterns.
    pub(crate) fn define_pattern(&mut self, pattern: &ast::Pattern, ty: Type) -> Result<LocalId, TypeError> {
        match &pattern.kind {
            ast::PatternKind::Ident { name, mutable, .. } => {
                let name_str = self.symbol_to_string(name.node);
                let local_id = self.resolver.define_local(
                    name_str.clone(),
                    ty.clone(),
                    *mutable,
                    pattern.span,
                )?;

                self.locals.push(hir::Local {
                    id: local_id,
                    ty,
                    mutable: *mutable,
                    name: Some(name_str),
                    span: pattern.span,
                });

                Ok(local_id)
            }
            ast::PatternKind::Wildcard => {
                // Anonymous local
                let local_id = self.resolver.next_local_id();
                self.locals.push(hir::Local {
                    id: local_id,
                    ty,
                    mutable: false,
                    name: None,
                    span: pattern.span,
                });
                Ok(local_id)
            }
            ast::PatternKind::Tuple { fields, .. } => {
                // Tuple destructuring pattern: let (x, y) = ...
                let elem_types = match ty.kind() {
                    hir::TypeKind::Tuple(elems) => elems.clone(),
                    hir::TypeKind::Infer(_) => {
                        // Type not yet known - create fresh variables for each element
                        (0..fields.len())
                            .map(|_| self.unifier.fresh_var())
                            .collect::<Vec<_>>()
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotATuple { ty: ty.clone() },
                            pattern.span,
                        ));
                    }
                };

                if fields.len() != elem_types.len() {
                    return Err(TypeError::new(
                        TypeErrorKind::WrongArity {
                            expected: elem_types.len(),
                            found: fields.len(),
                        },
                        pattern.span,
                    ));
                }

                if matches!(ty.kind(), hir::TypeKind::Infer(_)) {
                    let tuple_ty = Type::tuple(elem_types.clone());
                    self.unifier.unify(&ty, &tuple_ty, pattern.span)?;
                }

                let mut first_local_id = None;
                for (field_pat, elem_ty) in fields.iter().zip(elem_types.iter()) {
                    let local_id = self.define_pattern(field_pat, elem_ty.clone())?;
                    if first_local_id.is_none() {
                        first_local_id = Some(local_id);
                    }
                }

                Ok(first_local_id.unwrap_or_else(|| {
                    let local_id = self.resolver.next_local_id();
                    self.locals.push(hir::Local {
                        id: local_id,
                        ty: Type::unit(),
                        mutable: false,
                        name: None,
                        span: pattern.span,
                    });
                    local_id
                }))
            }
            ast::PatternKind::Paren(inner) => {
                self.define_pattern(inner, ty)
            }
            ast::PatternKind::Literal(_) => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "literal patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Path(_) => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "path patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::TupleStruct { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "tuple struct patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Rest => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "rest patterns (..) in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Ref { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "reference patterns (&x) in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Struct { path, fields, rest } => {
                // Struct pattern: let Point { x, y } = point;
                let struct_name = if path.segments.len() == 1 {
                    self.symbol_to_string(path.segments[0].name.node)
                } else if path.segments.len() == 2 {
                    self.symbol_to_string(path.segments[1].name.node)
                } else {
                    return Err(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "struct pattern paths with more than 2 segments".to_string(),
                        },
                        pattern.span,
                    ));
                };

                let struct_def_id = match ty.kind() {
                    TypeKind::Adt { def_id, .. } => *def_id,
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: ty.clone() },
                            pattern.span,
                        ));
                    }
                };

                let struct_info = self.struct_defs.get(&struct_def_id).cloned().ok_or_else(|| {
                    TypeError::new(
                        TypeErrorKind::TypeNotFound { name: struct_name.clone() },
                        pattern.span,
                    )
                })?;

                // Create a hidden local for the whole struct value
                let hidden_name = format!("__struct_{}", pattern.span.start);
                let hidden_local = self.resolver.next_local_id();
                self.locals.push(hir::Local {
                    id: hidden_local,
                    name: Some(hidden_name),
                    ty: ty.clone(),
                    mutable: false,
                    span: pattern.span,
                });

                // Process each field pattern
                let mut bound_fields = HashSet::new();
                for field_pattern in fields {
                    let field_name = self.symbol_to_string(field_pattern.name.node);

                    let field_info = struct_info.fields.iter()
                        .find(|f| f.name == field_name)
                        .ok_or_else(|| TypeError::new(
                            TypeErrorKind::NoField {
                                ty: ty.clone(),
                                field: field_name.clone(),
                            },
                            field_pattern.span,
                        ))?;

                    bound_fields.insert(field_name.clone());

                    if let Some(ref inner_pattern) = field_pattern.pattern {
                        self.define_pattern(inner_pattern, field_info.ty.clone())?;
                    } else {
                        let local_id = self.resolver.define_local(
                            field_name.clone(),
                            field_info.ty.clone(),
                            false,
                            pattern.span,
                        )?;
                        self.locals.push(hir::Local {
                            id: local_id,
                            name: Some(field_name),
                            ty: field_info.ty.clone(),
                            mutable: false,
                            span: field_pattern.span,
                        });
                    }
                }

                // If not using rest (..), verify all fields are bound
                if !*rest {
                    for field_info in &struct_info.fields {
                        if !bound_fields.contains(&field_info.name) {
                            return Err(TypeError::new(
                                TypeErrorKind::MissingField {
                                    ty: ty.clone(),
                                    field: field_info.name.clone(),
                                },
                                pattern.span,
                            ));
                        }
                    }
                }

                Ok(hidden_local)
            }
            ast::PatternKind::Slice { elements, rest_pos } => {
                // Slice pattern: let [first, second, ..] = arr;
                let elem_ty = match ty.kind() {
                    TypeKind::Array { element, size } => {
                        let num_patterns = if rest_pos.is_some() { elements.len() - 1 } else { elements.len() };
                        if num_patterns as u64 > *size {
                            return Err(TypeError::new(
                                TypeErrorKind::PatternMismatch {
                                    expected: ty.clone(),
                                    pattern: format!("slice pattern with {} elements", num_patterns),
                                },
                                pattern.span,
                            ));
                        }
                        element.clone()
                    }
                    TypeKind::Slice { element } => element.clone(),
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotIndexable { ty: ty.clone() },
                            pattern.span,
                        ));
                    }
                };

                // Create a hidden local for the whole array/slice value
                let hidden_name = format!("__slice_{}", pattern.span.start);
                let hidden_local = self.resolver.next_local_id();
                self.locals.push(hir::Local {
                    id: hidden_local,
                    name: Some(hidden_name),
                    ty: ty.clone(),
                    mutable: false,
                    span: pattern.span,
                });

                // Process each element pattern
                for (i, elem_pattern) in elements.iter().enumerate() {
                    // Handle rest pattern (..)
                    if rest_pos.is_some() && Some(i) == *rest_pos {
                        if let ast::PatternKind::Rest = &elem_pattern.kind {
                            continue;
                        }
                    }
                    self.define_pattern(elem_pattern, elem_ty.clone())?;
                }

                Ok(hidden_local)
            }
            ast::PatternKind::Or { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "or patterns (a | b) in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
            ast::PatternKind::Range { .. } => {
                Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "range patterns in let bindings".to_string(),
                    },
                    pattern.span,
                ))
            }
        }
    }

    /// Lower a pattern from AST to HIR.
    pub(crate) fn lower_pattern(&mut self, pattern: &ast::Pattern, expected_ty: &Type) -> Result<hir::Pattern, TypeError> {
        let kind = match &pattern.kind {
            ast::PatternKind::Wildcard => hir::PatternKind::Wildcard,
            ast::PatternKind::Ident { name, mutable, .. } => {
                let name_str = self.symbol_to_string(name.node);
                let local_id = self.resolver.define_local(
                    name_str.clone(),
                    expected_ty.clone(),
                    *mutable,
                    pattern.span,
                )?;

                self.locals.push(hir::Local {
                    id: local_id,
                    ty: expected_ty.clone(),
                    mutable: *mutable,
                    name: Some(name_str),
                    span: pattern.span,
                });

                hir::PatternKind::Binding {
                    local_id,
                    mutable: *mutable,
                    subpattern: None,
                }
            }
            ast::PatternKind::Literal(lit) => {
                hir::PatternKind::Literal(hir::LiteralValue::from_ast(&lit.kind))
            }
            ast::PatternKind::Tuple { fields, rest_pos } => {
                match expected_ty.kind() {
                    TypeKind::Tuple(elem_types) => {
                        if let Some(pos) = rest_pos {
                            let prefix_count = *pos;
                            let suffix_count = fields.len() - prefix_count;
                            let min_elems = prefix_count + suffix_count;

                            if elem_types.len() < min_elems {
                                return Err(TypeError::new(
                                    TypeErrorKind::PatternMismatch {
                                        expected: expected_ty.clone(),
                                        pattern: format!(
                                            "tuple pattern requires at least {} elements, found {}",
                                            min_elems, elem_types.len()
                                        ),
                                    },
                                    pattern.span,
                                ));
                            }

                            let mut hir_fields = Vec::new();
                            // Lower prefix patterns
                            for (i, field) in fields.iter().take(prefix_count).enumerate() {
                                hir_fields.push(self.lower_pattern(field, &elem_types[i])?);
                            }
                            // Add wildcards for skipped elements
                            let skipped = elem_types.len() - min_elems;
                            for i in 0..skipped {
                                let wildcard_ty = elem_types[prefix_count + i].clone();
                                hir_fields.push(hir::Pattern {
                                    kind: hir::PatternKind::Wildcard,
                                    ty: wildcard_ty,
                                    span: pattern.span,
                                });
                            }
                            // Lower suffix patterns
                            for (i, field) in fields.iter().skip(prefix_count).enumerate() {
                                let elem_idx = prefix_count + skipped + i;
                                hir_fields.push(self.lower_pattern(field, &elem_types[elem_idx])?);
                            }
                            hir::PatternKind::Tuple(hir_fields)
                        } else {
                            if fields.len() != elem_types.len() {
                                return Err(TypeError::new(
                                    TypeErrorKind::PatternMismatch {
                                        expected: expected_ty.clone(),
                                        pattern: format!("tuple pattern with {} elements", fields.len()),
                                    },
                                    pattern.span,
                                ));
                            }
                            let mut hir_fields = Vec::new();
                            for (field, elem_ty) in fields.iter().zip(elem_types.iter()) {
                                hir_fields.push(self.lower_pattern(field, elem_ty)?);
                            }
                            hir::PatternKind::Tuple(hir_fields)
                        }
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotATuple { ty: expected_ty.clone() },
                            pattern.span,
                        ));
                    }
                }
            }
            ast::PatternKind::TupleStruct { path, fields, .. } => {
                let path_str = if let Some(seg) = path.segments.first() {
                    self.symbol_to_string(seg.name.node)
                } else {
                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{path:?}") },
                        pattern.span,
                    ));
                };

                match self.resolver.lookup(&path_str) {
                    Some(Binding::Def(variant_def_id)) => {
                        if let Some(info) = self.resolver.def_info.get(&variant_def_id) {
                            if info.kind == hir::DefKind::Variant {
                                let enum_def_id = info.parent.ok_or_else(|| TypeError::new(
                                    TypeErrorKind::NotFound { name: format!("parent of variant {}", path_str) },
                                    pattern.span,
                                ))?;

                                let enum_info = self.enum_defs.get(&enum_def_id).ok_or_else(|| TypeError::new(
                                    TypeErrorKind::NotFound { name: format!("enum for variant {}", path_str) },
                                    pattern.span,
                                ))?;

                                let variant_info = enum_info.variants.iter()
                                    .find(|v| v.def_id == variant_def_id)
                                    .ok_or_else(|| TypeError::new(
                                        TypeErrorKind::NotFound { name: format!("variant {} in enum", path_str) },
                                        pattern.span,
                                    ))?;

                                let variant_idx = variant_info.index;
                                let variant_field_types: Vec<Type> = variant_info.fields.iter()
                                    .map(|f| f.ty.clone())
                                    .collect();

                                if fields.len() != variant_field_types.len() {
                                    return Err(TypeError::new(
                                        TypeErrorKind::WrongArity {
                                            expected: variant_field_types.len(),
                                            found: fields.len(),
                                        },
                                        pattern.span,
                                    ));
                                }

                                let mut hir_fields = Vec::new();
                                for (field, field_ty) in fields.iter().zip(variant_field_types.iter()) {
                                    hir_fields.push(self.lower_pattern(field, field_ty)?);
                                }

                                hir::PatternKind::Variant {
                                    def_id: variant_def_id,
                                    variant_idx,
                                    fields: hir_fields,
                                }
                            } else {
                                return Err(TypeError::new(
                                    TypeErrorKind::NotFound { name: path_str },
                                    pattern.span,
                                ));
                            }
                        } else {
                            return Err(TypeError::new(
                                TypeErrorKind::NotFound { name: path_str },
                                pattern.span,
                            ));
                        }
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotFound { name: path_str },
                            pattern.span,
                        ));
                    }
                }
            }
            ast::PatternKind::Rest => {
                return Err(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "rest patterns (..) are not yet supported".to_string(),
                    },
                    pattern.span,
                ));
            }
            ast::PatternKind::Ref { mutable, inner } => {
                match expected_ty.kind() {
                    TypeKind::Ref { inner: inner_ty, mutable: ty_mutable } => {
                        if *mutable && !ty_mutable {
                            return Err(TypeError::new(
                                TypeErrorKind::PatternMismatch {
                                    expected: expected_ty.clone(),
                                    pattern: "&mut pattern but type is &".to_string(),
                                },
                                pattern.span,
                            ));
                        }
                        let inner_pat = self.lower_pattern(inner, inner_ty)?;
                        hir::PatternKind::Ref {
                            mutable: *mutable,
                            inner: Box::new(inner_pat),
                        }
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::PatternMismatch {
                                expected: expected_ty.clone(),
                                pattern: "reference pattern".to_string(),
                            },
                            pattern.span,
                        ));
                    }
                }
            }
            ast::PatternKind::Struct { path, fields, rest } => {
                let (struct_def_id, _type_args) = match expected_ty.kind() {
                    TypeKind::Adt { def_id, args, .. } => (*def_id, args.clone()),
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: expected_ty.clone() },
                            pattern.span,
                        ));
                    }
                };

                if !path.segments.is_empty() {
                    let _path_name = self.symbol_to_string(path.segments[0].name.node);
                }

                let struct_info = self.struct_defs.get(&struct_def_id).cloned().ok_or_else(|| {
                    TypeError::new(
                        TypeErrorKind::TypeNotFound { name: format!("struct {:?}", struct_def_id) },
                        pattern.span,
                    )
                })?;

                let mut hir_fields = Vec::new();
                let mut bound_fields = HashSet::new();

                for field_pattern in fields {
                    let field_name = self.symbol_to_string(field_pattern.name.node);

                    let field_info = struct_info.fields.iter()
                        .find(|f| f.name == field_name)
                        .ok_or_else(|| TypeError::new(
                            TypeErrorKind::NoField {
                                ty: expected_ty.clone(),
                                field: field_name.clone(),
                            },
                            field_pattern.span,
                        ))?;

                    bound_fields.insert(field_name.clone());

                    let inner_pattern = if let Some(ref inner) = field_pattern.pattern {
                        self.lower_pattern(inner, &field_info.ty)?
                    } else {
                        let local_id = self.resolver.define_local(
                            field_name.clone(),
                            field_info.ty.clone(),
                            false,
                            field_pattern.span,
                        )?;
                        self.locals.push(hir::Local {
                            id: local_id,
                            name: Some(field_name),
                            ty: field_info.ty.clone(),
                            mutable: false,
                            span: field_pattern.span,
                        });
                        hir::Pattern {
                            kind: hir::PatternKind::Binding {
                                local_id,
                                mutable: false,
                                subpattern: None,
                            },
                            ty: field_info.ty.clone(),
                            span: field_pattern.span,
                        }
                    };

                    hir_fields.push(hir::FieldPattern {
                        field_idx: field_info.index,
                        pattern: inner_pattern,
                    });
                }

                if !*rest {
                    for field_info in &struct_info.fields {
                        if !bound_fields.contains(&field_info.name) {
                            return Err(TypeError::new(
                                TypeErrorKind::MissingField {
                                    ty: expected_ty.clone(),
                                    field: field_info.name.clone(),
                                },
                                pattern.span,
                            ));
                        }
                    }
                }

                hir::PatternKind::Struct {
                    def_id: struct_def_id,
                    fields: hir_fields,
                }
            }
            ast::PatternKind::Slice { elements, rest_pos } => {
                let elem_ty = match expected_ty.kind() {
                    TypeKind::Array { element, .. } => element.clone(),
                    TypeKind::Slice { element } => element.clone(),
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::PatternMismatch {
                                expected: expected_ty.clone(),
                                pattern: "slice pattern".to_string(),
                            },
                            pattern.span,
                        ));
                    }
                };

                let (prefix_pats, rest_pattern, suffix_pats) = if let Some(rest_idx) = rest_pos {
                    let rest_idx = *rest_idx;
                    let prefix: Vec<_> = elements.iter().take(rest_idx).cloned().collect();
                    let suffix: Vec<_> = elements.iter().skip(rest_idx + 1).cloned().collect();
                    let rest_pat = if rest_idx < elements.len() {
                        Some(Box::new(hir::Pattern {
                            kind: hir::PatternKind::Wildcard,
                            ty: Type::slice(elem_ty.clone()),
                            span: pattern.span,
                        }))
                    } else {
                        None
                    };
                    (prefix, rest_pat, suffix)
                } else {
                    (elements.clone(), None, Vec::new())
                };

                let mut prefix = Vec::new();
                for p in &prefix_pats {
                    prefix.push(self.lower_pattern(p, &elem_ty)?);
                }

                let mut suffix = Vec::new();
                for p in &suffix_pats {
                    suffix.push(self.lower_pattern(p, &elem_ty)?);
                }

                hir::PatternKind::Slice {
                    prefix,
                    slice: rest_pattern,
                    suffix,
                }
            }
            ast::PatternKind::Or(alternatives) => {
                if alternatives.is_empty() {
                    return Err(TypeError::new(
                        TypeErrorKind::PatternMismatch {
                            expected: expected_ty.clone(),
                            pattern: "empty or pattern".to_string(),
                        },
                        pattern.span,
                    ));
                }

                let mut hir_alternatives = Vec::new();
                for alt in alternatives {
                    hir_alternatives.push(self.lower_pattern(alt, expected_ty)?);
                }

                hir::PatternKind::Or(hir_alternatives)
            }
            ast::PatternKind::Range { start, end, inclusive } => {
                use crate::hir::ty::PrimitiveTy;

                match expected_ty.kind() {
                    TypeKind::Primitive(PrimitiveTy::Int(_) | PrimitiveTy::Uint(_) | PrimitiveTy::Char) => {}
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::PatternMismatch {
                                expected: expected_ty.clone(),
                                pattern: "range pattern (requires integer or char type)".to_string(),
                            },
                            pattern.span,
                        ));
                    }
                }

                let hir_start = if let Some(s) = start {
                    Some(Box::new(self.lower_pattern(s, expected_ty)?))
                } else {
                    None
                };

                let hir_end = if let Some(e) = end {
                    Some(Box::new(self.lower_pattern(e, expected_ty)?))
                } else {
                    None
                };

                hir::PatternKind::Range {
                    start: hir_start,
                    end: hir_end,
                    inclusive: *inclusive,
                }
            }
            ast::PatternKind::Path(path) => {
                let path_str = if let Some(seg) = path.segments.first() {
                    self.symbol_to_string(seg.name.node)
                } else {
                    return Err(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{path:?}") },
                        pattern.span,
                    ));
                };

                match self.resolver.lookup(&path_str) {
                    Some(Binding::Def(def_id)) => {
                        if let Some(info) = self.resolver.def_info.get(&def_id) {
                            if info.kind == hir::DefKind::Variant {
                                hir::PatternKind::Variant {
                                    def_id,
                                    variant_idx: 0, // Simplified
                                    fields: vec![],
                                }
                            } else {
                                return Err(TypeError::new(
                                    TypeErrorKind::NotFound { name: path_str },
                                    pattern.span,
                                ));
                            }
                        } else {
                            return Err(TypeError::new(
                                TypeErrorKind::NotFound { name: path_str },
                                pattern.span,
                            ));
                        }
                    }
                    _ => {
                        return Err(TypeError::new(
                            TypeErrorKind::NotFound { name: path_str },
                            pattern.span,
                        ));
                    }
                }
            }
            ast::PatternKind::Paren(inner) => {
                return self.lower_pattern(inner, expected_ty);
            }
        };

        Ok(hir::Pattern {
            kind,
            ty: expected_ty.clone(),
            span: pattern.span,
        })
    }
}
