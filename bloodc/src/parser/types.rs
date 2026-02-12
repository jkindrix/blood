//! Type parsing.

use super::Parser;
use crate::ast::*;
use crate::lexer::TokenKind;

impl<'src> Parser<'src> {
    /// Parse a type.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_type(&mut self) -> Type {
        let start = self.current.span;

        // Check for ownership qualifiers (both with and without @ prefix)
        if self.try_consume(TokenKind::Linear) || self.try_consume(TokenKind::AtLinear) {
            let inner = self.parse_type();
            return Type {
                kind: TypeKind::Ownership {
                    qualifier: OwnershipQualifier::Linear,
                    inner: Box::new(inner),
                },
                span: start.merge(self.previous.span),
            };
        }

        if self.try_consume(TokenKind::Affine) || self.try_consume(TokenKind::AtAffine) {
            let inner = self.parse_type();
            return Type {
                kind: TypeKind::Ownership {
                    qualifier: OwnershipQualifier::Affine,
                    inner: Box::new(inner),
                },
                span: start.merge(self.previous.span),
            };
        }

        // Parse primary type
        self.parse_primary_type()
    }

    /// Parse a primary type.
    fn parse_primary_type(&mut self) -> Type {
        let start = self.current.span;

        match self.current.kind {
            // Never type
            TokenKind::Not => {
                self.advance();
                Type {
                    kind: TypeKind::Never,
                    span: start,
                }
            }

            // Inferred type
            TokenKind::Ident if self.text(&start) == "_" => {
                self.advance();
                Type {
                    kind: TypeKind::Infer,
                    span: start,
                }
            }

            // Never type (alias for `!`)
            TokenKind::Ident if self.text(&start) == "never" => {
                self.advance();
                Type {
                    kind: TypeKind::Never,
                    span: start,
                }
            }

            // Reference type
            TokenKind::And => {
                self.advance();

                // Parse optional lifetime
                let lifetime = if self.check(TokenKind::Lifetime) {
                    self.advance();
                    Some(self.spanned_lifetime_symbol())
                } else {
                    None
                };

                // Parse optional mut
                let mutable = self.try_consume(TokenKind::Mut);

                let inner = self.parse_type();

                Type {
                    kind: TypeKind::Reference {
                        lifetime,
                        mutable,
                        inner: Box::new(inner),
                    },
                    span: start.merge(self.previous.span),
                }
            }

            // Pointer type
            TokenKind::Star => {
                self.advance();
                let mutable = if self.try_consume(TokenKind::Mut) {
                    true
                } else {
                    self.expect(TokenKind::Const);
                    false
                };

                let inner = self.parse_type();

                Type {
                    kind: TypeKind::Pointer {
                        mutable,
                        inner: Box::new(inner),
                    },
                    span: start.merge(self.previous.span),
                }
            }

            // Array or slice type
            TokenKind::LBracket => {
                self.advance();
                let element = self.parse_type();

                if self.try_consume(TokenKind::Semi) {
                    // Array type [T; N]
                    let size = self.parse_expr();
                    self.expect(TokenKind::RBracket);
                    Type {
                        kind: TypeKind::Array {
                            element: Box::new(element),
                            size: Box::new(size),
                        },
                        span: start.merge(self.previous.span),
                    }
                } else {
                    // Slice type [T]
                    self.expect(TokenKind::RBracket);
                    Type {
                        kind: TypeKind::Slice {
                            element: Box::new(element),
                        },
                        span: start.merge(self.previous.span),
                    }
                }
            }

            // Tuple type or parenthesized type
            TokenKind::LParen => {
                self.advance();

                // Unit type ()
                if self.try_consume(TokenKind::RParen) {
                    return Type {
                        kind: TypeKind::Tuple(Vec::new()),
                        span: start.merge(self.previous.span),
                    };
                }

                let first = self.parse_type();

                // Check for tuple
                if self.try_consume(TokenKind::Comma) {
                    let mut types = vec![first];
                    while !self.check(TokenKind::RParen) && !self.is_at_end() {
                        types.push(self.parse_type());
                        if !self.try_consume(TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen);
                    return Type {
                        kind: TypeKind::Tuple(types),
                        span: start.merge(self.previous.span),
                    };
                }

                self.expect(TokenKind::RParen);
                Type {
                    kind: TypeKind::Paren(Box::new(first)),
                    span: start.merge(self.previous.span),
                }
            }

            // Function type: `fn(T) -> U`, `fn(T) -> U / E`, `fn(T) / E`, `fn()`
            TokenKind::Fn => {
                self.advance();
                self.expect(TokenKind::LParen);

                let mut params = Vec::new();
                while !self.check(TokenKind::RParen) && !self.is_at_end() {
                    params.push(self.parse_type());
                    if !self.try_consume(TokenKind::Comma) {
                        break;
                    }
                }

                self.expect(TokenKind::RParen);

                // Return type is optional - defaults to unit if not present
                // This allows `fn() / {Effect}` syntax for effectful functions
                let return_type = if self.try_consume(TokenKind::Arrow) {
                    self.parse_type()
                } else {
                    // No arrow - use unit type as return type
                    Type {
                        kind: TypeKind::Tuple(Vec::new()),
                        span: self.previous.span,
                    }
                };

                let effects = if self.try_consume(TokenKind::Slash) {
                    Some(self.parse_effect_row())
                } else {
                    None
                };

                Type {
                    kind: TypeKind::Function {
                        params,
                        return_type: Box::new(return_type),
                        effects,
                    },
                    span: start.merge(self.previous.span),
                }
            }

            // Forall (higher-rank polymorphic) type: forall<T>. Type
            TokenKind::Forall => {
                self.advance();
                self.expect(TokenKind::Lt);

                let mut params = Vec::new();
                // Use check_closing_angle() to handle `>>` in nested contexts
                while !self.check_closing_angle() && !self.is_at_end() {
                    if self.check(TokenKind::TypeIdent) || self.check(TokenKind::Ident) {
                        self.advance();
                        params.push(self.spanned_symbol());
                    } else {
                        self.error_expected("type parameter name");
                        break;
                    }
                    if !self.try_consume(TokenKind::Comma) {
                        break;
                    }
                }

                self.expect_closing_angle();
                self.expect(TokenKind::Dot);
                let body = self.parse_type();

                Type {
                    kind: TypeKind::Forall {
                        params,
                        body: Box::new(body),
                    },
                    span: start.merge(self.previous.span),
                }
            }

            // Record type
            TokenKind::LBrace => self.parse_record_type(),

            // Type path
            TokenKind::TypeIdent
            | TokenKind::Ident
            | TokenKind::SelfUpper
            | TokenKind::Crate
            | TokenKind::Super => {
                let path = self.parse_type_path();
                Type {
                    span: path.span,
                    kind: TypeKind::Path(path),
                }
            }

            // Handle `mut Type` which is invalid syntax (should be `&mut Type` for mutable reference)
            // We need to consume both `mut` and the following type to prevent infinite loops
            TokenKind::Mut => {
                self.error_expected_one_of(&["type name", "`&`", "`*`", "`[`", "`(`", "`fn`", "`forall`", "`!`"]);
                self.advance(); // consume `mut`
                // Try to parse and discard the following type for better error recovery
                // Include contextual keywords in addition to standard type identifiers
                if self.check(TokenKind::TypeIdent)
                    || self.check_ident()
                    || self.check(TokenKind::SelfUpper)
                    || self.check(TokenKind::Crate)
                    || self.check(TokenKind::Super)
                {
                    let _ = self.parse_type_path(); // consume the following type
                }
                Type {
                    kind: TypeKind::Never,
                    span: start,
                }
            }

            _ => {
                self.error_expected_one_of(&["type name", "`&`", "`*`", "`[`", "`(`", "`fn`", "`forall`", "`!`"]);
                // Advance to prevent infinite loop during error recovery
                self.advance();
                Type {
                    kind: TypeKind::Never,
                    span: start,
                }
            }
        }
    }

    /// Parse a type path.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_type_path(&mut self) -> TypePath {
        let start = self.current.span;
        let mut segments = Vec::new();

        loop {
            // Allow contextual keywords as path segments (for identifier patterns)
            let name = if self.check(TokenKind::TypeIdent)
                || self.check_ident()
                || self.check(TokenKind::SelfUpper)
                || self.check(TokenKind::Crate)
                || self.check(TokenKind::Super)
            {
                self.advance();
                self.spanned_symbol()
            } else {
                break;
            };

            // Parse optional type arguments
            let args = if self.check(TokenKind::Lt) {
                Some(self.parse_type_args())
            } else {
                None
            };

            segments.push(TypePathSegment { name, args });

            // Check for path continuation
            if !self.try_consume(TokenKind::ColonColon) {
                break;
            }
        }

        TypePath {
            segments,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse type arguments <T, U, ...>.
    /// This handles `>>` disambiguation for nested generics like `Vec<Vec<T>>`.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_type_args(&mut self) -> TypeArgs {
        let start = self.current.span;
        self.expect(TokenKind::Lt);

        let mut args = Vec::new();

        // Use check_closing_angle() to handle `>`, `>>`, and `>>=`
        while !self.check_closing_angle() && !self.is_at_end() {
            // Could be a type, lifetime, or const
            let arg = if self.check(TokenKind::Lifetime) {
                self.advance();
                TypeArg::Lifetime(self.spanned_lifetime_symbol())
            } else if self.check(TokenKind::IntLit) || self.check(TokenKind::Minus) {
                // Integer literal or negative integer for const generics
                // Use parse_prefix_expr to avoid consuming `>` as comparison
                TypeArg::Const(self.parse_prefix_expr())
            } else if self.check(TokenKind::LBrace) {
                // Block expression for const generics: ::<{ N + 1 }>
                // Use parse_prefix_expr since block is a primary expression
                TypeArg::Const(self.parse_prefix_expr())
            } else {
                TypeArg::Type(self.parse_type())
            };

            args.push(arg);

            // Important: If pending_gt is set after parsing a type, it means we just
            // processed a `>>` split from a nested generic. The comma (if any) belongs
            // to the outer context (e.g., struct fields), not to our type args.
            // So we should NOT consume it - just exit the loop.
            if self.pending_gt.is_some() {
                break;
            }

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        // Use expect_closing_angle() to properly split `>>` tokens
        self.expect_closing_angle();

        TypeArgs {
            args,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a record type { x: T, y: U | R }.
    fn parse_record_type(&mut self) -> Type {
        let start = self.current.span;
        self.advance(); // consume '{'

        let mut fields = Vec::new();
        let mut rest = None;

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            // Check for row variable
            if self.try_consume(TokenKind::Or) {
                if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
                    self.advance();
                    rest = Some(self.spanned_symbol());
                }
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

            self.expect(TokenKind::Colon);
            let ty = self.parse_type();

            fields.push(RecordTypeField {
                name,
                ty,
                span: field_start.merge(self.previous.span),
            });

            if !self.try_consume(TokenKind::Comma) {
                // Check for trailing row variable
                if self.try_consume(TokenKind::Or)
                    && (self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent))
                {
                    self.advance();
                    rest = Some(self.spanned_symbol());
                }
                break;
            }
        }

        self.expect(TokenKind::RBrace);

        Type {
            kind: TypeKind::Record { fields, rest },
            span: start.merge(self.previous.span),
        }
    }

    /// Parse an effect row.
    #[must_use = "parsing has no effect if the result is not used"]
    pub fn parse_effect_row(&mut self) -> EffectRow {
        let start = self.current.span;

        // Pure effect
        if self.try_consume(TokenKind::Pure) {
            return EffectRow {
                kind: EffectRowKind::Pure,
                span: start.merge(self.previous.span),
            };
        }

        // Empty row {}
        if self.try_consume(TokenKind::LBrace) {
            if self.try_consume(TokenKind::RBrace) {
                return EffectRow {
                    kind: EffectRowKind::Pure,
                    span: start.merge(self.previous.span),
                };
            }

            let mut effects = Vec::new();
            let mut rest = None;

            loop {
                // Check for row variable (allow contextual keywords and type identifiers)
                if self.try_consume(TokenKind::Or) {
                    if self.check_ident() || self.check(TokenKind::TypeIdent) {
                        self.advance();
                        rest = Some(self.spanned_symbol());
                    }
                    break;
                }

                effects.push(self.parse_type());

                if !self.try_consume(TokenKind::Comma) {
                    // Check for trailing row variable (allow contextual keywords and type identifiers)
                    if self.try_consume(TokenKind::Or)
                        && (self.check_ident() || self.check(TokenKind::TypeIdent))
                    {
                        self.advance();
                        rest = Some(self.spanned_symbol());
                    }
                    break;
                }
            }

            self.expect(TokenKind::RBrace);

            return EffectRow {
                kind: EffectRowKind::Effects { effects, rest },
                span: start.merge(self.previous.span),
            };
        }

        // Just a type variable (allow contextual keywords and type identifiers)
        if self.check_ident() || self.check(TokenKind::TypeIdent) {
            self.advance();
            let var = self.spanned_symbol();
            return EffectRow {
                kind: EffectRowKind::Var(var),
                span: start.merge(self.previous.span),
            };
        }

        self.error_expected("effect row");
        EffectRow {
            kind: EffectRowKind::Pure,
            span: start,
        }
    }
}
