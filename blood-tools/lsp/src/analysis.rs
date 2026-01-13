//! Semantic Analysis for LSP
//!
//! Provides type information, symbol resolution, and navigation features
//! by integrating with the bloodc compiler.

use std::collections::HashMap;

use bloodc::ast::{self, Declaration, ExprKind, PatternKind, Statement};
use bloodc::hir;
use bloodc::{Parser, Span};
use bloodc::typeck;
use tower_lsp::lsp_types::*;

use crate::document::Document;

/// Analysis result from parsing and type-checking a document.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Symbols defined in the document with their locations and types.
    pub symbols: Vec<SymbolInfo>,
    /// Mapping from byte offset ranges to symbol info indices.
    pub symbol_at_offset: HashMap<usize, usize>,
    /// Inferred types for local variables, mapped from source span start to type string.
    /// Populated when type checking succeeds.
    pub inferred_types: HashMap<usize, String>,
}

/// Information about a symbol in the source code.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// The symbol name.
    pub name: String,
    /// The kind of symbol (function, variable, type, etc.).
    pub kind: SymbolKind,
    /// The span where the symbol is defined.
    pub def_span: Span,
    /// A human-readable description (type signature, etc.).
    pub description: String,
    /// Documentation comment, if any.
    pub doc: Option<String>,
    /// References to this symbol (spans where it's used).
    pub references: Vec<Span>,
}

/// Semantic analyzer that processes Blood source files.
pub struct SemanticAnalyzer;

impl SemanticAnalyzer {
    /// Creates a new semantic analyzer.
    pub fn new() -> Self {
        Self
    }

    /// Analyzes a document and returns symbol information.
    pub fn analyze(&self, doc: &Document) -> Option<AnalysisResult> {
        let text = doc.text();
        let mut parser = Parser::new(&text);

        let program = parser.parse_program().ok()?;
        let interner = parser.take_interner();

        let mut symbols = Vec::new();
        let mut symbol_at_offset = HashMap::new();
        let mut inferred_types = HashMap::new();

        // Collect symbols from declarations
        for decl in &program.declarations {
            self.collect_declaration_symbols(decl, &interner, &mut symbols, &mut symbol_at_offset);
        }

        // Attempt type checking to get inferred types
        // This may fail on incomplete code, which is fine - we just won't have inferred types
        if let Some(typed_result) = self.type_check_document(doc) {
            // Extract inferred types from the typed HIR
            for body in typed_result.bodies.values() {
                for local in &body.locals {
                    // Map the local's span start to its inferred type
                    let type_str = format!("{}", local.ty);
                    inferred_types.insert(local.span.start, type_str);
                }
            }

            // Update symbol descriptions with more precise types from type checking
            for symbol in &mut symbols {
                if let Some(type_str) = inferred_types.get(&symbol.def_span.start) {
                    if symbol.kind == SymbolKind::VARIABLE && symbol.description == "variable" {
                        symbol.description = format!("let {}: {}", symbol.name, type_str);
                    }
                }
            }
        }

        Some(AnalysisResult {
            symbols,
            symbol_at_offset,
            inferred_types,
        })
    }

    /// Attempts to type-check the document, returning the HIR crate if successful.
    fn type_check_document(&self, doc: &Document) -> Option<hir::Crate> {
        let text = doc.text();
        let mut parser = Parser::new(&text);

        let program = parser.parse_program().ok()?;
        let interner = parser.take_interner();

        // Run type checking - this may fail on incomplete code
        match typeck::check_program(&program, &text, interner) {
            Ok(hir_crate) => Some(hir_crate),
            Err(_) => None, // Type checking failed, but that's expected for incomplete code
        }
    }

    /// Collects symbols from a declaration.
    fn collect_declaration_symbols(
        &self,
        decl: &Declaration,
        interner: &string_interner::DefaultStringInterner,
        symbols: &mut Vec<SymbolInfo>,
        symbol_at_offset: &mut HashMap<usize, usize>,
    ) {
        match decl {
            Declaration::Function(fn_decl) => {
                let name = self.resolve_symbol(&fn_decl.name.node, interner);
                let params: Vec<String> = fn_decl.params.iter()
                    .map(|p| self.type_to_string(&p.ty, interner))
                    .collect();
                let ret = fn_decl.return_type.as_ref()
                    .map(|t| self.type_to_string(t, interner))
                    .unwrap_or_else(|| "()".to_string());

                let effects = fn_decl.effects.as_ref()
                    .map(|e| self.effect_row_to_string(e, interner))
                    .unwrap_or_default();

                let description = if effects.is_empty() {
                    format!("fn {}({}) -> {}", name, params.join(", "), ret)
                } else {
                    format!("fn {}({}) -> {} / {}", name, params.join(", "), ret, effects)
                };

                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::FUNCTION,
                    def_span: fn_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                // Map the function name span to this symbol
                for offset in fn_decl.name.span.start..fn_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }

                // Collect parameter symbols
                for param in &fn_decl.params {
                    self.collect_pattern_symbols(&param.pattern, interner, symbols, symbol_at_offset);
                }

                // Collect symbols from function body
                if let Some(body) = &fn_decl.body {
                    self.collect_block_symbols(body, interner, symbols, symbol_at_offset);
                }
            }
            Declaration::Struct(struct_decl) => {
                let name = self.resolve_symbol(&struct_decl.name.node, interner);
                let description = format!("struct {}", name);
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::STRUCT,
                    def_span: struct_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in struct_decl.name.span.start..struct_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }
            }
            Declaration::Enum(enum_decl) => {
                let name = self.resolve_symbol(&enum_decl.name.node, interner);
                let variants: Vec<String> = enum_decl.variants.iter()
                    .map(|v| self.resolve_symbol(&v.name.node, interner))
                    .collect();
                let description = format!("enum {} {{ {} }}", name, variants.join(", "));
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::ENUM,
                    def_span: enum_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in enum_decl.name.span.start..enum_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }

