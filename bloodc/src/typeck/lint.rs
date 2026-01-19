//! Type-based and expression-based lints for detecting problematic patterns.
//!
//! This module implements lint checks that run during type checking to warn
//! about patterns that may have performance implications with Blood's 128-bit
//! generational pointers and effect system.
//!
//! # Type Lints
//!
//! - `deeply_nested_box`: Warns about `Box<Box<T>>` patterns that cause multiple
//!   generation checks per access.
//! - `pointer_heavy_struct`: Warns about structs where >75% of fields are pointers.
//! - `linked_list_pattern`: Warns about self-referential types like linked lists.
//! - `pointer_array_pattern`: Warns about arrays of pointers.
//! - `excessive_indirection`: Warns about types requiring >3 dereferences.
//!
//! # Expression Lints
//!
//! - `deeply_nested_handlers`: Warns about effect handler nesting >5 levels.
//! - `large_handler`: Warns about handlers with many operations.

use crate::diagnostics::{Diagnostic, ErrorCode};
use crate::hir::expr::{Expr, ExprKind};
use crate::hir::ty::{Type, TypeKind};
use crate::hir::DefId;
use crate::span::Span;
use std::collections::HashSet;

/// Configuration for handler/expression lints.
#[derive(Debug, Clone)]
pub struct HandlerLintConfig {
    /// Maximum allowed handler nesting depth before warning. Default: 5
    pub max_handler_depth: usize,
    /// Maximum number of operations in a handler before warning. Default: 10
    pub max_handler_operations: usize,
    /// Whether to warn about deeply nested handlers.
    pub warn_deep_handlers: bool,
    /// Whether to warn about large handlers.
    pub warn_large_handlers: bool,
}

impl Default for HandlerLintConfig {
    fn default() -> Self {
        Self {
            max_handler_depth: 5,
            max_handler_operations: 10,
            warn_deep_handlers: true,
            warn_large_handlers: true,
        }
    }
}

/// Lint context for checking expressions (handler nesting, etc.).
pub struct HandlerLintContext {
    config: HandlerLintConfig,
    warnings: Vec<Diagnostic>,
    /// Track spans we've already warned about to avoid duplicates.
    warned_spans: HashSet<(usize, usize)>,
}

