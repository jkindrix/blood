//! Name resolution for Blood.
//!
//! This module resolves identifiers to their definitions, building
//! a scope hierarchy and populating the DefId map.

use std::collections::HashMap;

use crate::ast::Visibility;
use crate::hir::{DefId, DefKind, LocalId, Type, PrimitiveTy};
// Type primitives imported if needed for future expansion
use crate::span::Span;
use crate::diagnostics::Diagnostic;

use super::error::{TypeError, TypeErrorKind};

/// The name resolver.
#[derive(Debug)]
pub struct Resolver<'a> {
    /// The source code (reserved for future use in symbol resolution).
    _source: &'a str,
    /// All scopes, indexed by scope ID.
    scopes: Vec<Scope>,
    /// The current scope stack.
    scope_stack: Vec<usize>,
    /// Definition ID counter.
    next_def_id: u32,
    /// Local ID counter (per body).
    next_local_id: u32,
    /// Map from DefId to item info.
    pub def_info: HashMap<DefId, DefInfo>,
    /// Map from name to DefId for globals.
    globals: HashMap<String, DefId>,
    /// Errors encountered during resolution.
    pub errors: Vec<TypeError>,
}

/// Information about a definition.
#[derive(Debug, Clone)]
pub struct DefInfo {
    /// The kind of definition.
    pub kind: DefKind,
    /// The name of the definition.
    pub name: String,
    /// The span of the definition.
    pub span: Span,
    /// The type of this definition, if known.
    pub ty: Option<Type>,
    /// The parent definition (for associated items).
    pub parent: Option<DefId>,
    /// The visibility of this definition.
    pub visibility: Visibility,
}

/// A scope containing bindings.
#[derive(Debug, Clone)]
pub struct Scope {
    /// The kind of scope.
    pub kind: ScopeKind,
    /// Name bindings in this scope.
    pub bindings: HashMap<String, Binding>,
    /// Type bindings in this scope.
    pub type_bindings: HashMap<String, DefId>,
    /// The parent scope, if any.
    pub parent: Option<usize>,
    /// The span of this scope.
    pub span: Span,
}

/// The kind of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// The root/global scope.
    Root,
    /// A module scope.
    Module,
    /// A function scope.
    Function,
    /// A closure scope.
    Closure,
    /// A block scope.
    Block,
    /// A loop scope.
    Loop,
    /// A match arm scope.
    MatchArm,
    /// An impl block scope.
    Impl,
    /// A trait definition scope.
    Trait,
    /// An effect handler scope (with...handle block).
    Handler,
}

/// A binding in a scope.
#[derive(Debug, Clone)]
pub enum Binding {
    /// A local variable.
    Local {
        local_id: LocalId,
        ty: Type,
        mutable: bool,
        span: Span,
    },
    /// A definition (function, constant, etc.).
    Def(DefId),
    /// A method family (multiple functions with the same name for dispatch).
    /// Used for multiple dispatch where several function overloads share a name.
    Methods(Vec<DefId>),
}

impl<'a> Resolver<'a> {
    /// Create a new resolver.
    pub fn new(source: &'a str) -> Self {
        let mut resolver = Self {
            _source: source,
            scopes: Vec::new(),
            scope_stack: Vec::new(),
            next_def_id: 0,
            next_local_id: 0,
            def_info: HashMap::new(),
            globals: HashMap::new(),
            errors: Vec::new(),
        };

        // Create root scope
        resolver.scopes.push(Scope {
            kind: ScopeKind::Root,
            bindings: HashMap::new(),
            type_bindings: HashMap::new(),
            parent: None,
            span: Span::dummy(),
        });
        resolver.scope_stack.push(0);

        // Register primitive types in root scope
        resolver.register_primitive_types();

        resolver
    }

    /// Register primitive types in the root scope.
    fn register_primitive_types(&mut self) {
        // These don't get DefIds - they're handled specially
        // The root scope's type_bindings remain empty for primitives
        // since we check them via PrimitiveTy::from_name
    }

    /// Generate a new DefId.
    pub fn next_def_id(&mut self) -> DefId {
        let id = DefId::new(self.next_def_id);
        self.next_def_id += 1;
        id
    }

