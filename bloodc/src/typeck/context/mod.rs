//! Type checking context.
//!
//! The TypeContext is the main entry point for type checking. It coordinates
//! name resolution, type collection, and type inference.

use std::collections::HashMap;
use std::path::PathBuf;

use string_interner::{DefaultStringInterner, Symbol as _};

use crate::ast;
use crate::hir::{self, DefId, Type, TypeKind, TyVarId};
use crate::span::Span;

use super::error::TypeError;
use super::exhaustiveness;
use super::resolve::Resolver;
use super::unify::Unifier;

// Submodules
mod builtins;
mod traits;
mod collect;
mod check;
mod expr;
mod patterns;
mod closure;
mod suggestions;

/// The main type checking context.
pub struct TypeContext<'a> {
    /// The source code (reserved for future use in error messages).
    pub(crate) _source: &'a str,
    /// The path to the source file (for resolving external modules).
    pub(crate) source_path: Option<PathBuf>,
    /// The string interner for resolving symbols.
    pub(crate) interner: DefaultStringInterner,
    /// The name resolver.
    pub resolver: Resolver<'a>,
    /// The type unifier.
    pub unifier: Unifier,
    /// Type signatures for functions.
    pub(crate) fn_sigs: HashMap<DefId, hir::FnSig>,
    /// Struct definitions.
    pub(crate) struct_defs: HashMap<DefId, StructInfo>,
    /// Enum definitions.
    pub(crate) enum_defs: HashMap<DefId, EnumInfo>,
    /// Type alias definitions.
    pub(crate) type_aliases: HashMap<DefId, TypeAliasInfo>,
    /// Const definitions.
    pub(crate) const_defs: HashMap<DefId, ConstInfo>,
    /// Static definitions.
    pub(crate) static_defs: HashMap<DefId, StaticInfo>,
    /// Const items to type-check (includes full declaration for the value expression).
    pub(crate) pending_consts: Vec<(DefId, ast::ConstDecl)>,
    /// Static items to type-check (includes full declaration for the value expression).
    pub(crate) pending_statics: Vec<(DefId, ast::StaticDecl)>,
    /// Functions to type-check (includes full declaration for parameter names).
    /// The third element is the parent module DefId, if any.
    pub(crate) pending_bodies: Vec<(DefId, ast::FnDecl, Option<DefId>)>,
    /// Current module DefId during collection phase.
    /// Used to track parent module for functions defined in modules.
    pub(crate) current_module: Option<DefId>,
    /// The current function's return type.
    pub(crate) return_type: Option<Type>,
    /// The current function's DefId (for effect checking).
    pub(crate) current_fn: Option<DefId>,
    /// Stack of currently handled effects with their type arguments (from enclosing with...handle blocks).
    /// Each entry is (effect_id, effect_type_args).
    pub(crate) handled_effects: Vec<(DefId, Vec<Type>)>,
    /// Errors encountered.
    pub(crate) errors: Vec<TypeError>,
    /// Compiled bodies.
    pub(crate) bodies: HashMap<hir::BodyId, hir::Body>,
    /// Mapping from function DefId to its body.
    pub(crate) fn_bodies: HashMap<DefId, hir::BodyId>,
    /// Next body ID.
    pub(crate) next_body_id: u32,
    /// Local variables in current function.
    pub(crate) locals: Vec<hir::Local>,
    /// Current generic type parameters in scope (name -> TyVarId).
    /// This is populated when entering a generic function/struct/enum
    /// and cleared when leaving.
    pub(crate) generic_params: HashMap<String, TyVarId>,
    /// Next type parameter ID for generating unique TyVarIds.
    pub(crate) next_type_param_id: u32,
    /// Current const generic parameters in scope (name -> ConstParamId).
    pub(crate) const_params: HashMap<String, hir::ConstParamId>,
    /// Next const parameter ID for generating unique ConstParamIds.
    pub(crate) next_const_param_id: u32,
    /// Current lifetime parameters in scope (name -> LifetimeId).
    pub(crate) lifetime_params: HashMap<String, hir::LifetimeId>,
    /// Next lifetime ID for generating unique LifetimeIds.
    pub(crate) next_lifetime_id: u32,
    /// Builtin function names (DefId -> function name).
    /// Used by codegen to resolve runtime function calls.
    pub(crate) builtin_fns: HashMap<DefId, String>,
    /// Effect definitions.
    pub(crate) effect_defs: HashMap<DefId, EffectInfo>,
    /// Handler definitions.
    pub(crate) handler_defs: HashMap<DefId, HandlerInfo>,
    /// Effect annotations for functions (DefId -> list of effects the function uses).
    pub(crate) fn_effects: HashMap<DefId, Vec<EffectRef>>,
    /// Handlers to type-check (includes full declaration for operation bodies).
    pub(crate) pending_handlers: Vec<(DefId, ast::HandlerDecl)>,
    /// Impl block definitions (keyed by a synthetic DefId for the impl block itself).
    pub(crate) impl_blocks: Vec<ImplBlockInfo>,
    /// Mapping from method DefId to its Self type (for resolving `self` in methods).
    pub(crate) method_self_types: HashMap<DefId, Type>,
    /// Trait definitions.
    pub(crate) trait_defs: HashMap<DefId, TraitInfo>,
    /// Type parameter bounds: maps TyVarId to list of trait DefIds it must implement.
    /// Used for method lookup on generic type parameters.
    pub(crate) type_param_bounds: HashMap<TyVarId, Vec<DefId>>,
    /// Expected type for `resume(value)` in the current handler operation body.
    /// Set when entering a handler operation scope, used for E0303 error checking.
    pub(crate) current_resume_type: Option<Type>,
    /// The result type of `resume()` in the current handler operation body.
    /// For deep handlers, this is a type variable representing the continuation's return value.
    /// For shallow handlers, this is unit (resume must be in tail position).
    pub(crate) current_resume_result_type: Option<Type>,
    /// The kind of handler (Deep or Shallow) for the current operation body.
    /// Used for linearity checking - shallow handlers enforce single-shot continuation semantics.
    pub(crate) current_handler_kind: Option<crate::ast::HandlerKind>,
    /// Count of resume calls in the current handler operation body.
    /// Used to enforce single-shot semantics for shallow handlers (resume_count <= 1).
    pub(crate) resume_count_in_current_op: usize,
    /// FFI bridge block definitions.
    pub(crate) bridge_defs: Vec<BridgeInfo>,
    /// Module definitions.
    pub(crate) module_defs: HashMap<DefId, ModuleInfo>,
    /// Current impl block's Self type during collection phase.
    /// Set when collecting impl block method signatures so `Self` can be resolved.
    pub(crate) current_impl_self_ty: Option<Type>,
    /// Forall parameter environment for resolving type parameter names in forall bodies.
    /// Maps parameter names to their TyVarId when parsing forall<T>. expressions.
    pub(crate) forall_param_env: Vec<(crate::ast::Symbol, TyVarId)>,
    /// Tuple destructuring info: maps the hidden tuple local to the element locals.
    /// Used by MIR lowering to decompose tuple patterns.
    pub tuple_destructures: HashMap<hir::LocalId, Vec<hir::LocalId>>,
    /// Next loop ID for generating unique LoopIds.
    pub(crate) next_loop_id: u32,
    /// Stack of loops, with optional LoopId for each (None for unlabeled loops).
    /// The last entry is the innermost loop.
    pub(crate) loop_stack: Vec<hir::LoopId>,
    /// Mapping from label names to LoopIds for labeled loops.
    pub(crate) loop_labels: HashMap<String, hir::LoopId>,
}

