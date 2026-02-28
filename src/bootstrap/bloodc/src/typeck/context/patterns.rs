//! Pattern matching type checking.
//!
//! This module contains methods for defining and lowering patterns.

use std::collections::HashSet;

use string_interner::Symbol as _;

use crate::ast;
use crate::hir::{self, DefId, LocalId, Type, TypeKind};
use crate::hir::ty::TyVarId;

use super::TypeContext;
use super::super::error::{TypeError, TypeErrorKind};
use super::super::resolve::Binding;


impl<'a> TypeContext<'a> {
    /// Define a pattern, returning the local ID for simple patterns.
    pub(crate) fn define_pattern(&mut self, pattern: &ast::Pattern, ty: Type) -> Result<LocalId, Box<TypeError>> {
        match &pattern.kind {
            ast::PatternKind::Ident { name, mutable, by_ref, .. } => {
                let name_str = self.symbol_to_string(name.node);

                // If `by_ref` is true (e.g., `ref x` or `ref mut x`), the binding
                // gets a reference type rather than the matched type directly.
                let binding_ty = if *by_ref {
                    Type::reference(ty.clone(), *mutable)
                } else {
                    ty.clone()
                };

                let local_id = self.resolver.define_local(
                    name_str.clone(),
                    binding_ty.clone(),
                    *mutable,
                    pattern.span,
                )?;

                self.locals.push(hir::Local {
                    id: local_id,
                    ty: binding_ty,
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
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotATuple { ty: ty.clone() },
                            pattern.span,
                        )));
                    }
                };

