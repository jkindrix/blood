//! Declaration/item parsing.

use super::Parser;
use crate::ast::*;
use crate::diagnostics::ErrorCode;
use crate::lexer::TokenKind;
use crate::span::{Span, Spanned};

impl<'src> Parser<'src> {
    /// Parse a top-level declaration.
    pub(super) fn parse_declaration(&mut self) -> Option<Declaration> {
        let attrs = self.parse_attributes();
        let vis = self.parse_visibility();

        match self.current.kind {
            TokenKind::Fn => Some(Declaration::Function(self.parse_fn_decl(attrs, vis))),
            TokenKind::Const => {
                if self.peek_next_is_fn() {
                    Some(Declaration::Function(self.parse_fn_decl(attrs, vis)))
                } else {
                    Some(Declaration::Const(self.parse_const_decl(attrs, vis)))
                }
            }
            TokenKind::Async => Some(Declaration::Function(self.parse_fn_decl(attrs, vis))),
            TokenKind::Struct => Some(Declaration::Struct(self.parse_struct_decl(attrs, vis))),
            TokenKind::Enum => Some(Declaration::Enum(self.parse_enum_decl(attrs, vis))),
            TokenKind::Effect => Some(Declaration::Effect(self.parse_effect_decl(attrs))),
            TokenKind::Deep | TokenKind::Shallow | TokenKind::Handler => {
                Some(Declaration::Handler(self.parse_handler_decl(attrs)))
            }
            TokenKind::Trait => Some(Declaration::Trait(self.parse_trait_decl(attrs, vis))),
            TokenKind::Impl => Some(Declaration::Impl(self.parse_impl_block(attrs))),
            TokenKind::Type => Some(Declaration::Type(self.parse_type_decl(attrs, vis))),
            TokenKind::Static => Some(Declaration::Static(self.parse_static_decl(attrs, vis))),
            TokenKind::Bridge => Some(Declaration::Bridge(self.parse_bridge_decl(attrs))),
            TokenKind::Extern => Some(Declaration::Bridge(self.parse_extern_block(attrs))),
            TokenKind::Mod => Some(Declaration::Module(self.parse_mod_decl(attrs, vis))),
            TokenKind::Macro => Some(Declaration::Macro(self.parse_macro_decl(attrs, vis))),
            // Use declarations can now appear anywhere (after module declarations).
            // This enables `pub use` re-exports and more flexible import organization.
            TokenKind::Use => {
                // Warn if there are attributes on a use declaration (they're ignored)
                if !attrs.is_empty() {
                    // Emit warning for first attribute (representative of all)
                    self.warn_at(
                        attrs[0].span,
                        "attributes on use declarations are ignored",
                        ErrorCode::IgnoredAttributeOnUse,
                    );
                }
                Some(Declaration::Use(self.parse_import_with_visibility(vis)))
            }
            _ => {
                self.error_expected_one_of(&[
                    "`fn`", "`struct`", "`enum`", "`trait`", "`impl`",
                    "`effect`", "`handler`", "`type`", "`const`", "`static`",
                    "`bridge`", "`extern`", "`mod`", "`macro`",
                ]);
                self.synchronize();
                None
            }
        }
    }

    /// Check if the token after the current one is `fn`.
    ///
    /// This is called when current token is `const` and we need to distinguish
    /// between `const fn foo()` (function) and `const FOO: i32 = 42` (constant).
    fn peek_next_is_fn(&self) -> bool {
        self.check_next(TokenKind::Fn)
    }

    // ============================================================
    // Function Declaration
    // ============================================================

    pub(super) fn parse_fn_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> FnDecl {
        let start = self.current.span;

        // Parse qualifiers
        let mut qualifiers = FnQualifiers::default();

        while matches!(
            self.current.kind,
            TokenKind::Const | TokenKind::Async | TokenKind::AtUnsafe
        ) {
            match self.current.kind {
                TokenKind::Const => {
                    qualifiers.is_const = true;
                    self.advance();
                }
                TokenKind::Async => {
                    qualifiers.is_async = true;
                    self.advance();
                }
                TokenKind::AtUnsafe => {
                    qualifiers.is_unsafe = true;
                    self.advance();
                }
                _ => break,
            }
        }

        self.expect(TokenKind::Fn);

        // Parse name (allow contextual keywords as function names)
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("function name");
            Spanned::new(self.intern(""), self.current.span)
        };

        // Parse type parameters
        let type_params = self.parse_optional_type_params();

        // Parse parameters
        self.expect(TokenKind::LParen);
        let params = self.parse_params();
        self.expect(TokenKind::RParen);

        // Parse return type
        let return_type = if self.try_consume(TokenKind::Arrow) {
            Some(self.parse_type())
        } else {
            None
        };

        // Parse effect row
        let effects = if self.try_consume(TokenKind::Slash) {
            Some(self.parse_effect_row())
        } else {
            None
        };

        // Parse specification clauses
        let spec_clauses = self.parse_spec_clauses();

        // Parse where clause
        let where_clause = self.parse_optional_where_clause();

        // Parse body or semicolon
        let body = if self.check(TokenKind::LBrace) {
            Some(self.parse_block())
        } else {
            self.expect(TokenKind::Semi);
            None
        };

        FnDecl {
            attrs,
            vis,
            qualifiers,
            name,
            type_params,
            params,
            return_type,
            effects,
            spec_clauses,
            where_clause,
            body,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse specification clauses (requires, ensures, invariant, decreases).
    ///
    /// These appear between the effect row and the where clause. Multiple
    /// clauses of the same kind are allowed.
    fn parse_spec_clauses(&mut self) -> Vec<SpecClause> {
        let mut clauses = Vec::new();
        loop {
            let kind = match self.current.kind {
                TokenKind::Requires => SpecClauseKind::Requires,
                TokenKind::Ensures => SpecClauseKind::Ensures,
                TokenKind::Invariant => SpecClauseKind::Invariant,
                TokenKind::Decreases => SpecClauseKind::Decreases,
                _ => break,
            };
            let start = self.current.span;
            self.advance(); // consume keyword
            let expr = Box::new(self.parse_expr_no_struct());
            let span = start.merge(self.previous.span);
            clauses.push(SpecClause { kind, expr, span });
        }
        clauses
    }

    fn parse_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            params.push(self.parse_param());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        params
    }

    fn parse_param(&mut self) -> Param {
        let start = self.current.span;

        // Check for self parameter shorthands: self, &self, &mut self, mut self
        if let Some(param) = self.try_parse_self_param(start) {
            return param;
        }

        // Parse optional qualifier
        let qualifier = if self.try_consume(TokenKind::Linear) {
            Some(ParamQualifier::Linear)
        } else if self.try_consume(TokenKind::Affine) {
            Some(ParamQualifier::Affine)
        } else if self.try_consume(TokenKind::Mut) {
            Some(ParamQualifier::Mut)
        } else {
            None
        };

        // Parse pattern
        let pattern = self.parse_pattern();

        // Parse type annotation
        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        Param {
            qualifier,
            pattern,
            ty,
            span: start.merge(self.previous.span),
        }
    }

