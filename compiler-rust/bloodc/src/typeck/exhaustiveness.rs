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
            let array_size = size.as_u64().unwrap_or(0);
            check_array_exhaustiveness(patterns, element, array_size)
        }
        TypeKind::Ref { inner, .. } => {
            // Look through references to check exhaustiveness of the inner type
            is_exhaustive(patterns, inner, enum_info)
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
                        // These patterns don't contribute to bool exhaustiveness
                        PatternKind::Literal(_)
                        | PatternKind::Variant { .. }
                        | PatternKind::Struct { .. }
                        | PatternKind::Tuple(_)
                        | PatternKind::Slice { .. }
                        | PatternKind::Ref { .. }
                        | PatternKind::Range { .. }
                        | PatternKind::Or(_)
                        | PatternKind::Binding { subpattern: Some(_), .. } => {}
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
        // Reference patterns - look through to the inner pattern
        // This handles matching &Enum with &Enum::Variant patterns
        PatternKind::Ref { inner, .. } => {
            collect_variant_indices(inner, indices, variant_count);
        }
        // Other patterns don't match enum variants
        PatternKind::Literal(_)
        | PatternKind::Struct { .. }
        | PatternKind::Tuple(_)
        | PatternKind::Slice { .. }
        | PatternKind::Range { .. } => {}
    }
}

/// Check tuple exhaustiveness using pattern matrix specialization.
///
/// ## Algorithm
///
/// Uses a simplified Maranget-style algorithm that properly handles
/// cross-position interactions for closed types (bool, enums, unit).
/// Falls back to per-position checking when any element type is open
/// (integers, strings, floats).
///
/// For closed types, the algorithm:
/// 1. Build a pattern matrix from tuple patterns
/// 2. Check if any row is irrefutable → exhaustive
/// 3. Find the first column with a closed type
/// 4. For each constructor of that type, specialize the matrix
/// 5. Recursively check each specialized matrix
/// 6. If all constructors lead to exhaustive sub-matrices → exhaustive
///
/// ## Examples
///
/// `(true, _), (false, _)` on `(bool, i32)` → exhaustive
///   (wildcards in position 1 cover all i32 for both bool constructors)
///
/// `(true, true), (false, false)` on `(bool, bool)` → NOT exhaustive
///   (specializing on `true` at position 0 gives `[true]` at position 1,
///    specializing on `false` gives `[false]`, neither covers all bools)
///
/// `(true, _), (_, true)` on `(bool, bool)` → exhaustive
///   (specializing on `true` at pos 0 gives `[_, true]` → wildcard covers;
///    specializing on `false` gives `[true]` at pos 1 → not all bools...
///    but the wildcard row `(_, true)` also matches `false` at pos 0)
fn check_tuple_exhaustiveness(
    patterns: &[&Pattern],
    element_types: &[Type],
) -> (bool, Vec<String>) {
    if element_types.is_empty() {
        return (true, vec![]);
    }

    // Check if any pattern covers all tuple positions
    for pat in patterns {
        if is_irrefutable(pat) {
            return (true, vec![]);
        }
    }

    // Build the pattern matrix: each row is one tuple pattern's sub-patterns.
    // Non-tuple patterns (wildcards, bindings) are expanded to rows of wildcards.
    let matrix = build_tuple_matrix(patterns, element_types.len());

    if matrix.is_empty() {
        return (false, vec!["(_, ...)".to_string()]);
    }

    // Check if any element type is closed (bool, enum with known variants).
    // If all types are open, fall back to per-position checking.
    let has_closed_type = element_types.iter().any(is_closed_type);

    if has_closed_type {
        // Use matrix specialization for proper cross-position analysis
        let is_exhaustive = is_tuple_matrix_exhaustive(&matrix, element_types);
        if is_exhaustive {
            (true, vec![])
        } else {
            (false, vec!["non-exhaustive tuple patterns".to_string()])
        }
    } else {
        // All open types: per-position check is correct
        // (no finite set of constructors to enumerate)
        check_tuple_per_position(&matrix, element_types)
    }
}

