//! Pattern parsing.

use super::Parser;
use crate::ast::*;
use crate::lexer::TokenKind;
use crate::span::Spanned;

impl<'src> Parser<'src> {
    /// Parse a pattern.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_pattern(&mut self) -> Pattern {
        self.parse_pattern_or()
    }

    /// Parse a pattern without allowing or-patterns at the top level.
    /// Used for closure parameters where `|` delimits the parameter list.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_pattern_no_or(&mut self) -> Pattern {
        self.parse_pattern_primary()
    }

    /// Parse an or-pattern: `A | B | C`.
    fn parse_pattern_or(&mut self) -> Pattern {
        let start = self.current.span;
        let first = self.parse_pattern_primary();

        if self.check(TokenKind::Or) {
            let mut patterns = vec![first];
            while self.try_consume(TokenKind::Or) {
                patterns.push(self.parse_pattern_primary());
            }
            Pattern {
                kind: PatternKind::Or(patterns),
                span: start.merge(self.previous.span),
            }
        } else {
            first
        }
    }

    /// Parse a primary pattern.
    fn parse_pattern_primary(&mut self) -> Pattern {
        let start = self.current.span;

        match self.current.kind {
            // Wildcard pattern
            TokenKind::Ident if self.text(&start) == "_" => {
                self.advance();
                Pattern {
                    kind: PatternKind::Wildcard,
                    span: start,
                }
            }

            // Rest pattern
            TokenKind::DotDot => {
                self.advance();
                Pattern {
                    kind: PatternKind::Rest,
                    span: start,
                }
            }

            // Literal patterns (including negative)
            TokenKind::Minus => {
                self.advance();
                if self.check(TokenKind::IntLit) || self.check(TokenKind::FloatLit) {
                    let mut lit = self.parse_literal();
                    // Negate the literal
                    match &mut lit.kind {
                        LiteralKind::Int { .. } => {
                            // For simplicity, store as u128 and handle sign during type checking
                        }
                        LiteralKind::Float { value, .. } => {
                            *value = (-value.0).into();
                        }
                        // Only Int and Float literals can be preceded by minus.
                        // We already checked for IntLit/FloatLit token, so other variants
                        // should never occur here.
                        LiteralKind::String(_) | LiteralKind::Char(_) | LiteralKind::Bool(_) => {
                            // Unreachable: we check for IntLit/FloatLit before calling parse_literal
                        }
                    }
                    lit.span = start.merge(lit.span);
                    Pattern {
                        kind: PatternKind::Literal(lit),
                        span: start.merge(self.previous.span),
                    }
                } else {
                    self.error_expected("number after `-`");
                    Pattern {
                        kind: PatternKind::Wildcard,
                        span: start,
                    }
                }
            }

            TokenKind::IntLit
            | TokenKind::FloatLit
            | TokenKind::StringLit
            | TokenKind::CharLit
            | TokenKind::True
            | TokenKind::False => {
                let lit = self.parse_literal();

                // Check for range pattern
                if self.check(TokenKind::DotDot) || self.check(TokenKind::DotDotEq) {
                    let inclusive = self.current.kind == TokenKind::DotDotEq;
                    self.advance();

                    let end = if self.check(TokenKind::IntLit)
                        || self.check(TokenKind::FloatLit)
                        || self.check(TokenKind::CharLit)
                    {
                        let end_lit = self.parse_literal();
                        Some(Box::new(Pattern {
                            kind: PatternKind::Literal(end_lit),
                            span: self.previous.span,
                        }))
                    } else {
                        None
                    };

                    return Pattern {
                        kind: PatternKind::Range {
                            start: Some(Box::new(Pattern {
                                kind: PatternKind::Literal(lit),
                                span: start,
                            })),
                            end,
                            inclusive,
                        },
                        span: start.merge(self.previous.span),
                    };
                }

                Pattern {
                    kind: PatternKind::Literal(lit),
                    span: start.merge(self.previous.span),
                }
            }

            // Reference pattern
            TokenKind::And => {
                self.advance();
                let mutable = self.try_consume(TokenKind::Mut);
                let inner = self.parse_pattern_primary();
                Pattern {
                    kind: PatternKind::Ref {
                        mutable,
                        inner: Box::new(inner),
                    },
                    span: start.merge(self.previous.span),
                }
            }

            // Identifier pattern (with optional ref/mut and @ subpattern)
            TokenKind::Ref => {
                self.advance();
                let mutable = self.try_consume(TokenKind::Mut);

                let name = if self.check(TokenKind::Ident) {
                    self.advance();
                    self.spanned_symbol()
                } else {
                    self.error_expected("identifier");
                    Spanned::new(self.intern(""), self.current.span)
                };

                let subpattern = if self.try_consume(TokenKind::At) {
                    Some(Box::new(self.parse_pattern_primary()))
                } else {
                    None
                };

                Pattern {
                    kind: PatternKind::Ident {
                        by_ref: true,
                        mutable,
                        name,
                        subpattern,
                    },
                    span: start.merge(self.previous.span),
                }
            }

            TokenKind::Mut => {
                self.advance();

                let name = if self.check(TokenKind::Ident) {
                    self.advance();
                    self.spanned_symbol()
                } else {
                    self.error_expected("identifier");
                    Spanned::new(self.intern(""), self.current.span)
                };

                let subpattern = if self.try_consume(TokenKind::At) {
                    Some(Box::new(self.parse_pattern_primary()))
                } else {
                    None
                };

                Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: true,
                        name,
                        subpattern,
                    },
                    span: start.merge(self.previous.span),
                }
            }

            // Identifier, path, struct, or tuple struct pattern
            TokenKind::Ident | TokenKind::TypeIdent | TokenKind::SelfLower | TokenKind::SelfUpper => {
                self.parse_ident_or_path_pattern()
            }

            // Tuple pattern
            TokenKind::LParen => self.parse_tuple_pattern(),

            // Slice pattern
            TokenKind::LBracket => self.parse_slice_pattern(),

            _ => {
                self.error_expected_one_of(&["identifier", "literal", "`_`", "`(`", "`[`", "`..`"]);
                // Advance to prevent infinite loop during error recovery
                self.advance();
                Pattern {
                    kind: PatternKind::Wildcard,
                    span: start,
                }
            }
        }
    }

    /// Parse an identifier pattern or path-based pattern (struct/enum).
    fn parse_ident_or_path_pattern(&mut self) -> Pattern {
        let start = self.current.span;

        // Parse the path
        let path = self.parse_type_path();

        // Check what follows
        match self.current.kind {
            // Struct pattern
            TokenKind::LBrace => {
                self.advance();
                let (fields, rest) = self.parse_struct_pattern_fields();
                self.expect(TokenKind::RBrace);
                Pattern {
                    kind: PatternKind::Struct { path, fields, rest },
                    span: start.merge(self.previous.span),
                }
            }

            // Tuple struct pattern
            TokenKind::LParen => {
                self.advance();
                let (fields, rest_pos) = self.parse_tuple_pattern_fields();
                self.expect(TokenKind::RParen);
                Pattern {
                    kind: PatternKind::TupleStruct { path, fields, rest_pos },
                    span: start.merge(self.previous.span),
                }
            }

            // @ subpattern (only valid for single identifier)
            TokenKind::At if path.segments.len() == 1 && path.segments[0].args.is_none() => {
                self.advance();
                let name = path.segments[0].name.clone();
                let subpattern = Some(Box::new(self.parse_pattern_primary()));
                Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name,
                        subpattern,
                    },
                    span: start.merge(self.previous.span),
                }
            }

            // Range pattern with path as start
            TokenKind::DotDot | TokenKind::DotDotEq if path.segments.len() == 1 => {
                let inclusive = self.current.kind == TokenKind::DotDotEq;
                self.advance();

                let end = if self.current.kind.can_start_expr() {
                    Some(Box::new(self.parse_pattern_primary()))
                } else {
                    None
                };

                Pattern {
                    kind: PatternKind::Range {
                        start: Some(Box::new(Pattern {
                            kind: PatternKind::Path(path),
                            span: start,
                        })),
                        end,
                        inclusive,
                    },
                    span: start.merge(self.previous.span),
                }
            }

            _ => {
                // If it's a single identifier without args, treat as ident pattern
                if path.segments.len() == 1 && path.segments[0].args.is_none() {
                    let name = path.segments[0].name.clone();
                    Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name,
                            subpattern: None,
                        },
                        span: start.merge(self.previous.span),
                    }
                } else {
                    // Path pattern (e.g., None, std::option::None)
                    Pattern {
                        kind: PatternKind::Path(path),
                        span: start.merge(self.previous.span),
                    }
                }
            }
        }
    }

    /// Parse struct pattern fields.
    fn parse_struct_pattern_fields(&mut self) -> (Vec<StructPatternField>, bool) {
        let mut fields = Vec::new();
        let mut has_rest = false;

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            // Check for rest pattern
            if self.try_consume(TokenKind::DotDot) {
                has_rest = true;
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

            // Check for pattern (name: pattern) or shorthand (name)
            let pattern = if self.try_consume(TokenKind::Colon) {
                Some(self.parse_pattern())
            } else {
                None
            };

            fields.push(StructPatternField {
                name,
                pattern,
                span: field_start.merge(self.previous.span),
            });

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        (fields, has_rest)
    }

    /// Parse tuple pattern.
    fn parse_tuple_pattern(&mut self) -> Pattern {
        let start = self.current.span;
        self.advance(); // consume '('

        let (fields, rest_pos) = self.parse_tuple_pattern_fields();
        self.expect(TokenKind::RParen);

        // If single element with no comma and no rest, it's a paren pattern
        if fields.len() == 1 && rest_pos.is_none() {
            // Check if there was a trailing comma - if not, it's just paren
            // For now, treat single element as paren pattern
            // Safe: we just checked fields.len() == 1, so pop() returns the only element
            let field = fields.into_iter().next()
                .expect("BUG: checked len == 1 but no element found");
            return Pattern {
                kind: PatternKind::Paren(Box::new(field)),
                span: start.merge(self.previous.span),
            };
        }

        Pattern {
            kind: PatternKind::Tuple { fields, rest_pos },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse tuple pattern fields.
    fn parse_tuple_pattern_fields(&mut self) -> (Vec<Pattern>, Option<usize>) {
        let mut fields = Vec::new();
        let mut rest_pos = None;

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            // Check for rest pattern
            if self.try_consume(TokenKind::DotDot) {
                rest_pos = Some(fields.len());
                if !self.try_consume(TokenKind::Comma) {
                    break;
                }
                continue;
            }

            fields.push(self.parse_pattern());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        (fields, rest_pos)
    }

    /// Parse slice pattern.
    fn parse_slice_pattern(&mut self) -> Pattern {
        let start = self.current.span;
        self.advance(); // consume '['

        let mut elements = Vec::new();
        let mut rest_pos = None;

        while !self.check(TokenKind::RBracket) && !self.is_at_end() {
            // Check for rest pattern
            if self.try_consume(TokenKind::DotDot) {
                rest_pos = Some(elements.len());
                if !self.try_consume(TokenKind::Comma) {
                    break;
                }
                continue;
            }

            elements.push(self.parse_pattern());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        self.expect(TokenKind::RBracket);

        Pattern {
            kind: PatternKind::Slice { elements, rest_pos },
            span: start.merge(self.previous.span),
        }
    }
}