/// Information about a struct.
#[derive(Debug, Clone)]
pub struct StructInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub generics: Vec<TyVarId>,
}

/// Information about a field.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Type,
    pub index: u32,
}

/// Information about an enum.
#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub name: String,
    pub variants: Vec<VariantInfo>,
    pub generics: Vec<TyVarId>,
}

/// Information about a type alias.
#[derive(Debug, Clone)]
pub struct TypeAliasInfo {
    pub name: String,
    pub ty: Type,
    pub generics: Vec<TyVarId>,
}

/// Information about a const item.
#[derive(Debug, Clone)]
pub struct ConstInfo {
    /// The name of the constant.
    pub name: String,
    /// The type of the constant.
    pub ty: Type,
    /// The body ID containing the initializer expression.
    pub body_id: hir::BodyId,
    /// Source span.
    pub span: Span,
}

/// Information about a static item.
#[derive(Debug, Clone)]
pub struct StaticInfo {
    /// The name of the static.
    pub name: String,
    /// The type of the static.
    pub ty: Type,
    /// Whether this is a mutable static.
    pub is_mut: bool,
    /// The body ID containing the initializer expression.
    pub body_id: hir::BodyId,
    /// Source span.
    pub span: Span,
}

/// Information about a variant.
#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub index: u32,
    pub def_id: DefId,
}

/// Information about an effect.
#[derive(Debug, Clone)]
pub struct EffectInfo {
    pub name: String,
    pub operations: Vec<OperationInfo>,
    pub generics: Vec<TyVarId>,
}

/// Information about an effect operation.
#[derive(Debug, Clone)]
pub struct OperationInfo {
    pub name: String,
    pub params: Vec<Type>,
    pub return_ty: Type,
    pub def_id: DefId,
}

/// Information about a handler.
#[derive(Debug, Clone)]
pub struct HandlerInfo {
    pub name: String,
    /// Handler kind: deep (reify continuation) or shallow (single shot).
    pub kind: ast::HandlerKind,
    /// The effect this handler handles (DefId of the effect).
    pub effect_id: DefId,
    /// Type arguments to the effect (e.g., `State<i32>` -> `[i32]`).
    /// Used to substitute the effect's generic params when type-checking handler operations.
    pub effect_type_args: Vec<Type>,
    /// The operations implemented by this handler (name, body_id).
    pub operations: Vec<(String, hir::BodyId)>,
    pub generics: Vec<TyVarId>,
    /// State fields in the handler (used for struct-like initialization).
    pub fields: Vec<FieldInfo>,
    /// Return clause body ID, if present.
    pub return_clause_body_id: Option<hir::BodyId>,
}

/// A reference to an effect with type arguments.
/// For example, `State<i32>` would be EffectRef { def_id: State's DefId, type_args: [i32] }
#[derive(Debug, Clone)]
pub struct EffectRef {
    /// The effect definition this refers to.
    pub def_id: DefId,
    /// Type arguments instantiating the effect's generics.
    pub type_args: Vec<Type>,
}

/// Information about an impl block.
#[derive(Debug, Clone)]
pub struct ImplBlockInfo {
    /// The type being implemented for.
    pub self_ty: Type,
    /// The trait being implemented, if any (DefId of the trait).
    pub trait_ref: Option<DefId>,
    /// Generic type parameters.
    pub generics: Vec<TyVarId>,
    /// Associated functions (methods).
    pub methods: Vec<ImplMethodInfo>,
    /// Associated types (for trait impls).
    pub assoc_types: Vec<ImplAssocTypeInfo>,
    /// Associated constants.
    pub assoc_consts: Vec<ImplAssocConstInfo>,
    /// Source location of the impl block.
    pub span: Span,
}

/// Information about a method in an impl block.
#[derive(Debug, Clone)]
pub struct ImplMethodInfo {
    /// The method's DefId.
    pub def_id: DefId,
    /// The method name.
    pub name: String,
    /// Whether this is a static method (no self parameter).
    pub is_static: bool,
}

/// Information about an associated type in an impl block.
#[derive(Debug, Clone)]
pub struct ImplAssocTypeInfo {
    /// The name of the associated type.
    pub name: String,
    /// The concrete type.
    pub ty: Type,
}

/// Information about an associated constant in an impl block.
#[derive(Debug, Clone)]
pub struct ImplAssocConstInfo {
    /// The constant's DefId.
    pub def_id: DefId,
    /// The name of the constant.
    pub name: String,
    /// The type of the constant.
    pub ty: Type,
}

/// Information about a trait declaration.
#[derive(Debug, Clone)]
pub struct TraitInfo {
    /// The trait name.
    pub name: String,
    /// Generic type parameters.
    pub generics: Vec<TyVarId>,
    /// Supertrait bounds.
    pub supertraits: Vec<DefId>,
    /// Required methods.
    pub methods: Vec<TraitMethodInfo>,
    /// Associated types.
    pub assoc_types: Vec<TraitAssocTypeInfo>,
    /// Associated constants.
    pub assoc_consts: Vec<TraitAssocConstInfo>,
}

/// Information about a trait method.
#[derive(Debug, Clone)]
pub struct TraitMethodInfo {
    /// The method's DefId.
    pub def_id: DefId,
    /// The method name.
    pub name: String,
    /// The method signature.
    pub sig: hir::FnSig,
    /// Whether this method has a default implementation.
    pub has_default: bool,
}

/// Information about a trait associated type.
#[derive(Debug, Clone)]
pub struct TraitAssocTypeInfo {
    /// The associated type name.
    pub name: String,
    /// Default type, if any.
    pub default: Option<Type>,
}

