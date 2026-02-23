//! Method family collection for multiple dispatch.
//!
//! A method family is a set of methods sharing the same name. Each method
//! provides a different implementation based on argument types. This module
//! collects and organizes method families for dispatch resolution.
//!
//! # Example
//!
//! ```blood
//! // Method family: `add`
//! fn add(x: i32, y: i32) -> i32 { x + y }
//! fn add(x: f64, y: f64) -> f64 { x + y }
//! fn add(x: String, y: String) -> String { x.concat(y) }
//! ```
//!
//! See DISPATCH.md Section 2.2 for full specification.

use std::collections::HashMap;

use crate::hir::{DefId, Type, FnSig, TyVarId};
use super::dispatch::{MethodCandidate, TypeParam, Constraint, EffectRow};

/// A method family is a collection of methods with the same name.
#[derive(Debug, Clone, Default)]
pub struct MethodFamily {
    /// The method name.
    pub name: String,
    /// All methods in this family.
    pub methods: Vec<MethodCandidate>,
}

impl MethodFamily {
    /// Create a new empty method family.
    pub fn new(name: String) -> Self {
        Self {
            name,
            methods: Vec::new(),
        }
    }

    /// Add a method to this family.
    pub fn add_method(&mut self, method: MethodCandidate) {
        self.methods.push(method);
    }

    /// Get all methods with a specific arity.
    pub fn methods_with_arity(&self, arity: usize) -> Vec<&MethodCandidate> {
        self.methods
            .iter()
            .filter(|m| m.param_types.len() == arity)
            .collect()
    }

    /// Check if this family is empty.
    pub fn is_empty(&self) -> bool {
        self.methods.is_empty()
    }

    /// Get the number of methods in this family.
    pub fn len(&self) -> usize {
        self.methods.len()
    }
}

/// Registry of all method families in a module/crate.
#[derive(Debug, Default)]
pub struct MethodRegistry {
    /// Method families indexed by name.
    families: HashMap<String, MethodFamily>,
}

impl MethodRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            families: HashMap::new(),
        }
    }

    /// Register a method, adding it to the appropriate family.
    pub fn register_method(
        &mut self,
        def_id: DefId,
        name: String,
        sig: &FnSig,
        type_params: Vec<TypeParam>,
        effects: Option<EffectRow>,
    ) {
        self.register_method_with_trait(def_id, name, sig, type_params, effects, None);
    }

    /// Register a method with an associated trait (for diamond resolution).
    pub fn register_method_with_trait(
        &mut self,
        def_id: DefId,
        name: String,
        sig: &FnSig,
        type_params: Vec<TypeParam>,
        effects: Option<EffectRow>,
        trait_id: Option<DefId>,
    ) {
        let candidate = MethodCandidate {
            def_id,
            name: name.clone(),
            param_types: sig.inputs.clone(),
            return_type: sig.output.clone(),
            type_params,
            effects,
            trait_id,
        };

        self.families
            .entry(name.clone())
            .or_insert_with(|| MethodFamily::new(name))
            .add_method(candidate);
    }

    /// Register a method from just types (convenience method).
    pub fn register_method_simple(
        &mut self,
        def_id: DefId,
        name: String,
        param_types: Vec<Type>,
        return_type: Type,
    ) {
        let candidate = MethodCandidate {
            def_id,
            name: name.clone(),
            param_types,
            return_type,
            type_params: vec![],
            effects: None,
            trait_id: None,
        };

        self.families
            .entry(name.clone())
            .or_insert_with(|| MethodFamily::new(name))
            .add_method(candidate);
    }

    /// Get a method family by name.
    pub fn get_family(&self, name: &str) -> Option<&MethodFamily> {
        self.families.get(name)
    }

    /// Get all candidates for a method name.
    pub fn get_candidates(&self, name: &str) -> Vec<&MethodCandidate> {
        self.families
            .get(name)
            .map(|f| f.methods.iter().collect())
            .unwrap_or_default()
    }

    /// Check if a method family exists.
    pub fn has_family(&self, name: &str) -> bool {
        self.families.contains_key(name)
    }

    /// Get all method family names.
    pub fn family_names(&self) -> impl Iterator<Item = &str> {
        self.families.keys().map(String::as_str)
    }

    /// Get the number of method families.
    pub fn family_count(&self) -> usize {
        self.families.len()
    }

    /// Get the total number of methods across all families.
    pub fn method_count(&self) -> usize {
        self.families.values().map(|f| f.len()).sum()
    }
}

