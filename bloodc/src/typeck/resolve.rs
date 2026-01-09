//! Name resolution for Blood.
//!
//! This module resolves identifiers to their definitions, building
//! a scope hierarchy and populating the DefId map.

use std::collections::HashMap;

use crate::ast;
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
    fn current_scope_mut(&mut self) -> &mut Scope {
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

    /// Get symbol name from source.
    ///
    /// Note: This is a placeholder. In the real implementation,
    /// symbol resolution would use the string interner from the parser.
    pub fn get_symbol_name(&self, _symbol: ast::Symbol) -> &str {
        // Symbols are interned strings - we need to look them up
        // For now, this is a placeholder - actual implementation depends on interner
        // In the real implementation, we'd use the string interner
        ""
    }

    /// Report an error.
    pub fn error(&mut self, error: TypeError) {
        self.errors.push(error);
    }

    /// Convert errors to diagnostics.
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.errors.into_iter().map(|e| e.to_diagnostic()).collect()
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
