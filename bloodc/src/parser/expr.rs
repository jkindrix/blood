//! Expression parsing using Pratt parsing for operator precedence.

use super::Parser;
use crate::ast::*;
use crate::lexer::TokenKind;
use crate::span::{Span, Spanned};

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
        TokenKind::Plus | TokenKind::Minus | TokenKind::PlusPlus => Some(Precedence::Term),
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
        TokenKind::PlusPlus => Some(BinOp::Concat),
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

            // Check for `&mut` which is unary borrow, not binary AND
            // This handles cases like: `if x { return &mut []; } &mut self.buffer[0..1]`
            // where the `&` after the if block is a new unary expression, not binary AND
            if self.current.kind == TokenKind::And && self.check_next(TokenKind::Mut) {
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
                        // This should not be reachable: binary_precedence returned Some,
                        // so we should have a known binary operator. If we get here,
                        // there's a mismatch between binary_precedence and token_to_binop.
                        // Report an internal error rather than silently ignoring.
                        self.error_at(
                            self.previous.span,
                            &format!(
                                "internal parser error: binary_precedence accepted token {:?} but token_to_binop rejected it",
                                op_token
                            ),
                            crate::diagnostics::ErrorCode::UnexpectedToken,
                        );
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

    /// Try to convert an expression that looks like a field access chain into path segments.
    /// Returns None if the expression is not a valid path (e.g., contains method calls, indexing).
    /// This is used to handle `.TypeIdent::` patterns where field access was incorrectly parsed.
    fn expr_to_path_segments(expr: &Expr) -> Option<Vec<ExprPathSegment>> {
        match &expr.kind {
            ExprKind::Path(path) => {
                // Already a path - clone its segments
                Some(path.segments.clone())
            }
            ExprKind::Field { base, field } => {
                // Recursively convert base, then append this field name
                let mut segments = Self::expr_to_path_segments(base)?;
                if let FieldAccess::Named(name) = field {
                    segments.push(ExprPathSegment {
                        name: name.clone(),
                        args: None,
                    });
                    Some(segments)
                } else {
                    // Tuple index - not convertible to path
                    None
                }
            }
            _ => None, // Other expression kinds can't be converted to paths
        }
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
                    } else if self.check_ident() {
                        // Allow contextual keywords as field names
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
                    } else if self.check(TokenKind::TypeIdent) {
                        // TypeIdent after dot indicates a path continuation, not field access.
                        // Field names are always lowercase, so `.TypeIdent` must be a path.
                        // Convert the expression-so-far to path segments and continue as path.
                        if let Some(mut segments) = Self::expr_to_path_segments(&expr) {
                            let start = expr.span;
                            // Parse the remaining path segments starting with this TypeIdent
                            loop {
                                // Parse the segment name (TypeIdent or ident after ::)
                                if self.check(TokenKind::TypeIdent)
                                    || self.check_ident()
                                    || self.check(TokenKind::SelfUpper)
                                {
                                    self.advance();
                                    let name = self.spanned_symbol();

                                    // Parse optional turbofish
                                    let has_turbofish =
                                        self.check(TokenKind::ColonColon) && self.check_next(TokenKind::Lt);
                                    let args = if has_turbofish {
                                        self.advance(); // consume ::
                                        Some(self.parse_type_args())
                                    } else {
                                        None
                                    };

                                    segments.push(ExprPathSegment { name, args });

                                    if has_turbofish {
                                        break;
                                    }

                                    // Continue on `::` or `.TypeIdent`
                                    if self.try_consume(TokenKind::ColonColon) {
                                        continue;
                                    }
                                    if self.check(TokenKind::Dot)
                                        && self.next.kind == TokenKind::TypeIdent
                                    {
                                        self.advance(); // consume `.`
                                        continue;
                                    }
                                    break;
                                } else {
                                    break;
                                }
                            }

                            let span = start.merge(self.previous.span);
                            expr = Expr {
                                kind: ExprKind::Path(ExprPath { segments, span }),
                                span,
                            };
                        } else {
                            // Can't convert to path, give error
                            self.error_expected("field name or index");
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
        self.check_next(TokenKind::Lt)
    }

    /// Parse call arguments.
    fn parse_call_args(&mut self) -> Vec<CallArg> {
        let mut args = Vec::new();

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            let start = self.current.span;

            // Check for named argument: `name: expr`
            // We need lookahead to distinguish between `name: expr` and `name + expr`
            // Allow contextual keywords as argument names
            let name = if self.check_ident() {
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
                    // Build the initial path expression and continue if we see ::
                    let symbol = self.intern(maybe_name_text);
                    let mut segments = vec![ExprPathSegment {
                        name: Spanned::new(symbol, maybe_name),
                        args: None,
                    }];
                    let mut path_end = maybe_name;

                    // Continue parsing path segments if we see ::
                    // Allow contextual keywords as path segments
                    // Handle turbofish syntax (e.g., func::<T>)
                    while self.try_consume(TokenKind::ColonColon) {
                        // Check for turbofish: ::<TypeArgs>
                        if self.check(TokenKind::Lt) {
                            // Turbofish - parse type arguments for the last segment
                            let type_args = self.parse_type_args();
                            if let Some(last) = segments.last_mut() {
                                last.args = Some(type_args);
                            }
                            path_end = self.previous.span;
                            break; // Turbofish ends path parsing
                        } else if self.check_ident()
                            || self.check(TokenKind::TypeIdent)
                            || self.check(TokenKind::SelfLower)
                            || self.check(TokenKind::SelfUpper)
                        {
                            self.advance();
                            let seg_name = self.spanned_symbol();
                            path_end = self.previous.span;
                            segments.push(ExprPathSegment {
                                name: seg_name,
                                args: None,
                            });
                        } else {
                            self.error_expected("identifier or `<` after `::`");
                            break;
                        }
                    }

                    let path = ExprPath {
                        segments,
                        span: maybe_name.merge(path_end),
                    };

                    // Check for macro call (e.g., format!(...))
                    let mut expr = if self.check(TokenKind::Not) {
                        if self.check_next(TokenKind::LParen)
                            || self.check_next(TokenKind::LBracket)
                            || self.check_next(TokenKind::LBrace)
                        {
                            self.parse_macro_call(path, maybe_name)
                        } else {
                            Expr {
                                kind: ExprKind::Path(path),
                                span: maybe_name,
                            }
                        }
                    } else {
                        Expr {
                            kind: ExprKind::Path(path),
                            span: maybe_name,
                        }
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
            | TokenKind::ByteStringLit
            | TokenKind::RawStringLit
            | TokenKind::RawStringLitHash1
            | TokenKind::RawStringLitHash2
            | TokenKind::CharLit
            | TokenKind::ByteCharLit
            | TokenKind::True
            | TokenKind::False => {
                let lit = self.parse_literal();
                Expr {
                    span: lit.span,
                    kind: ExprKind::Literal(lit),
                }
            }

            // Handle expression: `handle expr { clauses }` or `handle expr with handler { clauses }`
            // Note: `handle` can also be used as an identifier (variable name or function name)
            TokenKind::Handle => {
                // Check if this is a handle expression or just an identifier
                // `handle` is an identifier when followed by:
                //   - Expression terminators: `)`, `,`, `;`, `}`
                //   - Field access: `.` (e.g., `handle.id`)
                //   - Function call: `(` (e.g., `handle(args)` as function call)
                // `handle` starts a handle expression otherwise (parses handle expr { ... })
                if self.check_next(TokenKind::RParen)
                    || self.check_next(TokenKind::Comma)
                    || self.check_next(TokenKind::Semi)
                    || self.check_next(TokenKind::RBrace)
                    || self.check_next(TokenKind::Dot)
                    || self.check_next(TokenKind::LParen)
                {
                    // `handle` used as identifier (e.g., `foo(handle)`, `handle.field`, `handle(args)`)
                    self.parse_path_or_struct_expr()
                } else {
                    // Handle expression: `handle expr { ... }` or `handle expr with handler ...`
                    self.parse_handle_expr()
                }
            }

            // Identifiers and paths (including contextual keywords usable as identifiers)
            // Note: Contextual keywords can be identifiers in names (fn default, field handle)
            // They are parsed as identifiers/paths first, not as special expressions.
            // Crate and Super can start paths (crate.module.Type, super.module.Type)
            // Reserved keywords (catch, try, finally, throw, yield) can be used as function names
            TokenKind::Ident | TokenKind::TypeIdent | TokenKind::SelfLower | TokenKind::SelfUpper
            | TokenKind::Default | TokenKind::Crate | TokenKind::Super
            | TokenKind::Op | TokenKind::Module | TokenKind::Ref | TokenKind::Pure
            | TokenKind::Linear | TokenKind::Affine | TokenKind::Extends
            | TokenKind::Handler | TokenKind::Effect | TokenKind::Deep | TokenKind::Shallow | TokenKind::Resume
            | TokenKind::Catch | TokenKind::Try | TokenKind::Finally | TokenKind::Throw | TokenKind::Yield => {
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

            // Closure (|| for zero params, |x| for params, move || or move |x|)
            TokenKind::Or | TokenKind::OrOr | TokenKind::Move => self.parse_closure_expr(),

            // Effect expressions
            TokenKind::Try => self.parse_try_with_expr(),
            TokenKind::With => self.parse_with_handle_expr(),
            TokenKind::Perform => self.parse_perform_expr(),
            // Note: `resume` is handled as an identifier in the path case above.
            // When followed by `(`, it gets parsed as a function call which is then
            // converted to ExprKind::Resume during postfix parsing.

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

            // Rust-style `unsafe { }` is not supported - provide helpful error
            TokenKind::Unsafe => {
                self.error_at_current(
                    "`unsafe { }` blocks use `@unsafe { }` syntax in Blood",
                    crate::diagnostics::ErrorCode::UnsupportedSyntax,
                );
                self.advance();
                // Parse as @unsafe to attempt recovery
                if self.check(TokenKind::LBrace) {
                    let block = self.parse_block();
                    Expr {
                        kind: ExprKind::Unsafe(block),
                        span: start.merge(self.previous.span),
                    }
                } else {
                    // Return placeholder to continue parsing
                    Expr {
                        kind: ExprKind::Tuple(Vec::new()),
                        span: start.merge(self.previous.span),
                    }
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

    /// Parse a path expression, struct literal, or macro call.
    fn parse_path_or_struct_expr(&mut self) -> Expr {
        let start = self.previous.span;
        let path = self.parse_expr_path();

        // Check for macro call syntax (e.g., println!, vec!, format!)
        if self.check(TokenKind::Not) {
            // Check if this looks like a macro call (followed by delimiter)
            if self.check_next(TokenKind::LParen)
                || self.check_next(TokenKind::LBracket)
                || self.check_next(TokenKind::LBrace)
            {
                return self.parse_macro_call(path, start);
            }
        }

        // Check for struct literal
        if self.check(TokenKind::LBrace) && !self.is_statement_context() {
            return self.parse_struct_literal(Some(self.path_to_type_path(&path)));
        }

        Expr {
            span: path.span,
            kind: ExprKind::Path(path),
        }
    }

    /// Parse a macro call after the path has been parsed.
    /// Called when we see `path!` followed by a delimiter.
    fn parse_macro_call(&mut self, path: ExprPath, start: Span) -> Expr {
        use crate::ast::MacroDelimiter;

        // Get the macro name for dispatch
        let macro_name = if let Some(seg) = path.segments.last() {
            self.interner_symbol_str(seg.name.node).to_string()
        } else {
            String::new()
        };

        // Consume the `!`
        self.advance();

        // Determine delimiter and parse accordingly
        let (delim, _open_kind, close_kind) = match self.current.kind {
            TokenKind::LParen => (MacroDelimiter::Paren, TokenKind::LParen, TokenKind::RParen),
            TokenKind::LBracket => (MacroDelimiter::Bracket, TokenKind::LBracket, TokenKind::RBracket),
            TokenKind::LBrace => (MacroDelimiter::Brace, TokenKind::LBrace, TokenKind::RBrace),
            _ => {
                self.error_at_current(
                    "expected `(`, `[`, or `{` after macro name",
                    crate::diagnostics::ErrorCode::UnexpectedToken,
                );
                return Expr {
                    span: start.merge(self.previous.span),
                    kind: ExprKind::Tuple(Vec::new()),
                };
            }
        };

        // Consume opening delimiter
        self.advance();

        // Parse based on macro name
        let kind = match macro_name.as_str() {
            // Format-style macros: format!("...", args), println!("...", args), etc.
            // Also includes todo! and unimplemented! which behave like panic! with default messages
            "format" | "println" | "print" | "eprintln" | "eprint" | "panic" | "todo"
            | "unimplemented" => self.parse_format_macro_args(delim, close_kind),

            // Write macros: write!(dest, "...", args) - destination first, then format string
            "write" | "writeln" => self.parse_write_macro_args(close_kind),

            // Vec macro: vec![1, 2, 3] or vec![0; 10]
            "vec" => {
                self.parse_vec_macro_args(close_kind)
            }

            // Assert macros: assert!(cond) or assert!(cond, "message")
            "assert" | "debug_assert" => {
                self.parse_assert_macro_args(close_kind)
            }

            // Dbg macro: dbg!(expr)
            "dbg" => {
                self.parse_dbg_macro_args(close_kind)
            }

            // Matches macro: matches!(expr, pattern)
            "matches" => {
                self.parse_matches_macro_args(close_kind)
            }

            // Unknown macro - store as custom for future user-defined macro support
            _ => {
                self.parse_custom_macro_args(delim, close_kind)
            }
        };

        let end_span = self.previous.span;
        Expr {
            span: start.merge(end_span),
            kind: ExprKind::MacroCall { path, kind },
        }
    }

    /// Parse format-style macro arguments: `"format string", arg1, arg2, ...`
    fn parse_format_macro_args(
        &mut self,
        _delim: crate::ast::MacroDelimiter,
        close_kind: TokenKind,
    ) -> crate::ast::MacroCallKind {
        use crate::ast::MacroCallKind;
        use crate::span::Spanned;

        // Empty macro call like println!()
        if self.check(close_kind) {
            self.advance();
            return MacroCallKind::Format {
                format_str: Spanned {
                    node: String::new(),
                    span: self.previous.span,
                },
                args: Vec::new(),
            };
        }

        // First argument should be a string literal (format string)
        let format_str = if self.check(TokenKind::StringLit) {
            let span = self.current.span;
            let s = self.parse_string_literal_content();
            Spanned { node: s, span }
        } else {
            // Not a string literal - treat as expression
            self.error_at_current(
                "expected string literal as format string",
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
            Spanned {
                node: String::new(),
                span: self.current.span,
            }
        };

        // Parse remaining arguments
        let mut args = Vec::new();
        while self.check(TokenKind::Comma) {
            self.advance(); // consume comma
            if self.check(close_kind) {
                break; // trailing comma
            }
            args.push(self.parse_expr());
        }

        // Consume closing delimiter
        if !self.check(close_kind) {
            self.error_at_current(
                &format!("expected `{}` to close macro", close_kind.description()),
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
        } else {
            self.advance();
        }

        MacroCallKind::Format { format_str, args }
    }

    /// Parse write! macro arguments: `dest, "format string", arg1, arg2, ...`
    fn parse_write_macro_args(&mut self, close_kind: TokenKind) -> crate::ast::MacroCallKind {
        use crate::ast::MacroCallKind;
        use crate::span::Spanned;

        // Empty macro call is an error for write!
        if self.check(close_kind) {
            self.error_at_current(
                "write! requires at least a destination",
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
            self.advance();
            return MacroCallKind::Write {
                dest: Box::new(Expr {
                    span: self.previous.span,
                    kind: ExprKind::Tuple(Vec::new()),
                }),
                format_str: Spanned {
                    node: String::new(),
                    span: self.previous.span,
                },
                args: Vec::new(),
            };
        }

        // First argument is the destination
        let dest = Box::new(self.parse_expr());

        // Expect comma after destination
        if !self.check(TokenKind::Comma) {
            self.error_at_current(
                "expected `,` after write! destination",
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
            // Try to recover
            if self.check(close_kind) {
                self.advance();
            }
            return MacroCallKind::Write {
                dest,
                format_str: Spanned {
                    node: String::new(),
                    span: self.previous.span,
                },
                args: Vec::new(),
            };
        }
        self.advance(); // consume comma

        // Second argument should be a string literal (format string)
        let format_str = if self.check(TokenKind::StringLit) {
            let span = self.current.span;
            let s = self.parse_string_literal_content();
            Spanned { node: s, span }
        } else {
            self.error_at_current(
                "expected string literal as format string",
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
            Spanned {
                node: String::new(),
                span: self.current.span,
            }
        };

        // Parse remaining arguments
        let mut args = Vec::new();
        while self.check(TokenKind::Comma) {
            self.advance(); // consume comma
            if self.check(close_kind) {
                break; // trailing comma
            }
            args.push(self.parse_expr());
        }

        // Consume closing delimiter
        if !self.check(close_kind) {
            self.error_at_current(
                &format!("expected `{}` to close macro", close_kind.description()),
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
        } else {
            self.advance();
        }

        MacroCallKind::Write {
            dest,
            format_str,
            args,
        }
    }

    /// Parse vec! macro arguments: `[1, 2, 3]` or `[0; 10]`
    fn parse_vec_macro_args(&mut self, close_kind: TokenKind) -> crate::ast::MacroCallKind {
        use crate::ast::{MacroCallKind, VecMacroArgs};

        // Empty vec
        if self.check(close_kind) {
            self.advance();
            return MacroCallKind::Vec(VecMacroArgs::List(Vec::new()));
        }

        // Parse first expression
        let first = self.parse_expr();

        // Check for repeat syntax: `[value; count]`
        if self.check(TokenKind::Semi) {
            self.advance();
            let count = self.parse_expr();

            if !self.check(close_kind) {
                self.error_at_current(
                    &format!("expected `{}` after repeat count", close_kind.description()),
                    crate::diagnostics::ErrorCode::UnexpectedToken,
                );
            } else {
                self.advance();
            }

            return MacroCallKind::Vec(VecMacroArgs::Repeat {
                value: Box::new(first),
                count: Box::new(count),
            });
        }

        // List syntax: `[a, b, c]`
        let mut elements = vec![first];
        while self.check(TokenKind::Comma) {
            self.advance();
            if self.check(close_kind) {
                break; // trailing comma
            }
            elements.push(self.parse_expr());
        }

        if !self.check(close_kind) {
            self.error_at_current(
                &format!("expected `{}` to close vec!", close_kind.description()),
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
        } else {
            self.advance();
        }

        MacroCallKind::Vec(VecMacroArgs::List(elements))
    }

    /// Parse assert! macro arguments: `(condition)` or `(condition, "message")`
    fn parse_assert_macro_args(&mut self, close_kind: TokenKind) -> crate::ast::MacroCallKind {
        use crate::ast::MacroCallKind;

        if self.check(close_kind) {
            self.error_at_current(
                "assert! requires a condition",
                crate::diagnostics::ErrorCode::ExpectedExpression,
            );
            self.advance();
            return MacroCallKind::Assert {
                condition: Box::new(Expr {
                    kind: ExprKind::Literal(Literal {
                        kind: LiteralKind::Bool(false),
                        span: self.previous.span,
                    }),
                    span: self.previous.span,
                }),
                message: None,
            };
        }

        let condition = self.parse_expr();
        let message = if self.check(TokenKind::Comma) {
            self.advance();
            if !self.check(close_kind) {
                Some(Box::new(self.parse_expr()))
            } else {
                None
            }
        } else {
            None
        };

        if !self.check(close_kind) {
            self.error_at_current(
                &format!("expected `{}` to close assert!", close_kind.description()),
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
        } else {
            self.advance();
        }

        MacroCallKind::Assert { condition: Box::new(condition), message }
    }

    /// Parse dbg! macro arguments: `(expr)`
    fn parse_dbg_macro_args(&mut self, close_kind: TokenKind) -> crate::ast::MacroCallKind {
        use crate::ast::MacroCallKind;

        if self.check(close_kind) {
            self.error_at_current(
                "dbg! requires an expression",
                crate::diagnostics::ErrorCode::ExpectedExpression,
            );
            self.advance();
            return MacroCallKind::Dbg(Box::new(Expr {
                kind: ExprKind::Tuple(Vec::new()),
                span: self.previous.span,
            }));
        }

        let expr = self.parse_expr();

        if !self.check(close_kind) {
            self.error_at_current(
                &format!("expected `{}` to close dbg!", close_kind.description()),
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
        } else {
            self.advance();
        }

        MacroCallKind::Dbg(Box::new(expr))
    }

    /// Parse matches! macro arguments: `(expr, pattern)`
    fn parse_matches_macro_args(&mut self, close_kind: TokenKind) -> crate::ast::MacroCallKind {
        use crate::ast::MacroCallKind;

        if self.check(close_kind) {
            self.error_at_current(
                "matches! requires an expression and a pattern",
                crate::diagnostics::ErrorCode::ExpectedExpression,
            );
            self.advance();
            // Return a dummy matches that will fail type checking
            return MacroCallKind::Matches {
                expr: Box::new(Expr {
                    kind: ExprKind::Tuple(Vec::new()),
                    span: self.previous.span,
                }),
                pattern: Box::new(crate::ast::Pattern {
                    kind: crate::ast::PatternKind::Wildcard,
                    span: self.previous.span,
                }),
            };
        }

        // Parse the expression to match against
        let expr = self.parse_expr();

        // Expect a comma
        if !self.check(TokenKind::Comma) {
            self.error_at_current(
                "expected `,` after expression in matches!",
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
        } else {
            self.advance();
        }

        // Parse the pattern
        let pattern = self.parse_pattern();

        // Consume closing delimiter
        if !self.check(close_kind) {
            self.error_at_current(
                &format!("expected `{}` to close matches!", close_kind.description()),
                crate::diagnostics::ErrorCode::UnexpectedToken,
            );
        } else {
            self.advance();
        }

        MacroCallKind::Matches {
            expr: Box::new(expr),
            pattern: Box::new(pattern),
        }
    }

    /// Parse custom/unknown macro arguments as raw content (for future expansion).
    fn parse_custom_macro_args(
        &mut self,
        delim: crate::ast::MacroDelimiter,
        close_kind: TokenKind,
    ) -> crate::ast::MacroCallKind {
        use crate::ast::MacroCallKind;

        // For now, just skip tokens until we find the matching close delimiter
        // Track nesting depth for nested delimiters
        let start_pos = self.current.span.start;
        let mut depth = 1;

        let (open_kind, close_check) = match delim {
            crate::ast::MacroDelimiter::Paren => (TokenKind::LParen, TokenKind::RParen),
            crate::ast::MacroDelimiter::Bracket => (TokenKind::LBracket, TokenKind::RBracket),
            crate::ast::MacroDelimiter::Brace => (TokenKind::LBrace, TokenKind::RBrace),
        };

        while depth > 0 && !self.check(TokenKind::Eof) {
            if self.current.kind == open_kind {
                depth += 1;
            } else if self.current.kind == close_check {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            self.advance();
        }

        let end_pos = self.current.span.start;
        let content = self.source[start_pos..end_pos].to_string();

        if self.check(close_kind) {
            self.advance();
        }

        // User-defined macros are parsed as Custom and expanded later by macro_expand
        MacroCallKind::Custom { delim, content }
    }

    /// Parse string literal content, handling escape sequences.
    fn parse_string_literal_content(&mut self) -> String {
        let span = self.current.span;
        let raw = &self.source[span.start..span.end];
        self.advance();

        // Remove quotes and process escapes
        if raw.len() >= 2 {
            let inner = &raw[1..raw.len() - 1];
            // For now, return as-is (escapes already processed by lexer)
            inner.to_string()
        } else {
            String::new()
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
        let start = self.current.span;

        // For identifiers/paths (including contextual keywords), don't allow struct literals but DO allow macros
        if matches!(
            self.current.kind,
            TokenKind::Ident | TokenKind::TypeIdent | TokenKind::SelfLower | TokenKind::SelfUpper
            | TokenKind::Default | TokenKind::Op | TokenKind::Module
            | TokenKind::Ref | TokenKind::Pure | TokenKind::Linear
            | TokenKind::Affine | TokenKind::Extends | TokenKind::Handler | TokenKind::Effect
            | TokenKind::Deep | TokenKind::Shallow | TokenKind::Crate | TokenKind::Super
        ) {
            let path = self.parse_expr_path();

            // Check for macro call syntax (e.g., matches!, vec!)
            if self.check(TokenKind::Not) {
                if self.check_next(TokenKind::LParen)
                    || self.check_next(TokenKind::LBracket)
                    || self.check_next(TokenKind::LBrace)
                {
                    return self.parse_macro_call(path, start);
                }
            }

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
    /// Blood uses `.` for module paths and `::` for type/item access:
    /// `crate.compiler.ast.expr.BinOp::Add`
    ///
    /// IMPORTANT: Only "path roots" can use `.` for module path continuation:
    /// - `crate.xxx` - module path
    /// - `super.xxx` - module path
    /// - `Self.xxx` - type path (Self is a type)
    /// - `TypeIdent.xxx` - type/module path (starts with uppercase)
    ///
    /// Lowercase identifiers (local variables, parameters) use `.` for field/method
    /// access, which is handled by postfix parsing, NOT path parsing.
    /// - `foo.bar()` - foo is a variable, .bar() is a method call
    /// - `self.field` - self is the receiver, .field is field access
    fn parse_expr_path(&mut self) -> ExprPath {
        let start = self.current.span;
        let mut segments = Vec::new();

        // Determine if the first token is a "path root" that can use `.` continuation.
        // Path roots: crate, super, Self (uppercase), TypeIdent (uppercase identifiers)
        // Non-path-roots: self (lowercase), Ident (lowercase variables/params)
        let is_path_root = matches!(
            self.current.kind,
            TokenKind::Crate | TokenKind::Super | TokenKind::SelfUpper | TokenKind::TypeIdent
        );

        // Track whether we've seen `::` as separator - once used, `.` should NOT continue path
        // e.g., `Option::Some.method()` - the `.method()` is NOT part of the path
        // e.g., `std.compiler.ast` - `.` IS part of the path (no `::` used)
        let mut used_double_colon = false;

        loop {
            // Allow contextual keywords as path segments
            // Also allow crate and super as path roots
            let name = if self.check_ident()
                || self.check(TokenKind::TypeIdent)
                || self.check(TokenKind::SelfLower)
                || self.check(TokenKind::SelfUpper)
                || self.check(TokenKind::Crate)
                || self.check(TokenKind::Super)
            {
                self.advance();
                self.spanned_symbol()
            } else {
                break;
            };

            // Parse optional turbofish (::< for explicit type arguments)
            let has_turbofish = self.check(TokenKind::ColonColon) && self.check_next(TokenKind::Lt);
            let args = if has_turbofish {
                // Consume :: and parse type arguments
                self.advance(); // consume ::
                Some(self.parse_type_args())
            } else {
                None
            };

            segments.push(ExprPathSegment { name, args });

            // If we parsed turbofish, don't continue path - the call/use comes next
            if has_turbofish {
                break;
            }

            // Check for path continuation
            // `::` is always valid for path continuation (e.g., Option::Some)
            if self.try_consume(TokenKind::ColonColon) {
                used_double_colon = true;
                continue;
            }

            // `.` is only valid for path roots (crate, super, Self, TypeIdent)
            // AND only if we haven't used `::` yet (once `::` is used, `.` is postfix)
            // For lowercase identifiers, `.` should be field/method access (postfix)
            if is_path_root && !used_double_colon && self.try_consume(TokenKind::Dot) {
                continue;
            }

            // Special case for non-path-roots: allow `.` continuation if we see:
            // 1. `.ident::` which unambiguously indicates a module path (e.g., std.cmp::min)
            // 2. `.TypeIdent` since TypeIdents (uppercase) are never field names (e.g., std.option.Option)
            // This distinguishes module paths from field access - field names are always lowercase.
            // NOTE: Once `::` has been used, `.` should NOT continue path (it's postfix then)
            if !is_path_root && !used_double_colon && self.check(TokenKind::Dot) {
                let next_is_type_ident = self.next.kind == TokenKind::TypeIdent;
                let next_is_ident = matches!(
                    self.next.kind,
                    TokenKind::Ident | TokenKind::TypeIdent
                );
                // Continue if TypeIdent (never a field) or if followed by ::
                if next_is_type_ident || (next_is_ident && self.check_after_next(TokenKind::ColonColon)) {
                    self.advance(); // consume the `.`
                    continue;
                }
            }

            // No valid continuation - end of path
            break;
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
            // Allow contextual keywords as field names
            let name = if self.check_ident() {
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

            // Allow trailing comma or semicolon (Blood allows both)
            if !self.try_consume(TokenKind::Comma) {
                self.try_consume(TokenKind::Semi);
            }
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

        // Handle || (zero parameters) vs |x| (with parameters)
        let params = if self.try_consume(TokenKind::OrOr) {
            // || - empty parameter list
            Vec::new()
        } else {
            // |params|
            self.expect(TokenKind::Or);
            let params = self.parse_closure_params();
            self.expect(TokenKind::Or);
            params
        };

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

    /// Parse a handle expression: `handle expr { clauses }` or `handle expr with handler { clauses }`
    ///
    /// Clauses can be:
    /// - `return(v) { body }` - return handler
    /// - `op_name(params) { body }` - operation handler
    fn parse_handle_expr(&mut self) -> Expr {
        use crate::ast::{InlineHandlerClause, InlineHandlerClauseKind};

        let start = self.current.span;
        self.advance(); // consume 'handle'

        // Parse the expression to handle
        let body = self.parse_expr_prec_no_struct(Precedence::None);

        // Parse optional `with handler`
        let handler = if self.try_consume(TokenKind::With) {
            if !self.check(TokenKind::LBrace) {
                // `with handler` form - parse handler expression
                Some(Box::new(self.parse_expr_prec_no_struct(Precedence::None)))
            } else {
                None
            }
        } else {
            None
        };

        // Parse handler clauses in braces
        self.expect(TokenKind::LBrace);

        let mut clauses = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let clause_start = self.current.span;

            // Determine if this is `return(...)` or `op_name(...)`
            let kind = if self.try_consume(TokenKind::Return) {
                InlineHandlerClauseKind::Return
            } else if self.check_ident() || self.check(TokenKind::TypeIdent) {
                self.advance();
                let name = self.spanned_symbol();
                InlineHandlerClauseKind::Operation(name)
            } else {
                self.error_expected("`return` or operation name");
                self.advance();
                continue;
            };

            // Parse parameters
            self.expect(TokenKind::LParen);
            let mut params = Vec::new();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                params.push(self.parse_pattern());
                if !self.try_consume(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RParen);

            // Parse body
            let body = self.parse_block();

            clauses.push(InlineHandlerClause {
                kind,
                params,
                body,
                span: clause_start.merge(self.previous.span),
            });
        }

        self.expect(TokenKind::RBrace);

        Expr {
            kind: ExprKind::Handle {
                body: Box::new(body),
                handler,
                clauses,
            },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a try-with expression: `try { body } with { handlers }`
    fn parse_try_with_expr(&mut self) -> Expr {
        use crate::ast::TryWithHandler;

        let start = self.current.span;
        self.advance(); // consume 'try'

        // Parse the body block
        let body = self.parse_block();

        // Expect 'with'
        self.expect(TokenKind::With);

        // Parse handler clauses in braces
        self.expect(TokenKind::LBrace);

        let mut handlers = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let handler_start = self.current.span;

            // Parse Effect.operation or Effect::operation pattern
            // Support both syntaxes for consistency:
            //   - Emit.emit(value) =>    (matches `perform Emit.emit(...)`)
            //   - Emit::emit(value) =>   (Rust-style path syntax)
            let effect_path = self.parse_type_path();

            // Check for dot syntax (like perform uses) or use :: path segments
            let (effect, operation) = if self.try_consume(TokenKind::Dot) {
                // Dot syntax: Effect.operation - matches `perform Emit.emit(...)`
                let op = if self.check_ident() || self.check(TokenKind::TypeIdent) {
                    self.advance();
                    self.spanned_symbol()
                } else {
                    self.error_expected("operation name after '.'");
                    Spanned::new(self.intern(""), self.current.span)
                };
                (effect_path, op)
            } else if effect_path.segments.len() >= 2 {
                // Double-colon syntax: Effect::operation
                // Take all but the last segment as effect path
                let mut effect_segments = effect_path.segments.clone();
                let op_segment = effect_segments.pop().unwrap();
                let new_effect_path = crate::ast::TypePath {
                    segments: effect_segments,
                    span: effect_path.span,
                };
                (new_effect_path, op_segment.name)
            } else if effect_path.segments.len() == 1 {
                // Single segment - this is just the operation, no effect path
                let op_segment = &effect_path.segments[0];
                let empty_effect = crate::ast::TypePath {
                    segments: vec![],
                    span: effect_path.span,
                };
                (empty_effect, op_segment.name.clone())
            } else {
                self.error_expected("Effect.operation or Effect::operation pattern");
                let empty = crate::ast::TypePath { segments: vec![], span: self.current.span };
                (empty, Spanned::new(self.intern(""), self.current.span))
            };

            // Parse parameters
            self.expect(TokenKind::LParen);
            let mut params = Vec::new();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                params.push(self.parse_pattern());
                if !self.try_consume(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RParen);

            // Expect '=>' or ':'
            if self.check(TokenKind::FatArrow) {
                self.advance();
            } else if self.check(TokenKind::Colon) {
                self.advance();
            } else {
                self.expect(TokenKind::FatArrow); // This will generate error
            }

            // Parse handler body
            let handler_body = self.parse_block();

            handlers.push(TryWithHandler {
                effect,
                operation,
                params,
                body: handler_body,
                span: handler_start.merge(self.previous.span),
            });

            // Consume optional comma between handlers
            self.try_consume(TokenKind::Comma);
        }

        self.expect(TokenKind::RBrace);

        Expr {
            kind: ExprKind::TryWith { body, handlers },
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
            // Allow contextual keywords as operation names
            let op = if self.check_ident() {
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

            // Defer statement: `defer { ... };`
            TokenKind::Defer => {
                self.advance();
                let body = self.parse_block();
                self.try_consume(TokenKind::Semi);
                Statement::Defer {
                    body,
                    span: start.merge(self.previous.span),
                }
            }

            // Items in blocks (local definitions)
            TokenKind::Fn
            | TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Type
            | TokenKind::Const
            | TokenKind::Use
            | TokenKind::Impl => {
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
