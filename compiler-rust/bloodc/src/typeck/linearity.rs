//! Linearity checking for Blood.
//!
//! This module implements linearity checking for linear and affine types:
//! - Linear types must be used exactly once
//! - Affine types may be used at most once
//!
//! The checker runs after type checking to analyze usage patterns.
//!
//! # Key Rules
//!
//! 1. **Must-use**: Linear values must be consumed (used) exactly once
//! 2. **At-most-once**: Affine values may be used zero or one time
//! 3. **Branch consistency**: Linear values must be consumed in all branches or none
//! 4. **No loop consumption**: Linear values defined outside a loop cannot be consumed inside

use std::collections::HashMap;

use crate::hir::{self, DefId, LocalId, Type};
use crate::hir::item::{ItemKind, StructKind};
use crate::hir::ty::{TypeKind, OwnershipQualifier};
use crate::span::Span;
use crate::typeck::error::{TypeError, TypeErrorKind};

/// Tracks the usage state of a linear or affine variable.
#[derive(Debug, Clone)]
struct VariableUsage {
    /// Name of the variable (for error messages).
    name: String,
    /// Type of the variable.
    ty: Type,
    /// Span where the variable was defined.
    def_span: Span,
    /// Whether this is linear (true) or affine (false).
    is_linear: bool,
    /// Number of times the variable has been used.
    use_count: usize,
    /// Span of the first use (for error messages).
    first_use: Option<Span>,
    /// Whether this variable was defined in an outer scope (before current loop).
    defined_before_loop: bool,
}

/// Linearity checker context.
pub struct LinearityChecker<'hir> {
    /// Map from local ID to usage tracking.
    locals: HashMap<LocalId, VariableUsage>,
    /// Current loop depth (0 = not in loop).
    loop_depth: usize,
    /// Errors accumulated during checking.
    errors: Vec<TypeError>,
    /// HIR crate for looking up ADT field types.
    hir_crate: &'hir hir::Crate,
}

impl<'hir> LinearityChecker<'hir> {
    /// Create a new linearity checker with access to the HIR crate.
    pub fn new(hir_crate: &'hir hir::Crate) -> Self {
        Self {
            locals: HashMap::new(),
            loop_depth: 0,
            errors: Vec::new(),
            hir_crate,
        }
    }

    /// Check linearity for a function body.
    pub fn check_body(&mut self, body: &hir::Body) -> Vec<TypeError> {
        // Register all locals that have linear/affine types
        for local in &body.locals {
            if let Some(usage) = self.create_usage_tracking(local) {
                self.locals.insert(local.id, usage);
            }
        }

        // Walk the expression tree to track usage
        self.check_expr(&body.expr);

        // At the end of the body, check that all linear values were consumed
        self.check_final_usage();

        std::mem::take(&mut self.errors)
    }

    /// Create usage tracking for a local if it has linear/affine type.
    fn create_usage_tracking(&self, local: &hir::Local) -> Option<VariableUsage> {
        let (is_linear, is_affine) = self.check_linearity(&local.ty);
        if is_linear || is_affine {
            Some(VariableUsage {
                name: local.name.clone().unwrap_or_else(|| "<unnamed>".to_string()),
                ty: local.ty.clone(),
                def_span: local.span,
                is_linear,
                use_count: 0,
                first_use: None,
                defined_before_loop: false,
            })
        } else {
            None
        }
    }