    /// Try to parse a self parameter shorthand.
    /// Returns Some(Param) if this is a self shorthand, None otherwise.
    /// Uses lookahead to avoid consuming tokens before we're sure it's a self param.
    fn try_parse_self_param(&mut self, start: Span) -> Option<Param> {
        // Case 1: `&self` - check: current is &, next is self
        if self.check(TokenKind::And) && self.check_next(TokenKind::SelfLower) {
            self.advance(); // consume &
            self.advance(); // consume self
            let self_span = self.previous.span;

            if self.check(TokenKind::Colon) {
                self.advance(); // consume :
                let ty = self.parse_type();
                return Some(Param {
                    qualifier: None,
                    pattern: Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: Spanned::new(self.intern("self"), self_span),
                            subpattern: None,
                        },
                        span: start.merge(self_span),
                    },
                    ty,
                    span: start.merge(self.previous.span),
                });
            }

            // No explicit type - use &Self
            let self_type = self.make_self_type(self_span);
            let ref_type = Type {
                kind: TypeKind::Reference {
                    lifetime: None,
                    mutable: false,
                    inner: Box::new(self_type),
                },
                span: start.merge(self_span),
            };

            return Some(Param {
                qualifier: None,
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: Spanned::new(self.intern("self"), self_span),
                        subpattern: None,
                    },
                    span: self_span,
                },
                ty: ref_type,
                span: start.merge(self_span),
            });
        }

        // Case 2: `&mut self` - check: current is &, next is mut, after that is self
        if self.check(TokenKind::And)
            && self.check_next(TokenKind::Mut)
            && self.check_after_next(TokenKind::SelfLower)
        {
            self.advance(); // consume &
            self.advance(); // consume mut
            self.advance(); // consume self
            let self_span = self.previous.span;

            if self.check(TokenKind::Colon) {
                self.advance(); // consume :
                let ty = self.parse_type();
                return Some(Param {
                    qualifier: None,
                    pattern: Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: Spanned::new(self.intern("self"), self_span),
                            subpattern: None,
                        },
                        span: start.merge(self_span),
                    },
                    ty,
                    span: start.merge(self.previous.span),
                });
            }

            // No explicit type - use &mut Self
            let self_type = self.make_self_type(self_span);
            let ref_type = Type {
                kind: TypeKind::Reference {
                    lifetime: None,
                    mutable: true,
                    inner: Box::new(self_type),
                },
                span: start.merge(self_span),
            };

            return Some(Param {
                qualifier: None,
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: Spanned::new(self.intern("self"), self_span),
                        subpattern: None,
                    },
                    span: self_span,
                },
                ty: ref_type,
                span: start.merge(self_span),
            });
        }

        // Case 3: `self` (without reference)
        if self.check(TokenKind::SelfLower) {
            self.advance(); // consume self
            let self_span = self.previous.span;

            if self.check(TokenKind::Colon) {
                self.advance(); // consume :
                let ty = self.parse_type();
                return Some(Param {
                    qualifier: None,
                    pattern: Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: Spanned::new(self.intern("self"), self_span),
                            subpattern: None,
                        },
                        span: self_span,
                    },
                    ty,
                    span: start.merge(self.previous.span),
                });
            }

            // No colon - this is a shorthand, type is Self
            let self_type = self.make_self_type(self_span);
            return Some(Param {
                qualifier: None,
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: Spanned::new(self.intern("self"), self_span),
                        subpattern: None,
                    },
                    span: self_span,
                },
                ty: self_type,
                span: start.merge(self_span),
            });
        }

        // Case 4: `mut self` - check: current is mut, next is self
        if self.check(TokenKind::Mut) && self.check_next(TokenKind::SelfLower) {
            let mut_span = self.current.span;
            self.advance(); // consume mut
            self.advance(); // consume self
            let self_span = self.previous.span;

            if self.check(TokenKind::Colon) {
                self.advance(); // consume :
                let ty = self.parse_type();
                return Some(Param {
                    qualifier: Some(ParamQualifier::Mut),
                    pattern: Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: true,
                            name: Spanned::new(self.intern("self"), self_span),
                            subpattern: None,
                        },
                        span: mut_span.merge(self_span),
                    },
                    ty,
                    span: start.merge(self.previous.span),
                });
            }

            // No colon - shorthand
            let self_type = self.make_self_type(self_span);
            return Some(Param {
                qualifier: Some(ParamQualifier::Mut),
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: true,
                        name: Spanned::new(self.intern("self"), self_span),
                        subpattern: None,
                    },
                    span: mut_span.merge(self_span),
                },
                ty: self_type,
                span: start.merge(self_span),
            });
        }

        None
    }

    /// Create a Self type reference.
    fn make_self_type(&mut self, span: Span) -> Type {
        let self_sym = self.intern("Self");
        Type {
            kind: TypeKind::Path(TypePath {
                segments: vec![TypePathSegment {
                    name: Spanned::new(self_sym, span),
                    args: None,
                }],
                span,
            }),
            span,
        }
    }

    // ============================================================
    // Type Parameters and Where Clause
    // ============================================================

    pub(super) fn parse_optional_type_params(&mut self) -> Option<TypeParams> {
        if !self.try_consume(TokenKind::Lt) {
            return None;
        }

        let start = self.previous.span;
        let mut params = Vec::new();

        // Use check_closing_angle() to handle `>>` in nested contexts
        while !self.check_closing_angle() && !self.is_at_end() {
            params.push(self.parse_generic_param());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        self.expect_closing_angle();

        Some(TypeParams {
            params,
            span: start.merge(self.previous.span),
        })
    }

    fn parse_generic_param(&mut self) -> GenericParam {
        let start = self.current.span;

        // Check for lifetime parameter: 'a
        if self.check(TokenKind::Lifetime) {
            self.advance();
            let name = self.spanned_lifetime_symbol();

            // Parse optional lifetime bounds: 'a: 'b + 'c
            let bounds = if self.try_consume(TokenKind::Colon) {
                let mut bounds = Vec::new();
                if self.check(TokenKind::Lifetime) {
                    self.advance();
                    bounds.push(self.spanned_lifetime_symbol());

                    while self.try_consume(TokenKind::Plus) {
                        if self.check(TokenKind::Lifetime) {
                            self.advance();
                            bounds.push(self.spanned_lifetime_symbol());
                        } else {
                            self.error_expected("lifetime");
                            break;
                        }
                    }
                }
                bounds
            } else {
                Vec::new()
            };

            return GenericParam::Lifetime(LifetimeParam {
                name,
                bounds,
                span: start.merge(self.previous.span),
            });
        }

        // Check for const parameter: const N: usize
        if self.check(TokenKind::Const) {
            self.advance(); // consume 'const'

            let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
                self.advance();
                self.spanned_symbol()
            } else {
                self.error_expected("const parameter name");
                Spanned::new(self.intern(""), self.current.span)
            };

            self.expect(TokenKind::Colon);
            let ty = self.parse_type();

            return GenericParam::Const(ConstParam {
                name,
                ty,
                span: start.merge(self.previous.span),
            });
        }

        // Default: type parameter
        let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("type parameter name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let bounds = if self.try_consume(TokenKind::Colon) {
            self.parse_type_bounds()
        } else {
            Vec::new()
        };

        GenericParam::Type(TypeParam {
            name,
            bounds,
            span: start.merge(self.previous.span),
        })
    }

    fn parse_type_bounds(&mut self) -> Vec<Type> {
        let mut bounds = vec![self.parse_type()];

        while self.try_consume(TokenKind::Plus) {
            bounds.push(self.parse_type());
        }

        bounds
    }

    pub(super) fn parse_optional_where_clause(&mut self) -> Option<WhereClause> {
        if !self.try_consume(TokenKind::Where) {
            return None;
        }

        let start = self.previous.span;
        let mut predicates = Vec::new();

        loop {
            predicates.push(self.parse_where_predicate());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }

            // Allow trailing comma
            if self.check(TokenKind::LBrace) || self.check(TokenKind::Semi) {
                break;
            }
        }

        Some(WhereClause {
            predicates,
            span: start.merge(self.previous.span),
        })
    }

    fn parse_where_predicate(&mut self) -> WherePredicate {
        let start = self.current.span;

        // Could be a lifetime bound or type bound
        if self.check(TokenKind::Lifetime) {
            self.advance();
            let lifetime = self.spanned_lifetime_symbol();
            self.expect(TokenKind::Colon);
            self.expect(TokenKind::Lifetime);
            let bounds = self.spanned_lifetime_symbol();

            WherePredicate::Lifetime {
                lifetime,
                bounds,
                span: start.merge(self.previous.span),
            }
        } else {
            let ty = self.parse_type();
            self.expect(TokenKind::Colon);
            let bounds = self.parse_type_bounds();

            WherePredicate::TypeBound {
                ty,
                bounds,
                span: start.merge(self.previous.span),
            }
        }
    }

    // ============================================================
    // Struct Declaration
    // ============================================================

    fn parse_struct_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> StructDecl {
        let start = self.current.span;
        self.advance(); // consume 'struct'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("struct name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let type_params = self.parse_optional_type_params();

        let body = if self.try_consume(TokenKind::Semi) {
            StructBody::Unit
        } else if self.check(TokenKind::LParen) {
            self.advance();
            let types = self.parse_tuple_fields();
            self.expect(TokenKind::RParen);
            self.expect(TokenKind::Semi);
            StructBody::Tuple(types)
        } else if self.check(TokenKind::LBrace) {
            self.advance();
            let fields = self.parse_struct_fields();
            self.expect(TokenKind::RBrace);
            StructBody::Record(fields)
        } else {
            self.error_expected_one_of(&["`{`", "`(`", "`;`"]);
            StructBody::Unit
        };

        StructDecl {
            attrs,
            vis,
            name,
            type_params,
            body,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_tuple_fields(&mut self) -> Vec<Type> {
        let mut types = Vec::new();

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            types.push(self.parse_type());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        types
    }

    fn parse_struct_fields(&mut self) -> Vec<StructField> {
        let mut fields = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            fields.push(self.parse_struct_field());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        fields
    }

    fn parse_struct_field(&mut self) -> StructField {
        let start = self.current.span;
        let attrs = self.parse_attributes();
        let vis = self.parse_visibility();

        // Allow contextual keywords as field names
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("field name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        StructField {
            attrs,
            vis,
            name,
            ty,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Enum Declaration
    // ============================================================

    fn parse_enum_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> EnumDecl {
        let start = self.current.span;
        self.advance(); // consume 'enum'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("enum name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let type_params = self.parse_optional_type_params();

        self.expect(TokenKind::LBrace);
        let variants = self.parse_enum_variants();
        self.expect(TokenKind::RBrace);

        EnumDecl {
            attrs,
            vis,
            name,
            type_params,
            variants,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_enum_variants(&mut self) -> Vec<EnumVariant> {
        let mut variants = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            variants.push(self.parse_enum_variant());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        variants
    }

    fn parse_enum_variant(&mut self) -> EnumVariant {
        let start = self.current.span;
        let attrs = self.parse_attributes();

        let name = if self.check(TokenKind::TypeIdent) || self.check(TokenKind::Ident) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("variant name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let body = if self.check(TokenKind::LParen) {
            self.advance();
            let types = self.parse_tuple_fields();
            self.expect(TokenKind::RParen);
            StructBody::Tuple(types)
        } else if self.check(TokenKind::LBrace) {
            self.advance();
            let fields = self.parse_struct_fields();
            self.expect(TokenKind::RBrace);
            StructBody::Record(fields)
        } else {
            StructBody::Unit
        };

        EnumVariant {
            attrs,
            name,
            body,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Effect Declaration
    // ============================================================

    fn parse_effect_decl(&mut self, attrs: Vec<Attribute>) -> EffectDecl {
        let start = self.current.span;
        self.advance(); // consume 'effect'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("effect name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let type_params = self.parse_optional_type_params();

        let extends = if self.try_consume(TokenKind::Extends) {
            let mut extends = vec![self.parse_type()];
            while self.try_consume(TokenKind::Comma) {
                extends.push(self.parse_type());
            }
            extends
        } else {
            Vec::new()
        };

        self.expect(TokenKind::LBrace);

        let mut operations = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            // Skip doc comments before operations (they're currently ignored)
            while matches!(self.current.kind, TokenKind::DocComment) {
                self.advance();
            }

            // Check if we're at a valid operation start
            if self.check(TokenKind::Op) {
                operations.push(self.parse_operation_decl());
            } else if self.check(TokenKind::RBrace) {
                // Done with operations
                break;
            } else {
                // Error recovery: skip invalid token and report error
                self.error_expected("keyword `op`");
                // Skip past this invalid item to prevent infinite loop
                // Advance until we hit `op`, `}`, or end of file
                while !self.check(TokenKind::Op)
                    && !self.check(TokenKind::RBrace)
                    && !self.is_at_end()
                {
                    self.advance();
                }
            }
        }

        self.expect(TokenKind::RBrace);

        EffectDecl {
            attrs,
            name,
            type_params,
            extends,
            operations,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_operation_decl(&mut self) -> OperationDecl {
        let start = self.current.span;
        self.expect(TokenKind::Op);

        // Allow contextual keywords as operation names
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("operation name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let type_params = self.parse_optional_type_params();

        self.expect(TokenKind::LParen);
        let params = self.parse_params();
        self.expect(TokenKind::RParen);

        self.expect(TokenKind::Arrow);
        let return_type = self.parse_type();

        self.expect(TokenKind::Semi);

        OperationDecl {
            name,
            type_params,
            params,
            return_type,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Handler Declaration
    // ============================================================

    fn parse_handler_decl(&mut self, attrs: Vec<Attribute>) -> HandlerDecl {
        let start = self.current.span;

        let kind = if self.try_consume(TokenKind::Deep) {
            HandlerKind::Deep
        } else if self.try_consume(TokenKind::Shallow) {
            HandlerKind::Shallow
        } else {
            HandlerKind::Deep
        };

        self.expect(TokenKind::Handler);

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("handler name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let type_params = self.parse_optional_type_params();

        self.expect(TokenKind::For);
        let effect = self.parse_type();

        let where_clause = self.parse_optional_where_clause();

        self.expect(TokenKind::LBrace);

        // Parse handler state
        let mut state = Vec::new();
        while self.check(TokenKind::Let) {
            state.push(self.parse_handler_state());
        }

        // Parse return clause
        let return_clause = if self.check(TokenKind::Return) {
            Some(self.parse_return_clause())
        } else {
            None
        };

        // Parse optional finally clause (GRAMMAR.md v0.6.0, ADR-036)
        // FinallyClause ::= 'finally' Block
        let finally_clause = if self.check(TokenKind::Finally) {
            self.advance(); // consume 'finally'
            Some(Box::new(self.parse_block()))
        } else {
            None
        };

        // Parse operation implementations
        let mut operations = Vec::new();
        while self.check(TokenKind::Op) {
            operations.push(self.parse_operation_impl());
        }

        self.expect(TokenKind::RBrace);

        HandlerDecl {
            attrs,
            kind,
            name,
            type_params,
            effect,
            where_clause,
            state,
            return_clause,
            finally_clause,
            operations,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_handler_state(&mut self) -> HandlerState {
        let start = self.current.span;
        self.advance(); // consume 'let'

        let is_mut = self.try_consume(TokenKind::Mut);

        // Allow contextual keywords as state variable names
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("state variable name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        let default = if self.try_consume(TokenKind::Eq) {
            Some(self.parse_expr())
        } else {
            None
        };

        // Consume the trailing semicolon if present
        self.try_consume(TokenKind::Semi);

        HandlerState {
            is_mut,
            name,
            ty,
            default,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_return_clause(&mut self) -> ReturnClause {
        let start = self.current.span;
        self.advance(); // consume 'return'

        self.expect(TokenKind::LParen);
        // Allow contextual keywords as parameter names
        let param = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("parameter name");
            Spanned::new(self.intern(""), self.current.span)
        };
        self.expect(TokenKind::RParen);

        let body = self.parse_block();

        ReturnClause {
            param,
            body,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_operation_impl(&mut self) -> OperationImpl {
        let start = self.current.span;
        self.advance(); // consume 'op'

        // Allow contextual keywords as operation names
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("operation name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::LParen);
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            params.push(self.parse_pattern());
            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen);

        let body = self.parse_block();

        OperationImpl {
            name,
            params,
            body,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Trait and Impl
    // ============================================================

    fn parse_trait_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> TraitDecl {
        let start = self.current.span;
        self.advance(); // consume 'trait'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("trait name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let type_params = self.parse_optional_type_params();

        let supertraits = if self.try_consume(TokenKind::Colon) {
            self.parse_type_bounds()
        } else {
            Vec::new()
        };

        let where_clause = self.parse_optional_where_clause();

        self.expect(TokenKind::LBrace);

        let mut items = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let attrs = self.parse_attributes();
            let vis = self.parse_visibility();

            match self.current.kind {
                TokenKind::Fn | TokenKind::Const | TokenKind::Async => {
                    items.push(TraitItem::Function(self.parse_fn_decl(attrs, vis)));
                }
                TokenKind::Type => {
                    items.push(TraitItem::Type(self.parse_type_decl(attrs, vis)));
                }
                // If we hit a top-level declaration keyword, break out of the trait block
                TokenKind::Impl
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Effect
                | TokenKind::Handler
                | TokenKind::Trait
                | TokenKind::Pub
                | TokenKind::Module => {
                    break;
                }
                _ => {
                    self.error_expected_one_of(&["`fn`", "`type`", "`}`"]);
                    self.synchronize();
                    // After synchronize, check if we're at a top-level keyword and should break
                    if matches!(
                        self.current.kind,
                        TokenKind::Impl
                            | TokenKind::Struct
                            | TokenKind::Enum
                            | TokenKind::Effect
                            | TokenKind::Handler
                            | TokenKind::Trait
                            | TokenKind::Pub
                            | TokenKind::Module
                    ) {
                        break;
                    }
                }
            }
        }

        self.expect(TokenKind::RBrace);

        TraitDecl {
            attrs,
            vis,
            name,
            type_params,
            supertraits,
            where_clause,
            items,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_impl_block(&mut self, attrs: Vec<Attribute>) -> ImplBlock {
        let start = self.current.span;
        self.advance(); // consume 'impl'

        let type_params = self.parse_optional_type_params();

        // This could be `impl Type` or `impl Trait for Type`
        let first_type = self.parse_type();

        let (trait_ty, self_ty) = if self.try_consume(TokenKind::For) {
            let self_ty = self.parse_type();
            (Some(first_type), self_ty)
        } else {
            (None, first_type)
        };

        let where_clause = self.parse_optional_where_clause();

        self.expect(TokenKind::LBrace);

        let mut items = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let attrs = self.parse_attributes();
            let vis = self.parse_visibility();

            match self.current.kind {
                TokenKind::Fn | TokenKind::Const | TokenKind::Async => {
                    items.push(ImplItem::Function(self.parse_fn_decl(attrs, vis)));
                }
                TokenKind::Type => {
                    items.push(ImplItem::Type(self.parse_type_decl(attrs, vis)));
                }
                // If we hit a top-level declaration keyword, break out of the impl block
                // This handles unclosed impl blocks gracefully
                TokenKind::Impl
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Effect
                | TokenKind::Handler
                | TokenKind::Trait
                | TokenKind::Pub
                | TokenKind::Module => {
                    break;
                }
                _ => {
                    self.error_expected_one_of(&["`fn`", "`type`", "`}`"]);
                    self.synchronize();
                    // After synchronize, check if we're at a top-level keyword and should break
                    if matches!(
                        self.current.kind,
                        TokenKind::Impl
                            | TokenKind::Struct
                            | TokenKind::Enum
                            | TokenKind::Effect
                            | TokenKind::Handler
                            | TokenKind::Trait
                            | TokenKind::Pub
                            | TokenKind::Module
                    ) {
                        break;
                    }
                }
            }
        }

        self.expect(TokenKind::RBrace);

        ImplBlock {
            attrs,
            type_params,
            trait_ty,
            self_ty,
            where_clause,
            items,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Type, Const, Static
    // ============================================================

    fn parse_type_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> TypeDecl {
        let start = self.current.span;
        self.advance(); // consume 'type'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("type name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let type_params = self.parse_optional_type_params();

        let ty = if self.try_consume(TokenKind::Eq) {
            Some(self.parse_type())
        } else {
            None
        };
        self.expect(TokenKind::Semi);

        TypeDecl {
            attrs,
            vis,
            name,
            type_params,
            ty,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_const_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> ConstDecl {
        let start = self.current.span;
        self.advance(); // consume 'const'

        let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("constant name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        self.expect(TokenKind::Eq);
        let value = self.parse_expr();
        self.expect(TokenKind::Semi);

        ConstDecl {
            attrs,
            vis,
            name,
            ty,
            value,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_static_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> StaticDecl {
        let start = self.current.span;
        self.advance(); // consume 'static'

        let is_mut = self.try_consume(TokenKind::Mut);

        let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("static name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        self.expect(TokenKind::Eq);
        let value = self.parse_expr();
        self.expect(TokenKind::Semi);

        StaticDecl {
            attrs,
            vis,
            is_mut,
            name,
            ty,
            value,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Bridge Declaration (FFI)
    // ============================================================

    fn parse_bridge_decl(&mut self, attrs: Vec<Attribute>) -> BridgeDecl {
        let start = self.current.span;
        self.advance(); // consume 'bridge'

        // Parse language specifier: "C", "C++", "wasm"
        let language = if self.check(TokenKind::StringLit) {
            let span = self.current.span;
            let text = self.current_text();
            // Strip quotes
            let inner = text.trim_start_matches('"').trim_end_matches('"');
            self.advance();
            Spanned::new(inner.to_string(), span)
        } else {
            self.error_expected("language string (e.g., \"C\")");
            Spanned::new(String::new(), self.current.span)
        };

        // Parse bridge name (optional - auto-generate if not provided)
        let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            // Auto-generate a unique name like extern blocks do
            let name_str = format!("__bridge_{}_{}", start.start_line, start.start);
            Spanned::new(self.intern(&name_str), start)
        };

        // Parse bridge body
        self.expect(TokenKind::LBrace);

        let mut items = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            if let Some(item) = self.parse_bridge_item() {
                items.push(item);
            }
        }

        self.expect(TokenKind::RBrace);

        BridgeDecl {
            attrs,
            language,
            name,
            items,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse an extern block: `extern "C" { fn foo(...); ... }`
    /// This is converted to a BridgeDecl with an auto-generated name.
    fn parse_extern_block(&mut self, attrs: Vec<Attribute>) -> BridgeDecl {
        let start = self.current.span;
        self.advance(); // consume 'extern'

        // Parse language specifier: "C", "Rust", etc.
        let language = if self.check(TokenKind::StringLit) {
            let span = self.current.span;
            let text = self.current_text();
            // Strip quotes
            let inner = text.trim_start_matches('"').trim_end_matches('"');
            self.advance();
            Spanned::new(inner.to_string(), span)
        } else {
            // Default to "C" if no language specified
            Spanned::new("C".to_string(), self.current.span)
        };

        // Generate a unique anonymous name for the extern block
        // Using span position to make it unique
        let name_str = format!("__extern_{}_{}", start.start_line, start.start);
        let name = Spanned::new(self.intern(&name_str), start);

        // Parse extern block body
        self.expect(TokenKind::LBrace);

        let mut items = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            if let Some(item) = self.parse_bridge_item() {
                items.push(item);
            }
        }

        self.expect(TokenKind::RBrace);

        BridgeDecl {
            attrs,
            language,
            name,
            items,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_item(&mut self) -> Option<BridgeItem> {
        let attrs = self.parse_attributes();

        // Check for link directive in attributes
        if self.is_link_attribute(&attrs) {
            if let Some(link) = self.extract_link_spec(&attrs) {
                return Some(BridgeItem::Link(link));
            }
        }

        // Check for contextual keyword `callback` (parsed as identifier)
        if self.check(TokenKind::Ident) && self.current_text() == "callback" {
            return Some(BridgeItem::Callback(self.parse_bridge_callback()));
        }

        match self.current.kind {
            TokenKind::Fn => Some(BridgeItem::Function(self.parse_bridge_fn(attrs))),
            TokenKind::Const => Some(BridgeItem::Const(self.parse_bridge_const())),
            TokenKind::Type => Some(self.parse_bridge_type(attrs)),
            TokenKind::Struct => Some(BridgeItem::Struct(self.parse_bridge_struct(attrs))),
            TokenKind::Enum => Some(BridgeItem::Enum(self.parse_bridge_enum(attrs))),
            TokenKind::Union => Some(BridgeItem::Union(self.parse_bridge_union(attrs))),
            _ => {
                self.error_expected_one_of(&[
                    "`fn`", "`const`", "`type`", "`struct`", "`enum`",
                    "`union`", "`callback`", "`#[link(...)]`",
                ]);
                self.synchronize();
                None
            }
        }
    }

    fn is_link_attribute(&self, attrs: &[Attribute]) -> bool {
        attrs.iter().any(|a| {
            a.path.len() == 1 && {
                let name = self.interner_symbol_str(a.path[0].node);
                name == "link"
            }
        })
    }

    fn extract_link_spec(&self, attrs: &[Attribute]) -> Option<LinkSpec> {
        for attr in attrs {
            if attr.path.len() == 1 {
                let name = self.interner_symbol_str(attr.path[0].node);
                if name == "link" {
                    // Extract link arguments
                    if let Some(AttributeArgs::List(args)) = &attr.args {
                        let mut lib_name = String::new();
                        let mut kind = None;
                        let mut wasm_module = None;

                        for arg in args {
                            if let AttributeArg::KeyValue(key, value) = arg {
                                let key_str = self.interner_symbol_str(key.node);
                                if key_str == "name" {
                                    if let LiteralKind::String(s) = &value.kind {
                                        lib_name = s.clone();
                                    }
                                } else if key_str == "kind" {
                                    if let LiteralKind::String(s) = &value.kind {
                                        kind = match s.as_str() {
                                            "dylib" => Some(LinkKind::Dylib),
                                            "static" => Some(LinkKind::Static),
                                            "framework" => Some(LinkKind::Framework),
                                            _ => None,
                                        };
                                    }
                                } else if key_str == "wasm_import_module" {
                                    if let LiteralKind::String(s) = &value.kind {
                                        wasm_module = Some(s.clone());
                                    }
                                }
                            }
                        }

                        return Some(LinkSpec {
                            name: lib_name,
                            kind,
                            wasm_import_module: wasm_module,
                            span: attr.span,
                        });
                    }
                }
            }
        }
        None
    }

    fn parse_bridge_fn(&mut self, attrs: Vec<Attribute>) -> BridgeFn {
        let start = self.current.span;
        self.advance(); // consume 'fn'

        // Allow contextual keywords as function names
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("function name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::LParen);
        let (params, is_variadic) = self.parse_bridge_params();
        self.expect(TokenKind::RParen);

        let return_type = if self.try_consume(TokenKind::Arrow) {
            Some(self.parse_type())
        } else {
            None
        };

        self.expect(TokenKind::Semi);

        BridgeFn {
            attrs,
            name,
            params,
            is_variadic,
            return_type,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_params(&mut self) -> (Vec<BridgeParam>, bool) {
        let mut params = Vec::new();
        let mut is_variadic = false;

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            // Check for variadic ...
            if self.try_consume(TokenKind::DotDot) {
                if self.try_consume(TokenKind::Dot) {
                    // This is ... (variadic)
                    is_variadic = true;
                    break;
                } else {
                    self.error_expected("`...` for variadic");
                    break;
                }
            }

            params.push(self.parse_bridge_param());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        (params, is_variadic)
    }

    fn parse_bridge_param(&mut self) -> BridgeParam {
        let start = self.current.span;
        let attrs = self.parse_attributes();

        // Extract ownership annotation from attributes
        let ownership = self.extract_ownership(&attrs);

        // Allow contextual keywords as parameter names
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("parameter name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        BridgeParam {
            name,
            ty,
            ownership,
            span: start.merge(self.previous.span),
        }
    }

    fn extract_ownership(&self, attrs: &[Attribute]) -> Option<BridgeOwnership> {
        for attr in attrs {
            if attr.path.len() == 1 {
                let name = self.interner_symbol_str(attr.path[0].node);
                match name {
                    "borrow" => return Some(BridgeOwnership::Borrow),
                    "transfer" => return Some(BridgeOwnership::Transfer),
                    "acquire" => return Some(BridgeOwnership::Acquire),
                    _ => {}
                }
            }
        }
        None
    }

    fn parse_bridge_const(&mut self) -> BridgeConst {
        let start = self.current.span;
        self.advance(); // consume 'const'

        let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("constant name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        self.expect(TokenKind::Eq);
        let value = self.parse_literal();

        self.expect(TokenKind::Semi);

        BridgeConst {
            name,
            ty,
            value,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_type(&mut self, _attrs: Vec<Attribute>) -> BridgeItem {
        let start = self.current.span;
        self.advance(); // consume 'type'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("type name");
            Spanned::new(self.intern(""), self.current.span)
        };

        // Check for alias: type Foo = Bar;
        // vs opaque: type Foo;
        if self.try_consume(TokenKind::Eq) {
            let ty = self.parse_type();
            self.expect(TokenKind::Semi);

            BridgeItem::TypeAlias(BridgeTypeAlias {
                name,
                ty,
                span: start.merge(self.previous.span),
            })
        } else {
            self.expect(TokenKind::Semi);

            BridgeItem::OpaqueType(BridgeOpaqueType {
                name,
                span: start.merge(self.previous.span),
            })
        }
    }

    fn parse_bridge_struct(&mut self, attrs: Vec<Attribute>) -> BridgeStruct {
        let start = self.current.span;
        self.advance(); // consume 'struct'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("struct name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::LBrace);
        let fields = self.parse_bridge_fields();
        self.expect(TokenKind::RBrace);

        BridgeStruct {
            attrs,
            name,
            fields,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_fields(&mut self) -> Vec<BridgeField> {
        let mut fields = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            fields.push(self.parse_bridge_field());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        fields
    }

    fn parse_bridge_field(&mut self) -> BridgeField {
        let start = self.current.span;

        // Allow contextual keywords as field names
        let name = if self.check_ident() {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("field name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        BridgeField {
            name,
            ty,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_enum(&mut self, attrs: Vec<Attribute>) -> BridgeEnum {
        let start = self.current.span;
        self.advance(); // consume 'enum'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("enum name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::LBrace);
        let variants = self.parse_bridge_enum_variants();
        self.expect(TokenKind::RBrace);

        BridgeEnum {
            attrs,
            name,
            variants,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_enum_variants(&mut self) -> Vec<BridgeEnumVariant> {
        let mut variants = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            variants.push(self.parse_bridge_enum_variant());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        variants
    }

    fn parse_bridge_enum_variant(&mut self) -> BridgeEnumVariant {
        let start = self.current.span;

        let name = if self.check(TokenKind::TypeIdent) || self.check(TokenKind::Ident) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("variant name");
            Spanned::new(self.intern(""), self.current.span)
        };

        let discriminant = if self.try_consume(TokenKind::Eq) {
            // Handle negative discriminants (-1, -42, etc.)
            let is_negative = self.try_consume(TokenKind::Minus);
            let mut lit = self.parse_literal();
            if is_negative {
                // Negate the literal value
                if let crate::ast::LiteralKind::Int { value, suffix } = &lit.kind {
                    lit.kind = crate::ast::LiteralKind::Int {
                        value: -(*value as i64) as u128,
                        suffix: *suffix,
                    };
                }
            }
            Some(lit)
        } else {
            None
        };

        BridgeEnumVariant {
            name,
            discriminant,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_union(&mut self, attrs: Vec<Attribute>) -> BridgeUnion {
        let start = self.current.span;
        self.advance(); // consume 'union'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("union name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::LBrace);
        let fields = self.parse_bridge_fields();
        self.expect(TokenKind::RBrace);

        BridgeUnion {
            attrs,
            name,
            fields,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_bridge_callback(&mut self) -> BridgeCallback {
        let start = self.current.span;
        self.advance(); // consume 'callback'

        let name = if self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("callback name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Eq);
        self.expect(TokenKind::Fn);

        self.expect(TokenKind::LParen);
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            params.push(self.parse_type());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen);

        let return_type = if self.try_consume(TokenKind::Arrow) {
            Some(self.parse_type())
        } else {
            None
        };

        self.expect(TokenKind::Semi);

        BridgeCallback {
            name,
            params,
            return_type,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Module Declaration
    // ============================================================

    /// Parse a module item: `mod foo;` or `mod foo { ... }`
    fn parse_mod_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> ModItemDecl {
        let start = self.current.span;
        self.advance(); // consume 'mod'

        let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("module name");
            Spanned::new(self.intern(""), self.current.span)
        };

        // Either `mod foo;` (external) or `mod foo { ... }` (inline)
        let body = if self.try_consume(TokenKind::Semi) {
            // External module - will be loaded from a file
            None
        } else if self.check(TokenKind::LBrace) {
            self.advance(); // consume '{'
            let mut declarations = Vec::new();
            while !self.check(TokenKind::RBrace) && !self.is_at_end() {
                if let Some(decl) = self.parse_declaration() {
                    declarations.push(decl);
                }
            }
            self.expect(TokenKind::RBrace);
            Some(declarations)
        } else {
            self.error_expected_one_of(&["`;`", "`{`"]);
            None
        };

        ModItemDecl {
            attrs,
            vis,
            name,
            body,
            span: start.merge(self.previous.span),
        }
    }

    // ============================================================
    // Macro Declaration
    // ============================================================

    /// Parse a macro declaration.
    ///
    /// Syntax:
    /// ```text
    /// // Single rule:
    /// macro name!($arg:expr) => { $arg + $arg }
    ///
    /// // Multiple rules:
    /// macro name! {
    ///     ($a:expr) => { ... },
    ///     ($a:expr, $b:expr) => { ... },
    /// }
    /// ```
    pub(super) fn parse_macro_decl(&mut self, attrs: Vec<Attribute>, vis: Visibility) -> MacroDecl {
        let start = self.current.span;
        self.advance(); // consume 'macro'

        // Parse macro name (optionally followed by `!`)
        let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("macro name");
            Spanned::new(self.intern(""), self.current.span)
        };

        // `!` after the name is optional (supports both `macro name!` and `macro name`)
        self.try_consume(TokenKind::Not);

        let rules = if self.check(TokenKind::LBrace) {
            // Multiple rules: macro name! { (pattern) => expansion, ... }
            self.advance(); // consume '{'
            self.parse_macro_rules()
        } else if self.check(TokenKind::LParen) || self.check(TokenKind::LBracket) {
            // Single rule: macro name!(pattern) => expansion
            let rule = self.parse_single_macro_rule();
            vec![rule]
        } else {
            self.error_expected_one_of(&["`{`", "`(`", "`[`"]);
            Vec::new()
        };

        MacroDecl {
            attrs,
            vis,
            name,
            rules,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse multiple macro rules inside `{ ... }`.
    fn parse_macro_rules(&mut self) -> Vec<MacroRule> {
        let mut rules = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let rule = self.parse_single_macro_rule();
            rules.push(rule);

            // Allow trailing comma or semicolon as rule separator (Rust uses semicolons)
            if !self.try_consume(TokenKind::Comma) && !self.try_consume(TokenKind::Semi) {
                break;
            }
        }

        self.expect(TokenKind::RBrace);
        rules
    }

    /// Parse a single macro rule: `(pattern) => expansion`
    fn parse_single_macro_rule(&mut self) -> MacroRule {
        let start = self.current.span;

        // Parse the pattern (in parentheses, brackets, or braces)
        let pattern = self.parse_macro_pattern();

        // Expect `=>`
        self.expect(TokenKind::FatArrow);

        // Parse the expansion
        let expansion = self.parse_macro_expansion();

        MacroRule {
            pattern,
            expansion,
            span: start.merge(self.previous.span),
        }
    }

    /// Parse a macro pattern.
    fn parse_macro_pattern(&mut self) -> MacroPattern {
        let start = self.current.span;

        let (delim, close_kind) = if self.check(TokenKind::LParen) {
            self.advance();
            (MacroDelimiter::Paren, TokenKind::RParen)
        } else if self.check(TokenKind::LBracket) {
            self.advance();
            (MacroDelimiter::Bracket, TokenKind::RBracket)
        } else if self.check(TokenKind::LBrace) {
            self.advance();
            (MacroDelimiter::Brace, TokenKind::RBrace)
        } else {
            self.error_expected_one_of(&["`(`", "`[`", "`{`"]);
            return MacroPattern {
                parts: Vec::new(),
                span: start,
            };
        };

        let parts = self.parse_macro_pattern_parts(close_kind);

        self.expect(close_kind);

        // Wrap in a group
        MacroPattern {
            parts: vec![MacroPatternPart::Group {
                delimiter: delim,
                pattern: parts,
                span: start.merge(self.previous.span),
            }],
            span: start.merge(self.previous.span),
        }
    }

    /// Parse the parts of a macro pattern until we hit the closing delimiter.
    fn parse_macro_pattern_parts(&mut self, close_kind: TokenKind) -> Vec<MacroPatternPart> {
        let mut parts = Vec::new();

        while !self.check(close_kind) && !self.is_at_end() {
            if self.check(TokenKind::Dollar) {
                parts.push(self.parse_macro_capture_or_repetition());
            } else if self.check(TokenKind::LParen) {
                // Nested group
                let start = self.current.span;
                self.advance();
                let inner = self.parse_macro_pattern_parts(TokenKind::RParen);
                self.expect(TokenKind::RParen);
                parts.push(MacroPatternPart::Group {
                    delimiter: MacroDelimiter::Paren,
                    pattern: inner,
                    span: start.merge(self.previous.span),
                });
            } else if self.check(TokenKind::LBracket) {
                let start = self.current.span;
                self.advance();
                let inner = self.parse_macro_pattern_parts(TokenKind::RBracket);
                self.expect(TokenKind::RBracket);
                parts.push(MacroPatternPart::Group {
                    delimiter: MacroDelimiter::Bracket,
                    pattern: inner,
                    span: start.merge(self.previous.span),
                });
            } else if self.check(TokenKind::LBrace) {
                let start = self.current.span;
                self.advance();
                let inner = self.parse_macro_pattern_parts(TokenKind::RBrace);
                self.expect(TokenKind::RBrace);
                parts.push(MacroPatternPart::Group {
                    delimiter: MacroDelimiter::Brace,
                    pattern: inner,
                    span: start.merge(self.previous.span),
                });
            } else {
                // Literal token to match
                let span = self.current.span;
                let kind = self.current.kind;
                self.advance();
                parts.push(MacroPatternPart::Token(kind, span));
            }
        }

        parts
    }

    /// Parse a macro capture `$name:frag` or repetition `$(...)*`.
    fn parse_macro_capture_or_repetition(&mut self) -> MacroPatternPart {
        let start = self.current.span;
        self.advance(); // consume '$'

        if self.check(TokenKind::LParen) {
            // Repetition: $(...)*
            self.advance(); // consume '('
            let inner = self.parse_macro_pattern_parts(TokenKind::RParen);
            self.expect(TokenKind::RParen);

            // Optional separator before repetition operator
            let separator = if !matches!(
                self.current.kind,
                TokenKind::Star | TokenKind::Plus | TokenKind::Question
            ) {
                let sep = self.current.kind;
                self.advance();
                Some(sep)
            } else {
                None
            };

            // Parse repetition kind
            let kind = if self.try_consume(TokenKind::Star) {
                RepetitionKind::ZeroOrMore
            } else if self.try_consume(TokenKind::Plus) {
                RepetitionKind::OneOrMore
            } else if self.try_consume(TokenKind::Question) {
                RepetitionKind::ZeroOrOne
            } else {
                self.error_expected_one_of(&["`*`", "`+`", "`?`"]);
                RepetitionKind::ZeroOrMore
            };

            MacroPatternPart::Repetition {
                pattern: inner,
                separator,
                kind,
                span: start.merge(self.previous.span),
            }
        } else if self.check(TokenKind::Ident) {
            // Capture: $name:frag
            self.advance();
            let name = self.spanned_symbol();

            self.expect(TokenKind::Colon);

            // Parse fragment specifier
            let frag_start = self.current.span;
            if self.check(TokenKind::Ident) {
                self.advance();
                let frag_str = self.text(&self.previous.span);
                let fragment = FragmentKind::from_str(frag_str).unwrap_or_else(|| {
                    use crate::diagnostics::ErrorCode;
                    self.error_at(self.previous.span, &format!("unknown fragment specifier `{}`", frag_str), ErrorCode::InvalidMacroFragment);
                    FragmentKind::Expr
                });

                MacroPatternPart::Capture {
                    name,
                    fragment,
                    span: start.merge(self.previous.span),
                }
            } else {
                self.error_expected("fragment specifier (expr, ty, pat, ident, literal, block, stmt, item, tt)");
                MacroPatternPart::Capture {
                    name,
                    fragment: FragmentKind::Expr,
                    span: start.merge(frag_start),
                }
            }
        } else {
            self.error_expected("capture name or `(`");
            MacroPatternPart::Token(TokenKind::Dollar, start)
        }
    }

    /// Parse a macro expansion.
    fn parse_macro_expansion(&mut self) -> MacroExpansion {
        let start = self.current.span;

        // Expansion is typically wrapped in braces, but can also be parentheses or brackets
        let (close_kind, delim) = if self.check(TokenKind::LBrace) {
            self.advance();
            (TokenKind::RBrace, MacroDelimiter::Brace)
        } else if self.check(TokenKind::LParen) {
            self.advance();
            (TokenKind::RParen, MacroDelimiter::Paren)
        } else if self.check(TokenKind::LBracket) {
            self.advance();
            (TokenKind::RBracket, MacroDelimiter::Bracket)
        } else {
            self.error_expected_one_of(&["`{`", "`(`", "`[`"]);
            return MacroExpansion {
                parts: Vec::new(),
                span: start,
            };
        };

        let inner_parts = self.parse_macro_expansion_parts(close_kind);
        self.expect(close_kind);

        // Wrap in a group
        MacroExpansion {
            parts: vec![MacroExpansionPart::Group {
                delimiter: delim,
                parts: inner_parts,
                span: start.merge(self.previous.span),
            }],
            span: start.merge(self.previous.span),
        }
    }

    /// Parse the parts of a macro expansion until we hit the closing delimiter.
    fn parse_macro_expansion_parts(&mut self, close_kind: TokenKind) -> Vec<MacroExpansionPart> {
        let mut parts = Vec::new();
        let mut current_tokens: Vec<MacroToken> = Vec::new();

        while !self.check(close_kind) && !self.is_at_end() {
            if self.check(TokenKind::Dollar) {
                // Flush accumulated tokens
                if !current_tokens.is_empty() {
                    parts.push(MacroExpansionPart::Tokens(std::mem::take(&mut current_tokens)));
                }

                let dollar_span = self.current.span;
                self.advance(); // consume '$'

                if self.check(TokenKind::LParen) {
                    // Repetition: $(...)*
                    self.advance(); // consume '('
                    let inner = self.parse_macro_expansion_parts(TokenKind::RParen);
                    self.expect(TokenKind::RParen);

                    // Optional separator before repetition operator
                    let separator = if !matches!(
                        self.current.kind,
                        TokenKind::Star | TokenKind::Plus | TokenKind::Question
                    ) {
                        let sep = MacroToken {
                            kind: self.current.kind,
                            span: self.current.span,
                            hygiene: HygieneId::default(),
                        };
                        self.advance();
                        Some(sep)
                    } else {
                        None
                    };

                    // Expect repetition operator
                    if !self.try_consume(TokenKind::Star)
                        && !self.try_consume(TokenKind::Plus)
                        && !self.try_consume(TokenKind::Question)
                    {
                        self.error_expected_one_of(&["`*`", "`+`", "`?`"]);
                    }

                    parts.push(MacroExpansionPart::Repetition {
                        parts: inner,
                        separator,
                        span: dollar_span.merge(self.previous.span),
                    });
                } else if self.check(TokenKind::Ident) {
                    // Substitution: $name
                    self.advance();
                    let name = self.spanned_symbol();
                    let name_span = name.span;

                    parts.push(MacroExpansionPart::Substitution {
                        name,
                        span: dollar_span.merge(name_span),
                    });
                } else {
                    // Just a dollar sign - emit it as a token
                    current_tokens.push(MacroToken {
                        kind: TokenKind::Dollar,
                        span: dollar_span,
                        hygiene: HygieneId::default(),
                    });
                }
            } else if self.check(TokenKind::LParen) {
                // Flush accumulated tokens
                if !current_tokens.is_empty() {
                    parts.push(MacroExpansionPart::Tokens(std::mem::take(&mut current_tokens)));
                }
                // Nested group
                let start = self.current.span;
                self.advance();
                let inner = self.parse_macro_expansion_parts(TokenKind::RParen);
                self.expect(TokenKind::RParen);
                parts.push(MacroExpansionPart::Group {
                    delimiter: MacroDelimiter::Paren,
                    parts: inner,
                    span: start.merge(self.previous.span),
                });
            } else if self.check(TokenKind::LBracket) {
                if !current_tokens.is_empty() {
                    parts.push(MacroExpansionPart::Tokens(std::mem::take(&mut current_tokens)));
                }
                let start = self.current.span;
                self.advance();
                let inner = self.parse_macro_expansion_parts(TokenKind::RBracket);
                self.expect(TokenKind::RBracket);
                parts.push(MacroExpansionPart::Group {
                    delimiter: MacroDelimiter::Bracket,
                    parts: inner,
                    span: start.merge(self.previous.span),
                });
            } else if self.check(TokenKind::LBrace) {
                if !current_tokens.is_empty() {
                    parts.push(MacroExpansionPart::Tokens(std::mem::take(&mut current_tokens)));
                }
                let start = self.current.span;
                self.advance();
                let inner = self.parse_macro_expansion_parts(TokenKind::RBrace);
                self.expect(TokenKind::RBrace);
                parts.push(MacroExpansionPart::Group {
                    delimiter: MacroDelimiter::Brace,
                    parts: inner,
                    span: start.merge(self.previous.span),
                });
            } else {
                // Regular token - accumulate it
                current_tokens.push(MacroToken {
                    kind: self.current.kind,
                    span: self.current.span,
                    hygiene: HygieneId::default(),
                });
                self.advance();
            }
        }

        // Flush any remaining tokens
        if !current_tokens.is_empty() {
            parts.push(MacroExpansionPart::Tokens(current_tokens));
        }

        parts
    }
}