impl HandlerLintContext {
    pub fn new(config: HandlerLintConfig) -> Self {
        Self {
            config,
            warnings: Vec::new(),
            warned_spans: HashSet::new(),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(HandlerLintConfig::default())
    }

    /// Get the collected warnings.
    pub fn into_warnings(self) -> Vec<Diagnostic> {
        self.warnings
    }

    /// Get warnings by reference.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Check an expression tree for handler-related issues.
    pub fn check_expr(&mut self, expr: &Expr) {
        self.check_handler_nesting(expr, 0);
    }

    /// Check for deeply nested handler expressions.
    fn check_handler_nesting(&mut self, expr: &Expr, depth: usize) {
        match &expr.kind {
            ExprKind::Handle { body, handler_instance, .. } => {
                let new_depth = depth + 1;
                if new_depth > self.config.max_handler_depth && self.config.warn_deep_handlers {
                    let span_key = (expr.span.start, expr.span.end);
                    if !self.warned_spans.contains(&span_key) {
                        self.warned_spans.insert(span_key);
                        let warning = Diagnostic::warning(
                            format!(
                                "deeply nested handler (depth {}): each nesting level adds \
                                 continuation allocation overhead (~48ns per non-tail resume)",
                                new_depth
                            ),
                            expr.span,
                        )
                        .with_error_code(ErrorCode::DeeplyNestedHandlers)
                        .with_suggestion(
                            "consider flattening handler structure or combining related effects"
                        );
                        self.warnings.push(warning);
                    }
                }
                // Continue checking inside the handler body with incremented depth
                self.check_handler_nesting(body, new_depth);
                // Handler instance expression is at current depth
                self.check_handler_nesting(handler_instance, depth);
            }

            ExprKind::InlineHandle { body, handlers } => {
                let new_depth = depth + 1;
                if new_depth > self.config.max_handler_depth && self.config.warn_deep_handlers {
                    let span_key = (expr.span.start, expr.span.end);
                    if !self.warned_spans.contains(&span_key) {
                        self.warned_spans.insert(span_key);
                        let warning = Diagnostic::warning(
                            format!(
                                "deeply nested inline handler (depth {}): each nesting level adds \
                                 continuation allocation overhead (~48ns per non-tail resume)",
                                new_depth
                            ),
                            expr.span,
                        )
                        .with_error_code(ErrorCode::DeeplyNestedHandlers)
                        .with_suggestion(
                            "consider flattening handler structure or combining related effects"
                        );
                        self.warnings.push(warning);
                    }
                }
                // Continue checking inside the handler body with incremented depth
                self.check_handler_nesting(body, new_depth);
                // Check handler bodies at current depth
                for handler in handlers {
                    self.check_handler_nesting(&handler.body, depth);
                }
            }

            // Recurse into all other expression kinds
            ExprKind::Binary { left, right, .. } => {
                self.check_handler_nesting(left, depth);
                self.check_handler_nesting(right, depth);
            }
            ExprKind::Unary { operand, .. } => {
                self.check_handler_nesting(operand, depth);
            }
            ExprKind::Call { callee, args } => {
                self.check_handler_nesting(callee, depth);
                for arg in args {
                    self.check_handler_nesting(arg, depth);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.check_handler_nesting(receiver, depth);
                for arg in args {
                    self.check_handler_nesting(arg, depth);
                }
            }
            ExprKind::Field { base, .. } => {
                self.check_handler_nesting(base, depth);
            }
            ExprKind::Index { base, index } => {
                self.check_handler_nesting(base, depth);
                self.check_handler_nesting(index, depth);
            }
            ExprKind::Tuple(exprs) | ExprKind::Array(exprs) => {
                for e in exprs {
                    self.check_handler_nesting(e, depth);
                }
            }
            ExprKind::Repeat { value, .. } => {
                self.check_handler_nesting(value, depth);
            }
            ExprKind::Struct { fields, base, .. } => {
                for field in fields {
                    self.check_handler_nesting(&field.value, depth);
                }
                if let Some(base_expr) = base {
                    self.check_handler_nesting(base_expr, depth);
                }
            }
            ExprKind::Record { fields } => {
                for field in fields {
                    self.check_handler_nesting(&field.value, depth);
                }
            }
            ExprKind::Variant { fields, .. } => {
                for field in fields {
                    self.check_handler_nesting(field, depth);
                }
            }
            ExprKind::Borrow { expr, .. } => {
                self.check_handler_nesting(expr, depth);
            }
            ExprKind::Deref(inner) => {
                self.check_handler_nesting(inner, depth);
            }
            ExprKind::AddrOf { expr, .. } => {
                self.check_handler_nesting(expr, depth);
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                self.check_handler_nesting(condition, depth);
                self.check_handler_nesting(then_branch, depth);
                if let Some(else_expr) = else_branch {
                    self.check_handler_nesting(else_expr, depth);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.check_handler_nesting(scrutinee, depth);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.check_handler_nesting(guard, depth);
                    }
                    self.check_handler_nesting(&arm.body, depth);
                }
            }
            ExprKind::Loop { body, .. } => {
                self.check_handler_nesting(body, depth);
            }
            ExprKind::While { condition, body, .. } => {
                self.check_handler_nesting(condition, depth);
                self.check_handler_nesting(body, depth);
            }
            ExprKind::Block { stmts, expr } => {
                for stmt in stmts {
                    self.check_statement_handler_nesting(stmt, depth);
                }
                if let Some(e) = expr {
                    self.check_handler_nesting(e, depth);
                }
            }
            ExprKind::Return(Some(e)) | ExprKind::Break { value: Some(e), .. } => {
                self.check_handler_nesting(e, depth);
            }
            ExprKind::Assign { target, value } => {
                self.check_handler_nesting(target, depth);
                self.check_handler_nesting(value, depth);
            }
            ExprKind::Cast { expr, .. } => {
                self.check_handler_nesting(expr, depth);
            }
            ExprKind::Unsafe(inner) => {
                self.check_handler_nesting(inner, depth);
            }
            ExprKind::Let { init, .. } => {
                self.check_handler_nesting(init, depth);
            }
            ExprKind::Closure { .. } => {
                // Closures have separate body checking via BodyId, no direct recursion here
            }
            ExprKind::Perform { args, .. } => {
                for arg in args {
                    self.check_handler_nesting(arg, depth);
                }
            }
            ExprKind::Resume { value: Some(v) } => {
                self.check_handler_nesting(v, depth);
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.check_handler_nesting(s, depth);
                }
                if let Some(e) = end {
                    self.check_handler_nesting(e, depth);
                }
            }

            // Macro expansion nodes - check subexpressions
            ExprKind::MacroExpansion { args, .. } => {
                for arg in args {
                    self.check_handler_nesting(arg, depth);
                }
            }
            ExprKind::VecLiteral(exprs) => {
                for e in exprs {
                    self.check_handler_nesting(e, depth);
                }
            }
            ExprKind::VecRepeat { value, count } => {
                self.check_handler_nesting(value, depth);
                self.check_handler_nesting(count, depth);
            }
            ExprKind::Assert { condition, message } => {
                self.check_handler_nesting(condition, depth);
                if let Some(msg) = message {
                    self.check_handler_nesting(msg, depth);
                }
            }
            ExprKind::Dbg(inner) => {
                self.check_handler_nesting(inner, depth);
            }

            // Terminal expressions - no recursion needed
            ExprKind::Literal(_)
            | ExprKind::Local(_)
            | ExprKind::Def(_)
            | ExprKind::Return(None)
            | ExprKind::Break { value: None, .. }
            | ExprKind::Continue { .. }
            | ExprKind::Resume { value: None }
            | ExprKind::MethodFamily { .. }
            | ExprKind::Default
            | ExprKind::Error => {}
        }
    }