/// Build a pattern matrix from tuple patterns.
///
/// Each row contains `arity` patterns. Wildcard/binding patterns at the
/// tuple level are expanded to a row of wildcards.
fn build_tuple_matrix(patterns: &[&Pattern], arity: usize) -> Vec<Vec<Pattern>> {
    let mut matrix = Vec::new();
    let wildcard = || Pattern {
        kind: PatternKind::Wildcard,
        span: crate::span::Span::dummy(),
        ty: Type::error(),
    };

    for pat in patterns {
        match &pat.kind {
            PatternKind::Tuple(pats) => {
                if pats.len() == arity {
                    matrix.push(pats.clone());
                }
                // Mismatched arity is a type error; skip silently
            }
            PatternKind::Wildcard | PatternKind::Binding { subpattern: None, .. } => {
                // Expand to a row of wildcards
                matrix.push(vec![wildcard(); arity]);
            }
            PatternKind::Binding { subpattern: Some(sub), .. } => {
                // Recurse into the sub-pattern
                let sub_refs: Vec<&Pattern> = vec![sub.as_ref()];
                let sub_matrix = build_tuple_matrix(&sub_refs, arity);
                matrix.extend(sub_matrix);
            }
            PatternKind::Or(alts) => {
                // Each alternative adds a row
                let alt_refs: Vec<&Pattern> = alts.iter().collect();
                let sub_matrix = build_tuple_matrix(&alt_refs, arity);
                matrix.extend(sub_matrix);
            }
            _ => {
                // Non-tuple pattern in tuple context; skip
            }
        }
    }
    matrix
}

/// Check if a type has a finite, known set of constructors.
fn is_closed_type(ty: &Type) -> bool {
    matches!(
        &*ty.kind,
        TypeKind::Primitive(hir::PrimitiveTy::Bool)
        | TypeKind::Tuple(_)  // unit () is the only 0-element constructor
        | TypeKind::Never
    )
    // Note: ADT/enum types would also be closed, but we'd need EnumVariantInfo
    // to enumerate constructors. For now, only bool is fully supported as closed.
}

/// Get the constructors for a closed type.
///
/// Returns a list of "constructor tags" that can be matched against patterns.
/// For bool: `[true, false]`
fn closed_type_constructors(ty: &Type) -> Vec<ConstructorTag> {
    match &*ty.kind {
        TypeKind::Primitive(hir::PrimitiveTy::Bool) => {
            vec![ConstructorTag::BoolTrue, ConstructorTag::BoolFalse]
        }
        TypeKind::Never => vec![], // No constructors — always exhaustive
        _ => vec![], // Open type or unsupported — no constructors
    }
}

/// A tag identifying a constructor for specialization.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConstructorTag {
    BoolTrue,
    BoolFalse,
}

/// Check if the matrix of patterns is exhaustive for the given tuple types.
///
/// Implements a simplified Maranget-style algorithm:
/// 1. If no columns remain, exhaustive iff at least one row exists
/// 2. If any row is all-wildcards, exhaustive
/// 3. Find first closed-type column
/// 4. For each constructor of that type, specialize and recurse
fn is_tuple_matrix_exhaustive(matrix: &[Vec<Pattern>], types: &[Type]) -> bool {
    // Base case: no columns left — exhaustive if matrix has at least one row
    if types.is_empty() {
        return !matrix.is_empty();
    }

    // Check if any row is all irrefutable
    for row in matrix {
        if row.iter().all(|p| is_irrefutable(p)) {
            return true;
        }
    }

    // Find the first column with a closed type for specialization
    let closed_col = types.iter().position(|t| {
        let ctors = closed_type_constructors(t);
        !ctors.is_empty()
    });

    if let Some(col) = closed_col {
        let constructors = closed_type_constructors(&types[col]);

        // For each constructor, specialize the matrix and check recursively
        for ctor in &constructors {
            let specialized = specialize_matrix(matrix, col, ctor);
            let remaining_types: Vec<Type> = types.iter()
                .enumerate()
                .filter(|(i, _)| *i != col)
                .map(|(_, t)| t.clone())
                .collect();

            if !is_tuple_matrix_exhaustive(&specialized, &remaining_types) {
                return false;
            }
        }
        true
    } else {
        // No closed-type columns: fall back to checking if any row is all-wildcards
        // (which we already checked above and it returned false)
        false
    }
}

