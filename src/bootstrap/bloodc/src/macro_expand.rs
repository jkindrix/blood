//! Macro expansion engine for user-defined declarative macros.
//!
//! This module handles the expansion of user-defined macros. The expansion
//! process happens after parsing and before type checking.
//!
//! # Expansion Pipeline
//!
//! 1. **Collection**: Gather all macro definitions from the AST
//! 2. **Matching**: Match macro invocations against patterns
//! 3. **Capture**: Extract values for pattern captures
//! 4. **Substitution**: Replace captures in the expansion template
//! 5. **Hygiene**: Apply fresh scopes to prevent variable capture
//! 6. **Re-parse**: Convert token stream back to AST nodes

use std::collections::HashMap;

use crate::ast::{
    Declaration, ElseBranch, Expr, ExprKind, FragmentKind, HygieneId, MacroCallKind, MacroDecl,
    MacroDelimiter, MacroExpansion, MacroExpansionPart, MacroPattern, MacroPatternPart,
    MacroToken, Program, RepetitionKind, Statement, TokenStream, TokenTree,
};
use crate::diagnostics::Diagnostic;
use crate::lexer::TokenKind;
use crate::span::Span;

/// Maximum recursion depth for macro expansion to prevent infinite loops.
const MAX_EXPANSION_DEPTH: u32 = 256;

/// Captured values from pattern matching.
#[derive(Debug, Clone)]
pub enum CapturedValue {
    /// Single captured expression/type/pattern/etc. with tokens and source text.
    Single {
        /// The captured token stream.
        tokens: TokenStream,
        /// The original source text that produced these tokens.
        source: String,
    },
    /// Repeated captures from a repetition pattern.
    Repeated(Vec<CapturedValue>),
}

/// Context for macro expansion.
pub struct MacroExpander {
    /// Map from macro name to its definition.
    macros: HashMap<String, MacroDecl>,
    /// String interner for symbol resolution.
    interner: string_interner::DefaultStringInterner,
    /// Source code for re-parsing (needed for Parser construction).
    source: String,
    /// Current macro content being processed (for text extraction).
    current_content: String,
    /// Current expansion depth.
    depth: u32,
    /// Collected errors during expansion.
    errors: Vec<Diagnostic>,
}

impl MacroExpander {
    /// Create a new macro expander.
    pub fn new(interner: string_interner::DefaultStringInterner) -> Self {
        Self {
            macros: HashMap::new(),
            interner,
            source: String::new(),
            current_content: String::new(),
            depth: 0,
            errors: Vec::new(),
        }
    }

    /// Create a new macro expander with source code for re-parsing.
    pub fn with_source(interner: string_interner::DefaultStringInterner, source: &str) -> Self {
        Self {
            macros: HashMap::new(),
            interner,
            source: source.to_string(),
            current_content: String::new(),
            depth: 0,
            errors: Vec::new(),
        }
    }

    /// Collect all macro definitions from a program.
    pub fn collect_macros(&mut self, program: &Program) {
        for decl in &program.declarations {
            if let Declaration::Macro(macro_decl) = decl {
                let name = self
                    .interner
                    .resolve(macro_decl.name.node)
                    .unwrap_or("")
                    .to_string();
                self.macros.insert(name, macro_decl.clone());
            }
        }
    }

    /// Expand all macro calls in a program.
    ///
    /// This modifies the program in place, replacing macro calls with their
    /// expanded forms. Returns any errors encountered during expansion.
    pub fn expand_program(&mut self, program: &mut Program) -> Vec<Diagnostic> {
        // First collect all macros
        self.collect_macros(program);

        // Then expand macro calls in declarations
        let mut i = 0;
        while i < program.declarations.len() {
            self.expand_declaration(&mut program.declarations[i]);
            i += 1;
        }

        std::mem::take(&mut self.errors)
    }

