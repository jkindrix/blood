//! Semantic Analysis for LSP
//!
//! Provides type information, symbol resolution, and navigation features
//! by integrating with the bloodc compiler.

use std::collections::HashMap;

use bloodc::ast::{self, Declaration, ExprKind, PatternKind, Statement};
use bloodc::{Parser, Span};
use tower_lsp::lsp_types::*;

use crate::document::Document;

/// Analysis result from parsing and type-checking a document.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Symbols defined in the document with their locations and types.
    pub symbols: Vec<SymbolInfo>,
    /// Mapping from byte offset ranges to symbol info indices.
    pub symbol_at_offset: HashMap<usize, usize>,
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

        // Collect symbols from declarations
        for decl in &program.declarations {
            self.collect_declaration_symbols(decl, &interner, &mut symbols, &mut symbol_at_offset);
        }

        Some(AnalysisResult {
            symbols,
            symbol_at_offset,
        })
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
