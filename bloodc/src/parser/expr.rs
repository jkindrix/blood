//! Expression parsing using Pratt parsing for operator precedence.

use super::Parser;
use crate::ast::*;
use crate::lexer::TokenKind;
use crate::span::Spanned;

/// Operator precedence levels (higher = binds tighter).
/// Based on GRAMMAR.md Section 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    None = 0,
    Pipe = 1,       // |>
    Assign = 2,     // = += -= etc.
    Range = 3,      // .. ..=
    Or = 4,         // ||
    And = 5,        // &&
    Comparison = 6, // == != < > <= >=
    BitOr = 7,      // |
    BitXor = 8,     // ^
    BitAnd = 9,     // &
    Shift = 10,     // << >>
    Term = 11,      // + -
    Factor = 12,    // * / %
    Cast = 13,      // as
    Unary = 14,     // ! - * &
    Call = 15,      // () [] .
}

impl Precedence {
    /// Get the next higher precedence level.
    fn next(self) -> Self {
        match self {
            Precedence::None => Precedence::Pipe,
            Precedence::Pipe => Precedence::Assign,
            Precedence::Assign => Precedence::Range,
            Precedence::Range => Precedence::Or,
            Precedence::Or => Precedence::And,
            Precedence::And => Precedence::Comparison,
            Precedence::Comparison => Precedence::BitOr,
            Precedence::BitOr => Precedence::BitXor,
            Precedence::BitXor => Precedence::BitAnd,
            Precedence::BitAnd => Precedence::Shift,
            Precedence::Shift => Precedence::Term,
            Precedence::Term => Precedence::Factor,
            Precedence::Factor => Precedence::Cast,
            Precedence::Cast => Precedence::Unary,
            Precedence::Unary => Precedence::Call,
            Precedence::Call => Precedence::Call,
        }
    }
}

/// Get the precedence of a binary operator token.
fn binary_precedence(kind: TokenKind) -> Option<Precedence> {
    match kind {
        TokenKind::Pipe => Some(Precedence::Pipe),
        TokenKind::Eq
        | TokenKind::PlusEq
        | TokenKind::MinusEq
        | TokenKind::StarEq
        | TokenKind::SlashEq
        | TokenKind::PercentEq
        | TokenKind::AndEq
        | TokenKind::OrEq
        | TokenKind::CaretEq
        | TokenKind::ShlEq
        | TokenKind::ShrEq => Some(Precedence::Assign),
        TokenKind::DotDot | TokenKind::DotDotEq => Some(Precedence::Range),
        TokenKind::OrOr => Some(Precedence::Or),
        TokenKind::AndAnd => Some(Precedence::And),
        TokenKind::EqEq
        | TokenKind::NotEq
        | TokenKind::Lt
        | TokenKind::Gt
        | TokenKind::LtEq
        | TokenKind::GtEq => Some(Precedence::Comparison),
        TokenKind::Or => Some(Precedence::BitOr),
        TokenKind::Caret => Some(Precedence::BitXor),
        TokenKind::And => Some(Precedence::BitAnd),
        TokenKind::Shl | TokenKind::Shr => Some(Precedence::Shift),
        TokenKind::Plus | TokenKind::Minus => Some(Precedence::Term),
        TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Some(Precedence::Factor),
        TokenKind::As => Some(Precedence::Cast),
        _ => None,
    }
}

/// Convert token kind to binary operator.
fn token_to_binop(kind: TokenKind) -> Option<BinOp> {
    match kind {
        TokenKind::Plus => Some(BinOp::Add),
        TokenKind::Minus => Some(BinOp::Sub),
        TokenKind::Star => Some(BinOp::Mul),
        TokenKind::Slash => Some(BinOp::Div),
        TokenKind::Percent => Some(BinOp::Rem),
        TokenKind::EqEq => Some(BinOp::Eq),
        TokenKind::NotEq => Some(BinOp::Ne),
        TokenKind::Lt => Some(BinOp::Lt),
        TokenKind::LtEq => Some(BinOp::Le),
        TokenKind::Gt => Some(BinOp::Gt),
        TokenKind::GtEq => Some(BinOp::Ge),
        TokenKind::AndAnd => Some(BinOp::And),
        TokenKind::OrOr => Some(BinOp::Or),
        TokenKind::And => Some(BinOp::BitAnd),
        TokenKind::Or => Some(BinOp::BitOr),
        TokenKind::Caret => Some(BinOp::BitXor),
        TokenKind::Shl => Some(BinOp::Shl),
        TokenKind::Shr => Some(BinOp::Shr),
        TokenKind::Pipe => Some(BinOp::Pipe),
        _ => None,
    }
}

