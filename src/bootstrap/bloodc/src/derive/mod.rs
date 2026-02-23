//! Derive macro expansion for Blood.
//!
//! This module implements built-in derive macros that generate method implementations
//! for types marked with `#[derive(...)]` attributes.
//!
//! # Supported Derives
//!
//! - `Debug` - Generates `fn debug(&self) -> String`
//! - `Clone` - Generates `fn clone(&self) -> Self`
//! - `Eq` - Generates `fn eq(&self, other: &Self) -> bool`
//! - `PartialEq` - Same as Eq
//! - `Default` - Generates `fn default() -> Self`
//! - `Hash` - Generates `fn hash(&self) -> u64`
//!
//! # Implementation Strategy
//!
//! Derives are expanded during the `into_hir()` phase after type collection:
//! 1. During `collect_struct/collect_enum`, extract `#[derive(...)]` attributes
//! 2. Store pending derive requests with the type's DefId
//! 3. In `into_hir()`, expand all derives by generating synthetic impl blocks

mod debug;
mod clone;
mod eq;
mod default;
mod hash;

use std::collections::HashMap;

use crate::hir::{
    DefId, Type, TypeKind, BodyId, LocalId, TyVarId,
    Expr, ExprKind, Body, FnSig, LiteralValue,
};
use crate::span::Span;
use crate::typeck::context::{StructInfo, EnumInfo, ImplBlockInfo, ImplMethodInfo};

/// The kind of derive macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeriveKind {
    Debug,
    Clone,
    Eq,
    PartialEq,
    Default,
    Hash,
}

impl DeriveKind {
    /// Parse a derive kind from a string.
    #[allow(clippy::should_implement_trait)] // Returns Option, not Result<_, Err> as FromStr requires
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Debug" => Some(DeriveKind::Debug),
            "Clone" => Some(DeriveKind::Clone),
            "Eq" => Some(DeriveKind::Eq),
            "PartialEq" => Some(DeriveKind::PartialEq),
            "Default" => Some(DeriveKind::Default),
            "Hash" => Some(DeriveKind::Hash),
            _ => None,
        }
    }

    /// Get the method name for this derive.
    pub fn method_name(&self) -> &'static str {
        match self {
            DeriveKind::Debug => "debug",
            DeriveKind::Clone => "clone",
            DeriveKind::Eq | DeriveKind::PartialEq => "eq",
            DeriveKind::Default => "default",
            DeriveKind::Hash => "hash",
        }
    }
}

/// A request to derive a trait for a type.
#[derive(Debug, Clone)]
pub struct DeriveRequest {
    /// The DefId of the type to derive for.
    pub type_def_id: DefId,
    /// The name of the type.
    pub type_name: String,
    /// The list of derives to generate.
    pub derives: Vec<DeriveKind>,
    /// Source span of the derive attribute.
    pub span: Span,
}

/// Information about a generated method for registration with the resolver.
#[derive(Debug, Clone)]
pub struct GeneratedMethod {
    /// The DefId of the method.
    pub def_id: DefId,
    /// The name of the method.
    pub name: String,
    /// The span of the method.
    pub span: Span,
}

/// Context for derive expansion.
pub struct DeriveExpander<'a> {
    /// Struct definitions from type checking.
    struct_defs: &'a HashMap<DefId, StructInfo>,
    /// Enum definitions from type checking.
    enum_defs: &'a HashMap<DefId, EnumInfo>,
    /// Where to add generated impl blocks.
    impl_blocks: &'a mut Vec<ImplBlockInfo>,
    /// Where to add generated function signatures.
    fn_sigs: &'a mut HashMap<DefId, FnSig>,
    /// Where to add generated bodies.
    bodies: &'a mut HashMap<BodyId, Body>,
    /// Where to add method self types.
    method_self_types: &'a mut HashMap<DefId, Type>,
    /// Where to add function body mappings.
    fn_bodies: &'a mut HashMap<DefId, BodyId>,
    /// Next DefId to allocate.
    next_def_id: u32,
    /// Next BodyId to allocate.
    next_body_id: u32,
    /// Generated methods (for resolver registration).
    generated_methods: Vec<GeneratedMethod>,
}

