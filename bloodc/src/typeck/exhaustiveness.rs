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
        TypeKind::Array { element, size } => {
            // For fixed-size arrays, check if patterns cover all elements
            check_array_exhaustiveness(patterns, element, *size)
        }
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
        // Slice is irrefutable only if it has a rest pattern and all subpatterns are irrefutable
        PatternKind::Slice { prefix, slice, suffix } => {
            slice.is_some()
                && prefix.iter().all(is_irrefutable)
                && suffix.iter().all(is_irrefutable)
                && slice.as_ref().map_or(true, |s| is_irrefutable(s))
        }
        // These patterns are always refutable (match specific values)
        PatternKind::Literal(_) => false,
        PatternKind::Variant { .. } => false,
        PatternKind::Range { .. } => false,
    }
}

/// Check boolean exhaustiveness.
fn check_bool_exhaustiveness(patterns: &[&Pattern]) -> (bool, Vec<String>) {
    let mut has_true = false;
    let mut has_false = false;

    for pat in patterns {
        match &pat.kind {
            // Wildcard covers all values
            PatternKind::Wildcard => {
                has_true = true;
                has_false = true;
            }
            // Binding without subpattern covers all values
            PatternKind::Binding { subpattern: None, .. } => {
                has_true = true;
                has_false = true;
            }
            // Binding with subpattern - check the subpattern
            PatternKind::Binding { subpattern: Some(sub), .. } => {
                if let PatternKind::Literal(hir::LiteralValue::Bool(b)) = &sub.kind {
                    if *b { has_true = true; } else { has_false = true; }
                }
            }
            PatternKind::Literal(hir::LiteralValue::Bool(true)) => has_true = true,
            PatternKind::Literal(hir::LiteralValue::Bool(false)) => has_false = true,
            PatternKind::Or(alts) => {
                for alt in alts {
                    match &alt.kind {
                        PatternKind::Wildcard => {
                            has_true = true;
                            has_false = true;
                        }
                        PatternKind::Binding { subpattern: None, .. } => {
                            has_true = true;
                            has_false = true;
                        }
                        PatternKind::Literal(hir::LiteralValue::Bool(b)) => {
                            if *b { has_true = true; } else { has_false = true; }
                        }
                        // Other patterns (Range, etc.) don't contribute to bool exhaustiveness
                        _ => {}
                    }
                }
            }
            // Other patterns (Literal non-bool, Struct, Tuple, etc.) don't match bool
            PatternKind::Literal(_)
            | PatternKind::Variant { .. }
            | PatternKind::Struct { .. }
            | PatternKind::Tuple(_)
            | PatternKind::Slice { .. }
            | PatternKind::Ref { .. }
            | PatternKind::Range { .. } => {}
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
        collect_variant_indices(pat, &mut covered_variants, enum_info.variant_count);
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
///
/// `variant_count` is the total number of variants in the enum, used when
/// a wildcard pattern is encountered to mark all variants as covered.
fn collect_variant_indices(pattern: &Pattern, indices: &mut HashSet<u32>, variant_count: u32) {
    match &pattern.kind {
        // Wildcard covers all variants
        PatternKind::Wildcard => {
            for idx in 0..variant_count {
                indices.insert(idx);
            }
        }
        // Binding without subpattern covers all variants
        PatternKind::Binding { subpattern: None, .. } => {
            for idx in 0..variant_count {
                indices.insert(idx);
            }
        }
        // Binding with subpattern - check the subpattern
        PatternKind::Binding { subpattern: Some(sub), .. } => {
            collect_variant_indices(sub, indices, variant_count);
        }
        PatternKind::Variant { variant_idx, .. } => {
            indices.insert(*variant_idx);
        }
        PatternKind::Or(alts) => {
            for alt in alts {
                collect_variant_indices(alt, indices, variant_count);
            }
        }
        // Other patterns don't match enum variants
        PatternKind::Literal(_)
        | PatternKind::Struct { .. }
        | PatternKind::Tuple(_)
        | PatternKind::Slice { .. }
        | PatternKind::Ref { .. }
        | PatternKind::Range { .. } => {}
    }
}

/// Check tuple exhaustiveness.
///
/// ## Algorithm
///
/// For tuple exhaustiveness, we check that every position is covered:
///
/// 1. **Quick check**: If any pattern is irrefutable (wildcard, binding), the tuple is covered
/// 2. **Position extraction**: Extract all tuple patterns and project to each position
/// 3. **Per-position check**: For each position i, collect patterns at position i and check
///    exhaustiveness against element_types[i]
///
/// ## Example
///
/// For `(bool, i32)` with patterns `[(true, _), (false, _)]`:
/// - Position 0: [true, false] against bool → exhaustive (both values covered)
/// - Position 1: [_, _] against i32 → exhaustive (wildcards cover all)
/// - Result: exhaustive
///
/// ## Limitation
///
/// This is a simplified algorithm that doesn't handle interactions between positions.
/// Full Maranget algorithm would use pattern matrices for cross-position analysis.
/// Example not caught: `[(true, 0), (false, 1)]` is NOT exhaustive for (bool, i32),
/// but this algorithm reports it as exhaustive because each position is individually covered.
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

/// Check array exhaustiveness.
///
/// ## Algorithm
///
/// For fixed-size arrays `[T; N]`, check if slice patterns cover all elements:
///
/// 1. **Quick check**: If any pattern is irrefutable (wildcard, binding), array is covered
/// 2. **Slice pattern extraction**: Extract patterns of form `[prefix.., rest, ..suffix]`
/// 3. **Length check**: A slice pattern matches if `prefix.len() + suffix.len() <= N`
///    - With rest (`..`): Can absorb N - prefix.len() - suffix.len() elements
///    - Without rest: Must match exactly if total length equals N
/// 4. **Element coverage**: If pattern can match, check that all element patterns are irrefutable
///
/// ## Example
///
/// For `[i32; 3]` with pattern `[a, .., b]`:
/// - prefix.len() = 1 (`a`)
/// - suffix.len() = 1 (`b`)
/// - rest absorbs 3 - 1 - 1 = 1 element
/// - Pattern covers array if `a` and `b` are irrefutable
///
/// ## Limitation
///
/// Like tuple checking, this is simplified. Doesn't handle patterns like
/// `[[1, ..], [.., 2]]` which together might cover all length-2 arrays.
fn check_array_exhaustiveness(
    patterns: &[&Pattern],
    element_type: &Type,
    size: u64,
) -> (bool, Vec<String>) {
    // For arrays, we check if the patterns cover all elements
    // An array pattern [a, b, c] matching [T; 3] is exhaustive if all element
    // patterns are irrefutable (wildcards or bindings)

    // First check for wildcard/binding that covers the whole array
    for pat in patterns {
        if is_irrefutable(pat) {
            return (true, vec![]);
        }
    }

    // Extract slice patterns that could match this array
    let slice_patterns: Vec<_> = patterns
        .iter()
        .filter_map(|p| {
            if let PatternKind::Slice { prefix, slice, suffix } = &p.kind {
                Some((prefix.as_slice(), slice.as_ref(), suffix.as_slice()))
            } else {
                None
            }
        })
        .collect();

    if slice_patterns.is_empty() {
        // No slice patterns at all - not exhaustive unless there's a wildcard (checked above)
        return (false, vec!["[_, _, ...]".to_string()]);
    }

    // For each slice pattern, check if it can cover the entire array
    for (prefix, slice, suffix) in &slice_patterns {
        // Pattern [p0, p1, .., sN-1, sN] matching [T; N]
        let pattern_min_len = prefix.len() + suffix.len();

        // Check if this pattern can match an array of size `size`
        if slice.is_some() {
            // Has a rest pattern (..) - can match any array with at least pattern_min_len elements
            if pattern_min_len <= size as usize {
                // Check if all subpatterns are irrefutable
                let all_irrefutable = prefix.iter().all(is_irrefutable)
                    && slice.as_ref().map_or(true, |p| is_irrefutable(p))
                    && suffix.iter().all(is_irrefutable);
                if all_irrefutable {
                    return (true, vec![]);
                }
            }
        } else {
            // No rest pattern - must match exact size
            if pattern_min_len == size as usize {
                // Check if all element patterns are irrefutable
                let all_irrefutable = prefix.iter().all(is_irrefutable);
                if all_irrefutable {
                    return (true, vec![]);
                }
            }
        }
    }

    // Check element-wise exhaustiveness for patterns that match the exact size
    for (prefix, slice, _suffix) in &slice_patterns {
        if slice.is_none() && prefix.len() == size as usize {
            // This pattern has exact size match - check each position
            let mut all_positions_exhaustive = true;
            for pat in prefix.iter() {
                let (is_exhaustive, _) = is_exhaustive(&[pat], element_type, None);
                if !is_exhaustive {
                    all_positions_exhaustive = false;
                    break;
                }
            }
            if all_positions_exhaustive {
                return (true, vec![]);
            }
        }
    }

    // Not exhaustive - suggest a wildcard pattern
    (false, vec!["_".to_string()])
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