    /// Check if a type is linear or affine.
    fn check_linearity(&self, ty: &Type) -> (bool, bool) {
        match ty.kind() {
            TypeKind::Ownership { qualifier: OwnershipQualifier::Linear, .. } => (true, false),
            TypeKind::Ownership { qualifier: OwnershipQualifier::Affine, .. } => (false, true),
            // Recursively check inner types
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                self.check_linearity(inner)
            }
            TypeKind::Tuple(tys) => {
                let mut is_linear = false;
                let mut is_affine = false;
                for t in tys {
                    let (l, a) = self.check_linearity(t);
                    is_linear |= l;
                    is_affine |= a;
                }
                (is_linear, is_affine)
            }
            TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                self.check_linearity(element)
            }
            TypeKind::Range { element, .. } => self.check_linearity(element),
            TypeKind::Record { fields, .. } => {
                let mut is_linear = false;
                let mut is_affine = false;
                for f in fields {
                    let (l, a) = self.check_linearity(&f.ty);
                    is_linear |= l;
                    is_affine |= a;
                }
                (is_linear, is_affine)
            }
            TypeKind::Forall { body, .. } => self.check_linearity(body),
            // ADTs may contain linear/affine fields — recurse into them.
            TypeKind::Adt { def_id, .. } => {
                self.check_adt_field_linearity(*def_id)
            }
            // These types cannot contain linear/affine values
            TypeKind::Fn { .. }
            | TypeKind::Closure { .. }
            | TypeKind::DynTrait { .. }
            | TypeKind::Primitive(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::Infer(_)
            | TypeKind::Param(_) => (false, false),
        }
    }

    /// Look up an ADT's field types and check if any are linear/affine.
    fn check_adt_field_linearity(&self, def_id: DefId) -> (bool, bool) {
        if let Some(item) = self.hir_crate.items.get(&def_id) {
            let field_types: Vec<&Type> = match &item.kind {
                ItemKind::Struct(s) => match &s.kind {
                    StructKind::Record(fields) | StructKind::Tuple(fields) => {
                        fields.iter().map(|f| &f.ty).collect()
                    }
                    StructKind::Unit => Vec::new(),
                },
                ItemKind::Enum(e) => {
                    e.variants.iter().flat_map(|v| match &v.fields {
                        StructKind::Record(fields) | StructKind::Tuple(fields) => {
                            fields.iter().map(|f| &f.ty).collect::<Vec<_>>()
                        }
                        StructKind::Unit => Vec::new(),
                    }).collect()
                }
                _ => return (false, false),
            };
            let mut is_linear = false;
            let mut is_affine = false;
            for field_ty in field_types {
                let (l, a) = self.check_linearity(field_ty);
                is_linear |= l;
                is_affine |= a;
            }
            (is_linear, is_affine)
        } else {
            // Unknown ADT — conservatively non-linear
            (false, false)
        }
    }

    /// Check an expression for variable usage.
    fn check_expr(&mut self, expr: &hir::Expr) {
        match &expr.kind {
            // Variable reference - this is a use
            hir::ExprKind::Local(local_id) => {
                self.record_use(*local_id, expr.span);
            }

            // Block/Region - check all statements and final expression
            hir::ExprKind::Block { stmts, expr: tail }
            | hir::ExprKind::Region { stmts, expr: tail, .. } => {
                for stmt in stmts {
                    self.check_stmt(stmt);
                }
                if let Some(e) = tail {
                    self.check_expr(e);
                }
            }

            // If expression - need to check branches
            hir::ExprKind::If { condition, then_branch, else_branch } => {
                self.check_expr(condition);

                // Save current usage state
                let saved_state = self.snapshot_usage();

                // Check then branch
                self.check_expr(then_branch);
                let then_usage = self.snapshot_usage();

                // Restore and check else branch
                self.restore_usage(saved_state.clone());
                if let Some(else_expr) = else_branch {
                    self.check_expr(else_expr);
                }
                let else_usage = self.snapshot_usage();

                // Verify consistent usage across branches
                self.verify_branch_consistency(&saved_state, &[then_usage, else_usage], expr.span);
            }

            // Match expression - check all arms
            hir::ExprKind::Match { scrutinee, arms } => {
                self.check_expr(scrutinee);

                if arms.is_empty() {
                    return;
                }

                // Save current usage state
                let saved_state = self.snapshot_usage();
                let mut arm_usages = Vec::with_capacity(arms.len());

                for arm in arms {
                    self.restore_usage(saved_state.clone());
                    // Pattern bindings are new locals, handled separately
                    if let Some(guard) = &arm.guard {
                        self.check_expr(guard);
                    }
                    self.check_expr(&arm.body);
                    arm_usages.push(self.snapshot_usage());
                }

                // Verify consistent usage across all arms
                self.verify_branch_consistency(&saved_state, &arm_usages, expr.span);
            }

            // Loop expressions - mark current linear vars as "defined before loop"
            hir::ExprKind::Loop { body, .. } => {
                self.enter_loop();
                self.check_expr(body);
                self.exit_loop();
            }

            hir::ExprKind::While { condition, body, .. } => {
                self.check_expr(condition);
                self.enter_loop();
                self.check_expr(body);
                self.exit_loop();
            }

            // Binary operations - check both sides
            hir::ExprKind::Binary { left, right, .. } => {
                self.check_expr(left);
                self.check_expr(right);
            }

            // Unary operations
            hir::ExprKind::Unary { operand, .. } => {
                self.check_expr(operand);
            }

            // Call expressions
            hir::ExprKind::Call { callee, args } => {
                self.check_expr(callee);
                for arg in args {
                    self.check_expr(arg);
                }
            }

            // Method calls
            hir::ExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }

            // Field access
            hir::ExprKind::Field { base, .. } => {
                self.check_expr(base);
            }

            // Index access
            hir::ExprKind::Index { base, index } => {
                self.check_expr(base);
                self.check_expr(index);
            }

            // Tuple/array literals
            hir::ExprKind::Tuple(exprs) | hir::ExprKind::Array(exprs) => {
                for e in exprs {
                    self.check_expr(e);
                }
            }

            // Array repeat
            hir::ExprKind::Repeat { value, .. } => {
                self.check_expr(value);
            }

            // Struct literal
            hir::ExprKind::Struct { fields, base, .. } => {
                for field in fields {
                    self.check_expr(&field.value);
                }
                if let Some(base_expr) = base {
                    self.check_expr(base_expr);
                }
            }

            // Record literal
            hir::ExprKind::Record { fields } => {
                for field in fields {
                    self.check_expr(&field.value);
                }
            }

            // Variant construction
            hir::ExprKind::Variant { fields, .. } => {
                for field in fields {
                    self.check_expr(field);
                }
            }

            // Borrow/AddrOf
            hir::ExprKind::Borrow { expr, .. } | hir::ExprKind::AddrOf { expr, .. } => {
                self.check_expr(expr);
            }

            // Dereference
            hir::ExprKind::Deref(inner) => {
                self.check_expr(inner);
            }

            // Cast
            hir::ExprKind::Cast { expr, .. } => {
                self.check_expr(expr);
            }

            // Return/break with value
            hir::ExprKind::Return(Some(e)) | hir::ExprKind::Break { value: Some(e), .. } => {
                self.check_expr(e);
            }

            // Closure - captures need special handling
            hir::ExprKind::Closure { captures, .. } => {
                // Check that captured linear values are handled correctly
                for capture in captures {
                    if let Some(usage) = self.locals.get(&capture.local_id) {
                        if usage.is_linear {
                            // Linear value captured by closure - this is a use
                            self.record_use(capture.local_id, expr.span);
                        }
                    }
                }
                // Note: body is a separate Body, will be checked separately
            }

            // Perform expression
            hir::ExprKind::Perform { args, .. } => {
                for arg in args {
                    self.check_expr(arg);
                }
            }

            // Handle expression
            hir::ExprKind::Handle { body, handler_instance, .. } => {
                self.check_expr(body);
                self.check_expr(handler_instance);
            }

            // Inline handle expression
            hir::ExprKind::InlineHandle { body, handlers, .. } => {
                self.check_expr(body);
                for handler in handlers {
                    self.check_expr(&handler.body);
                }
            }

            // Assign expression
            hir::ExprKind::Assign { target, value } => {
                self.check_expr(target);
                self.check_expr(value);
            }

            // Let expression
            hir::ExprKind::Let { init, .. } => {
                self.check_expr(init);
            }

            // Unsafe block
            hir::ExprKind::Unsafe(inner) => {
                self.check_expr(inner);
            }

            // Resume with value
            hir::ExprKind::Resume { value: Some(v) } => {
                self.check_expr(v);
            }

            // Range expression
            hir::ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.check_expr(s);
                }
                if let Some(e) = end {
                    self.check_expr(e);
                }
            }

            // Macro expansion - check all arguments
            hir::ExprKind::MacroExpansion { args, named_args, .. } => {
                for arg in args {
                    self.check_expr(arg);
                }
                for (_, arg) in named_args {
                    self.check_expr(arg);
                }
            }

            // Vec literal
            hir::ExprKind::VecLiteral(exprs) => {
                for e in exprs {
                    self.check_expr(e);
                }
            }

            // Vec repeat
            hir::ExprKind::VecRepeat { value, count } => {
                self.check_expr(value);
                self.check_expr(count);
            }

            // Assert expression
            hir::ExprKind::Assert { condition, message } => {
                self.check_expr(condition);
                if let Some(msg) = message {
                    self.check_expr(msg);
                }
            }

            // Debug expression
            hir::ExprKind::Dbg(inner) => {
                self.check_expr(inner);
            }

            // Slice/Vec length
            hir::ExprKind::SliceLen(inner) | hir::ExprKind::VecLen(inner) => {
                self.check_expr(inner);
            }

            // Array to slice coercion
            hir::ExprKind::ArrayToSlice { expr, .. } => {
                self.check_expr(expr);
            }

            // Literals and other leaf nodes - no variable usage
            hir::ExprKind::Literal(_)
            | hir::ExprKind::Def(_)
            | hir::ExprKind::Return(None)
            | hir::ExprKind::Break { value: None, .. }
            | hir::ExprKind::Continue { .. }
            | hir::ExprKind::Error
            | hir::ExprKind::MethodFamily { .. }
            | hir::ExprKind::ConstParam(_)
            | hir::ExprKind::Resume { value: None }
            | hir::ExprKind::Default => {}
        }
    }

    /// Check a statement for variable usage.
    fn check_stmt(&mut self, stmt: &hir::Stmt) {
        match stmt {
            hir::Stmt::Let { init, .. } => {
                if let Some(init_expr) = init {
                    self.check_expr(init_expr);
                }
            }
            hir::Stmt::Expr(expr) => {
                self.check_expr(expr);
            }
            hir::Stmt::Item(_) => {
                // Item declarations don't involve variable usage
            }
        }
    }

    /// Record a use of a local variable.
    fn record_use(&mut self, local_id: LocalId, span: Span) {
        if let Some(usage) = self.locals.get_mut(&local_id) {
            usage.use_count += 1;

            // Record first use for error messages
            if usage.first_use.is_none() {
                usage.first_use = Some(span);
            }

            // Check for multiple uses
            if usage.use_count > 1 {
                let error = if usage.is_linear {
                    TypeError::new(
                        TypeErrorKind::MultipleLinearUse {
                            name: usage.name.clone(),
                            ty: usage.ty.clone(),
                            first_use: usage.first_use.unwrap_or(usage.def_span),
                            second_use: span,
                        },
                        span,
                    )
                } else {
                    TypeError::new(
                        TypeErrorKind::MultipleAffineUse {
                            name: usage.name.clone(),
                            ty: usage.ty.clone(),
                            first_use: usage.first_use.unwrap_or(usage.def_span),
                            second_use: span,
                        },
                        span,
                    )
                };
                self.errors.push(error);
            }

            // Check for consumption in loop
            if usage.defined_before_loop && self.loop_depth > 0 && usage.is_linear {
                self.errors.push(TypeError::new(
                    TypeErrorKind::LinearConsumedInLoop {
                        name: usage.name.clone(),
                        ty: usage.ty.clone(),
                    },
                    span,
                ));
            }
        }
    }

    /// Enter a loop context.
    fn enter_loop(&mut self) {
        // Mark all current linear/affine locals as "defined before loop"
        if self.loop_depth == 0 {
            for usage in self.locals.values_mut() {
                usage.defined_before_loop = true;
            }
        }
        self.loop_depth += 1;
    }

    /// Exit a loop context.
    fn exit_loop(&mut self) {
        self.loop_depth -= 1;
        if self.loop_depth == 0 {
            for usage in self.locals.values_mut() {
                usage.defined_before_loop = false;
            }
        }
    }

    /// Take a snapshot of the current usage state.
    fn snapshot_usage(&self) -> HashMap<LocalId, usize> {
        self.locals.iter()
            .map(|(id, usage)| (*id, usage.use_count))
            .collect()
    }

    /// Restore usage state from a snapshot.
    fn restore_usage(&mut self, snapshot: HashMap<LocalId, usize>) {
        for (id, count) in snapshot {
            if let Some(usage) = self.locals.get_mut(&id) {
                usage.use_count = count;
            }
        }
    }

    /// Verify that linear variables are consumed consistently across branches.
    fn verify_branch_consistency(
        &mut self,
        before: &HashMap<LocalId, usize>,
        branches: &[HashMap<LocalId, usize>],
        span: Span,
    ) {
        if branches.is_empty() {
            return;
        }

        // Collect updates to apply after the loop
        let mut updates: Vec<(LocalId, usize)> = Vec::new();

        // For each linear variable, check if it was consumed in all branches or none
        for (local_id, usage) in &self.locals {
            if !usage.is_linear {
                continue;
            }

            let before_count = before.get(local_id).copied().unwrap_or(0);

            // Count how many branches consumed the variable
            let mut consumed_count = 0;
            let mut max_usage = before_count;

            for branch in branches {
                let branch_count = branch.get(local_id).copied().unwrap_or(0);
                if branch_count > before_count {
                    consumed_count += 1;
                }
                max_usage = max_usage.max(branch_count);
            }

            // If consumed in some but not all branches, that's an error
            if consumed_count > 0 && consumed_count < branches.len() {
                self.errors.push(TypeError::new(
                    TypeErrorKind::InconsistentLinearBranches {
                        name: usage.name.clone(),
                        ty: usage.ty.clone(),
                        consumed_count,
                        branch_count: branches.len(),
                    },
                    span,
                ));
            }

            // Record update to apply after loop
            updates.push((*local_id, max_usage));
        }

        // Apply updates
        for (local_id, max_usage) in updates {
            if let Some(u) = self.locals.get_mut(&local_id) {
                u.use_count = max_usage;
            }
        }
    }

    /// Check that all linear values were consumed at the end of the body.
    fn check_final_usage(&mut self) {
        for usage in self.locals.values() {
            if usage.is_linear && usage.use_count == 0 {
                self.errors.push(TypeError::new(
                    TypeErrorKind::UnusedLinearValue {
                        name: usage.name.clone(),
                        ty: usage.ty.clone(),
                    },
                    usage.def_span,
                ));
            }
        }
    }
}

/// Check linearity for all bodies in a crate.
pub fn check_crate_linearity(krate: &hir::Crate) -> Vec<TypeError> {
    let mut errors = Vec::new();

    for body in krate.bodies.values() {
        let mut checker = LinearityChecker::new(krate);
        errors.extend(checker.check_body(body));
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linearity_check_basic() {
        // Test will be implemented with actual HIR construction
        // For now, just verify the module compiles
        let krate = hir::Crate::new();
        let checker = LinearityChecker::new(&krate);
        assert!(checker.errors.is_empty());
    }
}