impl<'a> DeriveExpander<'a> {
    /// Create a new derive expander.
    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        struct_defs: &'a HashMap<DefId, StructInfo>,
        enum_defs: &'a HashMap<DefId, EnumInfo>,
        impl_blocks: &'a mut Vec<ImplBlockInfo>,
        fn_sigs: &'a mut HashMap<DefId, FnSig>,
        bodies: &'a mut HashMap<BodyId, Body>,
        method_self_types: &'a mut HashMap<DefId, Type>,
        fn_bodies: &'a mut HashMap<DefId, BodyId>,
        next_def_id: u32,
        next_body_id: u32,
    ) -> Self {
        Self {
            struct_defs,
            enum_defs,
            impl_blocks,
            fn_sigs,
            bodies,
            method_self_types,
            fn_bodies,
            next_def_id,
            next_body_id,
            generated_methods: Vec::new(),
        }
    }

    /// Expand all pending derives and return generated method info.
    pub fn expand_all(&mut self, requests: Vec<DeriveRequest>) -> Vec<GeneratedMethod> {
        for request in requests {
            self.expand_request(&request);
        }
        std::mem::take(&mut self.generated_methods)
    }

    /// Get the next body ID after expansion (for updating TypeContext).
    pub fn next_body_id(&self) -> u32 {
        self.next_body_id
    }

    /// Expand a single derive request.
    fn expand_request(&mut self, request: &DeriveRequest) {
        // Get type info - could be struct or enum
        let is_struct = self.struct_defs.contains_key(&request.type_def_id);
        let is_enum = self.enum_defs.contains_key(&request.type_def_id);

        if !is_struct && !is_enum {
            // Type not found - this shouldn't happen if collect phase worked correctly
            return;
        }

        // Generate impl blocks for each derive
        for derive_kind in &request.derives {
            match derive_kind {
                DeriveKind::Debug => {
                    if is_struct {
                        self.expand_debug_struct(request);
                    } else {
                        self.expand_debug_enum(request);
                    }
                }
                DeriveKind::Clone => {
                    if is_struct {
                        self.expand_clone_struct(request);
                    } else {
                        self.expand_clone_enum(request);
                    }
                }
                DeriveKind::Eq | DeriveKind::PartialEq => {
                    if is_struct {
                        self.expand_eq_struct(request);
                    } else {
                        self.expand_eq_enum(request);
                    }
                }
                DeriveKind::Default => {
                    if is_struct {
                        self.expand_default_struct(request);
                    } else {
                        self.expand_default_enum(request);
                    }
                }
                DeriveKind::Hash => {
                    if is_struct {
                        self.expand_hash_struct(request);
                    } else {
                        self.expand_hash_enum(request);
                    }
                }
            }
        }
    }

    /// Allocate a new DefId.
    fn alloc_def_id(&mut self) -> DefId {
        let id = DefId::new(self.next_def_id);
        self.next_def_id += 1;
        id
    }

    /// Allocate a new BodyId.
    fn alloc_body_id(&mut self) -> BodyId {
        let id = BodyId::new(self.next_body_id);
        self.next_body_id += 1;
        id
    }

    /// Get the Self type for a derive request.
    fn get_self_type(&self, request: &DeriveRequest) -> Type {
        // Get generics from struct or enum
        let generics = if let Some(struct_info) = self.struct_defs.get(&request.type_def_id) {
            struct_info.generics.iter()
                .map(|&tv| Type::new(TypeKind::Param(tv)))
                .collect()
        } else if let Some(enum_info) = self.enum_defs.get(&request.type_def_id) {
            enum_info.generics.iter()
                .map(|&tv| Type::new(TypeKind::Param(tv)))
                .collect()
        } else {
            Vec::new()
        };

        Type::adt(request.type_def_id, generics)
    }

    /// Get generics (TyVarIds) for a type.
    fn get_generics(&self, request: &DeriveRequest) -> Vec<TyVarId> {
        if let Some(struct_info) = self.struct_defs.get(&request.type_def_id) {
            struct_info.generics.clone()
        } else if let Some(enum_info) = self.enum_defs.get(&request.type_def_id) {
            enum_info.generics.clone()
        } else {
            Vec::new()
        }
    }

    /// Create an impl block info for a generated method.
    fn create_impl_block(
        &mut self,
        request: &DeriveRequest,
        method_def_id: DefId,
        method_name: &str,
        is_static: bool,
    ) {
        let self_ty = self.get_self_type(request);
        let generics = self.get_generics(request);

        let impl_block = ImplBlockInfo {
            self_ty,
            trait_ref: None,
            generics,
            methods: vec![ImplMethodInfo {
                def_id: method_def_id,
                name: method_name.to_string(),
                is_static,
            }],
            assoc_types: Vec::new(),
            assoc_consts: Vec::new(),
            span: request.span,
        };

        self.impl_blocks.push(impl_block);

        // Record the generated method for resolver registration
        self.generated_methods.push(GeneratedMethod {
            def_id: method_def_id,
            name: method_name.to_string(),
            span: request.span,
        });
    }

    // Derive implementations are in separate submodules
    fn expand_debug_struct(&mut self, request: &DeriveRequest) {
        debug::expand_struct(self, request);
    }

    fn expand_debug_enum(&mut self, request: &DeriveRequest) {
        debug::expand_enum(self, request);
    }

    fn expand_clone_struct(&mut self, request: &DeriveRequest) {
        clone::expand_struct(self, request);
    }

    fn expand_clone_enum(&mut self, request: &DeriveRequest) {
        clone::expand_enum(self, request);
    }

    fn expand_eq_struct(&mut self, request: &DeriveRequest) {
        eq::expand_struct(self, request);
    }

    fn expand_eq_enum(&mut self, request: &DeriveRequest) {
        eq::expand_enum(self, request);
    }

    fn expand_default_struct(&mut self, request: &DeriveRequest) {
        default::expand_struct(self, request);
    }

    fn expand_default_enum(&mut self, request: &DeriveRequest) {
        default::expand_enum(self, request);
    }

    fn expand_hash_struct(&mut self, request: &DeriveRequest) {
        hash::expand_struct(self, request);
    }

    fn expand_hash_enum(&mut self, request: &DeriveRequest) {
        hash::expand_enum(self, request);
    }
}

/// Helper to create a literal expression.
pub(crate) fn literal_expr(value: LiteralValue, ty: Type, span: Span) -> Expr {
    Expr::new(ExprKind::Literal(value), ty, span)
}

/// Helper to create a local variable expression.
pub(crate) fn local_expr(local_id: LocalId, ty: Type, span: Span) -> Expr {
    Expr::new(ExprKind::Local(local_id), ty, span)
}

/// Helper to create a string literal.
/// Note: String literals are str slices (ptr + len), not heap-allocated Strings.
pub(crate) fn string_literal(s: &str, span: Span) -> Expr {
    literal_expr(
        LiteralValue::String(s.to_string()),
        Type::str(),  // String literals are str slices
        span,
    )
}

/// Helper to create a boolean literal.
pub(crate) fn bool_literal(b: bool, span: Span) -> Expr {
    literal_expr(LiteralValue::Bool(b), Type::bool(), span)
}