/// Information about a trait associated constant.
#[derive(Debug, Clone)]
pub struct TraitAssocConstInfo {
    /// The constant's DefId.
    pub def_id: DefId,
    /// The constant name.
    pub name: String,
    /// The constant type.
    pub ty: Type,
    /// Whether this has a default value.
    pub has_default: bool,
}

/// Information about an FFI bridge block.
#[derive(Debug, Clone)]
pub struct BridgeInfo {
    /// The bridge name.
    pub name: String,
    /// The ABI/language (e.g., "C", "C++", "wasm").
    pub abi: String,
    /// Link specifications.
    pub link_specs: Vec<BridgeLinkSpec>,
    /// External functions in this bridge.
    pub extern_fns: Vec<BridgeFnInfo>,
    /// Opaque types.
    pub opaque_types: Vec<BridgeOpaqueInfo>,
    /// Type aliases.
    pub type_aliases: Vec<BridgeTypeAliasInfo>,
    /// FFI structs.
    pub structs: Vec<BridgeStructInfo>,
    /// FFI enums.
    pub enums: Vec<BridgeEnumInfo>,
    /// FFI unions.
    pub unions: Vec<BridgeUnionInfo>,
    /// FFI constants.
    pub consts: Vec<BridgeConstInfo>,
    /// Callback types.
    pub callbacks: Vec<BridgeCallbackInfo>,
    /// Source span.
    pub span: Span,
}

/// Link specification for an FFI bridge.
#[derive(Debug, Clone)]
pub struct BridgeLinkSpec {
    /// Library name to link.
    pub name: String,
    /// Link kind.
    pub kind: BridgeLinkKind,
    /// WASM import module name.
    pub wasm_import_module: Option<String>,
}

/// Link kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BridgeLinkKind {
    #[default]
    Dylib,
    Static,
    Framework,
}

/// External function info.
#[derive(Debug, Clone)]
pub struct BridgeFnInfo {
    /// DefId for this function.
    pub def_id: DefId,
    /// Function name.
    pub name: String,
    /// Parameter types.
    pub params: Vec<Type>,
    /// Return type.
    pub return_ty: Type,
    /// Link name (if different from name).
    pub link_name: Option<String>,
    /// Whether variadic.
    pub is_variadic: bool,
    /// Source span.
    pub span: Span,
}

/// Opaque type info.
#[derive(Debug, Clone)]
pub struct BridgeOpaqueInfo {
    /// DefId for this type.
    pub def_id: DefId,
    /// Type name.
    pub name: String,
    /// Source span.
    pub span: Span,
}

/// Bridge type alias info.
#[derive(Debug, Clone)]
pub struct BridgeTypeAliasInfo {
    /// DefId for this type.
    pub def_id: DefId,
    /// Type name.
    pub name: String,
    /// Aliased type.
    pub ty: Type,
    /// Source span.
    pub span: Span,
}

/// FFI struct info.
#[derive(Debug, Clone)]
pub struct BridgeStructInfo {
    /// DefId for this struct.
    pub def_id: DefId,
    /// Struct name.
    pub name: String,
    /// Fields.
    pub fields: Vec<BridgeFieldInfo>,
    /// Whether packed.
    pub is_packed: bool,
    /// Explicit alignment.
    pub align: Option<u32>,
    /// Source span.
    pub span: Span,
}

/// FFI field info.
#[derive(Debug, Clone)]
pub struct BridgeFieldInfo {
    /// Field name.
    pub name: String,
    /// Field type.
    pub ty: Type,
    /// Source span.
    pub span: Span,
}

/// FFI enum info.
#[derive(Debug, Clone)]
pub struct BridgeEnumInfo {
    /// DefId for this enum.
    pub def_id: DefId,
    /// Enum name.
    pub name: String,
    /// Representation type.
    pub repr: Type,
    /// Variants.
    pub variants: Vec<BridgeEnumVariantInfo>,
    /// Source span.
    pub span: Span,
}

/// FFI enum variant info.
#[derive(Debug, Clone)]
pub struct BridgeEnumVariantInfo {
    /// Variant name.
    pub name: String,
    /// Discriminant value.
    pub value: i64,
    /// Source span.
    pub span: Span,
}

/// FFI union info.
#[derive(Debug, Clone)]
pub struct BridgeUnionInfo {
    /// DefId for this union.
    pub def_id: DefId,
    /// Union name.
    pub name: String,
    /// Fields.
    pub fields: Vec<BridgeFieldInfo>,
    /// Source span.
    pub span: Span,
}

/// FFI constant info.
#[derive(Debug, Clone)]
pub struct BridgeConstInfo {
    /// DefId for this constant.
    pub def_id: DefId,
    /// Constant name.
    pub name: String,
    /// Constant type.
    pub ty: Type,
    /// Constant value.
    pub value: i64,
    /// Source span.
    pub span: Span,
}

/// Callback type info.
#[derive(Debug, Clone)]
pub struct BridgeCallbackInfo {
    /// DefId for this callback type.
    pub def_id: DefId,
    /// Callback name.
    pub name: String,
    /// Parameter types.
    pub params: Vec<Type>,
    /// Return type.
    pub return_ty: Type,
    /// Source span.
    pub span: Span,
}

/// Information about a module.
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// The module name.
    pub name: String,
    /// Items contained in this module.
    pub items: Vec<DefId>,
    /// Whether this is an external module (loaded from file).
    pub is_external: bool,
    /// Source span.
    pub span: Span,
}

impl<'a> TypeContext<'a> {
    /// Create a new type context.
    pub fn new(source: &'a str, interner: DefaultStringInterner) -> Self {
        let mut ctx = Self {
            _source: source,
            source_path: None,
            interner,
            resolver: Resolver::new(source),
            unifier: Unifier::new(),
            fn_sigs: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            type_aliases: HashMap::new(),
            const_defs: HashMap::new(),
            static_defs: HashMap::new(),
            pending_consts: Vec::new(),
            pending_statics: Vec::new(),
            pending_bodies: Vec::new(),
            current_module: None,
            return_type: None,
            current_fn: None,
            handled_effects: Vec::new(),
            errors: Vec::new(),
            bodies: HashMap::new(),
            fn_bodies: HashMap::new(),
            next_body_id: 0,
            locals: Vec::new(),
            generic_params: HashMap::new(),
            next_type_param_id: 0,
            const_params: HashMap::new(),
            next_const_param_id: 0,
            lifetime_params: HashMap::new(),
            next_lifetime_id: 1, // 0 is reserved for 'static
            builtin_fns: HashMap::new(),
            effect_defs: HashMap::new(),
            handler_defs: HashMap::new(),
            fn_effects: HashMap::new(),
            pending_handlers: Vec::new(),
            impl_blocks: Vec::new(),
            method_self_types: HashMap::new(),
            trait_defs: HashMap::new(),
            type_param_bounds: HashMap::new(),
            current_resume_type: None,
            current_resume_result_type: None,
            current_handler_kind: None,
            resume_count_in_current_op: 0,
            bridge_defs: Vec::new(),
            module_defs: HashMap::new(),
            current_impl_self_ty: None,
            forall_param_env: Vec::new(),
            tuple_destructures: HashMap::new(),
            next_loop_id: 0,
            loop_stack: Vec::new(),
            loop_labels: HashMap::new(),
        };
        ctx.register_builtins();
        ctx
    }