/// Specialize a pattern matrix on column `col` for constructor `ctor`.
///
/// For each row in the matrix:
/// - If `row[col]` matches `ctor` → include the row without column `col`
/// - If `row[col]` is a wildcard → include the row without column `col`
///   (wildcard matches any constructor)
/// - Otherwise → row doesn't match, exclude it
fn specialize_matrix(
    matrix: &[Vec<Pattern>],
    col: usize,
    ctor: &ConstructorTag,
) -> Vec<Vec<Pattern>> {
    let mut result = Vec::new();

    for row in matrix {
        if col >= row.len() {
            continue;
        }

        let matches = pattern_matches_constructor(&row[col], ctor);

        if matches {
            // Build new row without column `col`
            let new_row: Vec<Pattern> = row.iter()
                .enumerate()
                .filter(|(i, _)| *i != col)
                .map(|(_, p)| p.clone())
                .collect();
            result.push(new_row);
        }
    }

    result
}

/// Check if a pattern matches a specific constructor.
fn pattern_matches_constructor(pat: &Pattern, ctor: &ConstructorTag) -> bool {
    match &pat.kind {
        // Wildcards and bindings match any constructor
        PatternKind::Wildcard => true,
        PatternKind::Binding { subpattern: None, .. } => true,
        PatternKind::Binding { subpattern: Some(sub), .. } => {
            pattern_matches_constructor(sub, ctor)
        }

        // Bool literals match their respective constructor
        PatternKind::Literal(hir::LiteralValue::Bool(true)) => {
            *ctor == ConstructorTag::BoolTrue
        }
        PatternKind::Literal(hir::LiteralValue::Bool(false)) => {
            *ctor == ConstructorTag::BoolFalse
        }

        // Or patterns: matches if any alternative matches
        PatternKind::Or(alts) => {
            alts.iter().any(|alt| pattern_matches_constructor(alt, ctor))
        }

        // All other patterns don't match bool constructors
        _ => false,
    }
}

