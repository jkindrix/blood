//! Declaration/item parsing.

use super::Parser;
use crate::ast::*;
use crate::lexer::TokenKind;
use crate::span::Spanned;

impl<'src> Parser<'src> {
    /// Parse a top-level declaration.
    pub(super) fn parse_declaration(&mut self) -> Option<Declaration> {
        let attrs = self.parse_attributes();
        let vis = self.parse_visibility();

        match self.current.kind {
            TokenKind::Fn => Some(Declaration::Function(self.parse_fn_decl(attrs, vis))),
            TokenKind::Const => {
                if self.peek_is_fn() {
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
            _ => {
                self.error_expected_one_of(&[
                    "`fn`", "`struct`", "`enum`", "`trait`", "`impl`",
                    "`effect`", "`handler`", "`type`", "`const`", "`static`",
                ]);
                self.synchronize();
                None
            }
        }
    }

    fn peek_is_fn(&self) -> bool {
        // Look ahead to see if this is `const fn`
        // This is a bit hacky but works for now
        matches!(self.current.kind, TokenKind::Const | TokenKind::Async)
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

        // Parse name
        let name = if self.check(TokenKind::Ident) {
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
            where_clause,
            body,
            span: start.merge(self.previous.span),
        }
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

    // ============================================================
    // Type Parameters and Where Clause
    // ============================================================

    pub(super) fn parse_optional_type_params(&mut self) -> Option<TypeParams> {
        if !self.try_consume(TokenKind::Lt) {
            return None;
        }

        let start = self.previous.span;
        let mut params = Vec::new();

        while !self.check(TokenKind::Gt) && !self.is_at_end() {
            params.push(self.parse_type_param());

            if !self.try_consume(TokenKind::Comma) {
                break;
            }
        }

        self.expect(TokenKind::Gt);

        Some(TypeParams {
            params,
            span: start.merge(self.previous.span),
        })
    }

    fn parse_type_param(&mut self) -> TypeParam {
        let start = self.current.span;

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

        TypeParam {
            name,
            bounds,
            span: start.merge(self.previous.span),
        }
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
            let lifetime = self.spanned_symbol();
            self.expect(TokenKind::Colon);
            self.expect(TokenKind::Lifetime);
            let bounds = self.spanned_symbol();

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
        let vis = self.parse_visibility();

        let name = if self.check(TokenKind::Ident) {
            self.advance();
            self.spanned_symbol()
        } else {
            self.error_expected("field name");
            Spanned::new(self.intern(""), self.current.span)
        };

        self.expect(TokenKind::Colon);
        let ty = self.parse_type();

        StructField {
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
            operations.push(self.parse_operation_decl());
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

        let name = if self.check(TokenKind::Ident) {
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
            operations,
            span: start.merge(self.previous.span),
        }
    }

    fn parse_handler_state(&mut self) -> HandlerState {
        let start = self.current.span;
        self.advance(); // consume 'let'

        let is_mut = self.try_consume(TokenKind::Mut);

        let name = if self.check(TokenKind::Ident) {
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
        let param = if self.check(TokenKind::Ident) {
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

        let name = if self.check(TokenKind::Ident) {
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
                _ => {
                    self.error_expected_one_of(&["`fn`", "`type`"]);
                    self.synchronize();
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
                _ => {
                    self.error_expected_one_of(&["`fn`", "`type`"]);
                    self.synchronize();
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

        self.expect(TokenKind::Eq);
        let ty = self.parse_type();
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
}
