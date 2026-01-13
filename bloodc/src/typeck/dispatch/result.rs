//! Dispatch result types and errors.

use std::collections::HashMap;

use crate::hir::{DefId, Type};
use super::types::MethodCandidate;

/// Result of dispatch resolution.
#[derive(Debug)]
pub enum DispatchResult {
    /// A unique method was found.
    Resolved(MethodCandidate),
    /// No applicable methods found.
    NoMatch(NoMatchError),
    /// Multiple methods are ambiguous.
    Ambiguous(AmbiguityError),
}

/// Error when no method matches the arguments.
#[derive(Debug)]
pub struct NoMatchError {
    /// The method name that was called.
    pub method_name: String,
    /// The argument types provided.
    pub arg_types: Vec<Type>,
    /// All candidates that were considered.
    pub candidates: Vec<MethodCandidate>,
}

/// Error when multiple methods are ambiguous.
#[derive(Debug)]
pub struct AmbiguityError {
    /// The method name that was called.
    pub method_name: String,
    /// The argument types provided.
    pub arg_types: Vec<Type>,
    /// The ambiguous candidates (all maximal).
    pub candidates: Vec<MethodCandidate>,
}

impl AmbiguityError {
    /// Check if this is a diamond conflict (candidates from different traits).
    pub fn is_diamond_conflict(&self) -> bool {
        let trait_ids: Vec<_> = self.candidates
            .iter()
            .filter_map(|c| c.trait_id)
            .collect();

        // Diamond conflict if there are multiple distinct traits involved
        if trait_ids.len() < 2 {
            return false;
        }

        // Check if we have at least 2 different trait IDs
        let first = trait_ids[0];
        trait_ids.iter().any(|&id| id != first)
    }

    /// Get the conflicting trait IDs for diamond resolution.
    pub fn conflicting_trait_ids(&self) -> Vec<DefId> {
        let mut trait_ids: Vec<_> = self.candidates
            .iter()
            .filter_map(|c| c.trait_id)
            .collect();
        trait_ids.sort_by_key(|id| id.index);
        trait_ids.dedup();
        trait_ids
    }

    /// Generate a suggestion message for diamond resolution.
    pub fn diamond_suggestion(&self, trait_names: &HashMap<DefId, String>) -> String {
        let conflicting = self.conflicting_trait_ids();
        let trait_names_str: Vec<_> = conflicting
            .iter()
            .filter_map(|id| trait_names.get(id))
            .cloned()
            .collect();

        if trait_names_str.is_empty() {
            format!(
                "Use qualified syntax to resolve ambiguity: <Type as Trait>::{}()",
                self.method_name
            )
        } else {
            format!(
                "Use qualified syntax: <Type as {}>::{}() or <Type as {}>::{}()",
                trait_names_str.first().unwrap_or(&"Trait1".to_string()),
                self.method_name,
                trait_names_str.get(1).unwrap_or(&"Trait2".to_string()),
                self.method_name
            )
        }
    }
}

/// A function that checks if a type implements a trait.
pub type TraitChecker = dyn Fn(&Type, DefId) -> bool;