    /// Get the current DefId counter value without incrementing.
    /// Used for tracking which DefIds were created during a scope.
    pub fn current_def_id_counter(&self) -> u32 {
        self.next_def_id
    }

    /// Generate a new LocalId.
    pub fn next_local_id(&mut self) -> LocalId {
        let id = LocalId::new(self.next_local_id);
        self.next_local_id += 1;
        id
    }

    /// Reset local ID counter (for new function).
    pub fn reset_local_ids(&mut self) {
        self.next_local_id = 0;
    }

    /// Get the index of the current scope.
    ///
    /// # Panics
    /// Panics if the scope stack is empty. This should never happen as
    /// the root scope is always present (invariant established in `new()`).
    #[inline]
    fn current_scope_idx(&self) -> usize {
        *self.scope_stack.last()
            .expect("BUG: scope stack should never be empty - root scope must always be present")
    }

    /// Get the current scope.
    pub fn current_scope(&self) -> &Scope {
        let idx = self.current_scope_idx();
        &self.scopes[idx]
    }

    /// Get the current scope mutably.
    pub fn current_scope_mut(&mut self) -> &mut Scope {
        let idx = self.current_scope_idx();
        &mut self.scopes[idx]
    }

    /// Push a new scope.
    pub fn push_scope(&mut self, kind: ScopeKind, span: Span) {
        let parent = self.scope_stack.last().copied();
        let scope = Scope {
            kind,
            bindings: HashMap::new(),
            type_bindings: HashMap::new(),
            parent,
            span,
        };
        let idx = self.scopes.len();
        self.scopes.push(scope);
        self.scope_stack.push(idx);
    }

    /// Pop the current scope.
    pub fn pop_scope(&mut self) {
        self.scope_stack.pop();
    }

    /// Define a new item in the current scope.
    pub fn define_item(
        &mut self,
        name: String,
        kind: DefKind,
        span: Span,
    ) -> Result<DefId, TypeError> {
        // Check for duplicates in current scope
        if self.current_scope().bindings.contains_key(&name) {
            return Err(TypeError::new(
                TypeErrorKind::DuplicateDefinition { name },
                span,
            ));
        }

        let def_id = self.next_def_id();

        self.def_info.insert(def_id, DefInfo {
            kind,
            name: name.clone(),
            span,
            ty: None,
            parent: None,
            visibility: Visibility::Public,
        });

        self.current_scope_mut()
            .bindings
            .insert(name.clone(), Binding::Def(def_id));

        // Also add to globals if in root scope
        if self.current_scope().kind == ScopeKind::Root {
            self.globals.insert(name, def_id);
        }

        Ok(def_id)
    }

    /// Define a namespaced item that is NOT added to the global scope.
    ///
    /// Used for items like bridge functions that are accessed via namespace
    /// syntax (e.g., `libc::abs`) rather than direct name lookup.
    /// This avoids conflicts with global builtins that have the same name.
    pub fn define_namespaced_item(
        &mut self,
        name: String,
        kind: DefKind,
        span: Span,
    ) -> DefId {
        let def_id = self.next_def_id();

        self.def_info.insert(def_id, DefInfo {
            kind,
            name,
            span,
            ty: None,
            parent: None,
            visibility: Visibility::Public,
        });

        // Note: Do NOT add to current scope bindings or globals.
        // Namespaced items are accessed via their namespace (e.g., bridge_name::fn_name).

        def_id
    }