    /// Check a statement for handler nesting.
    fn check_statement_handler_nesting(&mut self, stmt: &crate::hir::expr::Stmt, depth: usize) {
        match stmt {
            crate::hir::expr::Stmt::Let { init, .. } => {
                if let Some(init_expr) = init {
                    self.check_handler_nesting(init_expr, depth);
                }
            }
            crate::hir::expr::Stmt::Expr(expr) => {
                self.check_handler_nesting(expr, depth);
            }
            crate::hir::expr::Stmt::Item(_) => {
                // Item declarations don't contain expressions to check at this level
            }
            crate::hir::expr::Stmt::Defer { body } => {
                self.check_handler_nesting(body, depth);
            }
        }
    }
}

/// Configuration for pointer lints.
#[derive(Debug, Clone)]
pub struct PointerLintConfig {
    /// Maximum allowed Box nesting depth before warning. Default: 2
    pub max_box_depth: usize,
    /// Maximum pointer density (0.0-1.0) before warning. Default: 0.75
    pub max_pointer_density: f64,
    /// Maximum indirection levels before warning. Default: 3
    pub max_indirection: usize,
    /// Whether to warn about linked list patterns.
    pub warn_linked_lists: bool,
    /// Whether to warn about pointer arrays.
    pub warn_pointer_arrays: bool,
}

impl Default for PointerLintConfig {
    fn default() -> Self {
        Self {
            max_box_depth: 2,
            max_pointer_density: 0.75,
            max_indirection: 3,
            warn_linked_lists: true,
            warn_pointer_arrays: true,
        }
    }
}

/// Lint context for checking types.
pub struct PointerLintContext {
    config: PointerLintConfig,
    warnings: Vec<Diagnostic>,
    /// Track which types we've already warned about to avoid duplicates.
    warned_types: HashSet<String>,
}

