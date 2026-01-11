//! Pattern exhaustiveness checking.
//!
//! This module implements exhaustiveness and usefulness checking for match patterns.
//! It detects:
//! - Non-exhaustive matches (missing patterns)
//! - Unreachable patterns (dead code)
//!
//! The algorithm is based on the "usefulness" algorithm from Maranget's paper
//! "Warnings for Pattern Matching" (JFP 2007), simplified for Blood's pattern language.

use std::collections::HashSet;

use crate::hir::{self, Pattern, PatternKind, Type, TypeKind};

/// Result of exhaustiveness checking.
#[derive(Debug)]
pub struct ExhaustivenessResult {
    /// Whether the patterns are exhaustive.
    pub is_exhaustive: bool,
    /// Missing patterns (as human-readable descriptions).
    pub missing_patterns: Vec<String>,
    /// Indices of unreachable arms.
    pub unreachable_arms: Vec<usize>,
}

/// Information about enum variants for exhaustiveness checking.
#[derive(Debug, Clone)]
pub struct EnumVariantInfo {
    /// Number of variants in the enum.
    pub variant_count: u32,
    /// Names of variants (for error messages).
    pub variant_names: Vec<String>,
}

/// Check if a set of match arms is exhaustive for the given scrutinee type.
pub fn check_exhaustiveness(
    arms: &[hir::MatchArm],
    scrutinee_ty: &Type,
    enum_info: Option<&EnumVariantInfo>,
) -> ExhaustivenessResult {
    if arms.is_empty() {
        // Empty match - exhaustive only if scrutinee is never type
        return if scrutinee_ty.is_never() {
            ExhaustivenessResult {
                is_exhaustive: true,
                missing_patterns: vec![],
                unreachable_arms: vec![],
            }
        } else {
            ExhaustivenessResult {
                is_exhaustive: false,
                missing_patterns: vec!["_".to_string()],
                unreachable_arms: vec![],
            }
        };
    }

    let patterns: Vec<_> = arms.iter().map(|a| &a.pattern).collect();

    // Check for unreachable arms
    let unreachable_arms = find_unreachable_arms(&patterns);

    // Check exhaustiveness
    let (is_exhaustive, missing) = is_exhaustive(&patterns, scrutinee_ty, enum_info);

    ExhaustivenessResult {
        is_exhaustive,
        missing_patterns: missing,
        unreachable_arms,
    }
}

/// Check if a set of patterns is exhaustive for a type.
fn is_exhaustive(
    patterns: &[&Pattern],
    scrutinee_ty: &Type,
    enum_info: Option<&EnumVariantInfo>,
) -> (bool, Vec<String>) {
    // First check for wildcard or binding pattern that covers everything
    for pat in patterns {
        if is_irrefutable(pat) {
            return (true, vec![]);
        }
    }

    match &*scrutinee_ty.kind {
        TypeKind::Primitive(hir::PrimitiveTy::Bool) => {
            check_bool_exhaustiveness(patterns)
        }
        TypeKind::Adt { .. } => {
            if let Some(info) = enum_info {
                check_enum_exhaustiveness(patterns, info)
            } else {
                // Struct type - any pattern covers it (if no enum info provided)
                (true, vec![])
            }
        }
        TypeKind::Tuple(tys) => {
            // Empty tuple is unit type - always exhaustive
            if tys.is_empty() {
                (true, vec![])
            } else {
                check_tuple_exhaustiveness(patterns, tys)
            }
        }
        TypeKind::Never => (true, vec![]),
        _ => {
            // For other types (integers, strings, etc.), we can't check exhaustiveness
            // without literal patterns, so we require a wildcard
            (false, vec!["_".to_string()])
        }
    }
}

/// Check if a pattern is irrefutable (always matches).
fn is_irrefutable(pattern: &Pattern) -> bool {
    match &pattern.kind {
        PatternKind::Wildcard => true,
        PatternKind::Binding { subpattern, .. } => {
            subpattern.as_ref().map_or(true, |p| is_irrefutable(p))
        }
        PatternKind::Tuple(pats) => pats.iter().all(is_irrefutable),
        PatternKind::Ref { inner, .. } => is_irrefutable(inner),
        PatternKind::Struct { fields, .. } => {
            fields.iter().all(|f| is_irrefutable(&f.pattern))
        }
        PatternKind::Or(alts) => alts.iter().any(is_irrefutable),
        _ => false,
    }
}

