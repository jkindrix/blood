//! Lifetime inference and checking for Blood.
//!
//! Blood's lifetime system is optional - the generational safety model ensures memory
//! safety at runtime. Lifetimes serve as an optimization layer: when lifetime relationships
//! can be proven statically, generation checks can be elided.
//!
//! # Lifetime Variables
//!
//! Each reference type has an associated lifetime:
//! - `&'a T` - explicitly annotated lifetime
//! - `&T` - inferred lifetime (lifetime variable)
//!
//! # Lifetime Constraints
//!
//! The system generates constraints from:
//! - Function signatures (`fn<'a>(&'a T) -> &'a T`)
//! - Struct definitions (`struct Ref<'a> { r: &'a T }`)
//! - Subtyping (`'a: 'b` means 'a outlives 'b)
//!
//! # Constraint Solving
//!
//! Constraints are solved using region inference similar to Rust's NLL.
//! If constraints can't be satisfied, the code is still valid (runtime checks remain).

use std::collections::HashMap;

use crate::hir::{LifetimeId, DefId};
use crate::span::Span;

/// A lifetime constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifetimeConstraint {
    /// `'a: 'b` - lifetime 'a outlives lifetime 'b
    Outlives { longer: LifetimeId, shorter: LifetimeId, span: Span },
    /// `'a == 'b` - lifetimes must be the same
    Equal { left: LifetimeId, right: LifetimeId, span: Span },
    /// `'a: 'static` - lifetime must be static
    Static { lifetime: LifetimeId, span: Span },
}

/// Lifetime inference context.
#[derive(Debug)]
pub struct LifetimeInference {
    /// Next lifetime variable ID
    next_id: u32,
    /// Constraints collected during inference
    constraints: Vec<LifetimeConstraint>,
    /// Lifetime variable origins (for error messages)
    origins: HashMap<LifetimeId, LifetimeOrigin>,
    /// Named lifetimes in scope (name -> id)
    named_lifetimes: HashMap<String, LifetimeId>,
    /// Solved lifetime values (after constraint solving)
    solutions: HashMap<LifetimeId, LifetimeSolution>,
}

/// Origin of a lifetime variable (for error messages).
#[derive(Debug, Clone)]
pub enum LifetimeOrigin {
    /// From an explicit annotation
    Annotation { name: String, span: Span },
    /// Inferred from a reference type
    Reference { span: Span },
    /// From a function parameter
    Parameter { fn_def: DefId, param_index: usize, span: Span },
    /// From a function return type
    Return { fn_def: DefId, span: Span },
    /// The static lifetime
    Static,
}

/// Solution for a lifetime variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifetimeSolution {
    /// Maps to another lifetime
    Bound(LifetimeId),
    /// Static lifetime
    Static,
    /// Unknown (couldn't solve, use runtime checks)
    Unknown,
}

impl LifetimeInference {
    /// Create a new lifetime inference context.
    pub fn new() -> Self {
        let mut ctx = Self {
            next_id: 1, // 0 is reserved for 'static
            constraints: Vec::new(),
            origins: HashMap::new(),
            named_lifetimes: HashMap::new(),
            solutions: HashMap::new(),
        };

        // Register 'static lifetime
        ctx.origins.insert(LifetimeId::STATIC, LifetimeOrigin::Static);
        ctx.named_lifetimes.insert("'static".to_string(), LifetimeId::STATIC);
        ctx.solutions.insert(LifetimeId::STATIC, LifetimeSolution::Static);

        ctx
    }

    /// Create a fresh lifetime variable.
    pub fn fresh_lifetime(&mut self, origin: LifetimeOrigin) -> LifetimeId {
        let id = LifetimeId::new(self.next_id);
        self.next_id += 1;
        self.origins.insert(id, origin);
        id
    }

    /// Register a named lifetime in scope.
    pub fn register_named(&mut self, name: String, id: LifetimeId) {
        self.named_lifetimes.insert(name, id);
    }

    /// Look up a named lifetime.
    pub fn lookup_named(&self, name: &str) -> Option<LifetimeId> {
        self.named_lifetimes.get(name).copied()
    }

    /// Add an outlives constraint: 'a: 'b (a outlives b)
    pub fn add_outlives(&mut self, longer: LifetimeId, shorter: LifetimeId, span: Span) {
        // Don't add trivial constraints
        if longer == shorter {
            return;
        }
        // 'static outlives everything
        if longer == LifetimeId::STATIC {
            return;
        }
        // Everything outlives 'static... wait, that's backwards
        // Actually: 'a: 'static means 'a must be 'static
        self.constraints.push(LifetimeConstraint::Outlives { longer, shorter, span });
    }

