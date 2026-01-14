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
            self.collect_declaration_symbols(decl, &text, &interner, &mut symbols, &mut symbol_at_offset);
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
        source: &str,
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

                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                    self.collect_block_symbols(body, source, interner, symbols, symbol_at_offset);
                }
            }
            Declaration::Struct(struct_decl) => {
                let name = self.resolve_symbol(&struct_decl.name.node, interner);
                let description = format!("struct {}", name);
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
                let doc = self.extract_doc_comment(source, self.get_decl_span_start(decl));

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
            Declaration::Module(mod_decl) => {
                // Collect module symbol
                let name = self.resolve_symbol(&mod_decl.name.node, interner);
                let description = format!("mod {}", name);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::MODULE,
                    def_span: mod_decl.name.span,
                    description,
                    doc: None,
                    references: Vec::new(),
                });

                for offset in mod_decl.name.span.start..mod_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
                }

                // Recursively collect symbols from inline module body
                if let Some(ref body) = mod_decl.body {
                    for inner_decl in body {
                        self.collect_declaration_symbols(inner_decl, source, interner, symbols, symbol_at_offset);
                    }
                }
            }
            Declaration::Macro(macro_decl) => {
                // Collect macro symbol
                let name = self.resolve_symbol(&macro_decl.name.node, interner);
                let description = format!("macro {}!", name);

                let idx = symbols.len();
                symbols.push(SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::FUNCTION, // Use FUNCTION as closest approximation for macros
                    def_span: macro_decl.name.span,
                    description,
                    doc: None,
                    references: Vec::new(),
                });

                for offset in macro_decl.name.span.start..macro_decl.name.span.end {
                    symbol_at_offset.insert(offset, idx);
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
        source: &str,
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
                    self.collect_expr_symbols(expr, source, interner, symbols, symbol_at_offset);
                }
                Statement::Item(decl) => {
                    self.collect_declaration_symbols(decl, source, interner, symbols, symbol_at_offset);
                }
            }
        }

        if let Some(expr) = &block.expr {
            self.collect_expr_symbols(expr, source, interner, symbols, symbol_at_offset);
        }
    }

    /// Collects symbols from an expression (for closures and nested items).
    fn collect_expr_symbols(
        &self,
        expr: &ast::Expr,
        source: &str,
        interner: &string_interner::DefaultStringInterner,
        symbols: &mut Vec<SymbolInfo>,
        symbol_at_offset: &mut HashMap<usize, usize>,
    ) {
        match &expr.kind {
            ExprKind::Closure { params, body, .. } => {
                for param in params {
                    self.collect_pattern_symbols(&param.pattern, interner, symbols, symbol_at_offset);
                }
                self.collect_expr_symbols(body, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Block(block) => {
                self.collect_block_symbols(block, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                self.collect_expr_symbols(condition, source, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(then_branch, source, interner, symbols, symbol_at_offset);
                if let Some(else_branch) = else_branch {
                    match else_branch {
                        ast::ElseBranch::Block(block) => {
                            self.collect_block_symbols(block, source, interner, symbols, symbol_at_offset);
                        }
                        ast::ElseBranch::If(if_expr) => {
                            self.collect_expr_symbols(if_expr, source, interner, symbols, symbol_at_offset);
                        }
                    }
                }
            }
            ExprKind::IfLet { pattern, scrutinee, then_branch, else_branch } => {
                self.collect_pattern_symbols(pattern, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(scrutinee, source, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(then_branch, source, interner, symbols, symbol_at_offset);
                if let Some(else_branch) = else_branch {
                    match else_branch {
                        ast::ElseBranch::Block(block) => {
                            self.collect_block_symbols(block, source, interner, symbols, symbol_at_offset);
                        }
                        ast::ElseBranch::If(if_expr) => {
                            self.collect_expr_symbols(if_expr, source, interner, symbols, symbol_at_offset);
                        }
                    }
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.collect_expr_symbols(scrutinee, source, interner, symbols, symbol_at_offset);
                for arm in arms {
                    self.collect_pattern_symbols(&arm.pattern, interner, symbols, symbol_at_offset);
                    self.collect_expr_symbols(&arm.body, source, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Loop { body, .. } | ExprKind::Unsafe(body) | ExprKind::Region { body, .. } => {
                self.collect_block_symbols(body, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::While { condition, body, .. } => {
                self.collect_expr_symbols(condition, source, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(body, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::WhileLet { pattern, scrutinee, body, .. } => {
                self.collect_pattern_symbols(pattern, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(scrutinee, source, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(body, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::For { pattern, iter, body, .. } => {
                self.collect_pattern_symbols(pattern, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(iter, source, interner, symbols, symbol_at_offset);
                self.collect_block_symbols(body, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::WithHandle { handler, body } => {
                self.collect_expr_symbols(handler, source, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(body, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::TryWith { body, handlers } => {
                self.collect_block_symbols(body, source, interner, symbols, symbol_at_offset);
                for handler in handlers {
                    self.collect_block_symbols(&handler.body, source, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_expr_symbols(left, source, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(right, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Unary { operand, .. } => {
                self.collect_expr_symbols(operand, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Call { callee, args, .. } => {
                self.collect_expr_symbols(callee, source, interner, symbols, symbol_at_offset);
                for arg in args {
                    self.collect_expr_symbols(&arg.value, source, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.collect_expr_symbols(receiver, source, interner, symbols, symbol_at_offset);
                for arg in args {
                    self.collect_expr_symbols(&arg.value, source, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Paren(base) => {
                self.collect_expr_symbols(base, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Index { base, index } => {
                self.collect_expr_symbols(base, source, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(index, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.collect_expr_symbols(elem, source, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Array(array) => {
                match array {
                    ast::ArrayExpr::List(elements) => {
                        for elem in elements {
                            self.collect_expr_symbols(elem, source, interner, symbols, symbol_at_offset);
                        }
                    }
                    ast::ArrayExpr::Repeat { value, count } => {
                        self.collect_expr_symbols(value, source, interner, symbols, symbol_at_offset);
                        self.collect_expr_symbols(count, source, interner, symbols, symbol_at_offset);
                    }
                }
            }
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let Some(value) = &field.value {
                        self.collect_expr_symbols(value, source, interner, symbols, symbol_at_offset);
                    }
                }
                if let Some(base) = base {
                    self.collect_expr_symbols(base, source, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Cast { expr, .. } | ExprKind::Return(Some(expr)) | ExprKind::Resume(expr) => {
                self.collect_expr_symbols(expr, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Assign { target, value } | ExprKind::AssignOp { target, value, .. } => {
                self.collect_expr_symbols(target, source, interner, symbols, symbol_at_offset);
                self.collect_expr_symbols(value, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.collect_expr_symbols(start, source, interner, symbols, symbol_at_offset);
                }
                if let Some(end) = end {
                    self.collect_expr_symbols(end, source, interner, symbols, symbol_at_offset);
                }
            }
            ExprKind::Break { value: Some(value), .. } => {
                self.collect_expr_symbols(value, source, interner, symbols, symbol_at_offset);
            }
            ExprKind::Perform { args, .. } => {
                for arg in args {
                    self.collect_expr_symbols(arg, source, interner, symbols, symbol_at_offset);
                }
            }
            // Macro call expressions - collect symbols from arguments
            ExprKind::MacroCall { kind, .. } => {
                match kind {
                    ast::MacroCallKind::Format { args, .. } => {
                        for arg in args {
                            self.collect_expr_symbols(arg, source, interner, symbols, symbol_at_offset);
                        }
                    }
                    ast::MacroCallKind::Vec(vec_args) => {
                        match vec_args {
                            ast::VecMacroArgs::List(exprs) => {
                                for e in exprs {
                                    self.collect_expr_symbols(e, source, interner, symbols, symbol_at_offset);
                                }
                            }
                            ast::VecMacroArgs::Repeat { value, count } => {
                                self.collect_expr_symbols(value, source, interner, symbols, symbol_at_offset);
                                self.collect_expr_symbols(count, source, interner, symbols, symbol_at_offset);
                            }
                        }
                    }
                    ast::MacroCallKind::Assert { condition, message } => {
                        self.collect_expr_symbols(condition, source, interner, symbols, symbol_at_offset);
                        if let Some(msg) = message {
                            self.collect_expr_symbols(msg, source, interner, symbols, symbol_at_offset);
                        }
                    }
                    ast::MacroCallKind::Dbg(inner) => {
                        self.collect_expr_symbols(inner, source, interner, symbols, symbol_at_offset);
                    }
                    ast::MacroCallKind::Matches { expr, pattern: _ } => {
                        // Collect symbols from the expression; patterns don't contribute new symbols
                        self.collect_expr_symbols(expr, source, interner, symbols, symbol_at_offset);
                    }
                    ast::MacroCallKind::Custom { .. } => {
                        // Custom macros are opaque - no symbols to collect
                    }
                }
            }
            // Terminal expressions with no nested symbols
            ExprKind::Literal(_)
            | ExprKind::Path(_)
            | ExprKind::Return(None)
            | ExprKind::Break { value: None, .. }
            | ExprKind::Continue { .. }
            | ExprKind::Default => {}
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
    ///
    /// Looks for `///` doc comments immediately preceding the declaration.
    /// Returns the concatenated doc comment content with leading `/// ` stripped.
    fn extract_doc_comment(&self, source: &str, decl_span_start: usize) -> Option<String> {
        // Find the line containing the declaration start
        let prefix = &source[..decl_span_start];

        // Split into lines, keeping track of their positions
        let lines: Vec<&str> = prefix.lines().collect();
        if lines.is_empty() {
            return None;
        }

        // Walk backwards from the last line before the declaration
        // collecting consecutive doc comment lines
        let mut doc_lines: Vec<&str> = Vec::new();

        for line in lines.iter().rev() {
            let trimmed = line.trim();

            // Check for doc comment (/// ...)
            if let Some(content) = trimmed.strip_prefix("///") {
                // Strip the leading space if present
                let content = content.strip_prefix(' ').unwrap_or(content);
                doc_lines.push(content);
            } else if trimmed.is_empty() {
                // Empty lines between doc comments and declaration are OK
                // but we stop if we already have doc lines
                if !doc_lines.is_empty() {
                    // Check if we've hit a non-doc-comment line
                    continue;
                }
            } else {
                // Non-doc-comment, non-empty line - stop
                break;
            }
        }

        if doc_lines.is_empty() {
            return None;
        }

        // Reverse since we collected bottom-up
        doc_lines.reverse();
        Some(doc_lines.join("\n"))
    }

    /// Gets the span start for a declaration.
    ///
    /// Returns the byte offset of the declaration's name (or start position),
    /// which is used to find doc comments preceding the declaration.
    fn get_decl_span_start(&self, decl: &Declaration) -> usize {
        // Use the declaration's overall span start, which is where doc comments
        // would be located (just before the declaration begins)
        decl.span().start
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
        let text = doc.text();
        let offset = doc.position_to_offset(position)?;

        // First, check if we're on a keyword or builtin
        if let Some(hover) = self.keyword_hover(&text, offset) {
            return Some(hover);
        }

        // Otherwise, look up symbol at offset
        let analysis = self.analyzer.analyze(doc)?;
        let symbol_idx = analysis.symbol_at_offset.get(&offset)?;
        let symbol = analysis.symbols.get(*symbol_idx)?;

        let mut content = format!("```blood\n{}\n```", symbol.description);

        if let Some(doc_comment) = &symbol.doc {
            content.push_str("\n\n---\n\n");
            content.push_str(doc_comment);
        }

        // Add examples for user-defined types based on their kind
        if let Some(example) = self.symbol_example(&symbol) {
            content.push_str("\n\n---\n\n**Example:**\n\n");
            content.push_str(&example);
        }

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: Some(self.span_to_range(&symbol.def_span, &text)),
        })
    }

    /// Provides hover for keywords and built-in constructs.
    fn keyword_hover(&self, text: &str, offset: usize) -> Option<Hover> {
        // Extract the word at the offset
        let word = self.extract_word_at_offset(text, offset)?;
        let word_start = self.find_word_start(text, offset)?;

        // Get documentation for keywords and builtins
        let (title, description, example) = match word.as_str() {
            "fn" => (
                "fn",
                "Declares a function with optional effect annotations.",
                r#"```blood
fn greet(name: str) -> str {
    format!("Hello, {}!", name)
}

// With effects
fn read_file(path: str) -> str / IO {
    // ...
}
```"#,
            ),
            "effect" => (
                "effect",
                "Declares an algebraic effect with operations that can be performed.",
                r#"```blood
effect State<T> {
    fn get() -> T;
    fn put(value: T);
}

// Usage
fn counter() -> i32 / State<i32> {
    let current = perform State::get();
    perform State::put(current + 1);
    current
}
```"#,
            ),
            "handler" => (
                "handler",
                "Declares an effect handler that provides implementations for effect operations.",
                r#"```blood
handler StateHandler<T> for State<T> {
    state: T,

    fn get() -> T {
        resume(self.state)
    }

    fn put(value: T) {
        self.state = value;
        resume(())
    }
}

// Usage
with StateHandler { state: 0 } {
    counter()
}
```"#,
            ),
            "perform" => (
                "perform",
                "Performs an effect operation, suspending the current computation until a handler provides a value.",
                r#"```blood
effect Log {
    fn log(message: str);
}

fn example() / Log {
    perform Log::log("Starting...");
    // computation continues after handler resumes
}
```"#,
            ),
            "resume" => (
                "resume",
                "Resumes a suspended computation within an effect handler, optionally passing a value.",
                r#"```blood
handler Logger for Log {
    fn log(message: str) {
        println!("{}", message);
        resume(())  // Resume with unit value
    }
}
```"#,
            ),
            "with" => (
                "with",
                "Runs a computation with a specific effect handler installed.",
                r#"```blood
let result = with MyHandler { state: 0 } {
    perform_some_operation()
};
```"#,
            ),
            "struct" => (
                "struct",
                "Declares a composite data type with named fields.",
                r#"```blood
struct Point {
    x: f64,
    y: f64,
}

// With generics
struct Pair<T, U> {
    first: T,
    second: U,
}

// Usage
let p = Point { x: 1.0, y: 2.0 };
```"#,
            ),
            "enum" => (
                "enum",
                "Declares a tagged union type with variants.",
                r#"```blood
enum Option<T> {
    Some(T),
    None,
}

enum Result<T, E> {
    Ok(T),
    Err(E),
}

// Pattern matching
match value {
    Option::Some(x) => println!("{}", x),
    Option::None => println!("no value"),
}
```"#,
            ),
            "match" => (
                "match",
                "Pattern matches a value against multiple patterns, executing the corresponding arm.",
                r#"```blood
match value {
    0 => println!("zero"),
    1..=9 => println!("single digit"),
    n if n < 0 => println!("negative"),
    _ => println!("other"),
}

// Destructuring
match point {
    Point { x: 0, y } => println!("on y-axis at {}", y),
    Point { x, y: 0 } => println!("on x-axis at {}", x),
    Point { x, y } => println!("at ({}, {})", x, y),
}
```"#,
            ),
            "let" => (
                "let",
                "Binds a value to a pattern, with optional type annotation.",
                r#"```blood
let x = 42;
let y: f64 = 3.14;
let (a, b) = (1, 2);
let Point { x, y } = point;
let mut counter = 0;
```"#,
            ),
            "if" => (
                "if",
                "Conditional expression that evaluates one of two branches.",
                r#"```blood
if condition {
    // then branch
} else {
    // else branch
}

// As expression
let result = if x > 0 { "positive" } else { "non-positive" };

// If-let for pattern matching
if let Some(value) = option {
    println!("{}", value);
}
```"#,
            ),
            "loop" => (
                "loop",
                "Creates an infinite loop that can be exited with `break`.",
                r#"```blood
loop {
    if should_stop() {
        break;
    }
}

// With value
let result = loop {
    if done() {
        break computed_value;
    }
};
```"#,
            ),
            "while" => (
                "while",
                "Loops while a condition is true.",
                r#"```blood
while condition {
    // loop body
}

// While-let for iteration
while let Some(item) = iter.next() {
    process(item);
}
```"#,
            ),
            "for" => (
                "for",
                "Iterates over elements of an iterator.",
                r#"```blood
for item in collection {
    process(item);
}

for i in 0..10 {
    println!("{}", i);
}

for (idx, value) in items.iter().enumerate() {
    println!("{}: {}", idx, value);
}
```"#,
            ),
            "trait" => (
                "trait",
                "Declares a trait (interface) that types can implement.",
                r#"```blood
trait Display {
    fn display(self) -> str;
}

impl Display for Point {
    fn display(self) -> str {
        format!("({}, {})", self.x, self.y)
    }
}
```"#,
            ),
            "impl" => (
                "impl",
                "Implements methods for a type, or implements a trait for a type.",
                r#"```blood
impl Point {
    fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn distance(self, other: Point) -> f64 {
        // ...
    }
}

impl Display for Point {
    fn display(self) -> str {
        format!("({}, {})", self.x, self.y)
    }
}
```"#,
            ),
            "println" | "println!" => (
                "println!",
                "Prints formatted output to stdout with a newline.",
                r#"```blood
println!("Hello, world!");
println!("x = {}", x);
println!("{} + {} = {}", a, b, a + b);
```"#,
            ),
            "print" | "print!" => (
                "print!",
                "Prints formatted output to stdout without a newline.",
                r#"```blood
print!("Loading...");
// do work
println!(" done!");
```"#,
            ),
            "format" | "format!" => (
                "format!",
                "Creates a formatted string.",
                r#"```blood
let msg = format!("Hello, {}!", name);
let coords = format!("({}, {})", x, y);
```"#,
            ),
            "eprintln" | "eprintln!" => (
                "eprintln!",
                "Prints formatted output to stderr with a newline.",
                r#"```blood
eprintln!("Error: {}", message);
```"#,
            ),
            "dbg" | "dbg!" => (
                "dbg!",
                "Debug macro that prints the expression and its value to stderr, then returns the value.",
                r#"```blood
let x = dbg!(compute_value());
// Prints: compute_value() = 42 (or similar)
// x is now 42
```"#,
            ),
            "vec" | "vec!" => (
                "vec!",
                "Creates an array from a list of elements or repeated values.",
                r#"```blood
let nums = vec![1, 2, 3, 4, 5];
let zeros = vec![0; 10];  // 10 zeros
```"#,
            ),
            "assert" | "assert!" => (
                "assert!",
                "Panics if the condition is false.",
                r#"```blood
assert!(x > 0);
assert!(result.is_ok(), "Operation failed");
```"#,
            ),
            "return" => (
                "return",
                "Returns a value from the current function.",
                r#"```blood
fn find(items: [i32], target: i32) -> bool {
    for item in items {
        if item == target {
            return true;
        }
    }
    false
}
```"#,
            ),
            "break" => (
                "break",
                "Exits the current loop, optionally with a value.",
                r#"```blood
loop {
    if done() {
        break;
    }
}

let result = loop {
    if found() {
        break value;
    }
};
```"#,
            ),
            "continue" => (
                "continue",
                "Skips to the next iteration of a loop.",
                r#"```blood
for i in 0..10 {
    if i % 2 == 0 {
        continue;
    }
    println!("{}", i);  // prints odd numbers
}
```"#,
            ),
            "pub" => (
                "pub",
                "Makes an item publicly visible outside its module.",
                r#"```blood
pub fn public_function() { }
pub struct PublicType { }
pub mod public_module { }
```"#,
            ),
            "mod" => (
                "mod",
                "Declares a module.",
                r#"```blood
mod utils {
    pub fn helper() { }
}

// Use items from module
utils::helper();
```"#,
            ),
            "use" => (
                "use",
                "Imports items from a module into the current scope.",
                r#"```blood
use std::io;
use std::collections::{HashMap, HashSet};
use crate::utils::helper;
```"#,
            ),
            "unsafe" => (
                "unsafe",
                "Marks a block or function as unsafe, allowing raw pointer operations.",
                r#"```blood
unsafe {
    *raw_ptr = value;
}

unsafe fn dangerous() { }
```"#,
            ),
            "pure" => (
                "pure",
                "Effect annotation indicating a function has no side effects.",
                r#"```blood
fn add(a: i32, b: i32) -> i32 / pure {
    a + b
}

// Pure functions can be freely memoized, reordered, or eliminated
```"#,
            ),
            "linear" => (
                "linear",
                "Type modifier indicating a value must be used exactly once.",
                r#"```blood
// Linear types prevent resource leaks and double-free errors
fn open_file(path: str) -> linear File / IO {
    // ...
}

// Must use the file exactly once
let file = open_file("data.txt");
write(file, "content");  // file consumed here
```"#,
            ),
            "mut" => (
                "mut",
                "Keyword for declaring mutable bindings or references.",
                r#"```blood
let mut counter = 0;
counter = counter + 1;

fn increment(x: &mut i32) {
    *x = *x + 1;
}
```"#,
            ),
            "const" => (
                "const",
                "Declares a compile-time constant.",
                r#"```blood
const MAX_SIZE: usize = 1024;
const PI: f64 = 3.14159265359;
const GREETING: str = "Hello";
```"#,
            ),
            "static" => (
                "static",
                "Declares a static variable with a fixed memory location.",
                r#"```blood
static mut COUNTER: i32 = 0;

// Access requires unsafe block
unsafe {
    COUNTER = COUNTER + 1;
}
```"#,
            ),
            "type" => (
                "type",
                "Declares a type alias.",
                r#"```blood
type Point = (f64, f64);
type Result<T> = std::Result<T, Error>;
type Callback = fn(i32) -> bool;
```"#,
            ),
            "bridge" => (
                "bridge",
                "Declares an FFI bridge block for foreign function interfaces.",
                r#"```blood
bridge "C" libc {
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
    fn strlen(s: *const u8) -> usize;
}

// Usage
let ptr = @unsafe { libc::malloc(1024) };
```"#,
            ),
            "extern" => (
                "extern",
                "Declares an external function from another language.",
                r#"```blood
extern "C" fn external_callback(data: *const u8);

// Import from C library
bridge "C" {
    extern fn puts(s: *const u8) -> i32;
}
```"#,
            ),
            "async" => (
                "async",
                "Marks a function as asynchronous, returning a future.",
                r#"```blood
async fn fetch_data(url: str) -> Result<str, Error> {
    let response = await http::get(url)?;
    response.text()
}

// Usage
let data = await fetch_data("https://example.com");
```"#,
            ),
            "await" => (
                "await",
                "Suspends execution until an async operation completes.",
                r#"```blood
async fn process() -> Result<(), Error> {
    let data = await fetch_data(url)?;
    let parsed = await parse_json(data)?;
    Ok(())
}
```"#,
            ),
            "where" => (
                "where",
                "Specifies trait bounds on generic type parameters.",
                r#"```blood
fn print_all<T>(items: [T])
where
    T: Display,
{
    for item in items {
        println!("{}", item.display());
    }
}
```"#,
            ),
            "as" => (
                "as",
                "Type casting operator.",
                r#"```blood
let x: i64 = 42;
let y = x as f64;  // Convert to float
let ptr = &x as *const i64;  // Convert to raw pointer
```"#,
            ),
            "self" => (
                "self",
                "Refers to the current instance in methods.",
                r#"```blood
impl Point {
    fn distance(self, other: Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    fn translate(&mut self, dx: f64, dy: f64) {
        self.x = self.x + dx;
        self.y = self.y + dy;
    }
}
```"#,
            ),
            "Self" => (
                "Self",
                "Refers to the implementing type in trait or impl blocks.",
                r#"```blood
impl Point {
    fn origin() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

trait Clone {
    fn clone(&self) -> Self;
}
```"#,
            ),
            "freeze" => (
                "freeze",
                "Creates an immutable, frozen copy of a value.",
                r#"```blood
let data = vec![1, 2, 3];
let frozen: Frozen<Vec<i32>> = freeze(data);

// frozen is deeply immutable and can be safely shared
```"#,
            ),
            "Frozen" => (
                "Frozen<T>",
                "A wrapper type indicating deep immutability.",
                r#"```blood
// Frozen values can be safely shared across threads
fn share_data(data: Frozen<Config>) -> impl Fn() {
    move || {
        println!("{}", data.name);
    }
}
```"#,
            ),
            "true" | "false" => (
                "bool",
                "Boolean literal values.",
                r#"```blood
let enabled = true;
let disabled = false;

if enabled && !disabled {
    // ...
}
```"#,
            ),
            "None" => (
                "None",
                "The absence of a value in an Option type.",
                r#"```blood
let maybe_value: Option<i32> = None;

match maybe_value {
    Some(x) => println!("Got {}", x),
    None => println!("No value"),
}
```"#,
            ),
            "Some" => (
                "Some",
                "Wraps a value in an Option type.",
                r#"```blood
let maybe_value: Option<i32> = Some(42);

if let Some(x) = maybe_value {
    println!("Got {}", x);
}
```"#,
            ),
            "Ok" => (
                "Ok",
                "Represents success in a Result type.",
                r#"```blood
fn divide(a: i32, b: i32) -> Result<i32, str> {
    if b == 0 {
        Err("division by zero")
    } else {
        Ok(a / b)
    }
}
```"#,
            ),
            "Err" => (
                "Err",
                "Represents an error in a Result type.",
                r#"```blood
fn parse_int(s: str) -> Result<i32, ParseError> {
    if s.is_empty() {
        Err(ParseError::Empty)
    } else {
        // ...
    }
}
```"#,
            ),
            "deep" | "shallow" => (
                "handler depth",
                "Handler depth determines how effect operations propagate.",
                r#"```blood
// Deep handler: intercepts effects recursively
deep handler Logger for Log {
    fn log(msg: str) {
        println!("[LOG] {}", msg);
        resume(())
    }
}

// Shallow handler: only handles the first effect
shallow handler Once for Ask {
    fn ask() -> i32 {
        resume(42)  // Only handles first ask
    }
}
```"#,
            ),
            _ => return None,
        };

        let word_end = word_start + word.len();
        let range = Range {
            start: self.offset_to_position(word_start, text),
            end: self.offset_to_position(word_end, text),
        };

        let content = format!(
            "```blood\n{}\n```\n\n{}\n\n---\n\n**Example:**\n\n{}",
            title, description, example
        );

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: Some(range),
        })
    }

    /// Extracts the word at the given offset.
    fn extract_word_at_offset(&self, text: &str, offset: usize) -> Option<String> {
        if offset >= text.len() {
            return None;
        }

        let bytes = text.as_bytes();

        // Find word start
        let mut start = offset;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_' || bytes[start - 1] == b'!') {
            start -= 1;
        }

        // Find word end
        let mut end = offset;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'!') {
            end += 1;
        }

        if start == end {
            return None;
        }

        Some(text[start..end].to_string())
    }

    /// Finds the start of the word at the given offset.
    fn find_word_start(&self, text: &str, offset: usize) -> Option<usize> {
        if offset >= text.len() {
            return None;
        }

        let bytes = text.as_bytes();
        let mut start = offset;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_' || bytes[start - 1] == b'!') {
            start -= 1;
        }

        Some(start)
    }

    /// Provides example code for user-defined symbols.
    fn symbol_example(&self, symbol: &SymbolInfo) -> Option<String> {
        match symbol.kind {
            SymbolKind::FUNCTION => {
                Some(format!(
                    "```blood\n// Call the function\nlet result = {}(args);\n```",
                    symbol.name
                ))
            }
            SymbolKind::STRUCT => {
                Some(format!(
                    "```blood\n// Create an instance\nlet instance = {} {{ /* fields */ }};\n```",
                    symbol.name
                ))
            }
            SymbolKind::ENUM => {
                Some(format!(
                    "```blood\n// Use a variant\nlet value = {}::VariantName;\n\n// Pattern match\nmatch value {{\n    {}::VariantName => {{ /* ... */ }}\n}}\n```",
                    symbol.name, symbol.name
                ))
            }
            SymbolKind::INTERFACE if symbol.description.contains("effect ") => {
                Some(format!(
                    "```blood\n// Perform an operation\nperform {}::operation();\n```",
                    symbol.name
                ))
            }
            SymbolKind::CLASS if symbol.description.contains("handler ") => {
                Some(format!(
                    "```blood\n// Use the handler\nwith {} {{ /* state */ }} {{\n    // computation\n}}\n```",
                    symbol.name
                ))
            }
            _ => None,
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

        // First, try to find a direct symbol at offset
        if let Some(symbol_idx) = analysis.symbol_at_offset.get(&offset) {
            if let Some(symbol) = analysis.symbols.get(*symbol_idx) {
                let range = self.span_to_range(&symbol.def_span, &text);
                return Some(Location {
                    uri: doc.uri().clone(),
                    range,
                });
            }
        }

        // Try to resolve qualified paths (e.g., Effect::operation)
        if let Some(location) = self.find_qualified_definition(doc, &text, offset, &analysis) {
            return Some(location);
        }

        // Try to find effect operation references in perform expressions
        if let Some(location) = self.find_effect_operation_definition(doc, &text, offset, &analysis) {
            return Some(location);
        }

        None
    }

    /// Finds definition for qualified paths like `Effect::operation`.
    fn find_qualified_definition(
        &self,
        doc: &Document,
        text: &str,
        offset: usize,
        analysis: &AnalysisResult,
    ) -> Option<Location> {
        // Extract the word and check if it's part of a qualified path
        let word = self.extract_word_at_offset(text, offset)?;

        // Look backwards for `::`  to find if this is part of a qualified name
        let before_offset = self.find_word_start(text, offset)?;

        // Check if there's `::` before this word
        if before_offset >= 2 {
            let potential_colons = &text[before_offset.saturating_sub(2)..before_offset];
            if potential_colons == "::" {
                // Find the qualifier (effect/struct/enum name) before ::
                let qualifier_end = before_offset - 2;
                if let Some(qualifier) = self.extract_word_ending_at(text, qualifier_end) {
                    // Look for the operation/method within the qualified type
                    return self.find_member_definition(doc, &qualifier, &word, analysis);
                }
            }
        }

        // Check if there's `::` after this word (cursor is on the qualifier)
        let word_end = before_offset + word.len();
        if word_end + 2 <= text.len() {
            let potential_colons = &text[word_end..word_end + 2];
            if potential_colons == "::" {
                // Find the member name after ::
                if let Some(member) = self.extract_word_starting_at(text, word_end + 2) {
                    return self.find_member_definition(doc, &word, &member, analysis);
                }
            }
        }

        None
    }

    /// Finds a member (operation, variant, method) definition within a type.
    fn find_member_definition(
        &self,
        doc: &Document,
        qualifier: &str,
        member: &str,
        analysis: &AnalysisResult,
    ) -> Option<Location> {
        // Search for a symbol that matches the member name and is associated with the qualifier
        for symbol in &analysis.symbols {
            // Check for effect operations
            if symbol.name == member {
                if symbol.description.contains(&format!("op {}(", member)) {
                    // This is an effect operation, check if it belongs to the right effect
                    // Look for the parent effect symbol
                    for parent in &analysis.symbols {
                        if parent.name == qualifier && parent.description.contains("effect ") {
                            // Found the effect, return the operation location
                            return Some(Location {
                                uri: doc.uri().clone(),
                                range: self.span_to_range(&symbol.def_span, &doc.text()),
                            });
                        }
                    }
                }

                // Check for enum variants
                if symbol.description.contains(&format!("variant of {}", qualifier)) {
                    return Some(Location {
                        uri: doc.uri().clone(),
                        range: self.span_to_range(&symbol.def_span, &doc.text()),
                    });
                }

                // Check for methods (impl methods)
                if symbol.description.contains(&format!("{}::{}", qualifier, member)) {
                    return Some(Location {
                        uri: doc.uri().clone(),
                        range: self.span_to_range(&symbol.def_span, &doc.text()),
                    });
                }
            }
        }

        None
    }

    /// Finds effect operation definitions from perform expressions.
    fn find_effect_operation_definition(
        &self,
        doc: &Document,
        text: &str,
        offset: usize,
        analysis: &AnalysisResult,
    ) -> Option<Location> {
        // Look for `perform` keyword before the offset to detect perform expressions
        let search_start = offset.saturating_sub(100); // Look back up to 100 chars
        let search_text = &text[search_start..offset.min(text.len())];

        // Check if we're in a perform expression
        if let Some(perform_pos) = search_text.rfind("perform ") {
            let perform_abs_pos = search_start + perform_pos;
            let after_perform = &text[perform_abs_pos + 8..];

            // Extract the effect::operation part
            // Pattern: perform Effect::operation(...)
            if let Some(paren_pos) = after_perform.find('(') {
                let path = after_perform[..paren_pos].trim();
                if let Some(colon_pos) = path.find("::") {
                    let effect_name = path[..colon_pos].trim();
                    let op_name = path[colon_pos + 2..].trim();

                    // Check if offset is within this path
                    let path_start = perform_abs_pos + 8;
                    let path_end = path_start + path.len();

                    if offset >= path_start && offset <= path_end + 1 {
                        // Try to find the operation definition
                        return self.find_member_definition(doc, effect_name, op_name, analysis);
                    }
                }
            }
        }

        None
    }

    /// Extracts the word at a given offset.
    fn extract_word_at_offset(&self, text: &str, offset: usize) -> Option<String> {
        if offset >= text.len() {
            return None;
        }

        let bytes = text.as_bytes();
        let mut start = offset;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            start -= 1;
        }

        let mut end = offset;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }

        if start == end {
            return None;
        }

        Some(text[start..end].to_string())
    }

    /// Finds the start of the word at the given offset.
    fn find_word_start(&self, text: &str, offset: usize) -> Option<usize> {
        if offset >= text.len() {
            return None;
        }

        let bytes = text.as_bytes();
        let mut start = offset;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            start -= 1;
        }

        Some(start)
    }

    /// Extracts a word ending at the given position.
    fn extract_word_ending_at(&self, text: &str, end: usize) -> Option<String> {
        if end == 0 || end > text.len() {
            return None;
        }

        let bytes = text.as_bytes();
        let mut start = end;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            start -= 1;
        }

        if start == end {
            return None;
        }

        Some(text[start..end].to_string())
    }

    /// Extracts a word starting at the given position.
    fn extract_word_starting_at(&self, text: &str, start: usize) -> Option<String> {
        if start >= text.len() {
            return None;
        }

        let bytes = text.as_bytes();
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }

        if start == end {
            return None;
        }

        Some(text[start..end].to_string())
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
    fn determine_context(&self, prefix: &str, text: &str, position: Position) -> CompletionContext {
        let trimmed = prefix.trim();

        // Check for handler context (inside handler body after 'handler Name for Effect')
        if self.is_in_handler_context(text, position) {
            return CompletionContext::Handler;
        }

        // Check for perform effect context (after 'perform ')
        if trimmed.ends_with("perform ") || trimmed.contains("perform ") {
            return CompletionContext::PerformEffect;
        }

        // Check for with handler context (after 'with ')
        if trimmed.ends_with("with ") {
            return CompletionContext::WithHandler;
        }

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

    /// Checks if the cursor is inside a handler definition.
    fn is_in_handler_context(&self, text: &str, position: Position) -> bool {
        // Look for 'handler' keyword before the current position
        let lines: Vec<&str> = text.lines().collect();
        let line_idx = position.line as usize;

        // Search backwards for handler declaration
        for i in (0..=line_idx).rev() {
            if let Some(line) = lines.get(i) {
                if line.trim().starts_with("handler ") && line.contains(" for ") {
                    // Check if we're after the opening brace
                    let after_handler: String = lines[i..=line_idx].join("\n");
                    let open_count = after_handler.matches('{').count();
                    let close_count = after_handler.matches('}').count();
                    return open_count > close_count;
                }
                // Stop if we hit a closing brace at the start of a line (end of previous block)
                if line.trim() == "}" {
                    return false;
                }
            }
        }
        false
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
                self.keyword_item("with", "Run with effect handler"),
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
            CompletionContext::Handler => vec![
                // Inside a handler, suggest common patterns
                self.snippet_item(
                    "fn",
                    "fn ${1:operation}(${2:params}) {\n    resume(${3:value})\n}",
                    "Handler operation implementation",
                ),
                self.keyword_item("resume", "Resume with a value"),
                self.keyword_item("self", "Handler state"),
            ],
            CompletionContext::PerformEffect => vec![
                // After 'perform', just let symbol completions handle effects
            ],
            CompletionContext::WithHandler => vec![
                // After 'with', just let symbol completions handle handlers
            ],
            CompletionContext::MemberAccess | CompletionContext::Import => vec![],
        }
    }

    /// Creates a snippet completion item.
    fn snippet_item(&self, label: &str, snippet: &str, description: &str) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some(description.to_string()),
            insert_text: Some(snippet.to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
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
                CompletionContext::Type => {
                    matches!(symbol.kind, SymbolKind::STRUCT | SymbolKind::ENUM | SymbolKind::TYPE_PARAMETER)
                }
                CompletionContext::Expression => true, // All symbols can appear in expressions
                CompletionContext::Effect => symbol.description.contains("effect "),
                CompletionContext::Pattern => {
                    matches!(symbol.kind, SymbolKind::STRUCT | SymbolKind::ENUM)
                }
                CompletionContext::Handler => {
                    // In handler context, show the effect's operations for implementation hints
                    symbol.kind == SymbolKind::METHOD && symbol.description.contains("op ")
                }
                CompletionContext::PerformEffect => {
                    // After 'perform', show effects and their operations
                    symbol.description.contains("effect ") || symbol.description.contains("op ")
                }
                CompletionContext::WithHandler => {
                    // After 'with', show handlers
                    symbol.description.contains("handler ")
                }
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
                SymbolKind::INTERFACE if symbol.description.contains("effect ") => {
                    Some(CompletionItemKind::INTERFACE)
                }
                SymbolKind::CLASS if symbol.description.contains("handler ") => {
                    Some(CompletionItemKind::CLASS)
                }
                _ => Some(CompletionItemKind::TEXT),
            };

            // Add special formatting for different contexts
            let (label, insert_text) = match context {
                CompletionContext::PerformEffect if symbol.description.contains("effect ") => {
                    // For effects, suggest the Effect::operation pattern
                    (symbol.name.clone(), Some(format!("{}::", symbol.name)))
                }
                CompletionContext::WithHandler if symbol.description.contains("handler ") => {
                    // For handlers, suggest the handler initialization pattern
                    (symbol.name.clone(), Some(format!("{} {{ }}", symbol.name)))
                }
                _ => (symbol.name.clone(), None),
            };

            let mut item = CompletionItem {
                label,
                kind,
                detail: Some(symbol.description.clone()),
                documentation: symbol.doc.as_ref().map(|d| {
                    Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: d.clone(),
                    })
                }),
                ..Default::default()
            };

            if let Some(text) = insert_text {
                item.insert_text = Some(text);
            }

            items.push(item);
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
    /// Inside a handler definition.
    Handler,
    /// After `perform` keyword (for effect operations).
    PerformEffect,
    /// After `with` keyword (for handler selection).
    WithHandler,
}