/// Convert compound assignment token to operator.
fn token_to_compound_op(kind: TokenKind) -> Option<BinOp> {
    match kind {
        TokenKind::PlusEq => Some(BinOp::Add),
        TokenKind::MinusEq => Some(BinOp::Sub),
        TokenKind::StarEq => Some(BinOp::Mul),
        TokenKind::SlashEq => Some(BinOp::Div),
        TokenKind::PercentEq => Some(BinOp::Rem),
        TokenKind::AndEq => Some(BinOp::BitAnd),
        TokenKind::OrEq => Some(BinOp::BitOr),
        TokenKind::CaretEq => Some(BinOp::BitXor),
        TokenKind::ShlEq => Some(BinOp::Shl),
        TokenKind::ShrEq => Some(BinOp::Shr),
        _ => None,
    }
}

impl<'src> Parser<'src> {
    /// Parse an expression.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_expr(&mut self) -> Expr {
        self.parse_expr_prec(Precedence::None)
    }

    /// Parse an expression with at least the given precedence.
    fn parse_expr_prec(&mut self, min_prec: Precedence) -> Expr {
        // Parse prefix/primary expression
        let left = self.parse_prefix_expr();
        self.parse_expr_prec_with(left, min_prec)
    }

    /// Continue parsing an expression with an already-parsed left-hand side.
    fn parse_expr_prec_with(&mut self, left: Expr, min_prec: Precedence) -> Expr {
        self.parse_expr_prec_with_inner(left, min_prec, false)
    }

    /// Continue parsing an expression with an already-parsed left-hand side, disallowing struct literals.
    fn parse_expr_prec_with_no_struct(&mut self, left: Expr, min_prec: Precedence) -> Expr {
        self.parse_expr_prec_with_inner(left, min_prec, true)
    }

    /// Inner implementation of parse_expr_prec_with with struct literal control.
    fn parse_expr_prec_with_inner(&mut self, mut left: Expr, min_prec: Precedence, no_struct: bool) -> Expr {
        // Parse binary operators
        while let Some(prec) = binary_precedence(self.current.kind) {
            if prec < min_prec {
                break;
            }

            let op_token = self.current.kind;
            self.advance();

            // Handle different operator types
            left = match op_token {
                TokenKind::As => {
                    let ty = self.parse_type();
                    let span = left.span.merge(self.previous.span);
                    Expr {
                        kind: ExprKind::Cast {
                            expr: Box::new(left),
                            ty,
                        },
                        span,
                    }
                }
                TokenKind::Eq => {
                    let value = if no_struct {
                        self.parse_expr_prec_no_struct(Precedence::Assign)
                    } else {
                        self.parse_expr_prec(Precedence::Assign)
                    };
                    let span = left.span.merge(value.span);
                    Expr {
                        kind: ExprKind::Assign {
                            target: Box::new(left),
                            value: Box::new(value),
                        },
                        span,
                    }
                }
                TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq |
                TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::AndEq |
                TokenKind::OrEq | TokenKind::CaretEq | TokenKind::ShlEq |
                TokenKind::ShrEq => {
                    // Compound assignment operators - guaranteed to have a matching bin_op
                    let bin_op = token_to_compound_op(op_token)
                        .expect("BUG: matched compound op token but conversion failed");
                    let value = if no_struct {
                        self.parse_expr_prec_no_struct(Precedence::Assign)
                    } else {
                        self.parse_expr_prec(Precedence::Assign)
                    };
                    let span = left.span.merge(value.span);
                    Expr {
                        kind: ExprKind::AssignOp {
                            op: bin_op,
                            target: Box::new(left),
                            value: Box::new(value),
                        },
                        span,
                    }
                }
                TokenKind::DotDot | TokenKind::DotDotEq => {
                    let inclusive = op_token == TokenKind::DotDotEq;
                    let end = if self.current.kind.can_start_expr() {
                        let rhs = if no_struct {
                            self.parse_expr_prec_no_struct(Precedence::Range.next())
                        } else {
                            self.parse_expr_prec(Precedence::Range.next())
                        };
                        Some(Box::new(rhs))
                    } else {
                        None
                    };
                    let span = left.span.merge(self.previous.span);
                    Expr {
                        kind: ExprKind::Range {
                            start: Some(Box::new(left)),
                            end,
                            inclusive,
                        },
                        span,
                    }
                }
                _ => {
                    if let Some(bin_op) = token_to_binop(op_token) {
                        // For right-associative operators, use same precedence
                        // For left-associative, use next higher
                        let next_prec = prec.next();
                        let right = if no_struct {
                            self.parse_expr_prec_no_struct(next_prec)
                        } else {
                            self.parse_expr_prec(next_prec)
                        };
                        let span = left.span.merge(right.span);
                        Expr {
                            kind: ExprKind::Binary {
                                op: bin_op,
                                left: Box::new(left),
                                right: Box::new(right),
                            },
                            span,
                        }
                    } else {
                        // Should not reach here
                        left
                    }
                }
            };
        }

        left
    }

    /// Parse a prefix expression or primary expression.
    fn parse_prefix_expr(&mut self) -> Expr {
        let start = self.current.span;

        match self.current.kind {
            // Unary operators
            TokenKind::Not => {
                self.advance();
                let operand = self.parse_prefix_expr();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_prefix_expr();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::Star => {
                self.advance();
                let operand = self.parse_prefix_expr();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Deref,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::And => {
                self.advance();
                let is_mut = self.try_consume(TokenKind::Mut);
                let operand = self.parse_prefix_expr();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: if is_mut {
                            UnaryOp::RefMut
                        } else {
                            UnaryOp::Ref
                        },
                        operand: Box::new(operand),
                    },
                    span,
                }
            }

            // Range with no start
            TokenKind::DotDot | TokenKind::DotDotEq => {
                let inclusive = self.current.kind == TokenKind::DotDotEq;
                self.advance();
                let end = if self.current.kind.can_start_expr() {
                    Some(Box::new(self.parse_expr_prec(Precedence::Range.next())))
                } else {
                    None
                };
                let span = start.merge(self.previous.span);
                Expr {
                    kind: ExprKind::Range {
                        start: None,
                        end,
                        inclusive,
                    },
                    span,
                }
            }

            // Return
            TokenKind::Return => {
                self.advance();
                let value = if self.current.kind.can_start_expr() {
                    Some(Box::new(self.parse_expr()))
                } else {
                    None
                };
                let span = start.merge(self.previous.span);
                Expr {
                    kind: ExprKind::Return(value),
                    span,
                }
            }

            // Break
            TokenKind::Break => {
                self.advance();
                let label = if self.check(TokenKind::Lifetime) {
                    self.advance();
                    Some(self.spanned_symbol())
                } else {
                    None
                };
                let value = if self.current.kind.can_start_expr() {
                    Some(Box::new(self.parse_expr()))
                } else {
                    None
                };
                let span = start.merge(self.previous.span);
                Expr {
                    kind: ExprKind::Break { label, value },
                    span,
                }
            }

            // Continue
            TokenKind::Continue => {
                self.advance();
                let label = if self.check(TokenKind::Lifetime) {
                    self.advance();
                    Some(self.spanned_symbol())
                } else {
                    None
                };
                let span = start.merge(self.previous.span);
                Expr {
                    kind: ExprKind::Continue { label },
                    span,
                }
            }

            // Primary expressions
            _ => self.parse_postfix_expr(),
        }
    }

    /// Parse postfix expressions (calls, indexing, field access).
    fn parse_postfix_expr(&mut self) -> Expr {
        let expr = self.parse_primary_expr();
        self.parse_postfix_continuation(expr)
    }

    /// Check if an expression is "block-like" (ends with `}` and can't be called).
    /// Block expressions like `{}`, `if`, `match`, `loop`, `while`, `for`, etc.
    /// cannot be followed by `(...)` to form a function call.
    fn is_block_like_expr(expr: &Expr) -> bool {
        matches!(
            expr.kind,
            ExprKind::Block(_)
                | ExprKind::If { .. }
                | ExprKind::IfLet { .. }
                | ExprKind::Match { .. }
                | ExprKind::Loop { .. }
                | ExprKind::While { .. }
                | ExprKind::WhileLet { .. }
                | ExprKind::For { .. }
                | ExprKind::Unsafe(_)
                | ExprKind::Region { .. }
                | ExprKind::WithHandle { .. }
        )
    }

    /// Continue parsing postfix operators on an already-parsed expression.
    fn parse_postfix_continuation(&mut self, mut expr: Expr) -> Expr {
        loop {
            match self.current.kind {
                // Function call - but NOT after block-like expressions
                // In `match x { ... } (y)`, the `(y)` is NOT a function call to the match result;
                // it's the start of a new expression (e.g., a pattern in the next match arm).
                TokenKind::LParen if !Self::is_block_like_expr(&expr) => {
                    self.advance();
                    let args = self.parse_call_args();
                    self.expect(TokenKind::RParen);
                    let span = expr.span.merge(self.previous.span);
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }

                // Index
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr();
                    self.expect(TokenKind::RBracket);
                    let span = expr.span.merge(self.previous.span);
                    expr = Expr {
                        kind: ExprKind::Index {
                            base: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    };
                }

                // Field access or method call
                TokenKind::Dot => {
                    self.advance();

                    // Check for tuple index
                    if self.check(TokenKind::IntLit) {
                        let index_span = self.current.span;
                        let text = self.text(&index_span);
                        let index: u32 = text.parse().unwrap_or(0);
                        self.advance();
                        let span = expr.span.merge(self.previous.span);
                        expr = Expr {
                            kind: ExprKind::Field {
                                base: Box::new(expr),
                                field: FieldAccess::Index(index, index_span),
                            },
                            span,
                        };
                    } else if self.check(TokenKind::Ident) {
                        self.advance();
                        let name = self.spanned_symbol();

                        // Check for method call
                        if self.check(TokenKind::LParen)
                            || self.check(TokenKind::ColonColon) && self.peek_is_lt()
                        {
                            // Parse optional turbofish
                            let type_args = if self.try_consume(TokenKind::ColonColon) {
                                Some(self.parse_type_args())
                            } else {
                                None
                            };

                            self.expect(TokenKind::LParen);
                            let args = self.parse_call_args();
                            self.expect(TokenKind::RParen);

                            let span = expr.span.merge(self.previous.span);
                            expr = Expr {
                                kind: ExprKind::MethodCall {
                                    receiver: Box::new(expr),
                                    method: name,
                                    type_args,
                                    args,
                                },
                                span,
                            };
                        } else {
                            let span = expr.span.merge(self.previous.span);
                            expr = Expr {
                                kind: ExprKind::Field {
                                    base: Box::new(expr),
                                    field: FieldAccess::Named(name),
                                },
                                span,
                            };
                        }
                    } else {
                        self.error_expected("field name or index");
                    }
                }

                // Question mark operator (try)
                TokenKind::Question => {
                    self.advance();
                    // Desugar to: match expr { Ok(x) => x, Err(e) => return Err(e) }
                    // For now, just make it a method call to .try()
                    let span = expr.span.merge(self.previous.span);
                    expr = Expr {
                        kind: ExprKind::MethodCall {
                            receiver: Box::new(expr),
                            method: Spanned::new(self.intern("try_"), self.previous.span),
                            type_args: None,
                            args: Vec::new(),
                        },
                        span,
                    };
                }

                _ => break,
            }
        }

        expr
    }

    /// Check if the next token after current is `<`.
    fn peek_is_lt(&self) -> bool {
        // We need lookahead here, but for simplicity, assume turbofish is always followed by <
        self.check(TokenKind::ColonColon)
    }

    /// Parse call arguments.
    fn parse_call_args(&mut self) -> Vec<CallArg> {
        let mut args = Vec::new();

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            let start = self.current.span;

            // Check for named argument: `name: expr`
            // We need lookahead to distinguish between `name: expr` and `name + expr`
            let name = if self.check(TokenKind::Ident) {
                // Peek at the next token to see if it's a `:`
                // We need to check if this is a named argument pattern
                let maybe_name = self.current.span;
                let maybe_name_text = self.text(&maybe_name);

                // Advance past the identifier
                self.advance();

                if self.try_consume(TokenKind::Colon) {
                    // This is a named argument: `name: value`
                    Some(Spanned::new(self.intern(maybe_name_text), maybe_name))
                } else {
                    // Not a named argument - we already consumed the identifier
                    // Build the initial path expression and continue parsing it
                    let symbol = self.intern(maybe_name_text);
                    let mut expr = Expr {
                        kind: ExprKind::Path(ExprPath {
                            segments: vec![ExprPathSegment {
                                name: Spanned::new(symbol, maybe_name),
                                args: None,
                            }],
                            span: maybe_name,
                        }),
                        span: maybe_name,
                    };

                    // Continue parsing as an expression (handles postfix ops and binary ops)
                    expr = self.parse_postfix_continuation(expr);
                    expr = self.parse_expr_prec_with(expr, Precedence::None);

                    let span = start.merge(expr.span);
                    args.push(CallArg {
                        name: None,
                        value: expr,
                        span,
                    });

                    if !self.try_consume(TokenKind::Comma) {
                        break;
                    }
                    continue;
                }
            } else {
                None
            };

            let value = self.parse_expr();
            let span = start.merge(value.span);
            args.push(CallArg { name, value, span });

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        args
    }

    /// Parse a primary expression.
    fn parse_primary_expr(&mut self) -> Expr {
        let start = self.current.span;

        match self.current.kind {
            // Literals
            TokenKind::IntLit
            | TokenKind::FloatLit
            | TokenKind::StringLit
            | TokenKind::RawStringLit
            | TokenKind::RawStringLitHash1
            | TokenKind::RawStringLitHash2
            | TokenKind::CharLit
            | TokenKind::True
            | TokenKind::False => {
                let lit = self.parse_literal();
                Expr {
                    span: lit.span,
                    kind: ExprKind::Literal(lit),
                }
            }

            // Identifiers and paths
            TokenKind::Ident | TokenKind::TypeIdent | TokenKind::SelfLower | TokenKind::SelfUpper => {
                self.parse_path_or_struct_expr()
            }

            // Parenthesized expression or tuple
            TokenKind::LParen => {
                self.advance();

                // Empty tuple
                if self.try_consume(TokenKind::RParen) {
                    return Expr {
                        kind: ExprKind::Tuple(Vec::new()),
                        span: start.merge(self.previous.span),
                    };
                }

                let first = self.parse_expr();

                // Tuple
                if self.try_consume(TokenKind::Comma) {
                    let mut elements = vec![first];
                    while !self.check(TokenKind::RParen) && !self.is_at_end() {
                        elements.push(self.parse_expr());
                        if !self.try_consume(TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen);
                    return Expr {
                        kind: ExprKind::Tuple(elements),
                        span: start.merge(self.previous.span),
                    };
                }

                self.expect(TokenKind::RParen);
                Expr {
                    kind: ExprKind::Paren(Box::new(first)),
                    span: start.merge(self.previous.span),
                }
            }

            // Array
            TokenKind::LBracket => self.parse_array_expr(),

            // Block or anonymous record literal
            // Detect record literal pattern: { identifier : ...
            // If we see { followed by identifier and then colon, it's a record literal
            TokenKind::LBrace => {
                if self.check_next(TokenKind::Ident) && self.check_after_next(TokenKind::Colon) {
                    // Anonymous record literal: { x: 10, y: 20 }
                    self.parse_struct_literal(None)
                } else {
                    // Regular block: { statements... }
                    let block = self.parse_block();
                    Expr {
                        span: block.span,
                        kind: ExprKind::Block(block),
                    }
                }
            }

            // If expression
            TokenKind::If => self.parse_if_expr(),

            // Match expression
            TokenKind::Match => self.parse_match_expr(),

            // Labeled loop expressions ('label: loop/while/for)
            TokenKind::Lifetime => {
                self.advance();
                let label = self.spanned_symbol();
                self.expect(TokenKind::Colon);
                match self.current.kind {
                    TokenKind::Loop => self.parse_loop_expr_with_label(Some(label)),
                    TokenKind::While => self.parse_while_expr_with_label(Some(label)),
                    TokenKind::For => self.parse_for_expr_with_label(Some(label)),
                    _ => {
                        self.error_expected_one_of(&["`loop`", "`while`", "`for`"]);
                        self.advance();
                        Expr {
                            kind: ExprKind::Tuple(Vec::new()),
                            span: start,
                        }
                    }
                }
            }

            // Loop expressions
            TokenKind::Loop => self.parse_loop_expr_with_label(None),
            TokenKind::While => self.parse_while_expr_with_label(None),
            TokenKind::For => self.parse_for_expr_with_label(None),

            // Closure
            TokenKind::Or | TokenKind::Move => self.parse_closure_expr(),

            // Effect expressions
            TokenKind::With => self.parse_with_handle_expr(),
            TokenKind::Perform => self.parse_perform_expr(),
            TokenKind::Resume => {
                self.advance();
                self.expect(TokenKind::LParen);
                let value = self.parse_expr();
                self.expect(TokenKind::RParen);
                Expr {
                    kind: ExprKind::Resume(Box::new(value)),
                    span: start.merge(self.previous.span),
                }
            }

            // Unsafe block
            TokenKind::AtUnsafe => {
                self.advance();
                let block = self.parse_block();
                Expr {
                    kind: ExprKind::Unsafe(block),
                    span: start.merge(self.previous.span),
                }
            }

            // Region
            TokenKind::Region => {
                self.advance();
                let name = if self.check(TokenKind::Lifetime) {
                    self.advance();
                    Some(self.spanned_symbol())
                } else {
                    None
                };
                let body = self.parse_block();
                Expr {
                    kind: ExprKind::Region { name, body },
                    span: start.merge(self.previous.span),
                }
            }

            // Some keywords can be used as identifiers in expression context
            TokenKind::Handler | TokenKind::Effect | TokenKind::Deep | TokenKind::Shallow => {
                // Treat as identifier
                self.advance();
                let name = self.spanned_symbol();
                let path = ExprPath {
                    segments: vec![ExprPathSegment { name, args: None }],
                    span: start.merge(self.previous.span),
                };

                // Check for struct literal
                if self.check(TokenKind::LBrace) && !self.is_statement_context() {
                    return self.parse_struct_literal(Some(self.path_to_type_path(&path)));
                }

                Expr {
                    span: path.span,
                    kind: ExprKind::Path(path),
                }
            }

            _ => {
                self.error_expected_one_of(&["literal", "identifier", "`(`", "`[`", "`{`", "`if`", "`match`"]);
                // Advance to prevent infinite loop during error recovery
                self.advance();
                Expr {
                    kind: ExprKind::Tuple(Vec::new()),
                    span: start,
                }
            }
        }
    }

    /// Parse a path expression or struct literal.
    fn parse_path_or_struct_expr(&mut self) -> Expr {
        let path = self.parse_expr_path();

        // Check for struct literal
        if self.check(TokenKind::LBrace) && !self.is_statement_context() {
            return self.parse_struct_literal(Some(self.path_to_type_path(&path)));
        }

        Expr {
            span: path.span,
            kind: ExprKind::Path(path),
        }
    }

    /// Convert expression path to type path.
    fn path_to_type_path(&self, path: &ExprPath) -> TypePath {
        TypePath {
            segments: path
                .segments
                .iter()
                .map(|s| TypePathSegment {
                    name: s.name.clone(),
                    args: s.args.clone(),
                })
                .collect(),
            span: path.span,
        }
    }

    /// Parse an expression that cannot end with a struct literal (for ambiguous contexts).
    /// Used in match scrutinees, if conditions, etc. where `{` starts a block.
    fn parse_expr_no_struct(&mut self) -> Expr {
        self.parse_expr_prec_no_struct(Precedence::None)
    }

    /// Parse an expression with precedence, but disallow struct literals at the end.
    fn parse_expr_prec_no_struct(&mut self, min_prec: Precedence) -> Expr {
        let left = self.parse_prefix_expr_no_struct();
        self.parse_expr_prec_with_no_struct(left, min_prec)
    }

    /// Parse a prefix expression that doesn't allow struct literals.
    fn parse_prefix_expr_no_struct(&mut self) -> Expr {
        let start = self.current.span;

        match self.current.kind {
            // Unary operators - these can still contain struct literals in their operands
            TokenKind::Not => {
                self.advance();
                let operand = self.parse_prefix_expr_no_struct();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_prefix_expr_no_struct();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::Star => {
                self.advance();
                let operand = self.parse_prefix_expr_no_struct();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Deref,
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            TokenKind::And => {
                self.advance();
                let is_mut = self.try_consume(TokenKind::Mut);
                let operand = self.parse_prefix_expr_no_struct();
                let span = start.merge(operand.span);
                Expr {
                    kind: ExprKind::Unary {
                        op: if is_mut {
                            UnaryOp::RefMut
                        } else {
                            UnaryOp::Ref
                        },
                        operand: Box::new(operand),
                    },
                    span,
                }
            }
            _ => self.parse_postfix_expr_no_struct(),
        }
    }

    /// Parse postfix expressions without allowing struct literals at the path level.
    fn parse_postfix_expr_no_struct(&mut self) -> Expr {
        let expr = self.parse_primary_expr_no_struct();
        self.parse_postfix_continuation(expr)
    }

    /// Parse a primary expression without allowing path-based struct literals.
    fn parse_primary_expr_no_struct(&mut self) -> Expr {
        let _start = self.current.span;

        // For identifiers/paths, don't allow struct literals
        if matches!(
            self.current.kind,
            TokenKind::Ident | TokenKind::TypeIdent | TokenKind::SelfLower | TokenKind::SelfUpper
        ) {
            let path = self.parse_expr_path();
            // Don't check for struct literal here - just return the path
            return Expr {
                span: path.span,
                kind: ExprKind::Path(path),
            };
        }

        // For other expressions, delegate to normal primary parsing
        self.parse_primary_expr()
    }

    /// Check if we're in a statement context where `{` would start a block, not a struct literal.
    /// This is used for contexts where we haven't explicitly used parse_expr_no_struct.
    fn is_statement_context(&self) -> bool {
        // Now that we use parse_expr_no_struct for ambiguous contexts (if, match, while),
        // this can return false for all other contexts (allowing struct literals).
        false
    }

    /// Parse an expression path.
    fn parse_expr_path(&mut self) -> ExprPath {
        let start = self.current.span;
        let mut segments = Vec::new();

        loop {
            let name = if self.check(TokenKind::Ident)
                || self.check(TokenKind::TypeIdent)
                || self.check(TokenKind::SelfLower)
                || self.check(TokenKind::SelfUpper)
            {
                self.advance();
                self.spanned_symbol()
            } else {
                break;
            };

            // Parse optional turbofish
            let args = if self.check(TokenKind::ColonColon) {
                // Peek to see if this is turbofish (::< vs :: for path)
                // For now, only parse turbofish if followed by <
                None
            } else {
                None
            };

            segments.push(ExprPathSegment { name, args });

            // Check for path continuation
            if !self.try_consume(TokenKind::ColonColon) {
                break;
            }
        }

        ExprPath {
            segments,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a struct literal.
    fn parse_struct_literal(&mut self, path: Option<TypePath>) -> Expr {
        let start = path
            .as_ref()
            .map(|p| p.span)
            .unwrap_or_else(|| self.current.span);

        self.expect(TokenKind::LBrace);

        let mut fields = Vec::new();
        let mut base = None;

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            // Check for base expression
            if self.try_consume(TokenKind::DotDot) {
                base = Some(Box::new(self.parse_expr()));
                break;
            }

            let field_start = self.current.span;
            let name = if self.check(TokenKind::Ident) {
                self.advance();
                self.spanned_symbol()
            } else {
                self.error_expected("field name");
                break;
            };

            let value = if self.try_consume(TokenKind::Colon) {
                Some(self.parse_expr())
            } else {
                None
            };

            fields.push(RecordExprField {
                name,
                value,
                span: field_start.merge(self.previous.span),
            });

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        self.expect(TokenKind::RBrace);

        Expr {
            kind: ExprKind::Record { path, fields, base },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse an array expression.
    fn parse_array_expr(&mut self) -> Expr {
        let start = self.current.span;
        self.advance(); // consume '['

        // Empty array
        if self.try_consume(TokenKind::RBracket) {
            return Expr {
                kind: ExprKind::Array(ArrayExpr::List(Vec::new())),
                span: start.merge(self.previous.span),
            };
        }

        let first = self.parse_expr();

        // Check for repeat syntax [value; count]
        if self.try_consume(TokenKind::Semi) {
            let count = self.parse_expr();
            self.expect(TokenKind::RBracket);
            return Expr {
                kind: ExprKind::Array(ArrayExpr::Repeat {
                    value: Box::new(first),
                    count: Box::new(count),
                }),
                span: start.merge(self.previous.span),
            };
        }

        // List syntax [a, b, c]
        let mut elements = vec![first];
        while self.try_consume(TokenKind::Comma) {
            if self.check(TokenKind::RBracket) {
                break;
            }
            elements.push(self.parse_expr());
        }

        self.expect(TokenKind::RBracket);

        Expr {
            kind: ExprKind::Array(ArrayExpr::List(elements)),
            span: start.merge(self.previous.span),
        }
    }

    /// Parse an if expression.
    fn parse_if_expr(&mut self) -> Expr {
        let start = self.current.span;
        self.advance(); // consume 'if'

        // Check for if-let: `if let PATTERN = EXPR { } else { }`
        if self.try_consume(TokenKind::Let) {
            let pattern = self.parse_pattern();
            self.expect(TokenKind::Eq);
            let scrutinee = self.parse_expr_no_struct();
            let then_branch = self.parse_block();

            let else_branch = if self.try_consume(TokenKind::Else) {
                if self.check(TokenKind::If) {
                    Some(ElseBranch::If(Box::new(self.parse_if_expr())))
                } else {
                    Some(ElseBranch::Block(self.parse_block()))
                }
            } else {
                None
            };

            return Expr {
                kind: ExprKind::IfLet {
                    pattern,
                    scrutinee: Box::new(scrutinee),
                    then_branch,
                    else_branch,
                },
                span: start.merge(self.previous.span),
            };
        }

        // Regular if: `if cond { } else { }`
        // Use parse_expr_no_struct to avoid ambiguity with struct literals
        let condition = self.parse_expr_no_struct();
        let then_branch = self.parse_block();

        let else_branch = if self.try_consume(TokenKind::Else) {
            if self.check(TokenKind::If) {
                Some(ElseBranch::If(Box::new(self.parse_if_expr())))
            } else {
                Some(ElseBranch::Block(self.parse_block()))
            }
        } else {
            None
        };

        Expr {
            kind: ExprKind::If {
                condition: Box::new(condition),
                then_branch,
                else_branch,
            },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a match expression.
    fn parse_match_expr(&mut self) -> Expr {
        let start = self.current.span;
        self.advance(); // consume 'match'

        // Use parse_expr_no_struct to avoid ambiguity with struct literals
        let scrutinee = self.parse_expr_no_struct();
        self.expect(TokenKind::LBrace);

        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            arms.push(self.parse_match_arm());

            // Allow trailing comma
            self.try_consume(TokenKind::Comma);
        }

        self.expect(TokenKind::RBrace);

        Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span: start.merge(self.previous.span),
        }
    }

    fn parse_match_arm(&mut self) -> MatchArm {
        let start = self.current.span;
        let pattern = self.parse_pattern();

        let guard = if self.try_consume(TokenKind::If) {
            Some(self.parse_expr())
        } else {
            None
        };

        self.expect(TokenKind::FatArrow);
        let body = self.parse_expr();

        MatchArm {
            pattern,
            guard,
            body,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a loop expression with an optional label.
    fn parse_loop_expr_with_label(&mut self, label: Option<Spanned<Symbol>>) -> Expr {
        let start = self.current.span;
        self.advance(); // consume 'loop'
        let body = self.parse_block();

        Expr {
            kind: ExprKind::Loop { label, body },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a while expression with an optional label.
    fn parse_while_expr_with_label(&mut self, label: Option<Spanned<Symbol>>) -> Expr {
        let start = self.current.span;
        self.advance(); // consume 'while'

        // Check for while-let: `while let PATTERN = EXPR { }`
        if self.try_consume(TokenKind::Let) {
            let pattern = self.parse_pattern();
            self.expect(TokenKind::Eq);
            // Use parse_expr_no_struct to avoid ambiguity with struct literals
            let scrutinee = self.parse_expr_no_struct();
            let body = self.parse_block();

            return Expr {
                kind: ExprKind::WhileLet {
                    label,
                    pattern,
                    scrutinee: Box::new(scrutinee),
                    body,
                },
                span: start.merge(self.previous.span),
            };
        }

        // Regular while: `while cond { }`
        // Use parse_expr_no_struct to avoid ambiguity with struct literals
        let condition = self.parse_expr_no_struct();
        let body = self.parse_block();

        Expr {
            kind: ExprKind::While {
                label,
                condition: Box::new(condition),
                body,
            },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a for expression with an optional label.
    fn parse_for_expr_with_label(&mut self, label: Option<Spanned<Symbol>>) -> Expr {
        let start = self.current.span;
        self.advance(); // consume 'for'
        let pattern = self.parse_pattern();
        self.expect(TokenKind::In);
        // Use parse_expr_no_struct to avoid ambiguity with struct literals
        let iter = self.parse_expr_no_struct();
        let body = self.parse_block();

        Expr {
            kind: ExprKind::For {
                label,
                pattern,
                iter: Box::new(iter),
                body,
            },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a closure expression.
    fn parse_closure_expr(&mut self) -> Expr {
        let start = self.current.span;
        let is_move = self.try_consume(TokenKind::Move);

        self.expect(TokenKind::Or);
        let params = self.parse_closure_params();
        self.expect(TokenKind::Or);

        let return_type = if self.try_consume(TokenKind::Arrow) {
            Some(self.parse_type())
        } else {
            None
        };

        let effects = if self.try_consume(TokenKind::Slash) {
            Some(self.parse_effect_row())
        } else {
            None
        };

        let body = if self.check(TokenKind::LBrace) {
            let block = self.parse_block();
            Expr {
                span: block.span,
                kind: ExprKind::Block(block),
            }
        } else {
            self.parse_expr()
        };

        Expr {
            kind: ExprKind::Closure {
                is_move,
                params,
                return_type,
                effects,
                body: Box::new(body),
            },
            span: start.merge(self.previous.span),
        }
    }

    fn parse_closure_params(&mut self) -> Vec<ClosureParam> {
        let mut params = Vec::new();

        while !self.check(TokenKind::Or) && !self.is_at_end() {
            let start = self.current.span;
            // Use parse_pattern_no_or to avoid consuming the closing `|` as an or-pattern
            let pattern = self.parse_pattern_no_or();
            let ty = if self.try_consume(TokenKind::Colon) {
                Some(self.parse_type())
            } else {
                None
            };

            params.push(ClosureParam {
                pattern,
                ty,
                span: start.merge(self.previous.span),
            });

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        params
    }

    /// Parse a with-handle expression.
    fn parse_with_handle_expr(&mut self) -> Expr {
        let start = self.current.span;
        self.advance(); // consume 'with'

        let handler = self.parse_expr();

        self.expect(TokenKind::Handle);

        let body = if self.check(TokenKind::LBrace) {
            let block = self.parse_block();
            Expr {
                span: block.span,
                kind: ExprKind::Block(block),
            }
        } else {
            self.parse_expr()
        };

        Expr {
            kind: ExprKind::WithHandle {
                handler: Box::new(handler),
                body: Box::new(body),
            },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a perform expression.
    fn parse_perform_expr(&mut self) -> Expr {
        let start = self.current.span;
        self.advance(); // consume 'perform'

        // Parse optional effect qualification
        let path = self.parse_type_path();
        let (effect, operation) = if self.try_consume(TokenKind::Dot) {
            let op = if self.check(TokenKind::Ident) {
                self.advance();
                self.spanned_symbol()
            } else {
                self.error_expected("operation name");
                Spanned::new(self.intern(""), self.current.span)
            };
            (Some(path), op)
        } else {
            // No dot, so path is just the operation name
            let op = path
                .segments
                .first()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| Spanned::new(self.intern(""), start));
            (None, op)
        };

        self.expect(TokenKind::LParen);
        let mut args = Vec::new();
        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            args.push(self.parse_expr());
            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen);

        Expr {
            kind: ExprKind::Perform {
                effect,
                operation,
                args,
            },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a block.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_block(&mut self) -> Block {
        let start = self.current.span;
        self.expect(TokenKind::LBrace);

        let mut statements = Vec::new();
        let mut expr = None;

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            // Try to parse a statement
            let stmt = self.parse_statement();

            match stmt {
                Statement::Expr {
                    expr: e,
                    has_semi: false,
                } if self.check(TokenKind::RBrace) => {
                    // Expression at end of block without semicolon = block expression
                    expr = Some(Box::new(e));
                    break;
                }
                _ => {
                    statements.push(stmt);
                }
            }
        }

        self.expect(TokenKind::RBrace);

        Block {
            statements,
            expr,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a statement.
    fn parse_statement(&mut self) -> Statement {
        let start = self.current.span;

        match self.current.kind {
            TokenKind::Let => {
                self.advance();
                let pattern = self.parse_pattern();
                let ty = if self.try_consume(TokenKind::Colon) {
                    Some(self.parse_type())
                } else {
                    None
                };
                let value = if self.try_consume(TokenKind::Eq) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                self.expect(TokenKind::Semi);
                Statement::Let {
                    pattern,
                    ty,
                    value,
                    span: start.merge(self.previous.span),
                }
            }

            // Items in blocks
            TokenKind::Fn
            | TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Type
            | TokenKind::Const => {
                if let Some(decl) = self.parse_declaration() {
                    Statement::Item(decl)
                } else {
                    // Error recovery - return empty expression statement
                    Statement::Expr {
                        expr: Expr {
                            kind: ExprKind::Tuple(Vec::new()),
                            span: self.current.span,
                        },
                        has_semi: true,
                    }
                }
            }

            _ => {
                let expr = self.parse_expr();
                let has_semi = self.try_consume(TokenKind::Semi);
                Statement::Expr { expr, has_semi }
            }
        }
    }
}