    /// Expand macro calls in a declaration.
    fn expand_declaration(&mut self, decl: &mut Declaration) {
        match decl {
            Declaration::Function(f) => {
                if let Some(body) = &mut f.body {
                    self.expand_block(body);
                }
            }
            Declaration::Const(c) => {
                self.expand_expr(&mut c.value);
            }
            Declaration::Static(s) => {
                self.expand_expr(&mut s.value);
            }
            // These declarations don't contain expressions that could include macro calls:
            // - Struct/Enum/Effect/Type: type definitions, no runtime expressions
            // - Bridge: FFI declarations, no Blood expressions
            // - Use/Module/Macro: organizational, no expressions
            Declaration::Struct(_)
            | Declaration::Enum(_)
            | Declaration::Effect(_)
            | Declaration::Type(_)
            | Declaration::Bridge(_)
            | Declaration::Module(_)
            | Declaration::Macro(_)
            | Declaration::Use(_) => {}
            // Handler/Trait/Impl contain function bodies which are handled
            // when those functions are visited individually during type checking.
            // The macro expander runs on the top-level program before lowering.
            Declaration::Handler(_)
            | Declaration::Trait(_)
            | Declaration::Impl(_) => {}
        }
    }

    /// Expand macro calls in a block.
    fn expand_block(&mut self, block: &mut crate::ast::Block) {
        for stmt in &mut block.statements {
            self.expand_statement(stmt);
        }
        if let Some(expr) = &mut block.expr {
            self.expand_expr(expr);
        }
    }

    /// Expand macro calls in an else branch.
    fn expand_else_branch(&mut self, else_branch: &mut ElseBranch) {
        match else_branch {
            ElseBranch::Block(block) => self.expand_block(block),
            ElseBranch::If(expr) => self.expand_expr(expr),
        }
    }

    /// Expand macro calls in a statement.
    fn expand_statement(&mut self, stmt: &mut Statement) {
        match stmt {
            Statement::Let { value, .. } => {
                if let Some(expr) = value {
                    self.expand_expr(expr);
                }
            }
            Statement::Expr { expr, .. } => {
                self.expand_expr(expr);
            }
            Statement::Item(decl) => {
                self.expand_declaration(decl);
            }
        }
    }

