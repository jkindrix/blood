//! Type checking context.
//!
//! The TypeContext is the main entry point for type checking. It coordinates
//! name resolution, type collection, and type inference.

use std::collections::HashMap;

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
    /// Functions to type-check (includes full declaration for parameter names).
    pub(crate) pending_bodies: Vec<(DefId, ast::FnDecl)>,
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

impl<'a> TypeContext<'a> {
    /// Create a new type context.
    pub fn new(source: &'a str, interner: DefaultStringInterner) -> Self {
        let mut ctx = Self {
            _source: source,
            interner,
            resolver: Resolver::new(source),
            unifier: Unifier::new(),
            fn_sigs: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            type_aliases: HashMap::new(),
            pending_bodies: Vec::new(),
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
        };
        ctx.register_builtins();
        ctx
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
                    // Constants - get type and body from collected info
                    if let (Some(ty), Some(body_id)) = (&info.ty, self.fn_bodies.get(&def_id).copied()) {
                        hir::ItemKind::Const {
                            ty: ty.clone(),
                            body_id,
                        }
                    } else {
                        // Constants must have both type and body - skip if missing
                        continue;
                    }
                }
                hir::DefKind::Static => {
                    // Statics - get type and body from collected info
                    if let (Some(ty), Some(body_id)) = (&info.ty, self.fn_bodies.get(&def_id).copied()) {
                        hir::ItemKind::Static {
                            ty: ty.clone(),
                            mutable: false, // Would need to track this
                            body_id,
                        }
                    } else {
                        // Statics must have both type and body - skip if missing
                        continue;
                    }
                }
                // Variants are part of enums, not top-level items
                hir::DefKind::Variant => continue,
                // Associated items in impl blocks - not standalone items
                hir::DefKind::AssocType | hir::DefKind::AssocConst | hir::DefKind::AssocFn => continue,
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
            };

            items.insert(def_id, hir::Item {
                def_id,
                kind,
                vis: ast::Visibility::Public,
                name: info.name,
                span: info.span,
            });
        }

        // Find main function
        let entry = items.iter()
            .find(|(_, item)| item.name == "main" || item.name.ends_with("_main"))
            .map(|(id, _)| *id);

        hir::Crate {
            items,
            bodies: self.bodies,
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
