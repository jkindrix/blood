//! Macro expansion pass for HIR.
//!
//! This module transforms macro HIR nodes (MacroExpansion, VecLiteral, etc.)
//! into equivalent regular HIR nodes before MIR lowering.
//!
//! The expansion happens after type checking has produced typed HIR, so we
//! have access to type information for the expanded expressions.

use std::collections::HashMap;

use crate::diagnostics::Diagnostic;
use crate::hir::{self, DefId, Expr, ExprKind, Type, BodyId, LiteralValue};
use crate::span::Span;

/// A part of a parsed format string.
#[derive(Debug)]
enum FormatPart {
    /// A literal string segment.
    Literal(String),
    /// A placeholder at the given argument index.
    Placeholder(usize),
}

/// Expand all macros in a HIR crate.
///
/// This transforms macro-specific HIR nodes into regular HIR that can be
/// lowered to MIR. Must be called after type checking and before MIR lowering.
pub fn expand_macros(crate_: &mut hir::Crate) -> Result<(), Vec<Diagnostic>> {
    let mut expander = MacroExpander::new(crate_);
    expander.expand_all()
}

/// The macro expander context.
struct MacroExpander<'a> {
    /// Reference to the HIR crate being expanded.
    crate_: &'a mut hir::Crate,
    /// Map from builtin function names to their DefIds.
    builtin_by_name: HashMap<String, DefId>,
    /// Collected diagnostics.
    diagnostics: Vec<Diagnostic>,
}

impl<'a> MacroExpander<'a> {
    fn new(crate_: &'a mut hir::Crate) -> Self {
        // Build reverse lookup from builtin name to DefId
        let builtin_by_name: HashMap<String, DefId> = crate_
            .builtin_fns
            .iter()
            .map(|(def_id, name)| (name.clone(), *def_id))
            .collect();

        Self {
            crate_,
            builtin_by_name,
            diagnostics: Vec::new(),
        }
    }