/// Builder for creating method candidates from AST.
pub struct MethodBuilder {
    /// Type parameters collected during parsing.
    type_params: Vec<TypeParam>,
    /// Constraints on type parameters.
    constraints: HashMap<String, Vec<Constraint>>,
    /// Next type parameter ID.
    next_type_param_id: u32,
}

impl MethodBuilder {
    /// Create a new method builder.
    pub fn new() -> Self {
        Self {
            type_params: Vec::new(),
            constraints: HashMap::new(),
            next_type_param_id: 0,
        }
    }

    /// Add a type parameter.
    pub fn add_type_param(&mut self, name: String) -> &mut Self {
        let id = TyVarId::new(self.next_type_param_id);
        self.next_type_param_id += 1;
        self.type_params.push(TypeParam {
            name,
            id,
            constraints: vec![],
        });
        self
    }

    /// Add a constraint to a type parameter.
    pub fn add_constraint(&mut self, param_name: &str, trait_name: String) -> &mut Self {
        self.constraints
            .entry(param_name.to_string())
            .or_default()
            .push(Constraint { trait_name });
        self
    }

    /// Build the method candidate.
    pub fn build(
        self,
        def_id: DefId,
        name: String,
        param_types: Vec<Type>,
        return_type: Type,
        effects: Option<EffectRow>,
    ) -> MethodCandidate {
        self.build_with_trait(def_id, name, param_types, return_type, effects, None)
    }

    /// Build the method candidate with an associated trait (for diamond resolution).
    pub fn build_with_trait(
        mut self,
        def_id: DefId,
        name: String,
        param_types: Vec<Type>,
        return_type: Type,
        effects: Option<EffectRow>,
        trait_id: Option<DefId>,
    ) -> MethodCandidate {
        // Apply collected constraints to type params
        for param in &mut self.type_params {
            if let Some(constraints) = self.constraints.remove(&param.name) {
                param.constraints = constraints;
            }
        }

        MethodCandidate {
            def_id,
            name,
            param_types,
            return_type,
            type_params: self.type_params,
            effects,
            trait_id,
        }
    }
}

impl Default for MethodBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Find similar method names for error suggestions.
///
/// Uses Levenshtein distance to find method names that are close to
/// the given name, useful for "did you mean?" suggestions.
pub fn find_similar_methods<'a>(
    registry: &'a MethodRegistry,
    name: &str,
    max_distance: usize,
) -> Vec<&'a str> {
    let mut similar = Vec::new();

    for family_name in registry.family_names() {
        let distance = levenshtein_distance(name, family_name);
        if distance <= max_distance && distance > 0 {
            similar.push(family_name);
        }
    }

    // Sort by distance (closest first)
    similar.sort_by_key(|s| levenshtein_distance(name, s));

    similar
}

/// Calculate Levenshtein distance between two strings.
#[allow(clippy::needless_range_loop)]
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            matrix[i][j] = std::cmp::min(
                std::cmp::min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1),
                matrix[i - 1][j - 1] + cost,
            );
        }
    }

    matrix[a_len][b_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_method_family() {
        let mut family = MethodFamily::new("add".to_string());
        assert!(family.is_empty());

        family.add_method(MethodCandidate {
            def_id: DefId::new(1),
            name: "add".to_string(),
            param_types: vec![Type::i32(), Type::i32()],
            return_type: Type::i32(),
            type_params: vec![],
            effects: None,
            trait_id: None,
        });

        assert_eq!(family.len(), 1);
        assert_eq!(family.methods_with_arity(2).len(), 1);
        assert_eq!(family.methods_with_arity(1).len(), 0);
    }

    #[test]
    fn test_method_registry() {
        let mut registry = MethodRegistry::new();

        registry.register_method_simple(
            DefId::new(1),
            "add".to_string(),
            vec![Type::i32(), Type::i32()],
            Type::i32(),
        );

        registry.register_method_simple(
            DefId::new(2),
            "add".to_string(),
            vec![Type::i64(), Type::i64()],
            Type::i64(),
        );

        assert!(registry.has_family("add"));
        assert_eq!(registry.get_candidates("add").len(), 2);
        assert!(!registry.has_family("subtract"));
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
    }

    #[test]
    fn test_find_similar() {
        let mut registry = MethodRegistry::new();
        registry.register_method_simple(
            DefId::new(1),
            "println".to_string(),
            vec![Type::str()],
            Type::unit(),
        );
        registry.register_method_simple(
            DefId::new(2),
            "print".to_string(),
            vec![Type::str()],
            Type::unit(),
        );

        let similar = find_similar_methods(&registry, "pront", 2);
        assert!(similar.contains(&"print"));
    }
}