    /// Expand macro calls in an expression.
    fn expand_expr(&mut self, expr: &mut Expr) {
        // First recursively expand in sub-expressions
        match &mut expr.kind {
            ExprKind::MacroCall { path, kind } => {
                // Check if this is a user-defined macro
                if let MacroCallKind::Custom { delim, content } = kind {
                    let macro_name = path
                        .segments
                        .last()
                        .map(|s| {
                            self.interner
                                .resolve(s.name.node)
                                .unwrap_or("")
                                .to_string()
                        })
                        .unwrap_or_default();

                    if let Some(macro_def) = self.macros.get(&macro_name).cloned() {
                        // Store content for capture extraction
                        self.current_content = content.clone();

                        // Convert content to token stream
                        let input = self.content_to_token_stream(content, *delim, expr.span);

                        // Try to expand the macro
                        match self.expand_macro_to_source(&macro_def, &input, expr.span) {
                            Ok(expanded_source) => {
                                // Re-parse the expanded source as an expression
                                // Use the same interner to ensure symbols are consistent
                                let mut parser = crate::parser::Parser::with_interner(
                                    &expanded_source,
                                    self.interner.clone()
                                );
                                match parser.parse_expr_for_macro() {
                                    Ok(parsed_expr) => {
                                        // Replace the entire expression with the parsed result
                                        expr.kind = parsed_expr.kind;
                                        expr.span = parsed_expr.span;
                                        // Merge new symbols back into our interner
                                        self.interner = parser.take_interner();
                                    }
                                    Err(parse_errors) => {
                                        for err in parse_errors {
                                            self.errors.push(Diagnostic::error(
                                                format!(
                                                    "parse error in macro expansion: {}",
                                                    err.message
                                                ),
                                                expr.span,
                                            ));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                self.errors.push(Diagnostic::error(e, expr.span));
                            }
                        }
                    }
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.expand_expr(left);
                self.expand_expr(right);
            }
            ExprKind::Unary { operand, .. } => {
                self.expand_expr(operand);
            }
            ExprKind::Call { callee, args, .. } => {
                self.expand_expr(callee);
                for arg in args {
                    self.expand_expr(&mut arg.value);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expand_expr(condition);
                self.expand_block(then_branch);
                if let Some(els) = else_branch {
                    self.expand_else_branch(els);
                }
            }
            ExprKind::IfLet {
                scrutinee,
                then_branch,
                else_branch,
                ..
            } => {
                self.expand_expr(scrutinee);
                self.expand_block(then_branch);
                if let Some(els) = else_branch {
                    self.expand_else_branch(els);
                }
            }
            ExprKind::WhileLet {
                scrutinee, body, ..
            } => {
                self.expand_expr(scrutinee);
                self.expand_block(body);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expand_expr(scrutinee);
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.expand_expr(guard);
                    }
                    self.expand_expr(&mut arm.body);
                }
            }
            ExprKind::Block(block) => {
                self.expand_block(block);
            }
            ExprKind::Array(arr) => match arr {
                crate::ast::ArrayExpr::List(exprs) => {
                    for e in exprs {
                        self.expand_expr(e);
                    }
                }
                crate::ast::ArrayExpr::Repeat { value, count } => {
                    self.expand_expr(value);
                    self.expand_expr(count);
                }
            },
            ExprKind::Tuple(exprs) => {
                for e in exprs {
                    self.expand_expr(e);
                }
            }
            ExprKind::Closure { body, .. } => {
                self.expand_expr(body);
            }
            ExprKind::Loop { body, .. } => {
                self.expand_block(body);
            }
            ExprKind::While {
                condition, body, ..
            } => {
                self.expand_expr(condition);
                self.expand_block(body);
            }
            ExprKind::For { iter, body, .. } => {
                self.expand_expr(iter);
                self.expand_block(body);
            }
            ExprKind::Return(value) => {
                if let Some(v) = value {
                    self.expand_expr(v);
                }
            }
            ExprKind::Field { base, .. } => {
                self.expand_expr(base);
            }
            ExprKind::Cast { expr, .. } => {
                self.expand_expr(expr);
            }
            ExprKind::Unsafe(block) => {
                self.expand_block(block);
            }
            ExprKind::Region { body, .. } => {
                self.expand_block(body);
            }
            ExprKind::Index { base, index } => {
                self.expand_expr(base);
                self.expand_expr(index);
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.expand_expr(receiver);
                for arg in args {
                    self.expand_expr(&mut arg.value);
                }
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.expand_expr(s);
                }
                if let Some(e) = end {
                    self.expand_expr(e);
                }
            }
            ExprKind::Assign { target, value } => {
                self.expand_expr(target);
                self.expand_expr(value);
            }
            ExprKind::AssignOp { target, value, .. } => {
                self.expand_expr(target);
                self.expand_expr(value);
            }
            ExprKind::Paren(expr) => {
                self.expand_expr(expr);
            }
            ExprKind::Perform { args, .. } => {
                for arg in args {
                    self.expand_expr(arg);
                }
            }
            ExprKind::Resume(expr) => {
                self.expand_expr(expr);
            }
            ExprKind::WithHandle { handler, body } => {
                self.expand_expr(handler);
                self.expand_expr(body);
            }
            ExprKind::TryWith { body, handlers } => {
                self.expand_block(body);
                for handler in handlers {
                    self.expand_block(&mut handler.body);
                }
            }
            ExprKind::InlineHandle { body, operations, .. } => {
                self.expand_expr(body);
                for op in operations {
                    self.expand_block(&mut op.body);
                }
            }
            ExprKind::InlineWithDo { body, operations, .. } => {
                self.expand_expr(body);
                for op in operations {
                    self.expand_block(&mut op.body);
                }
            }
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let Some(ref mut value) = field.value {
                        self.expand_expr(value);
                    }
                }
                if let Some(b) = base {
                    self.expand_expr(b);
                }
            }
            // Leaves - no sub-expressions to expand
            ExprKind::Literal(_)
            | ExprKind::Path(_)
            | ExprKind::Break { .. }
            | ExprKind::Continue { .. }
            | ExprKind::Default => {}
        }
    }

    /// Convert a content string to a token stream (re-lex).
    /// Returns tokens with spans relative to the content string (not original source).
    fn content_to_token_stream(
        &self,
        content: &str,
        _delim: MacroDelimiter,
        _span: Span,
    ) -> TokenStream {
        use crate::lexer::Lexer;

        let mut tokens = Vec::new();
        let lexer = Lexer::new(content);

        for token in lexer {
            if token.kind != TokenKind::Eof {
                tokens.push(TokenTree::Token(MacroToken {
                    kind: token.kind,
                    // Keep spans relative to content for source extraction
                    span: token.span,
                    hygiene: HygieneId::default(),
                }));
            }
        }

        TokenStream { tokens }
    }