/// Check boolean exhaustiveness.
fn check_bool_exhaustiveness(patterns: &[&Pattern]) -> (bool, Vec<String>) {
    let mut has_true = false;
    let mut has_false = false;

    for pat in patterns {
        match &pat.kind {
            PatternKind::Literal(hir::LiteralValue::Bool(true)) => has_true = true,
            PatternKind::Literal(hir::LiteralValue::Bool(false)) => has_false = true,
            PatternKind::Or(alts) => {
                for alt in alts {
                    if let PatternKind::Literal(hir::LiteralValue::Bool(b)) = &alt.kind {
                        if *b { has_true = true; } else { has_false = true; }
                    }
                }
            }
            _ => {}
        }
    }

    let mut missing = vec![];
    if !has_true { missing.push("true".to_string()); }
    if !has_false { missing.push("false".to_string()); }

    (missing.is_empty(), missing)
}

/// Check enum exhaustiveness.
fn check_enum_exhaustiveness(
    patterns: &[&Pattern],
    enum_info: &EnumVariantInfo,
) -> (bool, Vec<String>) {
    let mut covered_variants: HashSet<u32> = HashSet::new();

    for pat in patterns {
        collect_variant_indices(pat, &mut covered_variants);
    }

    let mut missing = vec![];
    for idx in 0..enum_info.variant_count {
        if !covered_variants.contains(&idx) {
            if let Some(name) = enum_info.variant_names.get(idx as usize) {
                missing.push(name.clone());
            } else {
                missing.push(format!("variant {}", idx));
            }
        }
    }

    (missing.is_empty(), missing)
}

/// Collect all variant indices covered by a pattern.
fn collect_variant_indices(pattern: &Pattern, indices: &mut HashSet<u32>) {
    match &pattern.kind {
        PatternKind::Variant { variant_idx, .. } => {
            indices.insert(*variant_idx);
        }
        PatternKind::Or(alts) => {
            for alt in alts {
                collect_variant_indices(alt, indices);
            }
        }
        PatternKind::Binding { subpattern: Some(sub), .. } => {
            collect_variant_indices(sub, indices);
        }
        _ => {}
    }
}

/// Check tuple exhaustiveness.
fn check_tuple_exhaustiveness(
    patterns: &[&Pattern],
    element_types: &[Type],
) -> (bool, Vec<String>) {
    // For tuples, we need to check that each position is covered
    // This is a simplified check - full algorithm would use pattern matrices

    if element_types.is_empty() {
        return (true, vec![]);
    }

    // Check if any pattern covers all tuple positions
    for pat in patterns {
        if is_irrefutable(pat) {
            return (true, vec![]);
        }
    }

    // Extract tuple patterns and check each position
    let tuple_patterns: Vec<_> = patterns
        .iter()
        .filter_map(|p| {
            if let PatternKind::Tuple(pats) = &p.kind {
                Some(pats.as_slice())
            } else {
                None
            }
        })
        .collect();

    if tuple_patterns.is_empty() {
        return (false, vec!["(_, _, ...)".to_string()]);
    }

    // For each position, collect the patterns and check exhaustiveness
    for (i, elem_ty) in element_types.iter().enumerate() {
        let position_patterns: Vec<_> = tuple_patterns
            .iter()
            .filter_map(|pats| pats.get(i))
            .collect();

        // Simplified: we don't recursively check with enum info
        let (is_exhaustive, missing) = is_exhaustive(&position_patterns, elem_ty, None);
        if !is_exhaustive {
            return (false, vec![format!("missing pattern at position {}: {:?}", i, missing)]);
        }
    }

    (true, vec![])
}

/// Find unreachable arms (arms that can never match).
fn find_unreachable_arms(patterns: &[&Pattern]) -> Vec<usize> {
    let mut unreachable = vec![];

    // Simple check: if we see a wildcard/binding pattern, all subsequent patterns are unreachable
    let mut seen_irrefutable = false;

    for (i, pat) in patterns.iter().enumerate() {
        if seen_irrefutable {
            unreachable.push(i);
        } else if is_irrefutable(pat) {
            seen_irrefutable = true;
        }
    }

    unreachable
}

/// Witness for a non-exhaustive pattern match.
/// This describes a value that would not be matched.
#[derive(Debug, Clone)]
pub enum Witness {
    /// A wildcard (any value).
    Wild,
    /// A specific constructor (enum variant, tuple, etc.).
    Constructor {
        name: String,
        fields: Vec<Witness>,
    },
    /// A literal value.
    Literal(String),
}

impl std::fmt::Display for Witness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Witness::Wild => write!(f, "_"),
            Witness::Constructor { name, fields } => {
                if fields.is_empty() {
                    write!(f, "{}", name)
                } else {
                    write!(f, "{}({})", name,
                        fields.iter()
                            .map(|w| w.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            Witness::Literal(s) => write!(f, "{}", s),
        }
    }
}