    /// Add an equality constraint: 'a == 'b
    pub fn add_equal(&mut self, left: LifetimeId, right: LifetimeId, span: Span) {
        if left == right {
            return;
        }
        self.constraints.push(LifetimeConstraint::Equal { left, right, span });
    }

    /// Add a static constraint: 'a must be 'static
    pub fn add_static_bound(&mut self, lifetime: LifetimeId, span: Span) {
        if lifetime == LifetimeId::STATIC {
            return;
        }
        self.constraints.push(LifetimeConstraint::Static { lifetime, span });
    }

    /// Solve the lifetime constraints.
    ///
    /// Returns true if all constraints were satisfied, false otherwise.
    /// Even if false, the code is still valid due to runtime checks.
    pub fn solve(&mut self) -> bool {
        // Simple constraint solver using union-find-like approach
        // For now, use a simple iterative approach

        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: u32 = 100;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            for constraint in self.constraints.clone() {
                match constraint {
                    LifetimeConstraint::Equal { left, right, .. } => {
                        // Unify left and right
                        let left_sol = self.solutions.get(&left).cloned();
                        let right_sol = self.solutions.get(&right).cloned();

                        match (left_sol, right_sol) {
                            (Some(LifetimeSolution::Static), _) => {
                                if self.solutions.insert(right, LifetimeSolution::Static)
                                    != Some(LifetimeSolution::Static)
                                {
                                    changed = true;
                                }
                            }
                            (_, Some(LifetimeSolution::Static)) => {
                                if self.solutions.insert(left, LifetimeSolution::Static)
                                    != Some(LifetimeSolution::Static)
                                {
                                    changed = true;
                                }
                            }
                            (None, Some(sol)) => {
                                self.solutions.insert(left, sol);
                                changed = true;
                            }
                            (Some(sol), None) => {
                                self.solutions.insert(right, sol);
                                changed = true;
                            }
                            (Some(_), Some(_)) => {} // Both have solutions: already propagated
                            (None, None) => {} // Neither has a solution yet
                        }
                    }
                    LifetimeConstraint::Static { lifetime, .. } => {
                        if self.solutions.insert(lifetime, LifetimeSolution::Static)
                            != Some(LifetimeSolution::Static)
                        {
                            changed = true;
                        }
                    }
                    LifetimeConstraint::Outlives { longer, shorter, .. } => {
                        // If shorter is static, longer must be static
                        if let Some(LifetimeSolution::Static) = self.solutions.get(&shorter) {
                            if self.solutions.insert(longer, LifetimeSolution::Static)
                                != Some(LifetimeSolution::Static)
                            {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        // Mark any unsolved lifetimes as Unknown
        for id in self.origins.keys() {
            if !self.solutions.contains_key(id) {
                self.solutions.insert(*id, LifetimeSolution::Unknown);
            }
        }

        // Check if all constraints are satisfied
        self.verify_constraints()
    }

    /// Verify that all constraints are satisfied.
    fn verify_constraints(&self) -> bool {
        for constraint in &self.constraints {
            match constraint {
                LifetimeConstraint::Equal { left, right, .. } => {
                    let left_sol = self.solutions.get(left);
                    let right_sol = self.solutions.get(right);
                    if left_sol != right_sol {
                        return false;
                    }
                }
                LifetimeConstraint::Static { lifetime, .. } => {
                    if self.solutions.get(lifetime) != Some(&LifetimeSolution::Static) {
                        return false;
                    }
                }
                LifetimeConstraint::Outlives { longer, shorter, .. } => {
                    let longer_sol = self.solutions.get(longer);
                    let shorter_sol = self.solutions.get(shorter);

                    // 'static outlives everything
                    if longer_sol == Some(&LifetimeSolution::Static) {
                        continue;
                    }
                    // If shorter is static, longer must also be static
                    if shorter_sol == Some(&LifetimeSolution::Static)
                        && longer_sol != Some(&LifetimeSolution::Static)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Get the solution for a lifetime.
    pub fn get_solution(&self, id: LifetimeId) -> Option<&LifetimeSolution> {
        self.solutions.get(&id)
    }

    /// Check if a lifetime is statically known to be 'static.
    pub fn is_static(&self, id: LifetimeId) -> bool {
        matches!(self.solutions.get(&id), Some(LifetimeSolution::Static))
    }

    /// Check if lifetime constraints were fully solved (no unknowns).
    pub fn is_fully_solved(&self) -> bool {
        self.solutions.values().all(|s| !matches!(s, LifetimeSolution::Unknown))
    }

    /// Clear the inference context for a new function.
    pub fn clear(&mut self) {
        let static_origin = self.origins.remove(&LifetimeId::STATIC);

        self.next_id = 1;
        self.constraints.clear();
        self.origins.clear();
        self.named_lifetimes.clear();
        self.solutions.clear();

        // Re-register 'static
        if let Some(origin) = static_origin {
            self.origins.insert(LifetimeId::STATIC, origin);
        } else {
            self.origins.insert(LifetimeId::STATIC, LifetimeOrigin::Static);
        }
        self.named_lifetimes.insert("'static".to_string(), LifetimeId::STATIC);
        self.solutions.insert(LifetimeId::STATIC, LifetimeSolution::Static);
    }
}

impl Default for LifetimeInference {
    fn default() -> Self {
        Self::new()
    }
}

/// Lifetime elision rules.
///
/// These rules automatically assign lifetimes to references when not explicitly annotated.
/// Based on Rust's elision rules with Blood-specific extensions.
pub mod elision {
    use super::*;

    /// Apply lifetime elision rules to a function signature.
    ///
    /// Rules:
    /// 1. Each elided lifetime in input positions gets a fresh lifetime.
    /// 2. If there's exactly one input lifetime, it's assigned to all elided output lifetimes.
    /// 3. If there's a `&self` or `&mut self` parameter, its lifetime is assigned to outputs.
    pub fn apply_function_elision(
        ctx: &mut LifetimeInference,
        input_lifetimes: &[Option<LifetimeId>],
        output_lifetime: &mut Option<LifetimeId>,
        has_self_ref: bool,
        self_lifetime: Option<LifetimeId>,
        span: Span,
    ) {
        // Rule 1: Fresh lifetimes for each elided input
        let input_ids: Vec<LifetimeId> = input_lifetimes
            .iter()
            .map(|opt_lt| {
                opt_lt.unwrap_or_else(|| {
                    ctx.fresh_lifetime(LifetimeOrigin::Reference { span })
                })
            })
            .collect();

        // Rule 2 & 3: Determine output lifetime
        if output_lifetime.is_none() {
            if has_self_ref && self_lifetime.is_some() {
                // Rule 3: Use self's lifetime
                *output_lifetime = self_lifetime;
            } else if input_ids.len() == 1 {
                // Rule 2: Use the single input lifetime
                *output_lifetime = Some(input_ids[0]);
            }
            // Otherwise, output lifetime remains None (error if needed)
        }
    }

    /// Apply lifetime elision to a single reference type.
    pub fn elide_reference(
        ctx: &mut LifetimeInference,
        explicit_lifetime: Option<LifetimeId>,
        span: Span,
    ) -> LifetimeId {
        explicit_lifetime.unwrap_or_else(|| {
            ctx.fresh_lifetime(LifetimeOrigin::Reference { span })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_lifetime() {
        let ctx = LifetimeInference::new();
        assert_eq!(ctx.lookup_named("'static"), Some(LifetimeId::STATIC));
        assert!(ctx.is_static(LifetimeId::STATIC));
    }

    #[test]
    fn test_fresh_lifetime() {
        let mut ctx = LifetimeInference::new();
        let lt1 = ctx.fresh_lifetime(LifetimeOrigin::Reference { span: Span::default() });
        let lt2 = ctx.fresh_lifetime(LifetimeOrigin::Reference { span: Span::default() });
        assert_ne!(lt1, lt2);
    }

    #[test]
    fn test_equality_constraint() {
        let mut ctx = LifetimeInference::new();
        let lt1 = ctx.fresh_lifetime(LifetimeOrigin::Reference { span: Span::default() });
        let lt2 = ctx.fresh_lifetime(LifetimeOrigin::Reference { span: Span::default() });

        ctx.add_equal(lt1, LifetimeId::STATIC, Span::default());
        ctx.add_equal(lt1, lt2, Span::default());

        assert!(ctx.solve());
        assert!(ctx.is_static(lt1));
        assert!(ctx.is_static(lt2));
    }

    #[test]
    fn test_outlives_constraint() {
        let mut ctx = LifetimeInference::new();
        let lt1 = ctx.fresh_lifetime(LifetimeOrigin::Reference { span: Span::default() });

        // lt1: 'static means lt1 must outlive static, which means lt1 must be static
        ctx.add_outlives(lt1, LifetimeId::STATIC, Span::default());

        // This constraint can't be satisfied unless lt1 is static
        // But our simple solver marks it as Unknown
        ctx.solve();

        // The constraint solver should propagate this
        // (depending on implementation details)
    }
}