    /// Define a function in the current scope, supporting multiple dispatch.
    ///
    /// Unlike `define_item`, this allows multiple functions with the same name.
    /// When a function name already exists:
    /// - If it's a `Binding::Def` pointing to a function, convert to `Binding::Methods`
    /// - If it's already `Binding::Methods`, add to the list
    /// - If it's something else (non-function), error as duplicate definition
    pub fn define_function(
        &mut self,
        name: String,
        span: Span,
        visibility: Visibility,
    ) -> Result<DefId, TypeError> {
        let def_id = self.next_def_id();

        self.def_info.insert(def_id, DefInfo {
            kind: DefKind::Fn,
            name: name.clone(),
            span,
            ty: None,
            parent: None,
            visibility,
        });

        // Check if a binding with this name already exists
        if let Some(existing) = self.current_scope().bindings.get(&name).cloned() {
            match existing {
                Binding::Def(existing_def_id) => {
                    // Check if the existing definition is a function
                    if let Some(info) = self.def_info.get(&existing_def_id) {
                        if info.kind == DefKind::Fn {
                            // Convert to method family with both functions
                            self.current_scope_mut()
                                .bindings
                                .insert(name.clone(), Binding::Methods(vec![existing_def_id, def_id]));
                        } else {
                            // Non-function with same name - error
                            return Err(TypeError::new(
                                TypeErrorKind::DuplicateDefinition { name },
                                span,
                            ));
                        }
                    } else {
                        // Shouldn't happen - def_id without info
                        return Err(TypeError::new(
                            TypeErrorKind::DuplicateDefinition { name },
                            span,
                        ));
                    }
                }
                Binding::Methods(mut methods) => {
                    // Already a method family, add to it
                    methods.push(def_id);
                    self.current_scope_mut()
                        .bindings
                        .insert(name.clone(), Binding::Methods(methods));
                }
                Binding::Local { .. } => {
                    // Can't define function with same name as local
                    return Err(TypeError::new(
                        TypeErrorKind::DuplicateDefinition { name },
                        span,
                    ));
                }
            }
        } else {
            // No existing binding - create new single definition
            self.current_scope_mut()
                .bindings
                .insert(name.clone(), Binding::Def(def_id));
        }

        // Also track in globals if in root scope
        // Note: For method families, we only track the first def_id in globals
        // The method family is resolved through the scope bindings
        if self.current_scope().kind == ScopeKind::Root {
            if !self.globals.contains_key(&name) {
                self.globals.insert(name, def_id);
            }
        }

        Ok(def_id)
    }

    /// Define a type in the current scope.
    pub fn define_type(
        &mut self,
        name: String,
        def_id: DefId,
        span: Span,
    ) -> Result<(), TypeError> {
        // Check for duplicates in current scope
        if self.current_scope().type_bindings.contains_key(&name) {
            return Err(TypeError::new(
                TypeErrorKind::DuplicateDefinition { name },
                span,
            ));
        }

        self.current_scope_mut()
            .type_bindings
            .insert(name, def_id);

        Ok(())
    }

    /// Import an existing definition under a (possibly different) name.
    ///
    /// This is used for `use` statements to bring definitions from other
    /// modules into the current scope without creating new DefIds.
    pub fn import_binding(
        &mut self,
        name: String,
        def_id: DefId,
        span: Span,
    ) -> Result<(), TypeError> {
        // Check for duplicates in current scope - allow if same DefId (re-import)
        if let Some(existing) = self.current_scope().bindings.get(&name) {
            if let Binding::Def(existing_id) = existing {
                if *existing_id != def_id {
                    return Err(TypeError::new(
                        TypeErrorKind::DuplicateDefinition { name },
                        span,
                    ));
                }
                // Same DefId - already imported, nothing to do
                return Ok(());
            } else {
                // Different kind of binding, that's a real conflict
                return Err(TypeError::new(
                    TypeErrorKind::DuplicateDefinition { name },
                    span,
                ));
            }
        }

        // Add as a Def binding (it's an alias to an existing definition)
        self.current_scope_mut()
            .bindings
            .insert(name.clone(), Binding::Def(def_id));

        // Also add to globals if in root scope
        if self.current_scope().kind == ScopeKind::Root {
            self.globals.insert(name, def_id);
        }

        Ok(())
    }

    /// Import a type binding under a (possibly different) name.
    pub fn import_type_binding(
        &mut self,
        name: String,
        def_id: DefId,
        span: Span,
    ) -> Result<(), TypeError> {
        // Check for duplicates in current scope - allow if same DefId (re-import)
        if let Some(&existing_id) = self.current_scope().type_bindings.get(&name) {
            if existing_id != def_id {
                return Err(TypeError::new(
                    TypeErrorKind::DuplicateDefinition { name },
                    span,
                ));
            }
            // Same DefId - already imported, nothing to do
            return Ok(());
        }

        self.current_scope_mut()
            .type_bindings
            .insert(name, def_id);

        Ok(())
    }