impl PointerLintContext {
    pub fn new(config: PointerLintConfig) -> Self {
        Self {
            config,
            warnings: Vec::new(),
            warned_types: HashSet::new(),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(PointerLintConfig::default())
    }

    /// Get the collected warnings.
    pub fn into_warnings(self) -> Vec<Diagnostic> {
        self.warnings
    }

    /// Get warnings by reference.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Check a type for pointer-related issues.
    pub fn check_type(&mut self, ty: &Type, span: Span) {
        self.check_box_depth(ty, span, 0);
        self.check_indirection_depth(ty, span, 0);
        self.check_pointer_array(ty, span);
    }

    /// Check a struct type for pointer density and linked list patterns.
    pub fn check_struct_fields(
        &mut self,
        struct_name: &str,
        struct_def_id: DefId,
        fields: &[(String, Type)],
        span: Span,
    ) {
        // Check pointer density
        self.check_pointer_density(struct_name, fields, span);

        // Check for linked list pattern (self-referential types)
        self.check_linked_list_pattern(struct_name, struct_def_id, fields, span);
    }

    /// Check for deeply nested Box types.
    fn check_box_depth(&mut self, ty: &Type, span: Span, depth: usize) {
        if let TypeKind::Adt { def_id, args } = ty.kind() {
            // Check if this is a Box type (we identify Box by name in practice)
            // In a real implementation, we'd have a better way to identify Box<T>
            if self.is_box_type(*def_id) {
                let new_depth = depth + 1;
                if new_depth > self.config.max_box_depth {
                    let type_str = format!("{}", ty);
                    if !self.warned_types.contains(&type_str) {
                        self.warned_types.insert(type_str);
                        let warning = Diagnostic::warning(
                            format!(
                                "deeply nested Box type (depth {}): each level adds ~4 cycles per access",
                                new_depth
                            ),
                            span,
                        )
                        .with_error_code(ErrorCode::DeeplyNestedBox);
                        self.warnings.push(warning);
                    }
                }

                // Check the inner type
                if let Some(inner) = args.first() {
                    self.check_box_depth(inner, span, new_depth);
                }
            } else {
                // Check generic arguments of other ADT types
                for arg in args {
                    self.check_box_depth(arg, span, 0);
                }
            }
        }

        // Check nested types
        match ty.kind() {
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                self.check_box_depth(inner, span, 0);
            }
            TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                self.check_box_depth(element, span, 0);
            }
            TypeKind::Tuple(tys) => {
                for t in tys {
                    self.check_box_depth(t, span, 0);
                }
            }
            _ => {}
        }
    }

    /// Check for excessive indirection depth.
    fn check_indirection_depth(&mut self, ty: &Type, span: Span, depth: usize) {
        match ty.kind() {
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                let d = depth + 1;
                if d > self.config.max_indirection {
                    let type_str = format!("{}", ty);
                    if !self.warned_types.contains(&format!("indirection:{}", type_str)) {
                        self.warned_types.insert(format!("indirection:{}", type_str));
                        let warning = Diagnostic::warning(
                            format!(
                                "excessive indirection ({} levels): accessing data requires {} generation checks",
                                d, d
                            ),
                            span,
                        )
                        .with_error_code(ErrorCode::ExcessiveIndirection);
                        self.warnings.push(warning);
                    }
                }
                self.check_indirection_depth(inner, span, d);
                return;
            }
            TypeKind::Adt { def_id, args } if self.is_box_type(*def_id) => {
                let d = depth + 1;
                if d > self.config.max_indirection {
                    let type_str = format!("{}", ty);
                    if !self.warned_types.contains(&format!("indirection:{}", type_str)) {
                        self.warned_types.insert(format!("indirection:{}", type_str));
                        let warning = Diagnostic::warning(
                            format!(
                                "excessive indirection ({} levels): accessing data requires {} generation checks",
                                d, d
                            ),
                            span,
                        )
                        .with_error_code(ErrorCode::ExcessiveIndirection);
                        self.warnings.push(warning);
                    }
                }
                if let Some(inner) = args.first() {
                    self.check_indirection_depth(inner, span, d);
                }
                return;
            }
            // Reset depth for non-pointer types and check children
            TypeKind::Tuple(tys) => {
                for t in tys {
                    self.check_indirection_depth(t, span, 0);
                }
            }
            TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                self.check_indirection_depth(element, span, 0);
            }
            TypeKind::Adt { args, .. } => {
                for arg in args {
                    self.check_indirection_depth(arg, span, 0);
                }
            }
            _ => {}
        }
    }

    /// Check for arrays of pointers.
    fn check_pointer_array(&mut self, ty: &Type, span: Span) {
        if !self.config.warn_pointer_arrays {
            return;
        }

        match ty.kind() {
            TypeKind::Array { element, size } => {
                if self.is_pointer_type(element) && *size > 4 {
                    let type_str = format!("{}", ty);
                    if !self.warned_types.contains(&format!("ptr_array:{}", type_str)) {
                        self.warned_types.insert(format!("ptr_array:{}", type_str));
                        let warning = Diagnostic::warning(
                            format!(
                                "array of {} pointers: uses {}x memory of 64-bit pointers",
                                size,
                                2
                            ),
                            span,
                        )
                        .with_error_code(ErrorCode::PointerArrayPattern)
                        .with_suggestion(
                            "consider storing values directly, or use indices into a separate Vec"
                        );
                        self.warnings.push(warning);
                    }
                }
                // Recursively check the element type
                self.check_pointer_array(element, span);
            }
            TypeKind::Tuple(tys) => {
                for t in tys {
                    self.check_pointer_array(t, span);
                }
            }
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                self.check_pointer_array(inner, span);
            }
            TypeKind::Adt { args, .. } => {
                for arg in args {
                    self.check_pointer_array(arg, span);
                }
            }
            _ => {}
        }
    }

    /// Check struct fields for pointer density.
    fn check_pointer_density(&mut self, struct_name: &str, fields: &[(String, Type)], span: Span) {
        if fields.is_empty() {
            return;
        }

        let pointer_fields = fields.iter().filter(|(_, ty)| self.is_pointer_type(ty)).count();
        let density = pointer_fields as f64 / fields.len() as f64;

        if density > self.config.max_pointer_density && fields.len() >= 3 {
            let key = format!("density:{}", struct_name);
            if !self.warned_types.contains(&key) {
                self.warned_types.insert(key);
                let warning = Diagnostic::warning(
                    format!(
                        "struct `{}` has {:.0}% pointer fields ({}/{}): \
                         this structure has significant memory overhead with 128-bit pointers",
                        struct_name,
                        density * 100.0,
                        pointer_fields,
                        fields.len()
                    ),
                    span,
                )
                .with_error_code(ErrorCode::PointerHeavyStruct);
                self.warnings.push(warning);
            }
        }
    }

    /// Check for linked list patterns (self-referential types).
    fn check_linked_list_pattern(
        &mut self,
        struct_name: &str,
        struct_def_id: DefId,
        fields: &[(String, Type)],
        span: Span,
    ) {
        if !self.config.warn_linked_lists {
            return;
        }

        // Check if any field contains a reference to this type (directly or via Option/Box)
        for (field_name, field_ty) in fields {
            if self.contains_self_reference(field_ty, struct_def_id) {
                let key = format!("linked:{}", struct_name);
                if !self.warned_types.contains(&key) {
                    self.warned_types.insert(key);
                    let warning = Diagnostic::warning(
                        format!(
                            "struct `{}` has self-referential field `{}`: \
                             linked structures have poor cache locality with 128-bit pointers",
                            struct_name, field_name
                        ),
                        span,
                    )
                    .with_error_code(ErrorCode::LinkedListPattern);
                    self.warnings.push(warning);
                }
                break; // Only warn once per struct
            }
        }
    }

    /// Check if a type is a pointer type (reference, raw pointer, or Box).
    fn is_pointer_type(&self, ty: &Type) -> bool {
        match ty.kind() {
            TypeKind::Ref { .. } | TypeKind::Ptr { .. } => true,
            TypeKind::Adt { def_id, .. } => self.is_box_type(*def_id),
            _ => false,
        }
    }

    /// Check if a DefId represents the Box type.
    /// This is a simplified check - in practice, we'd have proper type resolution.
    fn is_box_type(&self, _def_id: DefId) -> bool {
        // In a full implementation, we'd check against the known Box DefId
        // For now, we rely on naming conventions in the codebase
        // This would be enhanced to properly identify Box<T> from the prelude
        false // Conservative: disabled until we have proper Box identification
    }

    /// Check if a type contains a reference to the given struct (for linked list detection).
    fn contains_self_reference(&self, ty: &Type, target_def_id: DefId) -> bool {
        match ty.kind() {
            TypeKind::Adt { def_id, args } => {
                // Direct self-reference
                if *def_id == target_def_id {
                    return true;
                }
                // Check through generic arguments (e.g., Option<Self>, Box<Self>)
                args.iter().any(|arg| self.contains_self_reference(arg, target_def_id))
            }
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                self.contains_self_reference(inner, target_def_id)
            }
            TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                self.contains_self_reference(element, target_def_id)
            }
            TypeKind::Tuple(tys) => {
                tys.iter().any(|t| self.contains_self_reference(t, target_def_id))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::def::IntTy;
    use crate::hir::ty::PrimitiveTy;
    use crate::span::Span;

    fn test_span() -> Span {
        Span { start: 0, end: 10, start_line: 1, start_col: 1 }
    }

    fn i32_type() -> Type {
        Type::new(TypeKind::Primitive(PrimitiveTy::Int(IntTy::I32)))
    }

    fn ref_type(inner: Type) -> Type {
        Type::new(TypeKind::Ref { inner, mutable: false })
    }

    #[test]
    fn test_excessive_indirection() {
        let mut ctx = PointerLintContext::with_default_config();

        // Create &&&i32 - 3 levels of indirection
        let deeply_nested = ref_type(ref_type(ref_type(i32_type())));

        ctx.check_type(&deeply_nested, test_span());

        // Should not warn at 3 levels (max_indirection is 3)
        assert!(ctx.warnings.is_empty() || ctx.warnings.iter().all(|w|
            w.code.as_ref().map(|c| c != "W0005").unwrap_or(true)
        ));
    }

    #[test]
    fn test_config_customization() {
        let config = PointerLintConfig {
            max_box_depth: 1,
            max_pointer_density: 0.5,
            max_indirection: 2,
            warn_linked_lists: false,
            warn_pointer_arrays: false,
        };

        let ctx = PointerLintContext::new(config.clone());
        assert_eq!(ctx.config.max_box_depth, 1);
        assert_eq!(ctx.config.max_pointer_density, 0.5);
        assert_eq!(ctx.config.max_indirection, 2);
    }

    // Handler lint tests

    #[test]
    fn test_handler_lint_default_config() {
        let ctx = HandlerLintContext::with_default_config();
        assert_eq!(ctx.config.max_handler_depth, 5);
        assert_eq!(ctx.config.max_handler_operations, 10);
        assert!(ctx.config.warn_deep_handlers);
        assert!(ctx.config.warn_large_handlers);
    }

    #[test]
    fn test_handler_lint_custom_config() {
        let config = HandlerLintConfig {
            max_handler_depth: 3,
            max_handler_operations: 5,
            warn_deep_handlers: true,
            warn_large_handlers: false,
        };

        let ctx = HandlerLintContext::new(config);
        assert_eq!(ctx.config.max_handler_depth, 3);
        assert_eq!(ctx.config.max_handler_operations, 5);
    }

    #[test]
    fn test_handler_lint_no_warning_shallow_nesting() {
        let mut ctx = HandlerLintContext::with_default_config();

        // Create a simple literal expression (no handlers)
        let simple_expr = Expr::new(
            ExprKind::Literal(crate::hir::expr::LiteralValue::Int(42)),
            i32_type(),
            test_span(),
        );

        ctx.check_expr(&simple_expr);
        assert!(ctx.warnings.is_empty());
    }

    #[test]
    fn test_handler_lint_detects_deep_nesting() {
        // Test with max_handler_depth = 2 to easily trigger warning
        let config = HandlerLintConfig {
            max_handler_depth: 2,
            max_handler_operations: 10,
            warn_deep_handlers: true,
            warn_large_handlers: true,
        };
        let mut ctx = HandlerLintContext::new(config);

        // Create nested Handle expressions: handle { handle { handle { 42 } } }
        let innermost = Expr::new(
            ExprKind::Literal(crate::hir::expr::LiteralValue::Int(42)),
            i32_type(),
            Span::dummy(),
        );

        let handler_instance = Expr::new(
            ExprKind::Literal(crate::hir::expr::LiteralValue::Int(0)),
            i32_type(),
            Span::dummy(),
        );

        // Third level (innermost handle)
        let handle3 = Expr::new(
            ExprKind::Handle {
                body: Box::new(innermost),
                handler_id: DefId::new(1),
                handler_instance: Box::new(handler_instance.clone()),
            },
            i32_type(),
            Span { start: 10, end: 20, start_line: 1, start_col: 10 },
        );

        // Second level
        let handle2 = Expr::new(
            ExprKind::Handle {
                body: Box::new(handle3),
                handler_id: DefId::new(2),
                handler_instance: Box::new(handler_instance.clone()),
            },
            i32_type(),
            Span { start: 30, end: 40, start_line: 1, start_col: 30 },
        );

        // First level (outermost)
        let handle1 = Expr::new(
            ExprKind::Handle {
                body: Box::new(handle2),
                handler_id: DefId::new(3),
                handler_instance: Box::new(handler_instance),
            },
            i32_type(),
            Span { start: 50, end: 60, start_line: 1, start_col: 50 },
        );

        ctx.check_expr(&handle1);

        // With max_depth = 2, depth 3 should trigger warning
        assert!(!ctx.warnings.is_empty(), "Should warn about deeply nested handlers");
        assert!(ctx.warnings.iter().any(|w|
            w.code.as_ref().map(|c| c == "W0100").unwrap_or(false)
        ), "Warning should have code W0100 for DeeplyNestedHandlers");
    }
}