                if fields.len() != elem_types.len() {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::WrongArity {
                            expected: elem_types.len(),
                            found: fields.len(),
                        },
                        pattern.span,
                    )));
                }

                let tuple_ty = if matches!(ty.kind(), hir::TypeKind::Infer(_)) {
                    let tuple_ty = Type::tuple(elem_types.clone());
                    self.unifier.unify(&ty, &tuple_ty, pattern.span)?;
                    tuple_ty
                } else {
                    ty.clone()
                };

                // First, create all the element locals
                let mut element_locals = Vec::new();
                for (field_pat, elem_ty) in fields.iter().zip(elem_types.iter()) {
                    let local_id = self.define_pattern(field_pat, elem_ty.clone())?;
                    element_locals.push(local_id);
                }

                // Create a hidden tuple local that will hold the full tuple value
                // The MIR lowering will assign to this, then extract fields to element locals
                let tuple_local_id = self.resolver.next_local_id();
                self.locals.push(hir::Local {
                    id: tuple_local_id,
                    ty: tuple_ty,
                    mutable: false,
                    name: Some(format!("__tuple_{}", tuple_local_id.index)),
                    span: pattern.span,
                });

                // Record the tuple destructuring info for MIR lowering
                self.tuple_destructures.insert(tuple_local_id, element_locals);

                Ok(tuple_local_id)
            }
            ast::PatternKind::Paren(inner) => {
                self.define_pattern(inner, ty)
            }
            ast::PatternKind::Literal(_) => {
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "literal patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                )))
            }
            ast::PatternKind::Path(_) => {
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "path patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                )))
            }
            ast::PatternKind::TupleStruct { .. } => {
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "tuple struct patterns in let bindings (use match instead)".to_string(),
                    },
                    pattern.span,
                )))
            }
            ast::PatternKind::Rest => {
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "rest patterns (..) in let bindings".to_string(),
                    },
                    pattern.span,
                )))
            }
            ast::PatternKind::Ref { .. } => {
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "reference patterns (&x) in let bindings".to_string(),
                    },
                    pattern.span,
                )))
            }
            ast::PatternKind::Struct { path, fields, rest } => {
                // Struct pattern: let Point { x, y } = point;
                let struct_name = if path.segments.len() == 1 {
                    self.symbol_to_string(path.segments[0].name.node)
                } else if path.segments.len() == 2 {
                    self.symbol_to_string(path.segments[1].name.node)
                } else {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::UnsupportedFeature {
                            feature: "struct pattern paths with more than 2 segments".to_string(),
                        },
                        pattern.span,
                    )));
                };

                let struct_def_id = match ty.kind() {
                    TypeKind::Adt { def_id, .. } => *def_id,
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: ty.clone() },
                            pattern.span,
                        )));
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
                        .ok_or_else(|| Box::new(TypeError::new(
                            TypeErrorKind::NoField {
                                ty: ty.clone(),
                                field: field_name.clone(),
                            },
                            field_pattern.span,
                        )))?;

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
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::MissingField {
                                    ty: ty.clone(),
                                    field: field_info.name.clone(),
                                },
                                pattern.span,
                            )));
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
                        let array_size = size.as_u64().unwrap_or(0);
                        if num_patterns as u64 > array_size {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::PatternMismatch {
                                    expected: ty.clone(),
                                    pattern: format!("slice pattern with {} elements", num_patterns),
                                },
                                pattern.span,
                            )));
                        }
                        element.clone()
                    }
                    TypeKind::Slice { element } => element.clone(),
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotIndexable { ty: ty.clone() },
                            pattern.span,
                        )));
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
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "or patterns (a | b) in let bindings".to_string(),
                    },
                    pattern.span,
                )))
            }
            ast::PatternKind::Range { .. } => {
                Err(Box::new(TypeError::new(
                    TypeErrorKind::UnsupportedFeature {
                        feature: "range patterns in let bindings".to_string(),
                    },
                    pattern.span,
                )))
            }
        }
    }

    /// Lower a pattern from AST to HIR.
    ///
    /// # Algorithm
    ///
    /// Pattern lowering transforms AST patterns into HIR patterns while:
    /// 1. **Type checking**: Validates pattern structure matches expected type
    /// 2. **Variable binding**: Creates LocalIds for bound variables
    /// 3. **Generic substitution**: Substitutes type parameters in enum/struct patterns
    ///
    /// ## Pattern Kinds
    ///
    /// - **Wildcard** (`_`): Matches anything, no binding created
    /// - **Ident** (`x`, `mut x`): Creates binding with LocalId
    /// - **Literal** (`42`, `"hello"`): Matches specific value
    /// - **Tuple** (`(a, b, .., c)`): Destructures tuples, handles rest patterns
    /// - **Slice** (`[a, b, .., c]`): Destructures arrays/slices with rest
    /// - **Struct** (`Point { x, y }`): Matches struct fields by name
    /// - **TupleStruct** (`Some(x)`): Matches enum variants or tuple structs
    /// - **Or** (`A | B`): Matches any alternative (all must bind same vars)
    ///
    /// ## Rest Pattern Handling (`..`)
    ///
    /// Rest patterns in tuples/slices are complex because they have variable arity:
    /// - `rest_pos` (from parser): Index where `..` appears, elements array excludes `..`
    /// - `binding_rest_idx`: Index where `x @ ..` appears (named rest)
    ///
    /// Given `[a, b, .., c, d]` with 5-element array:
    /// - `prefix = [a, b]` (before rest)
    /// - `rest` matches middle elements
    /// - `suffix = [c, d]` (after rest)
    ///
    /// The rest slice length = array_len - prefix_len - suffix_len
    ///
    /// ## Generic Substitution
    ///
    /// For patterns like `Some(x)` where `Option<T>` is the expected type:
    /// 1. Resolve `Some` to the enum variant definition
    /// 2. Extract the variant's field types (with generic params)
    /// 3. Substitute `T` with the concrete type from expected_ty
    /// 4. Type check inner patterns against substituted field types
    ///
    /// ## Match Ergonomics
    ///
    /// When the expected type is a reference (`&T` or `&mut T`) but the pattern is not
    /// a reference pattern (`&pat`), the compiler automatically inserts an implicit
    /// reference pattern. This allows ergonomic patterns like:
    ///
    /// ```blood
    /// match &some_enum {
    ///     Variant { field } => { /* field is &T */ }
    /// }
    /// ```
    ///
    /// Without requiring the explicit:
    ///
    /// ```blood
    /// match &some_enum {
    ///     &Variant { field } => { /* field is T */ }
    /// }
    /// ```
    pub(crate) fn lower_pattern(&mut self, pattern: &ast::Pattern, expected_ty: &Type) -> Result<hir::Pattern, Box<TypeError>> {
        // Match ergonomics: if expected type is a reference but pattern is not a reference pattern,
        // automatically dereference the expected type and wrap result in an implicit Ref pattern.
        // This handles cases like `match &enum_value { Variant { field } => ... }`
        if let TypeKind::Ref { inner: inner_ty, mutable } = expected_ty.kind() {
            // Only apply match ergonomics for patterns that need it
            // (not for explicit reference patterns or simple bindings which handle refs themselves)
            let needs_ergonomics = matches!(
                &pattern.kind,
                ast::PatternKind::Struct { .. }
                    | ast::PatternKind::TupleStruct { .. }
                    | ast::PatternKind::Tuple { .. }
                    | ast::PatternKind::Literal(_)
            );

            if needs_ergonomics {
                // Lower the pattern against the inner (dereferenced) type
                let inner_pattern = self.lower_pattern(pattern, inner_ty)?;

                return Ok(hir::Pattern {
                    kind: hir::PatternKind::Ref {
                        mutable: *mutable,
                        inner: Box::new(inner_pattern),
                    },
                    ty: expected_ty.clone(),
                    span: pattern.span,
                });
            }
        }

        let kind = match &pattern.kind {
            ast::PatternKind::Wildcard => hir::PatternKind::Wildcard,
            ast::PatternKind::Ident { name, mutable, by_ref, .. } => {
                let name_str = self.symbol_to_string(name.node);

                // If `by_ref` is true (e.g., `ref x` or `ref mut x`), the binding
                // gets a reference type rather than the matched type directly.
                let binding_ty = if *by_ref {
                    Type::reference(expected_ty.clone(), *mutable)
                } else {
                    expected_ty.clone()
                };

                let local_id = self.resolver.define_local(
                    name_str.clone(),
                    binding_ty.clone(),
                    *mutable,
                    pattern.span,
                )?;

                self.locals.push(hir::Local {
                    id: local_id,
                    ty: binding_ty,
                    mutable: *mutable,
                    name: Some(name_str),
                    span: pattern.span,
                });

                hir::PatternKind::Binding {
                    local_id,
                    mutable: *mutable,
                    by_ref: *by_ref,
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
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::PatternMismatch {
                                        expected: expected_ty.clone(),
                                        pattern: format!(
                                            "tuple pattern requires at least {} elements, found {}",
                                            min_elems, elem_types.len()
                                        ),
                                    },
                                    pattern.span,
                                )));
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
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::PatternMismatch {
                                        expected: expected_ty.clone(),
                                        pattern: format!("tuple pattern with {} elements", fields.len()),
                                    },
                                    pattern.span,
                                )));
                            }
                            let mut hir_fields = Vec::new();
                            for (field, elem_ty) in fields.iter().zip(elem_types.iter()) {
                                hir_fields.push(self.lower_pattern(field, elem_ty)?);
                            }
                            hir::PatternKind::Tuple(hir_fields)
                        }
                    }
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotATuple { ty: expected_ty.clone() },
                            pattern.span,
                        )));
                    }
                }
            }
            ast::PatternKind::TupleStruct { path, fields, .. } => {
                // Handle both single-segment (Some(v)) and two-segment (Option::Some(v)) paths
                let (enum_def_id, variant_info) = if path.segments.len() == 2 {
                    // Two-segment path: EnumName::VariantName
                    let enum_name = self.symbol_to_string(path.segments[0].name.node);
                    let variant_name = self.symbol_to_string(path.segments[1].name.node);

                    // Look up the enum type
                    let def_id = match self.resolver.lookup_type(&enum_name) {
                        Some(def_id) => def_id,
                        None => return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotFound { name: enum_name },
                            pattern.span,
                        ))),
                    };

                    let enum_info = self.enum_defs.get(&def_id).cloned().ok_or_else(|| Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: enum_name.clone() },
                        pattern.span,
                    )))?;

                    let variant = enum_info.variants.iter()
                        .find(|v| v.name == variant_name)
                        .cloned()
                        .ok_or_else(|| Box::new(TypeError::new(
                            TypeErrorKind::NotFound { name: format!("{}::{}", enum_name, variant_name) },
                            pattern.span,
                        )))?;

                    (def_id, variant)
                } else if path.segments.len() == 1 {
                    // Single-segment path: direct variant reference (e.g., Some)
                    let path_str = self.symbol_to_string(path.segments[0].name.node);

                    match self.resolver.lookup(&path_str) {
                        Some(Binding::Def(variant_def_id)) => {
                            if let Some(info) = self.resolver.def_info.get(&variant_def_id) {
                                if info.kind == hir::DefKind::Variant {
                                    let enum_def_id = info.parent.ok_or_else(|| Box::new(TypeError::new(
                                        TypeErrorKind::NotFound { name: format!("parent of variant {}", path_str) },
                                        pattern.span,
                                    )))?;

                                    let enum_info = self.enum_defs.get(&enum_def_id).cloned().ok_or_else(|| Box::new(TypeError::new(
                                        TypeErrorKind::NotFound { name: format!("enum for variant {}", path_str) },
                                        pattern.span,
                                    )))?;

                                    let variant_info = enum_info.variants.iter()
                                        .find(|v| v.def_id == variant_def_id)
                                        .cloned()
                                        .ok_or_else(|| Box::new(TypeError::new(
                                            TypeErrorKind::NotFound { name: format!("variant {} in enum", path_str) },
                                            pattern.span,
                                        )))?;

                                    (enum_def_id, variant_info)
                                } else {
                                    return Err(Box::new(TypeError::new(
                                        TypeErrorKind::NotFound { name: path_str },
                                        pattern.span,
                                    )));
                                }
                            } else {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::NotFound { name: path_str },
                                    pattern.span,
                                )));
                            }
                        }
                        _ => {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::NotFound { name: path_str },
                                pattern.span,
                            )));
                        }
                    }
                } else if path.segments.len() == 3 {
                    // Three-segment path: module::EnumName::VariantName
                    let module_name = self.symbol_to_string(path.segments[0].name.node);
                    let enum_name = self.symbol_to_string(path.segments[1].name.node);
                    let variant_name = self.symbol_to_string(path.segments[2].name.node);

                    // Find the module
                    let mut module_def_id: Option<DefId> = None;
                    for (def_id, info) in &self.module_defs {
                        if info.name == module_name {
                            module_def_id = Some(*def_id);
                            break;
                        }
                    }

                    let mod_def_id = module_def_id.ok_or_else(|| Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("module '{}'", module_name) },
                        pattern.span,
                    )))?;

                    // Find the enum within the module's items OR reexports
                    let mut found_enum: Option<(DefId, super::EnumInfo)> = None;
                    if let Some(mod_info) = self.module_defs.get(&mod_def_id) {
                        // Check direct items first
                        for &item_def_id in &mod_info.items {
                            if let Some(enum_info) = self.enum_defs.get(&item_def_id).cloned() {
                                if enum_info.name == enum_name {
                                    found_enum = Some((item_def_id, enum_info));
                                    break;
                                }
                            }
                        }
                        // If not found in items, check reexports (from `pub use`)
                        if found_enum.is_none() {
                            for (reexport_name, reexport_def_id, _vis) in &mod_info.reexports {
                                if reexport_name == &enum_name {
                                    if let Some(enum_info) = self.enum_defs.get(reexport_def_id).cloned() {
                                        found_enum = Some((*reexport_def_id, enum_info));
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    let (enum_def_id, enum_info) = found_enum.ok_or_else(|| Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", module_name, enum_name) },
                        pattern.span,
                    )))?;

                    // Find the variant
                    let variant = enum_info.variants.iter()
                        .find(|v| v.name == variant_name)
                        .cloned()
                        .ok_or_else(|| Box::new(TypeError::new(
                            TypeErrorKind::NotFound { name: format!("{}::{}::{}", module_name, enum_name, variant_name) },
                            pattern.span,
                        )))?;

                    (enum_def_id, variant)
                } else {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{path:?}") },
                        pattern.span,
                    )));
                };

                // Get the enum info to access generics for substitution
                let enum_info = self.enum_defs.get(&enum_def_id).cloned().ok_or_else(|| Box::new(TypeError::new(
                    TypeErrorKind::NotFound { name: format!("enum {:?}", enum_def_id) },
                    pattern.span,
                )))?;

                // Build substitution map from generic params to concrete types from expected_ty
                let subst: std::collections::HashMap<TyVarId, Type> = if let TypeKind::Adt { args, .. } = expected_ty.kind() {
                    enum_info.generics.iter()
                        .zip(args.iter())
                        .map(|(&tyvar, ty)| (tyvar, ty.clone()))
                        .collect()
                } else {
                    std::collections::HashMap::new()
                };

                // Substitute type parameters in field types
                let variant_field_types: Vec<Type> = variant_info.fields.iter()
                    .map(|f| self.substitute_type_vars(&f.ty, &subst))
                    .collect();

                if fields.len() != variant_field_types.len() {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::WrongArity {
                            expected: variant_field_types.len(),
                            found: fields.len(),
                        },
                        pattern.span,
                    )));
                }

                let mut hir_fields = Vec::new();
                for (field, field_ty) in fields.iter().zip(variant_field_types.iter()) {
                    hir_fields.push(self.lower_pattern(field, field_ty)?);
                }

                hir::PatternKind::Variant {
                    def_id: variant_info.def_id,
                    variant_idx: variant_info.index,
                    fields: hir_fields,
                }
            }
            ast::PatternKind::Rest => {
                // Standalone rest pattern (..) matches anything and is irrefutable.
                // This is valid in Rust: `match x { .. => () }` matches any value.
                // We lower it to a wildcard pattern in HIR.
                hir::PatternKind::Wildcard
            }
            ast::PatternKind::Ref { mutable, inner } => {
                match expected_ty.kind() {
                    TypeKind::Ref { inner: inner_ty, mutable: ty_mutable } => {
                        if *mutable && !ty_mutable {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::PatternMismatch {
                                    expected: expected_ty.clone(),
                                    pattern: "&mut pattern but type is &".to_string(),
                                },
                                pattern.span,
                            )));
                        }
                        let inner_pat = self.lower_pattern(inner, inner_ty)?;
                        hir::PatternKind::Ref {
                            mutable: *mutable,
                            inner: Box::new(inner_pat),
                        }
                    }
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::PatternMismatch {
                                expected: expected_ty.clone(),
                                pattern: "reference pattern".to_string(),
                            },
                            pattern.span,
                        )));
                    }
                }
            }
            ast::PatternKind::Struct { path, fields, rest } => {
                let (adt_def_id, type_args) = match expected_ty.kind() {
                    TypeKind::Adt { def_id, args, .. } => (*def_id, args.clone()),
                    _ => {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotAStruct { ty: expected_ty.clone() },
                            pattern.span,
                        )));
                    }
                };

                // Check if this is an enum variant with struct-like syntax
                // e.g., Status::Active { value } where Status is an enum
                if let Some(enum_info) = self.enum_defs.get(&adt_def_id).cloned() {
                    // This is an enum - extract variant name from path
                    // The variant name is always the last segment
                    let variant_name = if path.segments.len() == 3 {
                        // module::EnumName::VariantName { ... }
                        self.symbol_to_string(path.segments[2].name.node)
                    } else if path.segments.len() == 2 {
                        // EnumName::VariantName { ... }
                        self.symbol_to_string(path.segments[1].name.node)
                    } else if path.segments.len() == 1 {
                        // Just VariantName { ... } (use expected type context)
                        self.symbol_to_string(path.segments[0].name.node)
                    } else {
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::NotFound { name: format!("invalid enum variant path: {path:?}") },
                            pattern.span,
                        )));
                    };

                    let variant_info = enum_info.variants.iter()
                        .find(|v| v.name == variant_name)
                        .cloned()
                        .ok_or_else(|| Box::new(TypeError::new(
                            TypeErrorKind::NotFound { name: format!("{}::{}", enum_info.name, variant_name) },
                            pattern.span,
                        )))?;

                    // Build substitution map from generic params to concrete types
                    let subst: std::collections::HashMap<TyVarId, Type> = enum_info.generics.iter()
                        .zip(type_args.iter())
                        .map(|(&tyvar, ty)| (tyvar, ty.clone()))
                        .collect();

                    // Collect fields with indices for proper ordering
                    let mut indexed_fields: Vec<(usize, hir::Pattern)> = Vec::new();
                    let mut bound_fields = HashSet::new();

                    for field_pattern in fields {
                        let field_name = self.symbol_to_string(field_pattern.name.node);

                        let field_info = variant_info.fields.iter()
                            .find(|f| f.name == field_name)
                            .ok_or_else(|| Box::new(TypeError::new(
                                TypeErrorKind::NoField {
                                    ty: expected_ty.clone(),
                                    field: field_name.clone(),
                                },
                                field_pattern.span,
                            )))?;

                        bound_fields.insert(field_name.clone());

                        let field_ty = self.substitute_type_vars(&field_info.ty, &subst);

                        let inner_pattern = if let Some(ref inner) = field_pattern.pattern {
                            self.lower_pattern(inner, &field_ty)?
                        } else {
                            let local_id = self.resolver.define_local(
                                field_name.clone(),
                                field_ty.clone(),
                                false,
                                field_pattern.span,
                            )?;
                            self.locals.push(hir::Local {
                                id: local_id,
                                name: Some(field_name),
                                ty: field_ty.clone(),
                                mutable: false,
                                span: field_pattern.span,
                            });
                            hir::Pattern {
                                kind: hir::PatternKind::Binding {
                                    local_id,
                                    mutable: false,
                                    by_ref: false,
                                    subpattern: None,
                                },
                                ty: field_ty,
                                span: field_pattern.span,
                            }
                        };

                        indexed_fields.push((field_info.index as usize, inner_pattern));
                    }

                    // Check all fields are provided unless `..` is used
                    if !*rest {
                        for field_info in &variant_info.fields {
                            if !bound_fields.contains(&field_info.name) {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::MissingField {
                                        ty: expected_ty.clone(),
                                        field: field_info.name.clone(),
                                    },
                                    pattern.span,
                                )));
                            }
                        }
                    }

                    // Sort fields by index
                    indexed_fields.sort_by_key(|(idx, _)| *idx);
                    let ordered_fields: Vec<hir::Pattern> = indexed_fields.into_iter()
                        .map(|(_, pat)| pat)
                        .collect();

                    return Ok(hir::Pattern {
                        kind: hir::PatternKind::Variant {
                            def_id: variant_info.def_id,
                            variant_idx: variant_info.index,
                            fields: ordered_fields,
                        },
                        ty: expected_ty.clone(),
                        span: pattern.span,
                    });
                }

                // Not an enum - handle as struct pattern
                let struct_def_id = adt_def_id;

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
                        .ok_or_else(|| Box::new(TypeError::new(
                            TypeErrorKind::NoField {
                                ty: expected_ty.clone(),
                                field: field_name.clone(),
                            },
                            field_pattern.span,
                        )))?;

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
                                by_ref: false,
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
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::MissingField {
                                    ty: expected_ty.clone(),
                                    field: field_info.name.clone(),
                                },
                                pattern.span,
                            )));
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
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::PatternMismatch {
                                expected: expected_ty.clone(),
                                pattern: "slice pattern".to_string(),
                            },
                            pattern.span,
                        )));
                    }
                };

                // Helper to check if a pattern is a binding rest pattern (x @ ..)
                fn is_binding_rest_pattern(pat: &ast::Pattern) -> bool {
                    matches!(
                        &pat.kind,
                        ast::PatternKind::Ident { subpattern: Some(sub), .. }
                            if matches!(sub.kind, ast::PatternKind::Rest)
                    )
                }

                // Find binding rest pattern (x @ ..) in elements if rest_pos is None
                let binding_rest_idx = if rest_pos.is_none() {
                    elements.iter().position(is_binding_rest_pattern)
                } else {
                    None
                };

                let (prefix_pats, rest_pattern, suffix_pats) = if let Some(rest_idx) = rest_pos {
                    let rest_idx = *rest_idx;
                    // Parser stores elements WITHOUT the Rest pattern itself.
                    // rest_idx is the index where rest pattern appears (before suffix elements).
                    // So prefix = elements[..rest_idx], suffix = elements[rest_idx..]
                    let prefix: Vec<_> = elements.iter().take(rest_idx).cloned().collect();
                    let suffix: Vec<_> = elements.iter().skip(rest_idx).cloned().collect();
                    // Rest pattern always exists when rest_pos is Some
                    let rest_pat = Some(Box::new(hir::Pattern {
                        kind: hir::PatternKind::Wildcard,
                        ty: Type::slice(elem_ty.clone()),
                        span: pattern.span,
                    }));
                    (prefix, rest_pat, suffix)
                } else if let Some(bind_idx) = binding_rest_idx {
                    // Handle binding rest pattern: [first, middle @ .., last]
                    let prefix: Vec<_> = elements.iter().take(bind_idx).cloned().collect();
                    let suffix: Vec<_> = elements.iter().skip(bind_idx + 1).cloned().collect();

                    // Extract the binding information from the ident @ .. pattern
                    let binding_pat = &elements[bind_idx];
                    let rest_pat = if let ast::PatternKind::Ident { name, mutable, by_ref, .. } = &binding_pat.kind {
                        // Create a binding pattern for the rest slice
                        // The base type is a slice of the element type
                        let base_slice_ty = Type::slice(elem_ty.clone());

                        // If by_ref is true (ref rest @ ..), the binding's type is a reference
                        // to the slice rather than the slice itself
                        let binding_ty = if *by_ref {
                            Type::reference(base_slice_ty, *mutable)
                        } else {
                            base_slice_ty
                        };

                        let name_str = self.symbol_to_string(name.node);

                        // Define the local variable for the binding
                        let local_id = self.resolver.define_local(
                            name_str.clone(),
                            binding_ty.clone(),
                            *mutable,
                            binding_pat.span,
                        )?;

                        self.locals.push(hir::Local {
                            id: local_id,
                            ty: binding_ty.clone(),
                            mutable: *mutable,
                            name: Some(name_str),
                            span: binding_pat.span,
                        });

                        // Create the HIR binding pattern
                        Some(Box::new(hir::Pattern {
                            kind: hir::PatternKind::Binding {
                                local_id,
                                mutable: *mutable,
                                by_ref: *by_ref,
                                subpattern: None,
                            },
                            ty: binding_ty,
                            span: binding_pat.span,
                        }))
                    } else {
                        // Should not happen due to is_binding_rest_pattern check
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
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::PatternMismatch {
                            expected: expected_ty.clone(),
                            pattern: "empty or pattern".to_string(),
                        },
                        pattern.span,
                    )));
                }

                // [P-Or]: Validate all alternatives bind the same variable names.
                let first_bindings = Self::collect_pattern_bindings(&alternatives[0]);
                for (i, alt) in alternatives.iter().enumerate().skip(1) {
                    let alt_bindings = Self::collect_pattern_bindings(alt);
                    if alt_bindings != first_bindings {
                        let resolve = |idx: &u32| -> String {
                            let sym = ast::Symbol::try_from_usize(*idx as usize)
                                .unwrap_or_else(|| ast::Symbol::try_from_usize(0).unwrap());
                            self.symbol_to_string(sym)
                        };
                        let missing: Vec<_> = first_bindings.difference(&alt_bindings)
                            .map(resolve).collect();
                        let extra: Vec<_> = alt_bindings.difference(&first_bindings)
                            .map(resolve).collect();
                        let mut msg = format!("or-pattern alternative {} binds different variables", i + 1);
                        if !missing.is_empty() {
                            msg.push_str(&format!("; missing: {}", missing.join(", ")));
                        }
                        if !extra.is_empty() {
                            msg.push_str(&format!("; extra: {}", extra.join(", ")));
                        }
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::PatternMismatch {
                                expected: expected_ty.clone(),
                                pattern: msg,
                            },
                            alt.span,
                        )));
                    }
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
                        return Err(Box::new(TypeError::new(
                            TypeErrorKind::PatternMismatch {
                                expected: expected_ty.clone(),
                                pattern: "range pattern (requires integer or char type)".to_string(),
                            },
                            pattern.span,
                        )));
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
                // Handle multi-segment paths like Color::Red (enum variants)
                if path.segments.len() == 2 {
                    // Two-segment path: EnumName::VariantName
                    let enum_name = self.symbol_to_string(path.segments[0].name.node);
                    let variant_name = self.symbol_to_string(path.segments[1].name.node);

                    // Look up the enum type
                    match self.resolver.lookup(&enum_name) {
                        Some(Binding::Def(def_id)) => {
                            // Find the enum info and variant
                            if let Some(enum_info) = self.enum_defs.get(&def_id) {
                                if let Some(variant) = enum_info.variants.iter().find(|v| v.name == variant_name) {
                                    hir::PatternKind::Variant {
                                        def_id: variant.def_id,
                                        variant_idx: variant.index,
                                        fields: vec![],
                                    }
                                } else {
                                    return Err(Box::new(TypeError::new(
                                        TypeErrorKind::NotFound { name: format!("{}::{}", enum_name, variant_name) },
                                        pattern.span,
                                    )));
                                }
                            } else {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::NotFound { name: enum_name },
                                    pattern.span,
                                )));
                            }
                        }
                        _ => {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::NotFound { name: enum_name },
                                pattern.span,
                            )));
                        }
                    }
                } else if path.segments.len() == 1 {
                    // Single-segment path: direct variant reference (e.g., None)
                    let path_str = self.symbol_to_string(path.segments[0].name.node);

                    match self.resolver.lookup(&path_str) {
                        Some(Binding::Def(def_id)) => {
                            if let Some(info) = self.resolver.def_info.get(&def_id) {
                                if info.kind == hir::DefKind::Variant {
                                    // Find the variant info to get the correct index
                                    let variant_idx = if let Some(parent_id) = info.parent {
                                        if let Some(enum_info) = self.enum_defs.get(&parent_id) {
                                            enum_info.variants.iter()
                                                .find(|v| v.def_id == def_id)
                                                .map(|v| v.index)
                                                .unwrap_or(0)
                                        } else {
                                            0
                                        }
                                    } else {
                                        0
                                    };

                                    hir::PatternKind::Variant {
                                        def_id,
                                        variant_idx,
                                        fields: vec![],
                                    }
                                } else {
                                    return Err(Box::new(TypeError::new(
                                        TypeErrorKind::NotFound { name: path_str },
                                        pattern.span,
                                    )));
                                }
                            } else {
                                return Err(Box::new(TypeError::new(
                                    TypeErrorKind::NotFound { name: path_str },
                                    pattern.span,
                                )));
                            }
                        }
                        _ => {
                            return Err(Box::new(TypeError::new(
                                TypeErrorKind::NotFound { name: path_str },
                                pattern.span,
                            )));
                        }
                    }
                } else if path.segments.len() == 3 {
                    // Three-segment path: module::EnumName::VariantName
                    let module_name = self.symbol_to_string(path.segments[0].name.node);
                    let enum_name = self.symbol_to_string(path.segments[1].name.node);
                    let variant_name = self.symbol_to_string(path.segments[2].name.node);

                    // Find the module
                    let mut module_def_id: Option<DefId> = None;
                    for (def_id, info) in &self.module_defs {
                        if info.name == module_name {
                            module_def_id = Some(*def_id);
                            break;
                        }
                    }

                    let mod_def_id = module_def_id.ok_or_else(|| Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("module '{}'", module_name) },
                        pattern.span,
                    )))?;

                    // Find the enum within the module's items OR reexports
                    let mut found_enum: Option<(DefId, super::EnumInfo)> = None;
                    if let Some(mod_info) = self.module_defs.get(&mod_def_id) {
                        // Check direct items first
                        for &item_def_id in &mod_info.items {
                            if let Some(enum_info) = self.enum_defs.get(&item_def_id).cloned() {
                                if enum_info.name == enum_name {
                                    found_enum = Some((item_def_id, enum_info));
                                    break;
                                }
                            }
                        }
                        // If not found in items, check reexports (from `pub use`)
                        if found_enum.is_none() {
                            for (reexport_name, reexport_def_id, _vis) in &mod_info.reexports {
                                if reexport_name == &enum_name {
                                    if let Some(enum_info) = self.enum_defs.get(reexport_def_id).cloned() {
                                        found_enum = Some((*reexport_def_id, enum_info));
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    let (_enum_def_id, enum_info) = found_enum.ok_or_else(|| Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{}::{}", module_name, enum_name) },
                        pattern.span,
                    )))?;

                    // Find the variant
                    let variant = enum_info.variants.iter()
                        .find(|v| v.name == variant_name)
                        .ok_or_else(|| Box::new(TypeError::new(
                            TypeErrorKind::NotFound { name: format!("{}::{}::{}", module_name, enum_name, variant_name) },
                            pattern.span,
                        )))?;

                    hir::PatternKind::Variant {
                        def_id: variant.def_id,
                        variant_idx: variant.index,
                        fields: vec![],
                    }
                } else {
                    return Err(Box::new(TypeError::new(
                        TypeErrorKind::NotFound { name: format!("{:?}", path) },
                        pattern.span,
                    )));
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

    /// Collect the set of variable names bound by an AST pattern.
    /// Used by [P-Or] validation to ensure all or-pattern alternatives
    /// bind the same variables. Uses Symbol indices for comparison; the
    /// or-pattern error message is constructed with indices since we don't
    /// have access to the string interner in a static context.
    fn collect_pattern_bindings(pattern: &ast::Pattern) -> HashSet<u32> {
        let mut names = HashSet::new();
        Self::collect_bindings_recursive(pattern, &mut names);
        names
    }

    fn collect_bindings_recursive(pattern: &ast::Pattern, names: &mut HashSet<u32>) {
        match &pattern.kind {
            ast::PatternKind::Ident { name, .. } => {
                names.insert(name.node.to_usize() as u32);
            }
            ast::PatternKind::Tuple { fields, .. } => {
                for f in fields {
                    Self::collect_bindings_recursive(f, names);
                }
            }
            ast::PatternKind::Struct { fields, .. } => {
                for f in fields {
                    if let Some(ref pat) = f.pattern {
                        Self::collect_bindings_recursive(pat, names);
                    } else {
                        // Shorthand: `Foo { x }` — the field name is the binding
                        names.insert(f.name.node.to_usize() as u32);
                    }
                }
            }
            ast::PatternKind::TupleStruct { fields, .. } => {
                for f in fields {
                    Self::collect_bindings_recursive(f, names);
                }
            }
            ast::PatternKind::Or(alternatives) => {
                // Nested or — collect from first (all should be same)
                if let Some(first) = alternatives.first() {
                    Self::collect_bindings_recursive(first, names);
                }
            }
            ast::PatternKind::Ref { inner, .. }
            | ast::PatternKind::Paren(inner) => {
                Self::collect_bindings_recursive(inner, names);
            }
            ast::PatternKind::Slice { elements, .. } => {
                for p in elements {
                    Self::collect_bindings_recursive(p, names);
                }
            }
            // These patterns don't bind names
            ast::PatternKind::Wildcard
            | ast::PatternKind::Literal(_)
            | ast::PatternKind::Rest
            | ast::PatternKind::Range { .. }
            | ast::PatternKind::Path(_) => {}
        }
    }
}