    /// Define a local variable in the current scope.
    pub fn define_local(
        &mut self,
        name: String,
        ty: Type,
        mutable: bool,
        span: Span,
    ) -> Result<LocalId, TypeError> {
        let local_id = self.next_local_id();

        self.current_scope_mut().bindings.insert(
            name,
            Binding::Local {
                local_id,
                ty,
                mutable,
                span,
            },
        );

        Ok(local_id)
    }

    /// Look up a name in the current scope chain.
    pub fn lookup(&self, name: &str) -> Option<Binding> {
        let mut scope_idx = Some(self.current_scope_idx());

        while let Some(idx) = scope_idx {
            let scope = &self.scopes[idx];
            if let Some(binding) = scope.bindings.get(name) {
                return Some(binding.clone());
            }
            scope_idx = scope.parent;
        }

        None
    }

    /// Look up a type name in the current scope chain.
    pub fn lookup_type(&self, name: &str) -> Option<DefId> {
        // First check if it's a primitive type
        if PrimitiveTy::from_name(name).is_some() {
            return None; // Primitives are handled specially
        }

        let mut scope_idx = Some(self.current_scope_idx());

        while let Some(idx) = scope_idx {
            let scope = &self.scopes[idx];
            if let Some(def_id) = scope.type_bindings.get(name) {
                return Some(*def_id);
            }
            scope_idx = scope.parent;
        }

        None
    }

    /// Check if we're inside a loop.
    pub fn in_loop(&self) -> bool {
        for &idx in self.scope_stack.iter().rev() {
            if self.scopes[idx].kind == ScopeKind::Loop {
                return true;
            }
            // Stop at function boundary
            if self.scopes[idx].kind == ScopeKind::Function {
                return false;
            }
        }
        false
    }

    /// Check if we're inside a function.
    pub fn in_function(&self) -> bool {
        for &idx in self.scope_stack.iter().rev() {
            if self.scopes[idx].kind == ScopeKind::Function {
                return true;
            }
        }
        false
    }

    /// Check if we're inside a handler (effect handler body).
    pub fn in_handler(&self) -> bool {
        for &idx in self.scope_stack.iter().rev() {
            if self.scopes[idx].kind == ScopeKind::Handler {
                return true;
            }
            // Stop at function boundary
            if self.scopes[idx].kind == ScopeKind::Function {
                return false;
            }
        }
        false
    }

    /// Report an error.
    pub fn error(&mut self, error: TypeError) {
        self.errors.push(error);
    }

    /// Convert errors to diagnostics.
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.errors.into_iter().map(|e| e.to_diagnostic()).collect()
    }

    /// Collect all visible value names in the current scope chain.
    ///
    /// This is used for generating "did you mean?" suggestions.
    pub fn collect_visible_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut scope_idx = Some(self.current_scope_idx());

        while let Some(idx) = scope_idx {
            let scope = &self.scopes[idx];
            for name in scope.bindings.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            scope_idx = scope.parent;
        }

        names
    }

    /// Collect all visible type names in the current scope chain.
    ///
    /// This is used for generating "did you mean?" suggestions for types.
    pub fn collect_visible_type_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut scope_idx = Some(self.current_scope_idx());

        while let Some(idx) = scope_idx {
            let scope = &self.scopes[idx];
            for name in scope.type_bindings.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            scope_idx = scope.parent;
        }

        // Also add primitive type names
        let primitives = [
            "i8", "i16", "i32", "i64", "i128", "isize",
            "u8", "u16", "u32", "u64", "u128", "usize",
            "f32", "f64", "bool", "char", "str", "String",
        ];
        for prim in primitives {
            if !names.contains(&prim.to_string()) {
                names.push(prim.to_string());
            }
        }

        names
    }
}

impl Scope {
    /// Check if this scope introduces a new namespace.
    pub fn is_namespace(&self) -> bool {
        matches!(
            self.kind,
            ScopeKind::Root | ScopeKind::Module | ScopeKind::Impl | ScopeKind::Trait
        )
    }
}