    /// Match input tokens against a pattern.
    fn match_pattern(
        &self,
        pattern: &MacroPattern,
        input: &TokenStream,
    ) -> Option<HashMap<String, CapturedValue>> {
        let mut captures = HashMap::new();
        let mut input_pos = 0;

        for part in &pattern.parts {
            match part {
                MacroPatternPart::Group {
                    delimiter: _, pattern, ..
                } => {
                    // For the outer group, match the delimiter and recurse
                    if input.tokens.is_empty() {
                        return None;
                    }

                    // Match contents directly for now (simplified)
                    if !self.match_pattern_parts(pattern, input, &mut input_pos, &mut captures) {
                        return None;
                    }
                }
                _ => {
                    // Handle other pattern parts
                    if !self.match_single_part(part, input, &mut input_pos, &mut captures) {
                        return None;
                    }
                }
            }
        }

        Some(captures)
    }

    /// Match pattern parts against input tokens.
    fn match_pattern_parts(
        &self,
        parts: &[MacroPatternPart],
        input: &TokenStream,
        pos: &mut usize,
        captures: &mut HashMap<String, CapturedValue>,
    ) -> bool {
        for part in parts {
            if !self.match_single_part(part, input, pos, captures) {
                return false;
            }
        }
        true
    }

    /// Match a single pattern part.
    fn match_single_part(
        &self,
        part: &MacroPatternPart,
        input: &TokenStream,
        pos: &mut usize,
        captures: &mut HashMap<String, CapturedValue>,
    ) -> bool {
        match part {
            MacroPatternPart::Token(kind, _) => {
                if *pos >= input.tokens.len() {
                    return false;
                }
                if let TokenTree::Token(t) = &input.tokens[*pos] {
                    if t.kind == *kind {
                        *pos += 1;
                        return true;
                    }
                }
                false
            }
            MacroPatternPart::Capture {
                name, fragment, ..
            } => {
                let name_str = self
                    .interner
                    .resolve(name.node)
                    .unwrap_or("")
                    .to_string();

                // Capture tokens based on fragment type
                if let Some(captured) = self.capture_fragment(*fragment, input, pos) {
                    captures.insert(name_str, captured);
                    true
                } else {
                    false
                }
            }
            MacroPatternPart::Repetition {
                pattern,
                separator,
                kind,
                ..
            } => {
                let mut repeated = Vec::new();
                let mut first = true;

                loop {
                    // Check if we need a separator
                    if !first {
                        if let Some(sep) = separator {
                            if *pos < input.tokens.len() {
                                if let TokenTree::Token(t) = &input.tokens[*pos] {
                                    if t.kind == *sep {
                                        *pos += 1;
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }

                    // Try to match the pattern
                    let start_pos = *pos;
                    let mut inner_captures = HashMap::new();
                    if self.match_pattern_parts(pattern, input, pos, &mut inner_captures) {
                        // Collect captures from this iteration
                        for (k, v) in inner_captures {
                            repeated.push((k, v));
                        }
                        first = false;
                    } else {
                        *pos = start_pos;
                        break;
                    }
                }

                // Check repetition constraints
                let count = repeated.len();
                match kind {
                    RepetitionKind::ZeroOrMore => {} // Always ok
                    RepetitionKind::OneOrMore => {
                        if count == 0 {
                            return false;
                        }
                    }
                    RepetitionKind::ZeroOrOne => {
                        if count > 1 {
                            return false;
                        }
                    }
                }

                // Store repeated captures
                // For now, we flatten - a proper implementation would group by name
                for (name, value) in repeated {
                    let entry = captures
                        .entry(name)
                        .or_insert(CapturedValue::Repeated(Vec::new()));
                    if let CapturedValue::Repeated(ref mut vec) = entry {
                        vec.push(value);
                    }
                }

                true
            }
            MacroPatternPart::Group {
                delimiter,
                pattern,
                ..
            } => {
                // Match a delimited group
                if *pos >= input.tokens.len() {
                    return false;
                }

                if let TokenTree::Group {
                    delimiter: d,
                    stream,
                    ..
                } = &input.tokens[*pos]
                {
                    if d == delimiter {
                        let mut inner_pos = 0;
                        if self.match_pattern_parts(pattern, stream, &mut inner_pos, captures) {
                            *pos += 1;
                            return true;
                        }
                    }
                }
                false
            }
        }
    }

    /// Capture a fragment based on its type, returning both tokens and source text.
    fn capture_fragment(
        &self,
        fragment: FragmentKind,
        input: &TokenStream,
        pos: &mut usize,
    ) -> Option<CapturedValue> {
        if *pos >= input.tokens.len() {
            return None;
        }

        // Helper to extract source text from a token
        let extract_source = |tree: &TokenTree| -> String {
            match tree {
                TokenTree::Token(t) => {
                    // Extract from current_content using span offsets
                    // The span start is relative to original source, not content
                    // For now, we reconstruct based on token kind
                    self.token_to_source(t)
                }
                TokenTree::Group { delimiter, stream, .. } => {
                    let (open, close) = match delimiter {
                        MacroDelimiter::Paren => ("(", ")"),
                        MacroDelimiter::Bracket => ("[", "]"),
                        MacroDelimiter::Brace => ("{", "}"),
                    };
                    let inner: String = stream.tokens.iter()
                        .map(|t| self.tree_to_source(t))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{}{}{}", open, inner, close)
                }
            }
        };

        match fragment {
            FragmentKind::Expr => {
                // For now, capture a single token or group
                // A proper implementation would parse expressions
                let tree = input.tokens.get(*pos)?.clone();
                let source = extract_source(&tree);
                *pos += 1;
                Some(CapturedValue::Single {
                    tokens: TokenStream { tokens: vec![tree] },
                    source,
                })
            }
            FragmentKind::Ident => {
                if let TokenTree::Token(t) = &input.tokens[*pos] {
                    if matches!(t.kind, TokenKind::Ident | TokenKind::TypeIdent) {
                        let tree = input.tokens[*pos].clone();
                        let source = extract_source(&tree);
                        *pos += 1;
                        return Some(CapturedValue::Single {
                            tokens: TokenStream { tokens: vec![tree] },
                            source,
                        });
                    }
                }
                None
            }
            FragmentKind::Literal => {
                if let TokenTree::Token(t) = &input.tokens[*pos] {
                    if matches!(
                        t.kind,
                        TokenKind::IntLit
                            | TokenKind::FloatLit
                            | TokenKind::StringLit
                            | TokenKind::CharLit
                    ) {
                        let tree = input.tokens[*pos].clone();
                        let source = extract_source(&tree);
                        *pos += 1;
                        return Some(CapturedValue::Single {
                            tokens: TokenStream { tokens: vec![tree] },
                            source,
                        });
                    }
                }
                None
            }
            FragmentKind::TokenTree => {
                let tree = input.tokens.get(*pos)?.clone();
                let source = extract_source(&tree);
                *pos += 1;
                Some(CapturedValue::Single {
                    tokens: TokenStream { tokens: vec![tree] },
                    source,
                })
            }
            FragmentKind::Ty
            | FragmentKind::Pat
            | FragmentKind::Block
            | FragmentKind::Stmt
            | FragmentKind::Item => {
                // For complex fragments, capture a single token tree for now
                let tree = input.tokens.get(*pos)?.clone();
                let source = extract_source(&tree);
                *pos += 1;
                Some(CapturedValue::Single {
                    tokens: TokenStream { tokens: vec![tree] },
                    source,
                })
            }
        }
    }

    /// Convert a token to its source text representation.
    fn token_to_source(&self, token: &MacroToken) -> String {
        let start = token.span.start;
        let end = token.span.end;

        // Helper to extract from a source string
        let extract_from = |source: &str| -> Option<String> {
            if !source.is_empty() && start < source.len() && end <= source.len() {
                Some(source[start..end].to_string())
            } else {
                None
            }
        };

        match &token.kind {
            TokenKind::IntLit | TokenKind::FloatLit => {
                // Try current_content first (for macro arguments), then original source (for template tokens)
                if let Some(text) = extract_from(&self.current_content) {
                    return text;
                }
                if let Some(text) = extract_from(&self.source) {
                    return text;
                }
                "0".to_string()
            }
            TokenKind::StringLit => {
                if let Some(text) = extract_from(&self.current_content) {
                    return text;
                }
                if let Some(text) = extract_from(&self.source) {
                    return text;
                }
                r#""""#.to_string()
            }
            TokenKind::CharLit => {
                if let Some(text) = extract_from(&self.current_content) {
                    return text;
                }
                if let Some(text) = extract_from(&self.source) {
                    return text;
                }
                "''".to_string()
            }
            TokenKind::Ident | TokenKind::TypeIdent => {
                // Try current_content first, then original source
                if let Some(text) = extract_from(&self.current_content) {
                    return text;
                }
                if let Some(text) = extract_from(&self.source) {
                    return text;
                }
                "_".to_string()
            }
            TokenKind::Plus => "+".to_string(),
            TokenKind::Minus => "-".to_string(),
            TokenKind::Star => "*".to_string(),
            TokenKind::Slash => "/".to_string(),
            TokenKind::Eq => "=".to_string(),
            TokenKind::EqEq => "==".to_string(),
            TokenKind::NotEq => "!=".to_string(),
            TokenKind::Lt => "<".to_string(),
            TokenKind::LtEq => "<=".to_string(),
            TokenKind::Gt => ">".to_string(),
            TokenKind::GtEq => ">=".to_string(),
            TokenKind::And => "&".to_string(),
            TokenKind::Or => "|".to_string(),
            TokenKind::OrOr => "||".to_string(),
            TokenKind::Not => "!".to_string(),
            TokenKind::Comma => ",".to_string(),
            TokenKind::Colon => ":".to_string(),
            TokenKind::Semi => ";".to_string(),
            TokenKind::Dot => ".".to_string(),
            TokenKind::LParen => "(".to_string(),
            TokenKind::RParen => ")".to_string(),
            TokenKind::LBracket => "[".to_string(),
            TokenKind::RBracket => "]".to_string(),
            TokenKind::LBrace => "{".to_string(),
            TokenKind::RBrace => "}".to_string(),
            // Keywords
            TokenKind::As => "as".to_string(),
            TokenKind::Fiber => "fiber".to_string(),
            TokenKind::Suspend => "suspend".to_string(),
            TokenKind::Break => "break".to_string(),
            TokenKind::Const => "const".to_string(),
            TokenKind::Continue => "continue".to_string(),
            TokenKind::Crate => "crate".to_string(),
            TokenKind::Deep => "deep".to_string(),
            TokenKind::Dyn => "dyn".to_string(),
            TokenKind::Effect => "effect".to_string(),
            TokenKind::Else => "else".to_string(),
            TokenKind::Enum => "enum".to_string(),
            TokenKind::Extends => "extends".to_string(),
            TokenKind::Extern => "extern".to_string(),
            TokenKind::False => "false".to_string(),
            TokenKind::Fn => "fn".to_string(),
            TokenKind::For => "for".to_string(),
            TokenKind::Forall => "forall".to_string(),
            TokenKind::Handler => "handler".to_string(),
            TokenKind::If => "if".to_string(),
            TokenKind::Impl => "impl".to_string(),
            TokenKind::In => "in".to_string(),
            TokenKind::Let => "let".to_string(),
            TokenKind::Linear => "linear".to_string(),
            TokenKind::Loop => "loop".to_string(),
            TokenKind::Match => "match".to_string(),
            TokenKind::Mod => "mod".to_string(),
            TokenKind::Module => "module".to_string(),
            TokenKind::Move => "move".to_string(),
            TokenKind::Mut => "mut".to_string(),
            TokenKind::Op => "op".to_string(),
            TokenKind::Perform => "perform".to_string(),
            TokenKind::Pub => "pub".to_string(),
            TokenKind::Pure => "pure".to_string(),
            TokenKind::Ref => "ref".to_string(),
            TokenKind::Region => "region".to_string(),
            TokenKind::Resume => "resume".to_string(),
            TokenKind::Return => "return".to_string(),
            TokenKind::Shallow => "shallow".to_string(),
            TokenKind::Static => "static".to_string(),
            TokenKind::Struct => "struct".to_string(),
            TokenKind::Super => "super".to_string(),
            TokenKind::Trait => "trait".to_string(),
            TokenKind::True => "true".to_string(),
            TokenKind::Type => "type".to_string(),
            TokenKind::Use => "use".to_string(),
            TokenKind::Where => "where".to_string(),
            TokenKind::While => "while".to_string(),
            TokenKind::With => "with".to_string(),
            TokenKind::Handle => "handle".to_string(),
            TokenKind::Affine => "affine".to_string(),
            TokenKind::Bridge => "bridge".to_string(),
            TokenKind::Abstract => "abstract".to_string(),
            TokenKind::Become => "become".to_string(),
            TokenKind::Box => "box".to_string(),
            TokenKind::Do => "do".to_string(),
            TokenKind::Final => "final".to_string(),
            TokenKind::Macro => "macro".to_string(),
            TokenKind::Override => "override".to_string(),
            TokenKind::Priv => "priv".to_string(),
            TokenKind::Typeof => "typeof".to_string(),
            TokenKind::Unsized => "unsized".to_string(),
            TokenKind::Virtual => "virtual".to_string(),
            TokenKind::Yield => "yield".to_string(),
            TokenKind::Try => "try".to_string(),
            TokenKind::Catch => "catch".to_string(),
            TokenKind::Finally => "finally".to_string(),
            TokenKind::Throw => "throw".to_string(),
            TokenKind::Union => "union".to_string(),
            TokenKind::Default => "default".to_string(),
            TokenKind::Unsafe => "unsafe".to_string(),
            // Other operators and punctuation
            TokenKind::Percent => "%".to_string(),
            TokenKind::Caret => "^".to_string(),
            TokenKind::Shl => "<<".to_string(),
            TokenKind::Shr => ">>".to_string(),
            TokenKind::Arrow => "->".to_string(),
            TokenKind::FatArrow => "=>".to_string(),
            TokenKind::Pipe => "|".to_string(),
            TokenKind::At => "@".to_string(),
            TokenKind::Hash => "#".to_string(),
            TokenKind::Question => "?".to_string(),
            TokenKind::Dollar => "$".to_string(),
            TokenKind::ColonColon => "::".to_string(),
            TokenKind::DotDot => "..".to_string(),
            TokenKind::DotDotEq => "..=".to_string(),
            TokenKind::PlusEq => "+=".to_string(),
            TokenKind::MinusEq => "-=".to_string(),
            TokenKind::StarEq => "*=".to_string(),
            TokenKind::SlashEq => "/=".to_string(),
            TokenKind::PercentEq => "%=".to_string(),
            TokenKind::AndEq => "&=".to_string(),
            TokenKind::OrEq => "|=".to_string(),
            TokenKind::CaretEq => "^=".to_string(),
            TokenKind::ShlEq => "<<=".to_string(),
            TokenKind::ShrEq => ">>=".to_string(),
            TokenKind::AndAnd => "&&".to_string(),
            _ => format!("{:?}", token.kind),
        }
    }

    /// Convert a token tree to source text.
    fn tree_to_source(&self, tree: &TokenTree) -> String {
        match tree {
            TokenTree::Token(t) => self.token_to_source(t),
            TokenTree::Group { delimiter, stream, .. } => {
                let (open, close) = match delimiter {
                    MacroDelimiter::Paren => ("(", ")"),
                    MacroDelimiter::Bracket => ("[", "]"),
                    MacroDelimiter::Brace => ("{", "}"),
                };
                let inner: String = stream.tokens.iter()
                    .map(|t| self.tree_to_source(t))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{}{}{}", open, inner, close)
            }
        }
    }

    /// Expand a macro to source text (for re-parsing).
    fn expand_macro_to_source(
        &mut self,
        macro_def: &MacroDecl,
        input: &TokenStream,
        span: Span,
    ) -> Result<String, String> {
        // Check recursion depth
        if self.depth >= MAX_EXPANSION_DEPTH {
            return Err(format!(
                "macro expansion exceeded maximum depth of {}",
                MAX_EXPANSION_DEPTH
            ));
        }
        self.depth += 1;

        // Try each rule in order
        for rule in &macro_def.rules {
            if let Some(captures) = self.match_pattern(&rule.pattern, input) {
                // Substitute to source text
                let source = self.substitute_to_source(&rule.expansion, &captures)?;

                self.depth -= 1;
                return Ok(source);
            }
        }

        self.depth -= 1;
        Err(format!(
            "no matching rule for macro invocation at {:?}",
            span
        ))
    }

    /// Substitute captures into expansion template, producing source code.
    fn substitute_to_source(
        &self,
        expansion: &MacroExpansion,
        captures: &HashMap<String, CapturedValue>,
    ) -> Result<String, String> {
        let mut result = String::new();

        for part in &expansion.parts {
            self.substitute_part_to_source(part, captures, &mut result)?;
        }

        Ok(result)
    }

    /// Substitute a single expansion part to source.
    fn substitute_part_to_source(
        &self,
        part: &MacroExpansionPart,
        captures: &HashMap<String, CapturedValue>,
        result: &mut String,
    ) -> Result<(), String> {
        match part {
            MacroExpansionPart::Tokens(tokens) => {
                for (i, t) in tokens.iter().enumerate() {
                    result.push_str(&self.token_to_source(t));

                    // Smart spacing within tokens
                    let needs_space = if let Some(next) = tokens.get(i + 1) {
                        match (&t.kind, &next.kind) {
                            // identifier followed by ! (macro call)
                            (TokenKind::Ident | TokenKind::TypeIdent, TokenKind::Not) => false,
                            // ! followed by delimiter (macro arguments)
                            (TokenKind::Not, TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace) => false,
                            // before semicolon or comma
                            (_, TokenKind::Semi | TokenKind::Comma) => false,
                            // before closing delimiters
                            (_, TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace) => false,
                            // after opening delimiters
                            (TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace, _) => false,
                            _ => true,
                        }
                    } else {
                        // Last token in this part - add space if it's not punctuation
                        // This ensures spacing between parts (e.g., "if" followed by "$x")
                        !matches!(t.kind,
                            TokenKind::Semi | TokenKind::Comma |
                            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace |
                            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace |
                            TokenKind::Not
                        )
                    };

                    if needs_space {
                        result.push(' ');
                    }
                }
            }
            MacroExpansionPart::Substitution { name, .. } => {
                let name_str = self.interner
                    .resolve(name.node)
                    .unwrap_or("")
                    .to_string();

                if let Some(captured) = captures.get(&name_str) {
                    match captured {
                        CapturedValue::Single { source, .. } => {
                            result.push_str(source);
                            result.push(' ');
                        }
                        CapturedValue::Repeated(items) => {
                            for item in items {
                                if let CapturedValue::Single { source, .. } = item {
                                    result.push_str(source);
                                    result.push(' ');
                                }
                            }
                        }
                    }
                } else {
                    return Err(format!("undefined capture: ${}", name_str));
                }
            }
            MacroExpansionPart::Repetition { parts, separator, .. } => {
                // Handle repetition substitution
                // Find a repeated capture to iterate over
                let mut repeat_count = 0;
                for part in parts {
                    if let MacroExpansionPart::Substitution { name, .. } = part {
                        let name_str = self.interner
                            .resolve(name.node)
                            .unwrap_or("")
                            .to_string();
                        if let Some(CapturedValue::Repeated(items)) = captures.get(&name_str) {
                            repeat_count = items.len();
                            break;
                        }
                    }
                }

                // Emit the repetition
                for i in 0..repeat_count {
                    if i > 0 {
                        if let Some(sep) = separator {
                            result.push_str(&self.token_to_source(sep));
                            result.push(' ');
                        }
                    }

                    // Create captures for this iteration
                    let mut iter_captures = HashMap::new();
                    for (name, value) in captures {
                        let iter_value = match value {
                            CapturedValue::Single { tokens, source } => CapturedValue::Single {
                                tokens: tokens.clone(),
                                source: source.clone(),
                            },
                            CapturedValue::Repeated(items) => {
                                if i < items.len() {
                                    items[i].clone()
                                } else {
                                    CapturedValue::Single {
                                        tokens: TokenStream::new(),
                                        source: String::new(),
                                    }
                                }
                            }
                        };
                        iter_captures.insert(name.clone(), iter_value);
                    }

                    // Substitute parts for this iteration
                    for p in parts {
                        self.substitute_part_to_source(p, &iter_captures, result)?;
                    }
                }
            }
            MacroExpansionPart::Group { delimiter, parts, .. } => {
                let (open, close) = match delimiter {
                    MacroDelimiter::Paren => ("(", ")"),
                    MacroDelimiter::Bracket => ("[", "]"),
                    MacroDelimiter::Brace => ("{", "}"),
                };
                result.push_str(open);
                for p in parts {
                    self.substitute_part_to_source(p, captures, result)?;
                }
                result.push_str(close);
            }
        }
        Ok(())
    }

    /// Get the interner for external use.
    pub fn interner(&self) -> &string_interner::DefaultStringInterner {
        &self.interner
    }

    /// Take ownership of the interner.
    pub fn into_interner(self) -> string_interner::DefaultStringInterner {
        self.interner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macro_expander_creation() {
        let interner = string_interner::DefaultStringInterner::new();
        let expander = MacroExpander::new(interner);
        assert!(expander.macros.is_empty());
    }
}