    /// Set the source file path (for resolving external modules).
    pub fn with_source_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.source_path = Some(path.into());
        self
    }

    /// Convert a Symbol to a String.
    ///
    /// Note: This is a temporary implementation. In the real implementation,
    /// we'd use the string interner from the parser.
    pub(crate) fn symbol_to_string(&self, symbol: ast::Symbol) -> String {
        self.interner.resolve(symbol)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("sym_{}", symbol.to_usize()))
    }

    /// Convert a Type to a string for display.
    pub(crate) fn type_to_string(&self, ty: &Type) -> String {
        format!("{}", ty)
    }

    /// Generate a fresh loop ID.
    pub(crate) fn fresh_loop_id(&mut self) -> hir::LoopId {
        let id = self.next_loop_id;
        self.next_loop_id += 1;
        hir::LoopId::new(id)
    }

    /// Enter a loop, optionally with a label.
    /// Returns the LoopId assigned to this loop.
    pub(crate) fn enter_loop(&mut self, label: Option<&str>) -> hir::LoopId {
        let loop_id = self.fresh_loop_id();
        self.loop_stack.push(loop_id);
        if let Some(name) = label {
            self.loop_labels.insert(name.to_string(), loop_id);
        }
        loop_id
    }

    /// Exit the current loop.
    pub(crate) fn exit_loop(&mut self, label: Option<&str>) {
        self.loop_stack.pop();
        if let Some(name) = label {
            self.loop_labels.remove(name);
        }
    }

    /// Find the LoopId for a labeled break/continue.
    /// If label is None, returns the innermost loop.
    /// If label is Some, looks up the label in the loop_labels map.
    pub(crate) fn find_loop(&self, label: Option<&str>) -> Option<hir::LoopId> {
        match label {
            None => self.loop_stack.last().copied(),
            Some(name) => self.loop_labels.get(name).copied(),
        }
    }

    /// Check if a type is a linear type (must be used exactly once).
    ///
    /// Linear types are specified with the `linear T` syntax in Blood.
    /// They are critical for effect handler linearity checking (E0304):
    /// multi-shot handlers cannot safely capture linear values.
    pub(crate) fn is_type_linear(&self, ty: &Type) -> bool {
        use crate::hir::ty::{TypeKind, OwnershipQualifier};
        match ty.kind() {
            TypeKind::Ownership { qualifier: OwnershipQualifier::Linear, .. } => true,
            // Check inner types recursively for ownership qualifiers
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                self.is_type_linear(inner)
            }
            TypeKind::Tuple(tys) => tys.iter().any(|t| self.is_type_linear(t)),
            TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                self.is_type_linear(element)
            }
            _ => false,
        }
    }

    /// Check if a type is an affine type (may be used at most once).
    ///
    /// Affine types are specified with the `affine T` syntax in Blood.
    /// Unlike linear types, affine values can be safely dropped without use.
    pub(crate) fn is_type_affine(&self, ty: &Type) -> bool {
        use crate::hir::ty::{TypeKind, OwnershipQualifier};
        match ty.kind() {
            TypeKind::Ownership { qualifier: OwnershipQualifier::Affine, .. } => true,
            // Check inner types recursively for ownership qualifiers
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                self.is_type_affine(inner)
            }
            TypeKind::Tuple(tys) => tys.iter().any(|t| self.is_type_affine(t)),
            TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                self.is_type_affine(element)
            }
            _ => false,
        }
    }

    // Static zonk functions that take a Unifier reference
    // These are used to resolve type variables before codegen

    /// Zonk all types in a body, using the given unifier for type resolution.
    fn zonk_body_with_unifier(unifier: &Unifier, body: hir::Body) -> hir::Body {
        let zonked_locals = body.locals.into_iter().map(|local| {
            hir::Local {
                id: local.id,
                ty: Self::zonk_type_with_unifier(unifier, &local.ty),
                mutable: local.mutable,
                name: local.name,
                span: local.span,
            }
        }).collect();
        let zonked_expr = Self::zonk_expr_with_unifier(unifier, body.expr);
        hir::Body {
            locals: zonked_locals,
            param_count: body.param_count,
            expr: zonked_expr,
            span: body.span,
            tuple_destructures: body.tuple_destructures,
        }
    }

    /// Resolve all type variables in a type using the given unifier's substitutions.
    fn zonk_type_with_unifier(unifier: &Unifier, ty: &Type) -> Type {
        let resolved = unifier.resolve(ty);
        match resolved.kind() {
            TypeKind::Fn { params, ret } => {
                let zonked_params: Vec<_> = params.iter().map(|p| Self::zonk_type_with_unifier(unifier, p)).collect();
                let zonked_ret = Self::zonk_type_with_unifier(unifier, ret);
                Type::function(zonked_params, zonked_ret)
            }
            TypeKind::Tuple(elems) => {
                let zonked: Vec<_> = elems.iter().map(|e| Self::zonk_type_with_unifier(unifier, e)).collect();
                Type::tuple(zonked)
            }
            TypeKind::Array { element, size } => {
                let zonked_elem = Self::zonk_type_with_unifier(unifier, element);
                Type::array(zonked_elem, *size)
            }
            TypeKind::Slice { element } => {
                let zonked_elem = Self::zonk_type_with_unifier(unifier, element);
                Type::slice(zonked_elem)
            }
            TypeKind::Ref { inner, mutable } => {
                let zonked_inner = Self::zonk_type_with_unifier(unifier, inner);
                Type::reference(zonked_inner, *mutable)
            }
            TypeKind::Ptr { inner, mutable } => {
                let zonked_inner = Self::zonk_type_with_unifier(unifier, inner);
                Type::new(TypeKind::Ptr { inner: zonked_inner, mutable: *mutable })
            }
            TypeKind::Adt { def_id, args } => {
                let zonked_args: Vec<_> = args.iter().map(|a| Self::zonk_type_with_unifier(unifier, a)).collect();
                Type::adt(*def_id, zonked_args)
            }
            TypeKind::Range { element, inclusive } => {
                let zonked_elem = Self::zonk_type_with_unifier(unifier, element);
                Type::new(TypeKind::Range { element: zonked_elem, inclusive: *inclusive })
            }
            _ => resolved,
        }
    }

    /// Zonk all types in an expression tree using the given unifier.
    fn zonk_expr_with_unifier(unifier: &Unifier, expr: hir::Expr) -> hir::Expr {
        let zonked_ty = Self::zonk_type_with_unifier(unifier, &expr.ty);
        let zonked_kind = match expr.kind {
            hir::ExprKind::Binary { op, left, right } => {
                hir::ExprKind::Binary {
                    op,
                    left: Box::new(Self::zonk_expr_with_unifier(unifier, *left)),
                    right: Box::new(Self::zonk_expr_with_unifier(unifier, *right)),
                }
            }
            hir::ExprKind::Unary { op, operand } => {
                hir::ExprKind::Unary {
                    op,
                    operand: Box::new(Self::zonk_expr_with_unifier(unifier, *operand)),
                }
            }
            hir::ExprKind::Call { callee, args } => {
                hir::ExprKind::Call {
                    callee: Box::new(Self::zonk_expr_with_unifier(unifier, *callee)),
                    args: args.into_iter().map(|a| Self::zonk_expr_with_unifier(unifier, a)).collect(),
                }
            }
            hir::ExprKind::Block { stmts, expr } => {
                let zonked_stmts = stmts.into_iter().map(|s| Self::zonk_stmt_with_unifier(unifier, s)).collect();
                let zonked_expr = expr.map(|e| Box::new(Self::zonk_expr_with_unifier(unifier, *e)));
                hir::ExprKind::Block { stmts: zonked_stmts, expr: zonked_expr }
            }
            hir::ExprKind::If { condition, then_branch, else_branch } => {
                hir::ExprKind::If {
                    condition: Box::new(Self::zonk_expr_with_unifier(unifier, *condition)),
                    then_branch: Box::new(Self::zonk_expr_with_unifier(unifier, *then_branch)),
                    else_branch: else_branch.map(|e| Box::new(Self::zonk_expr_with_unifier(unifier, *e))),
                }
            }
            hir::ExprKind::Match { scrutinee, arms } => {
                hir::ExprKind::Match {
                    scrutinee: Box::new(Self::zonk_expr_with_unifier(unifier, *scrutinee)),
                    arms: arms.into_iter().map(|arm| hir::MatchArm {
                        pattern: Self::zonk_pattern_with_unifier(unifier, arm.pattern),
                        guard: arm.guard.map(|g| Self::zonk_expr_with_unifier(unifier, g)),
                        body: Self::zonk_expr_with_unifier(unifier, arm.body),
                    }).collect(),
                }
            }
            hir::ExprKind::Loop { body, label } => {
                hir::ExprKind::Loop {
                    body: Box::new(Self::zonk_expr_with_unifier(unifier, *body)),
                    label,
                }
            }
            hir::ExprKind::While { condition, body, label } => {
                hir::ExprKind::While {
                    condition: Box::new(Self::zonk_expr_with_unifier(unifier, *condition)),
                    body: Box::new(Self::zonk_expr_with_unifier(unifier, *body)),
                    label,
                }
            }
            hir::ExprKind::Return(val) => {
                hir::ExprKind::Return(val.map(|v| Box::new(Self::zonk_expr_with_unifier(unifier, *v))))
            }
            hir::ExprKind::Break { value, label } => {
                hir::ExprKind::Break {
                    value: value.map(|v| Box::new(Self::zonk_expr_with_unifier(unifier, *v))),
                    label,
                }
            }
            hir::ExprKind::Assign { target, value } => {
                hir::ExprKind::Assign {
                    target: Box::new(Self::zonk_expr_with_unifier(unifier, *target)),
                    value: Box::new(Self::zonk_expr_with_unifier(unifier, *value)),
                }
            }
            hir::ExprKind::Field { base, field_idx } => {
                hir::ExprKind::Field {
                    base: Box::new(Self::zonk_expr_with_unifier(unifier, *base)),
                    field_idx,
                }
            }
            hir::ExprKind::Index { base, index } => {
                hir::ExprKind::Index {
                    base: Box::new(Self::zonk_expr_with_unifier(unifier, *base)),
                    index: Box::new(Self::zonk_expr_with_unifier(unifier, *index)),
                }
            }
            hir::ExprKind::Range { start, end, inclusive } => {
                hir::ExprKind::Range {
                    start: start.map(|s| Box::new(Self::zonk_expr_with_unifier(unifier, *s))),
                    end: end.map(|e| Box::new(Self::zonk_expr_with_unifier(unifier, *e))),
                    inclusive,
                }
            }
            hir::ExprKind::Cast { expr: inner, target_ty } => {
                hir::ExprKind::Cast {
                    expr: Box::new(Self::zonk_expr_with_unifier(unifier, *inner)),
                    target_ty: Self::zonk_type_with_unifier(unifier, &target_ty),
                }
            }
            hir::ExprKind::Tuple(elems) => {
                hir::ExprKind::Tuple(elems.into_iter().map(|e| Self::zonk_expr_with_unifier(unifier, e)).collect())
            }
            hir::ExprKind::Array(elems) => {
                hir::ExprKind::Array(elems.into_iter().map(|e| Self::zonk_expr_with_unifier(unifier, e)).collect())
            }
            hir::ExprKind::Repeat { value, count } => {
                hir::ExprKind::Repeat {
                    value: Box::new(Self::zonk_expr_with_unifier(unifier, *value)),
                    count,
                }
            }
            hir::ExprKind::Struct { def_id, fields, base } => {
                hir::ExprKind::Struct {
                    def_id,
                    fields: fields.into_iter().map(|f| hir::FieldExpr {
                        field_idx: f.field_idx,
                        value: Self::zonk_expr_with_unifier(unifier, f.value),
                    }).collect(),
                    base: base.map(|b| Box::new(Self::zonk_expr_with_unifier(unifier, *b))),
                }
            }
            hir::ExprKind::Variant { def_id, variant_idx, fields } => {
                hir::ExprKind::Variant {
                    def_id,
                    variant_idx,
                    fields: fields.into_iter().map(|e| Self::zonk_expr_with_unifier(unifier, e)).collect(),
                }
            }
            hir::ExprKind::Closure { body_id, captures } => {
                hir::ExprKind::Closure { body_id, captures }
            }
            hir::ExprKind::Perform { effect_id, op_index, args } => {
                hir::ExprKind::Perform {
                    effect_id,
                    op_index,
                    args: args.into_iter().map(|a| Self::zonk_expr_with_unifier(unifier, a)).collect(),
                }
            }
            hir::ExprKind::Resume { value } => {
                hir::ExprKind::Resume {
                    value: value.map(|v| Box::new(Self::zonk_expr_with_unifier(unifier, *v))),
                }
            }
            hir::ExprKind::Handle { body, handler_id, handler_instance } => {
                hir::ExprKind::Handle {
                    body: Box::new(Self::zonk_expr_with_unifier(unifier, *body)),
                    handler_id,
                    handler_instance: Box::new(Self::zonk_expr_with_unifier(unifier, *handler_instance)),
                }
            }
            hir::ExprKind::Deref(inner) => {
                hir::ExprKind::Deref(Box::new(Self::zonk_expr_with_unifier(unifier, *inner)))
            }
            hir::ExprKind::Borrow { expr: inner, mutable } => {
                hir::ExprKind::Borrow {
                    expr: Box::new(Self::zonk_expr_with_unifier(unifier, *inner)),
                    mutable,
                }
            }
            hir::ExprKind::AddrOf { expr: inner, mutable } => {
                hir::ExprKind::AddrOf {
                    expr: Box::new(Self::zonk_expr_with_unifier(unifier, *inner)),
                    mutable,
                }
            }
            hir::ExprKind::MethodCall { receiver, method, args } => {
                hir::ExprKind::MethodCall {
                    receiver: Box::new(Self::zonk_expr_with_unifier(unifier, *receiver)),
                    method,
                    args: args.into_iter().map(|a| Self::zonk_expr_with_unifier(unifier, a)).collect(),
                }
            }
            hir::ExprKind::Let { pattern, init } => {
                hir::ExprKind::Let {
                    pattern: Self::zonk_pattern_with_unifier(unifier, pattern),
                    init: Box::new(Self::zonk_expr_with_unifier(unifier, *init)),
                }
            }
            hir::ExprKind::Unsafe(inner) => {
                hir::ExprKind::Unsafe(Box::new(Self::zonk_expr_with_unifier(unifier, *inner)))
            }
            hir::ExprKind::Record { fields } => {
                hir::ExprKind::Record {
                    fields: fields.into_iter().map(|f| hir::RecordFieldExpr {
                        name: f.name,
                        value: Self::zonk_expr_with_unifier(unifier, f.value),
                    }).collect(),
                }
            }
            // Macro expansion nodes - zonk subexpressions
            hir::ExprKind::MacroExpansion { macro_name, format_str, args } => {
                hir::ExprKind::MacroExpansion {
                    macro_name,
                    format_str,
                    args: args.into_iter().map(|a| Self::zonk_expr_with_unifier(unifier, a)).collect(),
                }
            }
            hir::ExprKind::VecLiteral(exprs) => {
                hir::ExprKind::VecLiteral(
                    exprs.into_iter().map(|e| Self::zonk_expr_with_unifier(unifier, e)).collect()
                )
            }
            hir::ExprKind::VecRepeat { value, count } => {
                hir::ExprKind::VecRepeat {
                    value: Box::new(Self::zonk_expr_with_unifier(unifier, *value)),
                    count: Box::new(Self::zonk_expr_with_unifier(unifier, *count)),
                }
            }
            hir::ExprKind::Assert { condition, message } => {
                hir::ExprKind::Assert {
                    condition: Box::new(Self::zonk_expr_with_unifier(unifier, *condition)),
                    message: message.map(|m| Box::new(Self::zonk_expr_with_unifier(unifier, *m))),
                }
            }
            hir::ExprKind::Dbg(inner) => {
                hir::ExprKind::Dbg(Box::new(Self::zonk_expr_with_unifier(unifier, *inner)))
            }
            kind @ (hir::ExprKind::Literal(_)
                | hir::ExprKind::Local(_)
                | hir::ExprKind::Def(_)
                | hir::ExprKind::Continue { .. }
                | hir::ExprKind::MethodFamily { .. }
                | hir::ExprKind::Default
                | hir::ExprKind::Error) => kind,
        };
        hir::Expr::new(zonked_kind, zonked_ty, expr.span)
    }

    /// Zonk all types in a pattern using the given unifier.
    fn zonk_pattern_with_unifier(unifier: &Unifier, pattern: hir::Pattern) -> hir::Pattern {
        let zonked_ty = Self::zonk_type_with_unifier(unifier, &pattern.ty);
        let zonked_kind = match pattern.kind {
            hir::PatternKind::Struct { def_id, fields } => {
                hir::PatternKind::Struct {
                    def_id,
                    fields: fields.into_iter().map(|f| hir::FieldPattern {
                        field_idx: f.field_idx,
                        pattern: Self::zonk_pattern_with_unifier(unifier, f.pattern),
                    }).collect(),
                }
            }
            hir::PatternKind::Variant { def_id, variant_idx, fields } => {
                hir::PatternKind::Variant {
                    def_id,
                    variant_idx,
                    fields: fields.into_iter().map(|p| Self::zonk_pattern_with_unifier(unifier, p)).collect(),
                }
            }
            hir::PatternKind::Tuple(fields) => {
                hir::PatternKind::Tuple(fields.into_iter().map(|p| Self::zonk_pattern_with_unifier(unifier, p)).collect())
            }
            hir::PatternKind::Slice { prefix, slice, suffix } => {
                hir::PatternKind::Slice {
                    prefix: prefix.into_iter().map(|p| Self::zonk_pattern_with_unifier(unifier, p)).collect(),
                    slice: slice.map(|p| Box::new(Self::zonk_pattern_with_unifier(unifier, *p))),
                    suffix: suffix.into_iter().map(|p| Self::zonk_pattern_with_unifier(unifier, p)).collect(),
                }
            }
            hir::PatternKind::Or(alts) => {
                hir::PatternKind::Or(alts.into_iter().map(|p| Self::zonk_pattern_with_unifier(unifier, p)).collect())
            }
            hir::PatternKind::Ref { mutable, inner } => {
                hir::PatternKind::Ref {
                    mutable,
                    inner: Box::new(Self::zonk_pattern_with_unifier(unifier, *inner)),
                }
            }
            hir::PatternKind::Range { start, end, inclusive } => {
                hir::PatternKind::Range {
                    start: start.map(|p| Box::new(Self::zonk_pattern_with_unifier(unifier, *p))),
                    end: end.map(|p| Box::new(Self::zonk_pattern_with_unifier(unifier, *p))),
                    inclusive,
                }
            }
            hir::PatternKind::Binding { local_id, mutable, subpattern } => {
                hir::PatternKind::Binding {
                    local_id,
                    mutable,
                    subpattern: subpattern.map(|p| Box::new(Self::zonk_pattern_with_unifier(unifier, *p))),
                }
            }
            kind @ (hir::PatternKind::Wildcard
                | hir::PatternKind::Literal(_)) => kind,
        };
        hir::Pattern {
            kind: zonked_kind,
            ty: zonked_ty,
            span: pattern.span,
        }
    }

    /// Zonk all types in a statement using the given unifier.
    fn zonk_stmt_with_unifier(unifier: &Unifier, stmt: hir::Stmt) -> hir::Stmt {
        match stmt {
            hir::Stmt::Let { local_id, init } => {
                hir::Stmt::Let {
                    local_id,
                    init: init.map(|e| Self::zonk_expr_with_unifier(unifier, e)),
                }
            }
            hir::Stmt::Expr(expr) => hir::Stmt::Expr(Self::zonk_expr_with_unifier(unifier, expr)),
            hir::Stmt::Item(def_id) => hir::Stmt::Item(def_id),
        }
    }

    /// Convert to HIR crate.
    pub fn into_hir(self) -> hir::Crate {
        let mut items = HashMap::new();

        // Convert collected definitions to HIR items
        for (def_id, info) in self.resolver.def_info {
            let kind = match info.kind {
                hir::DefKind::Fn => {
                    if let Some(sig) = self.fn_sigs.get(&def_id) {
                        hir::ItemKind::Fn(hir::FnDef {
                            sig: sig.clone(),
                            body_id: self.fn_bodies.get(&def_id).copied(),
                            generics: hir::Generics::empty(),
                        })
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Struct => {
                    if let Some(struct_info) = self.struct_defs.get(&def_id) {
                        let fields: Vec<_> = struct_info.fields.iter().map(|f| {
                            hir::FieldDef {
                                index: f.index,
                                name: Some(f.name.clone()),
                                ty: f.ty.clone(),
                                vis: ast::Visibility::Public,
                                span: info.span,
                            }
                        }).collect();

                        hir::ItemKind::Struct(hir::StructDef {
                            generics: hir::Generics::empty(),
                            kind: hir::StructKind::Record(fields),
                        })
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Enum => {
                    if let Some(enum_info) = self.enum_defs.get(&def_id) {
                        let variants: Vec<_> = enum_info.variants.iter().map(|v| {
                            let fields: Vec<_> = v.fields.iter().map(|f| {
                                hir::FieldDef {
                                    index: f.index,
                                    name: Some(f.name.clone()),
                                    ty: f.ty.clone(),
                                    vis: ast::Visibility::Public,
                                    span: info.span,
                                }
                            }).collect();

                            hir::Variant {
                                index: v.index,
                                name: v.name.clone(),
                                fields: if fields.is_empty() {
                                    hir::StructKind::Unit
                                } else {
                                    hir::StructKind::Record(fields)
                                },
                                def_id: v.def_id,
                                span: info.span,
                            }
                        }).collect();

                        hir::ItemKind::Enum(hir::EnumDef {
                            generics: hir::Generics::empty(),
                            variants,
                        })
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Effect => {
                    if let Some(effect_info) = self.effect_defs.get(&def_id) {
                        let operations: Vec<_> = effect_info.operations.iter().map(|op| {
                            hir::EffectOp {
                                name: op.name.clone(),
                                inputs: op.params.clone(),
                                output: op.return_ty.clone(),
                                def_id: op.def_id,
                                span: Span::default(),
                            }
                        }).collect();

                        hir::ItemKind::Effect {
                            generics: hir::Generics::empty(),
                            operations,
                        }
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Handler => {
                    if let Some(handler_info) = self.handler_defs.get(&def_id) {
                        // Convert handler state fields
                        let state: Vec<_> = handler_info.fields.iter().map(|f| {
                            hir::HandlerState {
                                name: f.name.clone(),
                                ty: f.ty.clone(),
                                mutable: true, // Handler state is typically mutable
                                default: None, // Default values handled at instantiation
                            }
                        }).collect();

                        // Convert ast::HandlerKind to hir::HandlerKind
                        let hir_kind = match handler_info.kind {
                            ast::HandlerKind::Deep => hir::HandlerKind::Deep,
                            ast::HandlerKind::Shallow => hir::HandlerKind::Shallow,
                        };

                        // Build operation list from body IDs
                        let operations: Vec<hir::HandlerOp> = handler_info.operations.iter()
                            .map(|(name, body_id)| hir::HandlerOp {
                                name: name.clone(),
                                body_id: *body_id,
                                span: info.span,
                            })
                            .collect();

                        // Build return clause if present
                        let return_clause = handler_info.return_clause_body_id.map(|body_id| {
                            hir::ReturnClause {
                                param: "x".to_string(), // Default param name
                                body_id,
                                span: info.span,
                            }
                        });

                        hir::ItemKind::Handler {
                            generics: hir::Generics::empty(),
                            kind: hir_kind,
                            effect: Type::adt(handler_info.effect_id, Vec::new()),
                            state,
                            operations,
                            return_clause,
                        }
                    } else {
                        continue;
                    }
                }
                hir::DefKind::Const => {
                    // Constants - get type and body from const_defs
                    if let Some(const_info) = self.const_defs.get(&def_id) {
                        hir::ItemKind::Const {
                            ty: const_info.ty.clone(),
                            body_id: const_info.body_id,
                        }
                    } else {
                        // Constants must have info - skip if missing
                        continue;
                    }
                }
                hir::DefKind::Static => {
                    // Statics - get type and body from static_defs
                    if let Some(static_info) = self.static_defs.get(&def_id) {
                        hir::ItemKind::Static {
                            ty: static_info.ty.clone(),
                            mutable: static_info.is_mut,
                            body_id: static_info.body_id,
                        }
                    } else {
                        // Statics must have info - skip if missing
                        continue;
                    }
                }
                // Variants are part of enums, not top-level items
                hir::DefKind::Variant => continue,
                // Associated functions are like regular functions - they need to be compiled
                hir::DefKind::AssocFn => {
                    if let Some(sig) = self.fn_sigs.get(&def_id) {
                        hir::ItemKind::Fn(hir::FnDef {
                            sig: sig.clone(),
                            body_id: self.fn_bodies.get(&def_id).copied(),
                            generics: hir::Generics::empty(),
                        })
                    } else {
                        continue;
                    }
                }
                // Associated types and consts are part of impl blocks, not standalone items
                hir::DefKind::AssocType | hir::DefKind::AssocConst => continue,
                // TypeAlias, Trait not yet implemented
                hir::DefKind::TypeAlias | hir::DefKind::Trait => continue,
                // Closures are inline, not top-level items
                hir::DefKind::Closure => continue,
                // Type/lifetime/const params are not items
                hir::DefKind::TyParam | hir::DefKind::LifetimeParam | hir::DefKind::ConstParam => continue,
                // Local variables are not items
                hir::DefKind::Local => continue,
                // Effect operations are part of effects, not standalone
                hir::DefKind::EffectOp => continue,
                // Fields are part of structs, not standalone
                hir::DefKind::Field => continue,
                // Modules are namespaces, handle separately
                hir::DefKind::Mod => {
                    if let Some(module_info) = self.module_defs.get(&def_id) {
                        hir::ItemKind::Module(hir::ModuleDef {
                            items: module_info.items.clone(),
                            is_external: module_info.is_external,
                        })
                    } else {
                        continue;
                    }
                }
            };

            items.insert(def_id, hir::Item {
                def_id,
                kind,
                vis: ast::Visibility::Public,
                name: info.name,
                span: info.span,
            });
        }

        // Convert bridge definitions to HIR Bridge items
        for bridge_info in &self.bridge_defs {
            // Create a synthetic DefId for the bridge block
            let bridge_def_id = DefId::new(items.len() as u32 + 0x8000_0000);

            // Convert link specs
            let link_specs: Vec<hir::LinkSpec> = bridge_info.link_specs.iter()
                .map(|ls| hir::LinkSpec {
                    name: ls.name.clone(),
                    kind: match ls.kind {
                        BridgeLinkKind::Dylib => hir::LinkKind::Dylib,
                        BridgeLinkKind::Static => hir::LinkKind::Static,
                        BridgeLinkKind::Framework => hir::LinkKind::Framework,
                    },
                    wasm_import_module: ls.wasm_import_module.clone(),
                })
                .collect();

            // Convert extern functions
            let extern_fns: Vec<hir::ExternFnItem> = bridge_info.extern_fns.iter()
                .map(|ef| hir::ExternFnItem {
                    def_id: ef.def_id,
                    name: ef.name.clone(),
                    sig: hir::FnSig::new(ef.params.clone(), ef.return_ty.clone()),
                    link_name: ef.link_name.clone(),
                    is_variadic: ef.is_variadic,
                    span: ef.span,
                })
                .collect();

            // Convert opaque types
            let opaque_types: Vec<hir::OpaqueType> = bridge_info.opaque_types.iter()
                .map(|ot| hir::OpaqueType {
                    def_id: ot.def_id,
                    name: ot.name.clone(),
                    span: ot.span,
                })
                .collect();

            // Convert type aliases
            let type_aliases: Vec<hir::BridgeTypeAlias> = bridge_info.type_aliases.iter()
                .map(|ta| hir::BridgeTypeAlias {
                    def_id: ta.def_id,
                    name: ta.name.clone(),
                    ty: ta.ty.clone(),
                    span: ta.span,
                })
                .collect();

            // Convert structs
            let structs: Vec<hir::FfiStruct> = bridge_info.structs.iter()
                .map(|s| hir::FfiStruct {
                    def_id: s.def_id,
                    name: s.name.clone(),
                    fields: s.fields.iter()
                        .map(|f| hir::FfiField {
                            name: f.name.clone(),
                            ty: f.ty.clone(),
                            span: f.span,
                        })
                        .collect(),
                    is_packed: s.is_packed,
                    align: s.align,
                    span: s.span,
                })
                .collect();

            // Convert enums
            let enums: Vec<hir::FfiEnum> = bridge_info.enums.iter()
                .map(|e| hir::FfiEnum {
                    def_id: e.def_id,
                    name: e.name.clone(),
                    repr: e.repr.clone(),
                    variants: e.variants.iter()
                        .map(|v| hir::FfiEnumVariant {
                            name: v.name.clone(),
                            value: v.value,
                            span: v.span,
                        })
                        .collect(),
                    span: e.span,
                })
                .collect();

            // Convert unions
            let unions: Vec<hir::FfiUnion> = bridge_info.unions.iter()
                .map(|u| hir::FfiUnion {
                    def_id: u.def_id,
                    name: u.name.clone(),
                    fields: u.fields.iter()
                        .map(|f| hir::FfiField {
                            name: f.name.clone(),
                            ty: f.ty.clone(),
                            span: f.span,
                        })
                        .collect(),
                    span: u.span,
                })
                .collect();

            // Convert constants
            let consts: Vec<hir::FfiConst> = bridge_info.consts.iter()
                .map(|c| hir::FfiConst {
                    def_id: c.def_id,
                    name: c.name.clone(),
                    ty: c.ty.clone(),
                    value: c.value,
                    span: c.span,
                })
                .collect();

            // Convert callbacks
            let callbacks: Vec<hir::FfiCallback> = bridge_info.callbacks.iter()
                .map(|cb| hir::FfiCallback {
                    def_id: cb.def_id,
                    name: cb.name.clone(),
                    params: cb.params.clone(),
                    return_type: cb.return_ty.clone(),
                    span: cb.span,
                })
                .collect();

            let bridge_def = hir::BridgeDef {
                abi: bridge_info.abi.clone(),
                link_specs,
                extern_fns,
                opaque_types,
                type_aliases,
                structs,
                enums,
                unions,
                consts,
                callbacks,
            };

            items.insert(bridge_def_id, hir::Item {
                def_id: bridge_def_id,
                kind: hir::ItemKind::Bridge(bridge_def),
                vis: ast::Visibility::Public,
                name: bridge_info.name.clone(),
                span: bridge_info.span,
            });
        }

        // Find main function
        let entry = items.iter()
            .find(|(_, item)| item.name == "main" || item.name.ends_with("_main"))
            .map(|(id, _)| *id);

        // Zonk all bodies to resolve type variables
        // We need to clone the unifier to avoid partial move issues
        let unifier = self.unifier.clone();
        let mut zonked_bodies = HashMap::new();
        for (id, body) in self.bodies {
            zonked_bodies.insert(id, Self::zonk_body_with_unifier(&unifier, body));
        }

        hir::Crate {
            items,
            bodies: zonked_bodies,
            entry,
            builtin_fns: self.builtin_fns,
        }
    }

    /// Get enum variant info for exhaustiveness checking.
    pub(crate) fn get_enum_variant_info(&self, ty: &Type) -> Option<exhaustiveness::EnumVariantInfo> {
        match ty.kind() {
            TypeKind::Adt { def_id, .. } => {
                self.enum_defs.get(def_id).map(|enum_info| exhaustiveness::EnumVariantInfo {
                        variant_count: enum_info.variants.len() as u32,
                        variant_names: enum_info.variants.iter()
                            .map(|v| v.name.clone())
                            .collect(),
                    })
            }
            _ => None,
        }
    }
}