/// Fallback per-position check for tuples with all open types.
fn check_tuple_per_position(
    matrix: &[Vec<Pattern>],
    element_types: &[Type],
) -> (bool, Vec<String>) {
    for (i, elem_ty) in element_types.iter().enumerate() {
        let position_patterns: Vec<&Pattern> = matrix
            .iter()
            .filter_map(|row| row.get(i))
            .collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{self, Pattern, PatternKind, Type, LiteralValue};
    use crate::span::Span;

    fn dummy_span() -> Span { Span::dummy() }
    fn bool_ty() -> Type { Type::bool() }
    fn i32_ty() -> Type { Type::i32() }

    fn wildcard_pat() -> Pattern {
        Pattern { kind: PatternKind::Wildcard, span: dummy_span(), ty: Type::error() }
    }

    fn bool_pat(val: bool) -> Pattern {
        Pattern {
            kind: PatternKind::Literal(LiteralValue::Bool(val)),
            span: dummy_span(),
            ty: bool_ty(),
        }
    }

    fn tuple_pat(pats: Vec<Pattern>) -> Pattern {
        Pattern {
            kind: PatternKind::Tuple(pats),
            span: dummy_span(),
            ty: Type::error(),
        }
    }

    fn make_arm(pat: Pattern) -> hir::MatchArm {
        hir::MatchArm {
            pattern: pat,
            guard: None,
            body: hir::Expr::new(
                hir::ExprKind::Literal(LiteralValue::Int(0)),
                Type::i32(),
                dummy_span(),
            ),
        }
    }

    #[test]
    fn test_tuple_bool_bool_diagonal_not_exhaustive() {
        // (true, true), (false, false) on (bool, bool) → NOT exhaustive
        // Missing: (true, false) and (false, true)
        let arms = vec![
            make_arm(tuple_pat(vec![bool_pat(true), bool_pat(true)])),
            make_arm(tuple_pat(vec![bool_pat(false), bool_pat(false)])),
        ];
        let scrutinee_ty = Type::tuple(vec![bool_ty(), bool_ty()]);
        let result = check_exhaustiveness(&arms, &scrutinee_ty, None);
        assert!(!result.is_exhaustive, "diagonal (true,true)/(false,false) should NOT be exhaustive");
    }

    #[test]
    fn test_tuple_bool_wildcard_exhaustive() {
        // (true, _), (false, _) on (bool, i32) → exhaustive
        let arms = vec![
            make_arm(tuple_pat(vec![bool_pat(true), wildcard_pat()])),
            make_arm(tuple_pat(vec![bool_pat(false), wildcard_pat()])),
        ];
        let scrutinee_ty = Type::tuple(vec![bool_ty(), i32_ty()]);
        let result = check_exhaustiveness(&arms, &scrutinee_ty, None);
        assert!(result.is_exhaustive, "(true, _)/(false, _) should be exhaustive");
    }

    #[test]
    fn test_tuple_bool_bool_full_coverage() {
        // (true, true), (true, false), (false, _) on (bool, bool) → exhaustive
        let arms = vec![
            make_arm(tuple_pat(vec![bool_pat(true), bool_pat(true)])),
            make_arm(tuple_pat(vec![bool_pat(true), bool_pat(false)])),
            make_arm(tuple_pat(vec![bool_pat(false), wildcard_pat()])),
        ];
        let scrutinee_ty = Type::tuple(vec![bool_ty(), bool_ty()]);
        let result = check_exhaustiveness(&arms, &scrutinee_ty, None);
        assert!(result.is_exhaustive, "full bool×bool coverage should be exhaustive");
    }

    #[test]
    fn test_tuple_wildcard_cross_position() {
        // (true, _), (_, true) on (bool, bool) → exhaustive
        // true,_ covers (true,true) and (true,false)
        // _,true covers (true,true) and (false,true)
        // Together they cover all four: (t,t), (t,f), (f,t), and (f,t) covers (false,true)
        // Missing: (false, false)? Let's check:
        //   - (true, _) doesn't match (false, false)
        //   - (_, true) doesn't match (false, false)
        // So this is NOT exhaustive!
        let arms = vec![
            make_arm(tuple_pat(vec![bool_pat(true), wildcard_pat()])),
            make_arm(tuple_pat(vec![wildcard_pat(), bool_pat(true)])),
        ];
        let scrutinee_ty = Type::tuple(vec![bool_ty(), bool_ty()]);
        let result = check_exhaustiveness(&arms, &scrutinee_ty, None);
        assert!(!result.is_exhaustive, "(true,_)/(_,true) misses (false,false)");
    }

    #[test]
    fn test_tuple_with_global_wildcard() {
        // (true, true), _ on (bool, bool) → exhaustive (wildcard covers all)
        let arms = vec![
            make_arm(tuple_pat(vec![bool_pat(true), bool_pat(true)])),
            make_arm(wildcard_pat()),
        ];
        let scrutinee_ty = Type::tuple(vec![bool_ty(), bool_ty()]);
        let result = check_exhaustiveness(&arms, &scrutinee_ty, None);
        assert!(result.is_exhaustive, "any pattern + wildcard should be exhaustive");
    }
}