                // Add variant symbols
                for variant in &enum_decl.variants {
                    let variant_name = self.resolve_symbol(&variant.name.node, interner);
                    let variant_idx = symbols.len();
                    symbols.push(SymbolInfo {
                        name: variant_name,
                        kind: SymbolKind::ENUM_MEMBER,
                        def_span: variant.name.span,
                        description: format!("variant of {}", self.resolve_symbol(&enum_decl.name.node, interner)),
                        doc: None,
                        references: Vec::new(),
                    });

                    for offset in variant.name.span.start..variant.name.span.end {
                        symbol_at_offset.insert(offset, variant_idx);
                    }
                }
            }
            Declaration::Effect(effect_decl) => {
                let name = self.resolve_symbol(&effect_decl.name.node, interner);
                let ops: Vec<String> = effect_decl.operations.iter()
                    .map(|op| self.resolve_symbol(&op.name.node, interner))
                    .collect();
                let description = format!("effect {} {{ {} }}", name, ops.join(", "));
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::INTERFACE,
                    def_span: effect_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in effect_decl.name.span.start..effect_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }

                // Add operation symbols
                for op in &effect_decl.operations {
                    let op_name = self.resolve_symbol(&op.name.node, interner);
                    let params: Vec<String> = op.params.iter()
                        .map(|p| self.type_to_string(&p.ty, interner))
                        .collect();
                    let ret = self.type_to_string(&op.return_type, interner);

                    let op_idx = symbols.len();
                    symbols.push(SymbolInfo {
                        name: op_name.clone(),
                        kind: SymbolKind::METHOD,
                        def_span: op.name.span,
                        description: format!("op {}({}) -> {}", op_name, params.join(", "), ret),
                        doc: None,
                        references: Vec::new(),
                    });

                    for offset in op.name.span.start..op.name.span.end {
                        symbol_at_offset.insert(offset, op_idx);
                    }
                }
            }
            Declaration::Handler(handler_decl) => {
                let name = self.resolve_symbol(&handler_decl.name.node, interner);
                let effect_name = self.type_to_string(&handler_decl.effect, interner);
                let kind = match handler_decl.kind {
                    ast::HandlerKind::Deep => "deep",
                    ast::HandlerKind::Shallow => "shallow",
                };
                let description = format!("{} handler {} for {}", kind, name, effect_name);
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::CLASS,
                    def_span: handler_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in handler_decl.name.span.start..handler_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }
            }
            Declaration::Trait(trait_decl) => {
                let name = self.resolve_symbol(&trait_decl.name.node, interner);
                let description = format!("trait {}", name);
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::INTERFACE,
                    def_span: trait_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in trait_decl.name.span.start..trait_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }
            }
            Declaration::Type(type_decl) => {
                let name = self.resolve_symbol(&type_decl.name.node, interner);
                let aliased = self.type_to_string(&type_decl.ty, interner);
                let description = format!("type {} = {}", name, aliased);
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::TYPE_PARAMETER,
                    def_span: type_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in type_decl.name.span.start..type_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }
            }
            Declaration::Const(const_decl) => {
                let name = self.resolve_symbol(&const_decl.name.node, interner);
                let ty = self.type_to_string(&const_decl.ty, interner);
                let description = format!("const {}: {}", name, ty);
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::CONSTANT,
                    def_span: const_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in const_decl.name.span.start..const_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }
            }
            Declaration::Static(static_decl) => {
                let name = self.resolve_symbol(&static_decl.name.node, interner);
                let ty = self.type_to_string(&static_decl.ty, interner);
                let mut_str = if static_decl.is_mut { "mut " } else { "" };
                let description = format!("static {}{}: {}", mut_str, name, ty);
                let doc = self.extract_doc_comment(decl);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::VARIABLE,
                    def_span: static_decl.name.span,
                    description,
                    doc,
                    references: Vec::new(),
                });

                for offset in static_decl.name.span.start..static_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }
            }
            Declaration::Impl(impl_block) => {
                // Collect method symbols from impl block
                for item in &impl_block.items {
                    if let ast::ImplItem::Function(fn_decl) = item {
                        let name = self.resolve_symbol(&fn_decl.name.node, interner);
                        let params: Vec<String> = fn_decl.params.iter()
                            .map(|p| self.type_to_string(&p.ty, interner))
                            .collect();
                        let ret = fn_decl.return_type.as_ref()
                            .map(|t| self.type_to_string(t, interner))
                            .unwrap_or_else(|| "()".to_string());

                        let self_ty = self.type_to_string(&impl_block.self_ty, interner);
                        let description = format!("fn {}::{}({}) -> {}", self_ty, name, params.join(", "), ret);

                        let idx = symbols.len();
                        symbols.push(SymbolInfo {
                            name,
                            kind: SymbolKind::METHOD,
                            def_span: fn_decl.name.span,
                            description,
                            doc: None,
                            references: Vec::new(),
                        });

                        for offset in fn_decl.name.span.start..fn_decl.name.span.end {
                            symbol_at_offset.insert(offset, idx);
                        }
                    }
                }
            }
            Declaration::Bridge(bridge_decl) => {
                // Collect FFI function symbols from bridge block
                let bridge_name = self.resolve_symbol(&bridge_decl.name.node, interner);
                for item in &bridge_decl.items {
                    if let ast::BridgeItem::Function(fn_decl) = item {
                        let name = self.resolve_symbol(&fn_decl.name.node, interner);
                        let description = format!("extern \"{}\" fn {} (from bridge {})",
                            bridge_decl.language.node, name, bridge_name);

                        let idx = symbols.len();
                        symbols.push(SymbolInfo {
                            name,
                            kind: SymbolKind::FUNCTION,
                            def_span: fn_decl.name.span,
                            description,
                            doc: None,
                            references: Vec::new(),
                        });

                        for offset in fn_decl.name.span.start..fn_decl.name.span.end {
                            symbol_at_offset.insert(offset, idx);
                        }
                    }
                }
            }
        }
    }

    /// Collects symbols from a pattern (for let bindings and function params).
    fn collect_pattern_symbols(
        &self,
        pattern: &ast::Pattern,
        interner: &string_interner::DefaultStringInterner,
        symbols: &mut Vec<SymbolInfo>,
        symbol_at_offset: &mut HashMap<usize, usize>,
    ) {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                let var_name = self.resolve_symbol(&name.node, interner);
                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name: var_name,
                    kind: SymbolKind::VARIABLE,
                    def_span: name.span,
                    description: "variable".to_string(),
                    doc: None,
                    references: Vec::new(),
                });

                for offset in name.span.start..name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }
            }
            PatternKind::Tuple { fields, .. } => {
                for field in fields {
                    self.collect_pattern_symbols(field, interner, symbols, symbol_at_offset);
                }
            }
            PatternKind::Struct { fields, .. } => {
                for field in fields {
                    if let Some(pat) = &field.pattern {
                        self.collect_pattern_symbols(pat, interner, symbols, symbol_at_offset);
                    }
                }
            }
            PatternKind::TupleStruct { fields, .. } => {
                for field in fields {
                    self.collect_pattern_symbols(field, interner, symbols, symbol_at_offset);
                }
            }
            PatternKind::Ref { inner, .. } => {
                self.collect_pattern_symbols(inner, interner, symbols, symbol_at_offset);
            }
            PatternKind::Or(patterns) => {
                for pat in patterns {
                    self.collect_pattern_symbols(pat, interner, symbols, symbol_at_offset);
                }
            }
            PatternKind::Paren(inner) => {
                self.collect_pattern_symbols(inner, interner, symbols, symbol_at_offset);
            }
            PatternKind::Slice { elements, .. } => {
                for elem in elements {
                    self.collect_pattern_symbols(elem, interner, symbols, symbol_at_offset);
                }
            }
            PatternKind::Wildcard
            | PatternKind::Rest
            | PatternKind::Literal(_)
            | PatternKind::Path(_)
            | PatternKind::Range { .. } => {}
        }
    }

    /// Collects symbols from a block.
    fn collect_block_symbols(
        &self,
        block: &ast::Block,
        interner: &string_interner::DefaultStringInterner,
        symbols: &mut Vec<SymbolInfo>,
        symbol_at_offset: &mut HashMap<usize, usize>,
    ) {
        for stmt in &block.statements {
            match stmt {
                Statement::Let { pattern, ty, .. } => {
                    // For let bindings, we can add type info if available
                    if let PatternKind::Ident { name, .. } = &pattern.kind {
                        let var_name = self.resolve_symbol(&name.node, interner);
                        let type_info = ty.as_ref()
                            .map(|t| format!(": {}", self.type_to_string(t, interner)))
                            .unwrap_or_default();

                        let idx = symbols.len();
                        symbols.push(SymbolInfo {
                            name: var_name.clone(),
                            kind: SymbolKind::VARIABLE,
                            def_span: name.span,
                            description: format!("let {}{}", var_name, type_info),
                            doc: None,
                            references: Vec::new(),
                        });

                        for offset in name.span.start..name.span.end {
                            symbol_at_offset.insert(offset, idx);
                        }
                    } else {
                        self.collect_pattern_symbols(pattern, interner, symbols, symbol_at_offset);
                    }
                }
                Statement::Expr { expr, .. } => {
                    self.collect_expr_symbols(expr, interner, symbols, symbol_at_offset);
                }
                Statement::Item(decl) => {
                    self.collect_declaration_symbols(decl, interner, symbols, symbol_at_offset);
                }
            }
        }

        if let Some(expr) = &block.expr {
            self.collect_expr_symbols(expr, interner, symbols, symbol_at_offset);
        }
    }

    /// Collects symbols from an expression (for closures and nested items).
    fn collect_expr_symbols(
        &self,
        expr: &ast::Expr,
        interner: &string_interner::DefaultStringInterner,
        symbols: &mut Vec<SymbolInfo>,
        symbol_at_offset: &mut HashMap<usize, usize>,
    ) {
        match &expr.kind {
            ExprKind::Closure { params, body, .. } => {
                for param in params {
                    self.collect_pattern_symbols(&param.pattern, interner, symbols, symbol_at_offset);
                }
                self.collect_expr_symbols(body, interner, symbols, symbol_at_offset);
            }
            ExprKind::Block(block) => {
                self.collect_block_symbols(block, interner, symbols, symbol_at_offset);
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                self.collect_expr_symbols(condition, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(then_branch, interner, symbols, symbol_at_offset);
                if let Some(else_branch) = else_branch {
                    match else_branch {
                        ast::ElseBranch::Block(block) => {
                            self.collect_block_symbols(block, interner, symbols, symbol_at_offset);
                        }
                        ast::ElseBranch::If(if_expr) => {
                            self.collect_expr_symbols(if_expr, interner, symbols, symbol_at_offset);
                        }
                    }
                }
            }
            ExprKind::IfLet { pattern, scrutinee, then_branch, else_branch } => {
                self.collect_pattern_symbols(pattern, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(scrutinee, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(then_branch, interner, symbols, symbol_at_offset);
                if let Some(else_branch) = else_branch {
                    match else_branch {
                        ast::ElseBranch::Block(block) => {
                            self.collect_block_symbols(block, interner, symbols, symbol_at_offset);
                        }
                        ast::ElseBranch::If(if_expr) => {
                            self.collect_expr_symbols(if_expr, interner, symbols, symbol_at_offset);
                        }
                    }
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.collect_expr_symbols(scrutinee, interner, symbols, symbol_at_offset);
                for arm in arms {
                    self.collect_pattern_symbols(&arm.pattern, interner, symbols, symbol_at_offset);
                    self.collect_expr_symbols(&arm.body, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Loop { body, .. } | ExprKind::Unsafe(body) | ExprKind::Region { body, .. } => {
                self.collect_block_symbols(body, interner, symbols, symbol_at_offset);
            }
            ExprKind::While { condition, body, .. } => {
                self.collect_expr_symbols(condition, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(body, interner, symbols, symbol_at_offset);
            }
            ExprKind::WhileLet { pattern, scrutinee, body, .. } => {
                self.collect_pattern_symbols(pattern, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(scrutinee, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(body, interner, symbols, symbol_at_offset);
            }
            ExprKind::For { pattern, iter, body, .. } => {
                self.collect_pattern_symbols(pattern, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(iter, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(body, interner, symbols, symbol_at_offset);
            }
            ExprKind::WithHandle { handler, body } => {
                self.collect_expr_symbols(handler, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(body, interner, symbols, symbol_at_offset);
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_expr_symbols(left, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(right, interner, symbols, symbol_at_offset);
            }
            ExprKind::Unary { operand, .. } => {
                self.collect_expr_symbols(operand, interner, symbols, symbol_at_offset);
            }
            ExprKind::Call { callee, args, .. } => {
                self.collect_expr_symbols(callee, interner, symbols, symbol_at_offset);
                for arg in args {
                    self.collect_expr_symbols(&arg.value, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.collect_expr_symbols(receiver, interner, symbols, symbol_at_offset);
                for arg in args {
                    self.collect_expr_symbols(&arg.value, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Paren(base) => {
                self.collect_expr_symbols(base, interner, symbols, symbol_at_offset);
            }
            ExprKind::Index { base, index } => {
                self.collect_expr_symbols(base, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(index, interner, symbols, symbol_at_offset);
            }
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.collect_expr_symbols(elem, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Array(array) => {
                match array {
                    ast::ArrayExpr::List(elements) => {
                        for elem in elements {
                            self.collect_expr_symbols(elem, interner, symbols, symbol_at_offset);
                        }
                    }
                    ast::ArrayExpr::Repeat { value, count } => {
                        self.collect_expr_symbols(value, interner, symbols, symbol_at_offset);
                        self.collect_expr_symbols(count, interner, symbols, symbol_at_offset);
                    }
                }
            }
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let Some(value) = &field.value {
                        self.collect_expr_symbols(value, interner, symbols, symbol_at_offset);
                    }
                }
                if let Some(base) = base {
                    self.collect_expr_symbols(base, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Cast { expr, .. } | ExprKind::Return(Some(expr)) | ExprKind::Resume(expr) => {
                self.collect_expr_symbols(expr, interner, symbols, symbol_at_offset);
            }
            ExprKind::Assign { target, value } | ExprKind::AssignOp { target, value, .. } => {
                self.collect_expr_symbols(target, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(value, interner, symbols, symbol_at_offset);
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.collect_expr_symbols(start, interner, symbols, symbol_at_offset);
                }
                if let Some(end) = end {
                    self.collect_expr_symbols(end, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Break { value: Some(value), .. } => {
                self.collect_expr_symbols(value, interner, symbols, symbol_at_offset);
            }
            ExprKind::Perform { args, .. } => {
                for arg in args {
                    self.collect_expr_symbols(arg, interner, symbols, symbol_at_offset);
                }
            }
            // Terminal expressions with no nested symbols
            ExprKind::Literal(_)
            | ExprKind::Path(_)
            | ExprKind::Return(None)
            | ExprKind::Break { value: None, .. }
            | ExprKind::Continue { .. } => {}
        }
    }

    /// Resolves a symbol to its string representation.
    fn resolve_symbol(
        &self,
        symbol: &ast::Symbol,
        interner: &string_interner::DefaultStringInterner,
    ) -> String {
        use string_interner::Symbol as _;
        interner.resolve(*symbol)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("sym_{}", symbol.to_usize()))
    }

    /// Converts an AST type to a string.
    fn type_to_string(
        &self,
        ty: &ast::Type,
        interner: &string_interner::DefaultStringInterner,
    ) -> String {
        match &ty.kind {
            ast::TypeKind::Path(path) => {
                path.segments.iter()
                    .map(|seg| self.resolve_symbol(&seg.name.node, interner))
                    .collect::<Vec<_>>()
                    .join("::")
            }
            ast::TypeKind::Reference { mutable, inner, .. } => {
                let mut_str = if *mutable { "mut " } else { "" };
                format!("&{}{}", mut_str, self.type_to_string(inner, interner))
            }
            ast::TypeKind::Pointer { mutable, inner } => {
                let mut_str = if *mutable { "mut " } else { "const " };
                format!("*{}{}", mut_str, self.type_to_string(inner, interner))
            }
            ast::TypeKind::Array { element, .. } => {
                format!("[{}; _]", self.type_to_string(element, interner))
            }
            ast::TypeKind::Slice { element } => {
                format!("[{}]", self.type_to_string(element, interner))
            }
            ast::TypeKind::Tuple(elements) if elements.is_empty() => "()".to_string(),
            ast::TypeKind::Tuple(elements) => {
                let inner: Vec<_> = elements.iter()
                    .map(|t| self.type_to_string(t, interner))
                    .collect();
                format!("({})", inner.join(", "))
            }
            ast::TypeKind::Function { params, return_type, .. } => {
                let params_str: Vec<_> = params.iter()
                    .map(|t| self.type_to_string(t, interner))
                    .collect();
                format!("fn({}) -> {}", params_str.join(", "), self.type_to_string(return_type, interner))
            }
            ast::TypeKind::Never => "!".to_string(),
            ast::TypeKind::Infer => "_".to_string(),
            ast::TypeKind::Paren(inner) => self.type_to_string(inner, interner),
            ast::TypeKind::Record { fields, .. } => {
                let field_strs: Vec<_> = fields.iter()
                    .map(|f| format!("{}: {}",
                        self.resolve_symbol(&f.name.node, interner),
                        self.type_to_string(&f.ty, interner)))
                    .collect();
                format!("{{ {} }}", field_strs.join(", "))
            }
            ast::TypeKind::Ownership { qualifier, inner } => {
                let qual = match qualifier {
                    ast::OwnershipQualifier::Linear => "linear",
                    ast::OwnershipQualifier::Affine => "affine",
                };
                format!("{} {}", qual, self.type_to_string(inner, interner))
            }
            ast::TypeKind::Forall { params, body } => {
                let param_strs: Vec<_> = params.iter()
                    .map(|p| self.resolve_symbol(&p.node, interner))
                    .collect();
                format!("forall<{}>. {}", param_strs.join(", "), self.type_to_string(body, interner))
            }
        }
    }

    /// Converts an effect row to a string.
    fn effect_row_to_string(
        &self,
        row: &ast::EffectRow,
        interner: &string_interner::DefaultStringInterner,
    ) -> String {
        match &row.kind {
            ast::EffectRowKind::Pure => "pure".to_string(),
            ast::EffectRowKind::Var(name) => self.resolve_symbol(&name.node, interner),
            ast::EffectRowKind::Effects { effects, rest } => {
                let mut parts: Vec<String> = effects.iter()
                    .map(|e| self.type_to_string(e, interner))
                    .collect();
                if let Some(rest) = rest {
                    parts.push(format!("| {}", self.resolve_symbol(&rest.node, interner)));
                }
                format!("{{{}}}", parts.join(", "))
            }
        }
    }

    /// Extracts documentation comment from a declaration.
    fn extract_doc_comment(&self, _decl: &Declaration) -> Option<String> {
        // TODO: Parse doc comments from the source once we have trivia preservation
        None
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for hover information.
pub struct HoverProvider {
    analyzer: SemanticAnalyzer,
}

impl HoverProvider {
    /// Creates a new hover provider.
    pub fn new() -> Self {
        Self {
            analyzer: SemanticAnalyzer::new(),
        }
    }

    /// Provides hover information for a position in a document.
    pub fn hover(&self, doc: &Document, position: Position) -> Option<Hover> {
        let analysis = self.analyzer.analyze(doc)?;
        let offset = doc.position_to_offset(position)?;

        // Find symbol at offset
        let symbol_idx = analysis.symbol_at_offset.get(&offset)?;
        let symbol = analysis.symbols.get(*symbol_idx)?;

        let mut content = format!("```blood\n{}\n```", symbol.description);

        if let Some(doc_comment) = &symbol.doc {
            content.push_str("\n\n---\n\n");
            content.push_str(doc_comment);
        }

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: Some(self.span_to_range(&symbol.def_span, &doc.text())),
        })
    }

    /// Converts a span to an LSP range.
    fn span_to_range(&self, span: &Span, text: &str) -> Range {
        let start = self.offset_to_position(span.start, text);
        let end = self.offset_to_position(span.end, text);
        Range { start, end }
    }

    /// Converts a byte offset to an LSP position.
    fn offset_to_position(&self, offset: usize, text: &str) -> Position {
        let mut line = 0u32;
        let mut col = 0u32;
        let mut current = 0;

        for ch in text.chars() {
            if current >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            current += ch.len_utf8();
        }

        Position { line, character: col }
    }
}

impl Default for HoverProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for go-to-definition functionality.
pub struct DefinitionProvider {
    analyzer: SemanticAnalyzer,
}

impl DefinitionProvider {
    /// Creates a new definition provider.
    pub fn new() -> Self {
        Self {
            analyzer: SemanticAnalyzer::new(),
        }
    }

    /// Provides definition location for a symbol at a position.
    pub fn definition(&self, doc: &Document, position: Position) -> Option<Location> {
        let analysis = self.analyzer.analyze(doc)?;
        let text = doc.text();
        let offset = doc.position_to_offset(position)?;

        // Find symbol at offset
        let symbol_idx = analysis.symbol_at_offset.get(&offset)?;
        let symbol = analysis.symbols.get(*symbol_idx)?;

        let range = self.span_to_range(&symbol.def_span, &text);

        Some(Location {
            uri: doc.uri().clone(),
            range,
        })
    }

    /// Converts a span to an LSP range.
    fn span_to_range(&self, span: &Span, text: &str) -> Range {
        let start = self.offset_to_position(span.start, text);
        let end = self.offset_to_position(span.end, text);
        Range { start, end }
    }

    /// Converts a byte offset to an LSP position.
    fn offset_to_position(&self, offset: usize, text: &str) -> Position {
        let mut line = 0u32;
        let mut col = 0u32;
        let mut current = 0;

        for ch in text.chars() {
            if current >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            current += ch.len_utf8();
        }

        Position { line, character: col }
    }
}

impl Default for DefinitionProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for find references functionality.
pub struct ReferencesProvider {
    analyzer: SemanticAnalyzer,
}

impl ReferencesProvider {
    /// Creates a new references provider.
    pub fn new() -> Self {
        Self {
            analyzer: SemanticAnalyzer::new(),
        }
    }

    /// Finds all references to the symbol at a position.
    pub fn references(
        &self,
        doc: &Document,
        position: Position,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let analysis = self.analyzer.analyze(doc)?;
        let text = doc.text();
        let offset = doc.position_to_offset(position)?;

        // Find symbol at offset
        let symbol_idx = analysis.symbol_at_offset.get(&offset)?;
        let symbol = analysis.symbols.get(*symbol_idx)?;

        let mut locations = Vec::new();

        // Include the declaration if requested
        if include_declaration {
            locations.push(Location {
                uri: doc.uri().clone(),
                range: self.span_to_range(&symbol.def_span, &text),
            });
        }

        // Search for all occurrences of the symbol name in the document
        let name = &symbol.name;
        for (idx, _) in text.match_indices(name) {
            // Skip the declaration itself (already added if include_declaration)
            if include_declaration && idx == symbol.def_span.start {
                continue;
            }
            // Also skip if this is the definition span
            if !include_declaration && idx == symbol.def_span.start {
                continue;
            }

            // Simple word boundary check (ensure this is a whole word, not a substring)
            let before_char = if idx > 0 {
                text.as_bytes().get(idx - 1).copied()
            } else {
                None
            };
            let after_char = text.as_bytes().get(idx + name.len()).copied();

            let is_ident_char = |c: Option<u8>| -> bool {
                c.map(|c| c.is_ascii_alphanumeric() || c == b'_').unwrap_or(false)
            };

            if !is_ident_char(before_char) && !is_ident_char(after_char) {
                // Create a span with line/column info for this occurrence
                let pos = self.offset_to_position(idx, &text);
                let span = Span::new(
                    idx,
                    idx + name.len(),
                    pos.line + 1, // Span uses 1-indexed lines
                    pos.character + 1, // Span uses 1-indexed columns
                );
                locations.push(Location {
                    uri: doc.uri().clone(),
                    range: self.span_to_range(&span, &text),
                });
            }
        }

        // Remove duplicates (the declaration may appear twice)
        locations.sort_by_key(|l| (l.range.start.line, l.range.start.character));
        locations.dedup_by(|a, b| {
            a.range.start.line == b.range.start.line
                && a.range.start.character == b.range.start.character
        });

        if locations.is_empty() {
            None
        } else {
            Some(locations)
        }
    }

    /// Converts a span to an LSP range.
    fn span_to_range(&self, span: &Span, text: &str) -> Range {
        let start = self.offset_to_position(span.start, text);
        let end = self.offset_to_position(span.end, text);
        Range { start, end }
    }

    /// Converts a byte offset to an LSP position.
    fn offset_to_position(&self, offset: usize, text: &str) -> Position {
        let mut line = 0u32;
        let mut col = 0u32;
        let mut current = 0;

        for ch in text.chars() {
            if current >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            current += ch.len_utf8();
        }

        Position { line, character: col }
    }
}

impl Default for ReferencesProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for code completion functionality.
pub struct CompletionProvider {
    analyzer: SemanticAnalyzer,
}

impl CompletionProvider {
    /// Creates a new completion provider.
    pub fn new() -> Self {
        Self {
            analyzer: SemanticAnalyzer::new(),
        }
    }

    /// Provides code completions at a position.
    pub fn completions(&self, doc: &Document, position: Position) -> Vec<CompletionItem> {
        let mut items = Vec::new();
        let text = doc.text();

        // Get the current line and determine context
        let line_idx = position.line as usize;
        let lines: Vec<&str> = text.lines().collect();
        let current_line = lines.get(line_idx).unwrap_or(&"");
        let col = position.character as usize;
        let prefix = if col <= current_line.len() {
            &current_line[..col]
        } else {
            current_line
        };

        // Determine completion context
        let context = self.determine_context(prefix, &text, position);

        // Add keyword completions based on context
        items.extend(self.keyword_completions(&context));

        // Add symbol completions from the document
        if let Some(analysis) = self.analyzer.analyze(doc) {
            items.extend(self.symbol_completions(&analysis, &context, prefix));
        }

        // Sort by relevance and filter duplicates
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items.dedup_by(|a, b| a.label == b.label);

        items
    }

    /// Determines the completion context from the cursor position.
    fn determine_context(&self, prefix: &str, _text: &str, _position: Position) -> CompletionContext {
        let trimmed = prefix.trim();

        // Check for method/field access
        if trimmed.ends_with('.') || trimmed.contains(". ") {
            return CompletionContext::MemberAccess;
        }

        // Check for type context
        if trimmed.contains(": ") && !trimmed.contains("=")
            || trimmed.ends_with(':')
            || trimmed.ends_with("-> ")
        {
            return CompletionContext::Type;
        }

        // Check for effect context
        if trimmed.contains("/ ") || trimmed.ends_with('/') {
            return CompletionContext::Effect;
        }

        // Check for import context
        if trimmed.starts_with("use ") || trimmed.starts_with("import ") {
            return CompletionContext::Import;
        }

        // Check for pattern context (in match arms)
        if trimmed.contains("=>") || trimmed.ends_with("match ") || trimmed.contains("| ") {
            return CompletionContext::Pattern;
        }

        // Default to expression context
        CompletionContext::Expression
    }

    /// Provides keyword completions based on context.
    fn keyword_completions(&self, context: &CompletionContext) -> Vec<CompletionItem> {
        match context {
            CompletionContext::Expression => vec![
                self.keyword_item("fn", "Function declaration"),
                self.keyword_item("let", "Variable binding"),
                self.keyword_item("effect", "Effect declaration"),
                self.keyword_item("handler", "Effect handler"),
                self.keyword_item("perform", "Perform effect operation"),
                self.keyword_item("resume", "Resume continuation"),
                self.keyword_item("match", "Pattern matching"),
                self.keyword_item("if", "Conditional expression"),
                self.keyword_item("while", "While loop"),
                self.keyword_item("loop", "Infinite loop"),
                self.keyword_item("for", "For loop"),
                self.keyword_item("return", "Return from function"),
                self.keyword_item("break", "Break from loop"),
                self.keyword_item("continue", "Continue to next iteration"),
                self.keyword_item("struct", "Struct declaration"),
                self.keyword_item("enum", "Enum declaration"),
                self.keyword_item("trait", "Trait declaration"),
                self.keyword_item("impl", "Implementation block"),
                self.keyword_item("pub", "Public visibility"),
                self.keyword_item("true", "Boolean true"),
                self.keyword_item("false", "Boolean false"),
            ],
            CompletionContext::Type => vec![
                self.type_item("i32", "32-bit signed integer"),
                self.type_item("i64", "64-bit signed integer"),
                self.type_item("u32", "32-bit unsigned integer"),
                self.type_item("u64", "64-bit unsigned integer"),
                self.type_item("f32", "32-bit float"),
                self.type_item("f64", "64-bit float"),
                self.type_item("bool", "Boolean type"),
                self.type_item("char", "Unicode character"),
                self.type_item("String", "Owned string"),
                self.type_item("()", "Unit type"),
            ],
            CompletionContext::Effect => vec![
                self.keyword_item("pure", "Pure (no effects)"),
            ],
            CompletionContext::Pattern => vec![
                self.keyword_item("_", "Wildcard pattern"),
            ],
            CompletionContext::MemberAccess | CompletionContext::Import => vec![],
        }
    }

    /// Provides symbol completions from the document analysis.
    fn symbol_completions(
        &self,
        analysis: &AnalysisResult,
        context: &CompletionContext,
        prefix: &str,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        // Extract the identifier being typed
        let ident = prefix
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();

        for symbol in &analysis.symbols {
            // Filter by context
            let matches_context = match context {
                CompletionContext::Type => matches!(symbol.kind, SymbolKind::STRUCT | SymbolKind::ENUM | SymbolKind::TYPE_PARAMETER),
                CompletionContext::Expression => true, // All symbols can appear in expressions
                CompletionContext::Effect => symbol.description.contains("effect "),
                CompletionContext::Pattern => matches!(symbol.kind, SymbolKind::STRUCT | SymbolKind::ENUM),
                CompletionContext::MemberAccess => true, // Need type info to filter properly
                CompletionContext::Import => true,
            };

            if !matches_context {
                continue;
            }

            // Filter by prefix
            if !ident.is_empty() && !symbol.name.starts_with(&ident) {
                continue;
            }

            let kind = match symbol.kind {
                SymbolKind::FUNCTION => Some(CompletionItemKind::FUNCTION),
                SymbolKind::STRUCT => Some(CompletionItemKind::STRUCT),
                SymbolKind::ENUM => Some(CompletionItemKind::ENUM),
                SymbolKind::VARIABLE => Some(CompletionItemKind::VARIABLE),
                SymbolKind::CONSTANT => Some(CompletionItemKind::CONSTANT),
                SymbolKind::TYPE_PARAMETER => Some(CompletionItemKind::TYPE_PARAMETER),
                SymbolKind::FIELD => Some(CompletionItemKind::FIELD),
                SymbolKind::METHOD => Some(CompletionItemKind::METHOD),
                _ => Some(CompletionItemKind::TEXT),
            };

            items.push(CompletionItem {
                label: symbol.name.clone(),
                kind,
                detail: Some(symbol.description.clone()),
                documentation: symbol.doc.as_ref().map(|d| {
                    Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: d.clone(),
                    })
                }),
                ..Default::default()
            });
        }

        items
    }

    /// Creates a keyword completion item.
    fn keyword_item(&self, keyword: &str, description: &str) -> CompletionItem {
        CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(description.to_string()),
            ..Default::default()
        }
    }

    /// Creates a type completion item.
    fn type_item(&self, name: &str, description: &str) -> CompletionItem {
        CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some(description.to_string()),
            ..Default::default()
        }
    }
}

impl Default for CompletionProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Context for code completion.
#[derive(Debug, Clone, PartialEq)]
enum CompletionContext {
    /// Normal expression position.
    Expression,
    /// Type annotation position.
    Type,
    /// Effect annotation position.
    Effect,
    /// Pattern position (in match, let).
    Pattern,
    /// Member access (after `.`).
    MemberAccess,
    /// Import path.
    Import,
}