    /// Expand all macros in all bodies.
    fn expand_all(&mut self) -> Result<(), Vec<Diagnostic>> {
        // Collect body IDs first to avoid borrow issues
        let body_ids: Vec<BodyId> = self.crate_.bodies.keys().copied().collect();

        for body_id in body_ids {
            // Take the body out to avoid borrow conflicts
            if let Some(mut body) = self.crate_.bodies.remove(&body_id) {
                body.expr = self.expand_expr(body.expr);
                self.crate_.bodies.insert(body_id, body);
            }
        }

        if self.diagnostics.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.diagnostics))
        }
    }

    /// Expand macros in an expression.
    fn expand_expr(&mut self, expr: Expr) -> Expr {
        let span = expr.span;
        let ty = expr.ty.clone();

        let kind = match expr.kind {
            // === Macro nodes to expand ===

            ExprKind::MacroExpansion { macro_name, format_str, args } => {
                self.expand_format_macro(&macro_name, &format_str, args, &ty, span)
            }

            ExprKind::VecLiteral(exprs) => {
                self.expand_vec_literal(exprs, &ty, span)
            }

            ExprKind::VecRepeat { value, count } => {
                self.expand_vec_repeat(*value, *count, &ty, span)
            }

            ExprKind::Assert { condition, message } => {
                self.expand_assert(*condition, message.map(|m| *m), span)
            }

            ExprKind::Dbg(inner) => {
                self.expand_dbg(*inner, &ty, span)
            }

            // === Recursively expand all compound expressions ===

            ExprKind::Binary { op, left, right } => {
                ExprKind::Binary {
                    op,
                    left: Box::new(self.expand_expr(*left)),
                    right: Box::new(self.expand_expr(*right)),
                }
            }

            ExprKind::Unary { op, operand } => {
                ExprKind::Unary {
                    op,
                    operand: Box::new(self.expand_expr(*operand)),
                }
            }

            ExprKind::Call { callee, args } => {
                ExprKind::Call {
                    callee: Box::new(self.expand_expr(*callee)),
                    args: args.into_iter().map(|a| self.expand_expr(a)).collect(),
                }
            }

            ExprKind::MethodCall { receiver, method, args } => {
                ExprKind::MethodCall {
                    receiver: Box::new(self.expand_expr(*receiver)),
                    method,
                    args: args.into_iter().map(|a| self.expand_expr(a)).collect(),
                }
            }

            ExprKind::Block { stmts, expr: tail } => {
                let expanded_stmts = stmts.into_iter().map(|s| self.expand_stmt(s)).collect();
                let expanded_tail = tail.map(|e| Box::new(self.expand_expr(*e)));
                ExprKind::Block {
                    stmts: expanded_stmts,
                    expr: expanded_tail,
                }
            }

            ExprKind::If { condition, then_branch, else_branch } => {
                ExprKind::If {
                    condition: Box::new(self.expand_expr(*condition)),
                    then_branch: Box::new(self.expand_expr(*then_branch)),
                    else_branch: else_branch.map(|e| Box::new(self.expand_expr(*e))),
                }
            }

            ExprKind::Match { scrutinee, arms } => {
                ExprKind::Match {
                    scrutinee: Box::new(self.expand_expr(*scrutinee)),
                    arms: arms.into_iter().map(|arm| hir::MatchArm {
                        pattern: arm.pattern,
                        guard: arm.guard.map(|g| self.expand_expr(g)),
                        body: self.expand_expr(arm.body),
                    }).collect(),
                }
            }

            ExprKind::Loop { body, label } => {
                ExprKind::Loop {
                    body: Box::new(self.expand_expr(*body)),
                    label,
                }
            }

            ExprKind::While { condition, body, label } => {
                ExprKind::While {
                    condition: Box::new(self.expand_expr(*condition)),
                    body: Box::new(self.expand_expr(*body)),
                    label,
                }
            }

            ExprKind::Return(val) => {
                ExprKind::Return(val.map(|v| Box::new(self.expand_expr(*v))))
            }

            ExprKind::Break { label, value } => {
                ExprKind::Break {
                    label,
                    value: value.map(|v| Box::new(self.expand_expr(*v))),
                }
            }

            ExprKind::Assign { target, value } => {
                ExprKind::Assign {
                    target: Box::new(self.expand_expr(*target)),
                    value: Box::new(self.expand_expr(*value)),
                }
            }

            ExprKind::Field { base, field_idx } => {
                ExprKind::Field {
                    base: Box::new(self.expand_expr(*base)),
                    field_idx,
                }
            }

            ExprKind::Index { base, index } => {
                ExprKind::Index {
                    base: Box::new(self.expand_expr(*base)),
                    index: Box::new(self.expand_expr(*index)),
                }
            }

            ExprKind::Tuple(elems) => {
                ExprKind::Tuple(elems.into_iter().map(|e| self.expand_expr(e)).collect())
            }

            ExprKind::Array(elems) => {
                ExprKind::Array(elems.into_iter().map(|e| self.expand_expr(e)).collect())
            }

            ExprKind::Repeat { value, count } => {
                ExprKind::Repeat {
                    value: Box::new(self.expand_expr(*value)),
                    count,
                }
            }

            ExprKind::Struct { def_id, fields, base } => {
                ExprKind::Struct {
                    def_id,
                    fields: fields.into_iter().map(|f| hir::FieldExpr {
                        field_idx: f.field_idx,
                        value: self.expand_expr(f.value),
                    }).collect(),
                    base: base.map(|b| Box::new(self.expand_expr(*b))),
                }
            }

            ExprKind::Record { fields } => {
                ExprKind::Record {
                    fields: fields.into_iter().map(|f| hir::RecordFieldExpr {
                        name: f.name,
                        value: self.expand_expr(f.value),
                    }).collect(),
                }
            }

            ExprKind::Variant { def_id, variant_idx, fields } => {
                ExprKind::Variant {
                    def_id,
                    variant_idx,
                    fields: fields.into_iter().map(|f| self.expand_expr(f)).collect(),
                }
            }

            ExprKind::Cast { expr: inner, target_ty } => {
                ExprKind::Cast {
                    expr: Box::new(self.expand_expr(*inner)),
                    target_ty,
                }
            }

            ExprKind::Borrow { expr: inner, mutable } => {
                ExprKind::Borrow {
                    expr: Box::new(self.expand_expr(*inner)),
                    mutable,
                }
            }

            ExprKind::Deref(inner) => {
                ExprKind::Deref(Box::new(self.expand_expr(*inner)))
            }

            ExprKind::AddrOf { expr: inner, mutable } => {
                ExprKind::AddrOf {
                    expr: Box::new(self.expand_expr(*inner)),
                    mutable,
                }
            }

            ExprKind::Unsafe(inner) => {
                ExprKind::Unsafe(Box::new(self.expand_expr(*inner)))
            }

            ExprKind::Let { pattern, init } => {
                ExprKind::Let {
                    pattern,
                    init: Box::new(self.expand_expr(*init)),
                }
            }

            ExprKind::Perform { effect_id, op_index, args } => {
                ExprKind::Perform {
                    effect_id,
                    op_index,
                    args: args.into_iter().map(|a| self.expand_expr(a)).collect(),
                }
            }

            ExprKind::Resume { value } => {
                ExprKind::Resume {
                    value: value.map(|v| Box::new(self.expand_expr(*v))),
                }
            }

            ExprKind::Handle { body, handler_id, handler_instance } => {
                ExprKind::Handle {
                    body: Box::new(self.expand_expr(*body)),
                    handler_id,
                    handler_instance: Box::new(self.expand_expr(*handler_instance)),
                }
            }

            ExprKind::Range { start, end, inclusive } => {
                ExprKind::Range {
                    start: start.map(|s| Box::new(self.expand_expr(*s))),
                    end: end.map(|e| Box::new(self.expand_expr(*e))),
                    inclusive,
                }
            }

            ExprKind::Closure { body_id, captures } => {
                // Closure bodies are expanded separately via body_ids loop
                ExprKind::Closure { body_id, captures }
            }

            // === Leaf nodes - no expansion needed ===
            kind @ (ExprKind::Literal(_)
                | ExprKind::Local(_)
                | ExprKind::Def(_)
                | ExprKind::Continue { .. }
                | ExprKind::MethodFamily { .. }
                | ExprKind::Default
                | ExprKind::Error) => kind,
        };

        Expr::new(kind, ty, span)
    }

    /// Expand a statement.
    fn expand_stmt(&mut self, stmt: hir::Stmt) -> hir::Stmt {
        match stmt {
            hir::Stmt::Let { local_id, init } => {
                hir::Stmt::Let {
                    local_id,
                    init: init.map(|e| self.expand_expr(e)),
                }
            }
            hir::Stmt::Expr(expr) => {
                hir::Stmt::Expr(self.expand_expr(expr))
            }
            hir::Stmt::Item(def_id) => {
                hir::Stmt::Item(def_id)
            }
        }
    }

    /// Expand a format macro (format!, println!, print!, etc.)
    fn expand_format_macro(
        &mut self,
        macro_name: &str,
        format_str: &str,
        args: Vec<Expr>,
        _ty: &Type,
        span: Span,
    ) -> ExprKind {
        // Expand arguments recursively first
        let expanded_args: Vec<Expr> = args.into_iter()
            .map(|a| self.expand_expr(a))
            .collect();

        if expanded_args.is_empty() {
            // Simple case: no format args, just a string literal
            match macro_name {
                "println" => self.make_println_str_call(format_str, span),
                "print" => self.make_print_str_call(format_str, span),
                "eprintln" | "eprint" => {
                    // For now, treat eprintln/eprint same as println/print
                    if macro_name.ends_with("ln") {
                        self.make_println_str_call(format_str, span)
                    } else {
                        self.make_print_str_call(format_str, span)
                    }
                }
                "panic" => self.make_panic_call(format_str, span),
                "todo" => {
                    // todo! with message: panic with "not yet implemented: message"
                    if format_str.is_empty() {
                        self.make_panic_call("not yet implemented", span)
                    } else {
                        self.make_panic_call(&format!("not yet implemented: {}", format_str), span)
                    }
                }
                "unimplemented" => {
                    // unimplemented! with message: panic with "not implemented: message"
                    if format_str.is_empty() {
                        self.make_panic_call("not implemented", span)
                    } else {
                        self.make_panic_call(&format!("not implemented: {}", format_str), span)
                    }
                }
                "format" => {
                    // format! with no args just returns the string
                    ExprKind::Literal(LiteralValue::String(format_str.to_string()))
                }
                _ => {
                    self.diagnostics.push(Diagnostic::error(
                        format!("unsupported format macro: {}!", macro_name),
                        span,
                    ));
                    ExprKind::Error
                }
            }
        } else {
            // Complex case: has format args - build interpolated string
            match self.build_format_string(format_str, expanded_args, span) {
                Ok(formatted_expr) => {
                    match macro_name {
                        "println" => self.make_println_expr_call(formatted_expr, span),
                        "print" => self.make_print_expr_call(formatted_expr, span),
                        "eprintln" | "eprint" => {
                            if macro_name.ends_with("ln") {
                                self.make_println_expr_call(formatted_expr, span)
                            } else {
                                self.make_print_expr_call(formatted_expr, span)
                            }
                        }
                        "panic" => self.make_panic_expr_call(formatted_expr, span),
                        "todo" => {
                            // Prepend "not yet implemented: " to the message
                            let prefix_expr = Expr::new(
                                ExprKind::Literal(LiteralValue::String("not yet implemented: ".to_string())),
                                Type::str(),
                                span,
                            );
                            match self.make_str_concat(prefix_expr, formatted_expr, span) {
                                Ok(combined) => self.make_panic_expr_call(combined, span),
                                Err(e) => {
                                    self.diagnostics.push(Diagnostic::error(e, span));
                                    ExprKind::Error
                                }
                            }
                        }
                        "unimplemented" => {
                            // Prepend "not implemented: " to the message
                            let prefix_expr = Expr::new(
                                ExprKind::Literal(LiteralValue::String("not implemented: ".to_string())),
                                Type::str(),
                                span,
                            );
                            match self.make_str_concat(prefix_expr, formatted_expr, span) {
                                Ok(combined) => self.make_panic_expr_call(combined, span),
                                Err(e) => {
                                    self.diagnostics.push(Diagnostic::error(e, span));
                                    ExprKind::Error
                                }
                            }
                        }
                        "format" => {
                            // format! returns the string
                            formatted_expr.kind
                        }
                        _ => {
                            self.diagnostics.push(Diagnostic::error(
                                format!("unsupported format macro: {}!", macro_name),
                                span,
                            ));
                            ExprKind::Error
                        }
                    }
                }
                Err(err_msg) => {
                    self.diagnostics.push(Diagnostic::error(err_msg, span));
                    ExprKind::Error
                }
            }
        }
    }

    /// Parse format string and build concatenated result.
    ///
    /// Handles format strings like "x = {}, y = {}" with positional arguments.
    fn build_format_string(
        &mut self,
        format_str: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<Expr, String> {
        // Parse the format string to find {} placeholders
        let parts = self.parse_format_string(format_str)?;

        // Check that all referenced argument indices are valid
        // For positional arguments like {0}, {1}, {2}, we need max_index < args.len()
        // For sequential {} arguments, placeholder_count should match args.len()
        let max_index = parts.iter()
            .filter_map(|p| match p {
                FormatPart::Placeholder(idx) => Some(*idx),
                FormatPart::Literal(_) => None,
            })
            .max();

        if let Some(max_idx) = max_index {
            if max_idx >= args.len() {
                return Err(format!(
                    "format argument index {} is out of range (only {} arguments supplied)",
                    max_idx,
                    args.len()
                ));
            }
        }

        // Build the concatenated string expression
        let mut result_parts: Vec<Expr> = Vec::new();

        for part in parts {
            match part {
                FormatPart::Literal(s) => {
                    if !s.is_empty() {
                        result_parts.push(Expr::new(
                            ExprKind::Literal(LiteralValue::String(s)),
                            Type::str(),
                            span,
                        ));
                    }
                }
                FormatPart::Placeholder(idx) => {
                    // Use the placeholder index to get the correct argument
                    // This supports both sequential {} and explicit {0}, {1} syntax
                    if idx >= args.len() {
                        return Err(format!(
                            "positional argument {} is out of range (only {} arguments supplied)",
                            idx,
                            args.len()
                        ));
                    }
                    let arg = &args[idx];
                    let stringified = self.convert_to_string(arg.clone(), span)?;
                    result_parts.push(stringified);
                }
            }
        }

        // Concatenate all parts
        if result_parts.is_empty() {
            // Empty format string
            Ok(Expr::new(
                ExprKind::Literal(LiteralValue::String(String::new())),
                Type::str(),
                span,
            ))
        } else if result_parts.len() == 1 {
            Ok(result_parts.remove(0))
        } else {
            // Build chain of str_concat calls
            let mut result = result_parts.remove(0);
            for part in result_parts {
                result = self.make_str_concat(result, part, span)?;
            }
            Ok(result)
        }
    }

    /// Parse a format string into parts (literals and placeholders).
    fn parse_format_string(&self, format_str: &str) -> Result<Vec<FormatPart>, String> {
        let mut parts = Vec::new();
        let mut current_literal = String::new();
        let mut chars = format_str.chars().peekable();
        let mut placeholder_idx = 0;

        while let Some(c) = chars.next() {
            if c == '{' {
                if chars.peek() == Some(&'{') {
                    // Escaped {{ -> literal {
                    chars.next();
                    current_literal.push('{');
                } else if chars.peek() == Some(&'}') {
                    // {} placeholder
                    chars.next();
                    if !current_literal.is_empty() {
                        parts.push(FormatPart::Literal(std::mem::take(&mut current_literal)));
                    }
                    parts.push(FormatPart::Placeholder(placeholder_idx));
                    placeholder_idx += 1;
                } else {
                    // Check for positional or named arguments (not yet supported)
                    let mut placeholder_content = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == '}' {
                            chars.next();
                            break;
                        }
                        placeholder_content.push(ch);
                        chars.next();
                    }

                    // For now, only support empty {} or simple positional {0}, {1}
                    if placeholder_content.is_empty() {
                        if !current_literal.is_empty() {
                            parts.push(FormatPart::Literal(std::mem::take(&mut current_literal)));
                        }
                        parts.push(FormatPart::Placeholder(placeholder_idx));
                        placeholder_idx += 1;
                    } else if let Ok(idx) = placeholder_content.parse::<usize>() {
                        // Explicit positional argument {0}, {1}, etc.
                        if !current_literal.is_empty() {
                            parts.push(FormatPart::Literal(std::mem::take(&mut current_literal)));
                        }
                        parts.push(FormatPart::Placeholder(idx));
                    } else {
                        return Err(format!(
                            "unsupported format specifier: {{{}}}; only {{}} and {{n}} are supported",
                            placeholder_content
                        ));
                    }
                }
            } else if c == '}' {
                if chars.peek() == Some(&'}') {
                    // Escaped }} -> literal }
                    chars.next();
                    current_literal.push('}');
                } else {
                    return Err("unmatched '}' in format string".to_string());
                }
            } else {
                current_literal.push(c);
            }
        }

        if !current_literal.is_empty() {
            parts.push(FormatPart::Literal(current_literal));
        }

        Ok(parts)
    }

    /// Convert an expression to a string using the appropriate conversion function.
    fn convert_to_string(&mut self, expr: Expr, span: Span) -> Result<Expr, String> {
        use hir::{TypeKind, PrimitiveTy, IntTy, UintTy, FloatTy};

        let ty = &expr.ty;

        // Determine which conversion function to use based on the type
        let (conv_fn_name, needs_conversion) = match ty.kind() {
            TypeKind::Primitive(p) => match p {
                PrimitiveTy::Int(IntTy::I32) => ("int_to_string", true),
                PrimitiveTy::Int(IntTy::I64) => ("i64_to_string", true),
                PrimitiveTy::Uint(UintTy::U64) => ("u64_to_string", true),
                PrimitiveTy::Bool => ("bool_to_string", true),
                PrimitiveTy::Char => ("char_to_string", true),
                PrimitiveTy::Float(FloatTy::F32) => ("f32_to_string", true),
                PrimitiveTy::Float(FloatTy::F64) => ("f64_to_string", true),
                PrimitiveTy::Int(IntTy::I8) => {
                    return Err("i8 format arguments not yet supported".to_string());
                }
                PrimitiveTy::Int(IntTy::I16) => {
                    return Err("i16 format arguments not yet supported".to_string());
                }
                PrimitiveTy::Int(IntTy::I128) => {
                    return Err("i128 format arguments not yet supported".to_string());
                }
                PrimitiveTy::Int(IntTy::Isize) => {
                    // Treat isize as i64
                    ("i64_to_string", true)
                }
                PrimitiveTy::Uint(UintTy::U8) => {
                    return Err("u8 format arguments not yet supported".to_string());
                }
                PrimitiveTy::Uint(UintTy::U16) => {
                    return Err("u16 format arguments not yet supported".to_string());
                }
                PrimitiveTy::Uint(UintTy::U32) => {
                    return Err("u32 format arguments not yet supported".to_string());
                }
                PrimitiveTy::Uint(UintTy::U128) => {
                    return Err("u128 format arguments not yet supported".to_string());
                }
                PrimitiveTy::Uint(UintTy::Usize) => {
                    // Treat usize as u64
                    ("u64_to_string", true)
                }
                PrimitiveTy::Str => ("", false), // Already a string
                PrimitiveTy::String => ("", false), // Already a string
                PrimitiveTy::Unit => {
                    return Err("cannot format unit type".to_string());
                }
                PrimitiveTy::Never => {
                    return Err("cannot format never type".to_string());
                }
            },
            _ => {
                return Err(format!(
                    "cannot format value of type {}; only primitive types are supported",
                    ty
                ));
            }
        };

        if !needs_conversion {
            // Already a string, just return the expression
            return Ok(expr);
        }

        // Look up the conversion function
        let conv_def_id = match self.builtin_by_name.get(conv_fn_name) {
            Some(&id) => id,
            None => {
                return Err(format!("builtin function '{}' not found", conv_fn_name));
            }
        };

        // Build the conversion call
        let fn_expr = Expr::new(
            ExprKind::Def(conv_def_id),
            Type::function(vec![ty.clone()], Type::str()),
            span,
        );

        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(fn_expr),
                args: vec![expr],
            },
            Type::str(),
            span,
        ))
    }

    /// Create a str_concat call.
    fn make_str_concat(&mut self, left: Expr, right: Expr, span: Span) -> Result<Expr, String> {
        // Note: str_concat maps to blood_str_concat in the runtime
        let concat_def_id = match self.builtin_by_name.get("blood_str_concat") {
            Some(&id) => id,
            None => {
                return Err("builtin function 'str_concat' not found".to_string());
            }
        };

        let fn_expr = Expr::new(
            ExprKind::Def(concat_def_id),
            Type::function(vec![Type::str(), Type::str()], Type::str()),
            span,
        );

        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(fn_expr),
                args: vec![left, right],
            },
            Type::str(),
            span,
        ))
    }

    /// Create a println call with a pre-built string expression.
    fn make_println_expr_call(&mut self, str_expr: Expr, span: Span) -> ExprKind {
        if let Some(&def_id) = self.builtin_by_name.get("println_str") {
            let fn_expr = Expr::new(
                ExprKind::Def(def_id),
                Type::function(vec![Type::str()], Type::unit()),
                span,
            );

            ExprKind::Call {
                callee: Box::new(fn_expr),
                args: vec![str_expr],
            }
        } else {
            self.diagnostics.push(Diagnostic::error(
                "builtin function 'println_str' not found".to_string(),
                span,
            ));
            ExprKind::Error
        }
    }

    /// Create a print call with a pre-built string expression.
    fn make_print_expr_call(&mut self, str_expr: Expr, span: Span) -> ExprKind {
        if let Some(&def_id) = self.builtin_by_name.get("print_str") {
            let fn_expr = Expr::new(
                ExprKind::Def(def_id),
                Type::function(vec![Type::str()], Type::unit()),
                span,
            );

            ExprKind::Call {
                callee: Box::new(fn_expr),
                args: vec![str_expr],
            }
        } else {
            self.diagnostics.push(Diagnostic::error(
                "builtin function 'print_str' not found".to_string(),
                span,
            ));
            ExprKind::Error
        }
    }

    /// Create a panic call with a pre-built string expression.
    fn make_panic_expr_call(&mut self, str_expr: Expr, span: Span) -> ExprKind {
        if let Some(&def_id) = self.builtin_by_name.get("panic") {
            let fn_expr = Expr::new(
                ExprKind::Def(def_id),
                Type::function(vec![Type::str()], Type::never()),
                span,
            );

            ExprKind::Call {
                callee: Box::new(fn_expr),
                args: vec![str_expr],
            }
        } else {
            self.diagnostics.push(Diagnostic::error(
                "builtin function 'panic' not found".to_string(),
                span,
            ));
            ExprKind::Error
        }
    }

    /// Create a call to println_str(s).
    fn make_println_str_call(&mut self, s: &str, span: Span) -> ExprKind {
        if let Some(&def_id) = self.builtin_by_name.get("println_str") {
            self.make_str_call(def_id, s, span)
        } else {
            self.diagnostics.push(Diagnostic::error(
                "builtin function 'println_str' not found".to_string(),
                span,
            ));
            ExprKind::Error
        }
    }

    /// Create a call to print_str(s).
    fn make_print_str_call(&mut self, s: &str, span: Span) -> ExprKind {
        if let Some(&def_id) = self.builtin_by_name.get("print_str") {
            self.make_str_call(def_id, s, span)
        } else {
            self.diagnostics.push(Diagnostic::error(
                "builtin function 'print_str' not found".to_string(),
                span,
            ));
            ExprKind::Error
        }
    }

    /// Create a call to panic(s).
    fn make_panic_call(&mut self, s: &str, span: Span) -> ExprKind {
        if let Some(&def_id) = self.builtin_by_name.get("panic") {
            self.make_str_call(def_id, s, span)
        } else {
            self.diagnostics.push(Diagnostic::error(
                "builtin function 'panic' not found".to_string(),
                span,
            ));
            ExprKind::Error
        }
    }

    /// Create a call to a string-taking function.
    fn make_str_call(&self, fn_def_id: DefId, s: &str, span: Span) -> ExprKind {
        // Create the function reference
        let fn_expr = Expr::new(
            ExprKind::Def(fn_def_id),
            Type::function(vec![Type::str()], Type::unit()),
            span,
        );

        // Create the string literal argument
        let str_arg = Expr::new(
            ExprKind::Literal(LiteralValue::String(s.to_string())),
            Type::str(),
            span,
        );

        ExprKind::Call {
            callee: Box::new(fn_expr),
            args: vec![str_arg],
        }
    }

    /// Expand vec![...] literal to array literal.
    fn expand_vec_literal(
        &mut self,
        exprs: Vec<Expr>,
        _ty: &Type,
        _span: Span,
    ) -> ExprKind {
        // Expand all element expressions
        let expanded_exprs: Vec<Expr> = exprs.into_iter()
            .map(|e| self.expand_expr(e))
            .collect();

        // vec![1, 2, 3] expands to [1, 2, 3]
        ExprKind::Array(expanded_exprs)
    }

    /// Expand vec![value; count] repeat to array repeat expression.
    fn expand_vec_repeat(
        &mut self,
        value: Expr,
        count: Expr,
        _ty: &Type,
        _span: Span,
    ) -> ExprKind {
        let expanded_value = self.expand_expr(value);

        // Extract the count as a constant (typeck already validated it's a constant)
        let count_val = match &count.kind {
            ExprKind::Literal(LiteralValue::Int(n)) => *n as u64,
            _ => {
                // Fallback - shouldn't happen if typeck is correct
                0
            }
        };

        // vec![0; 10] expands to [0; 10]
        ExprKind::Repeat {
            value: Box::new(expanded_value),
            count: count_val,
        }
    }

    /// Expand assert!(condition) or assert!(condition, message).
    fn expand_assert(
        &mut self,
        condition: Expr,
        message: Option<Expr>,
        span: Span,
    ) -> ExprKind {
        let expanded_condition = self.expand_expr(condition);
        let expanded_message = message.map(|m| self.expand_expr(m));

        // assert!(cond) expands to:
        //   if !cond { panic("assertion failed") }
        //
        // assert!(cond, msg) expands to:
        //   if !cond { panic(msg) }

        // Create the negated condition: !condition
        let negated = Expr::new(
            ExprKind::Unary {
                op: crate::ast::UnaryOp::Not,
                operand: Box::new(expanded_condition),
            },
            Type::bool(),
            span,
        );

        // Create the panic call for the then branch
        let panic_msg = if let Some(msg_expr) = expanded_message {
            // Use the provided message
            msg_expr
        } else {
            // Default message
            Expr::new(
                ExprKind::Literal(LiteralValue::String("assertion failed".to_string())),
                Type::str(),
                span,
            )
        };

        let panic_call = if let Some(&panic_id) = self.builtin_by_name.get("panic") {
            let fn_expr = Expr::new(
                ExprKind::Def(panic_id),
                Type::function(vec![Type::str()], Type::never()),
                span,
            );
            Expr::new(
                ExprKind::Call {
                    callee: Box::new(fn_expr),
                    args: vec![panic_msg],
                },
                Type::never(),
                span,
            )
        } else {
            self.diagnostics.push(Diagnostic::error(
                "builtin function 'panic' not found for assert!".to_string(),
                span,
            ));
            return ExprKind::Error;
        };

        // Create the if expression: if !cond { panic(...) }
        // The else branch is unit (returns ())
        ExprKind::If {
            condition: Box::new(negated),
            then_branch: Box::new(Expr::new(
                ExprKind::Block {
                    stmts: vec![],
                    expr: Some(Box::new(panic_call)),
                },
                Type::never(),
                span,
            )),
            else_branch: Some(Box::new(Expr::new(
                ExprKind::Tuple(vec![]),
                Type::unit(),
                span,
            ))),
        }
    }

    /// Expand dbg!(expr).
    ///
    /// dbg!(expr) expands to:
    /// {
    ///     let __dbg_val = expr;
    ///     eprintln_str(convert_to_string(__dbg_val));
    ///     __dbg_val
    /// }
    ///
    /// This prints the value to stderr and returns it.
    fn expand_dbg(
        &mut self,
        inner: Expr,
        ty: &Type,
        span: Span,
    ) -> ExprKind {
        let expanded = self.expand_expr(inner);

        // Try to convert to string for printing
        match self.convert_to_string(expanded.clone(), span) {
            Ok(stringified) => {
                // Create the eprintln call
                let print_call = self.make_eprintln_expr_call(stringified, span);

                // Build a block that prints and returns the value:
                // { eprintln_str(str_val); expr }
                let print_stmt = hir::Stmt::Expr(Expr::new(
                    print_call,
                    Type::unit(),
                    span,
                ));

                ExprKind::Block {
                    stmts: vec![print_stmt],
                    expr: Some(Box::new(Expr::new(
                        expanded.kind,
                        ty.clone(),
                        span,
                    ))),
                }
            }
            Err(_) => {
                // If we can't convert to string, just return the expression
                // (for unsupported types, dbg! still evaluates but doesn't print)
                expanded.kind
            }
        }
    }

    /// Create an eprintln call with a pre-built string expression.
    fn make_eprintln_expr_call(&mut self, str_expr: Expr, span: Span) -> ExprKind {
        if let Some(&def_id) = self.builtin_by_name.get("eprintln_str") {
            let fn_expr = Expr::new(
                ExprKind::Def(def_id),
                Type::function(vec![Type::str()], Type::unit()),
                span,
            );

            ExprKind::Call {
                callee: Box::new(fn_expr),
                args: vec![str_expr],
            }
        } else {
            // Fall back to println_str if eprintln_str not available
            if let Some(&def_id) = self.builtin_by_name.get("println_str") {
                let fn_expr = Expr::new(
                    ExprKind::Def(def_id),
                    Type::function(vec![Type::str()], Type::unit()),
                    span,
                );

                ExprKind::Call {
                    callee: Box::new(fn_expr),
                    args: vec![str_expr],
                }
            } else {
                self.diagnostics.push(Diagnostic::error(
                    "builtin function 'eprintln_str' not found".to_string(),
                    span,
                ));
                ExprKind::Error
            }
        }
    }
}
