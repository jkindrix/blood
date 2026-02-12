//! Suggestion helpers for TypeContext.
//!
//! This module provides methods for generating "did you mean?" suggestions
//! when names or types cannot be resolved.

use crate::hir::Type;
use crate::span::Span;

use super::TypeContext;
use super::super::error::{TypeError, TypeErrorKind};
use super::super::suggestion::{suggest_similar, format_suggestions};

impl<'a> TypeContext<'a> {
    /// Create a "name not found" error with suggestions.
    ///
    /// Searches for similar names in the current scope and adds a
    /// "did you mean?" suggestion if close matches are found.
    pub(crate) fn error_name_not_found(&self, name: &str, span: Span) -> Box<TypeError> {
        let visible_names = self.resolver.collect_visible_names();
        let suggestions = suggest_similar(name, visible_names.iter().map(|s| s.as_str()));

        let mut error = TypeError::new(
            TypeErrorKind::NotFound { name: name.to_string() },
            span,
        );

        if let Some(help) = format_suggestions(&suggestions) {
            error = error.with_help(help);
        }

        Box::new(error)
    }

    /// Create a "type not found" error with suggestions.
    ///
    /// Searches for similar type names in the current scope and adds a
    /// "did you mean?" suggestion if close matches are found.
    pub(crate) fn error_type_not_found(&self, name: &str, span: Span) -> Box<TypeError> {
        let visible_types = self.resolver.collect_visible_type_names();
        let suggestions = suggest_similar(name, visible_types.iter().map(|s| s.as_str()));

        let mut error = TypeError::new(
            TypeErrorKind::TypeNotFound { name: name.to_string() },
            span,
        );

        if let Some(help) = format_suggestions(&suggestions) {
            error = error.with_help(help);
        }

        Box::new(error)
    }

    /// Create an "unknown field" error with suggestions.
    ///
    /// Searches for similar field names on the struct type and adds a
    /// "did you mean?" suggestion if close matches are found.
    pub(crate) fn error_unknown_field(&self, ty: &Type, field: &str, span: Span) -> Box<TypeError> {
        let field_names = self.collect_field_names(ty);
        let suggestions = suggest_similar(field, field_names.iter().map(|s| s.as_str()));

        let mut error = TypeError::new(
            TypeErrorKind::UnknownField {
                ty: ty.clone(),
                field: field.to_string(),
            },
            span,
        );

        if let Some(help) = format_suggestions(&suggestions) {
            error = error.with_help(help);
        }

        Box::new(error)
    }

    /// Create a "method not found" error with suggestions.
    ///
    /// Searches for similar method names on the type and adds a
    /// "did you mean?" suggestion if close matches are found.
    pub(crate) fn error_method_not_found(&self, ty: &Type, method: &str, span: Span) -> Box<TypeError> {
        let method_names = self.collect_method_names(ty);
        let suggestions = suggest_similar(method, method_names.iter().map(|s| s.as_str()));

        let mut error = TypeError::new(
            TypeErrorKind::MethodNotFound {
                ty: ty.clone(),
                method: method.to_string(),
            },
            span,
        );

        if let Some(help) = format_suggestions(&suggestions) {
            error = error.with_help(help);
        }

        Box::new(error)
    }

    /// Collect field names for a given type.
    fn collect_field_names(&self, ty: &Type) -> Vec<String> {
        use crate::hir::TypeKind;

        match ty.kind() {
            TypeKind::Adt { def_id, .. } => {
                if let Some(struct_info) = self.struct_defs.get(def_id) {
                    struct_info.fields.iter()
                        .map(|f| f.name.clone())
                        .collect()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Collect method names for a given type.
    fn collect_method_names(&self, ty: &Type) -> Vec<String> {
        use crate::hir::TypeKind;

        let mut method_names = Vec::new();

        // Collect methods from impl blocks
        for impl_block in &self.impl_blocks {
            if self.types_match_for_impl(&impl_block.self_ty, ty) {
                for method in &impl_block.methods {
                    if !method_names.contains(&method.name) {
                        method_names.push(method.name.clone());
                    }
                }
            }
        }

        // For type parameters, collect methods from trait bounds
        if let TypeKind::Param(ty_var_id) = ty.kind() {
            if let Some(bounds) = self.type_param_bounds.get(ty_var_id) {
                for &trait_def_id in bounds {
                    if let Some(trait_info) = self.trait_defs.get(&trait_def_id) {
                        for method in &trait_info.methods {
                            if !method_names.contains(&method.name) {
                                method_names.push(method.name.clone());
                            }
                        }
                    }
                }
            }
        }

        method_names
    }
}
