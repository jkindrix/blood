//! "Did you mean?" suggestions for typos.
//!
//! This module provides typo detection and suggestion functionality using
//! Levenshtein distance (edit distance) to find similar names in scope.

/// Maximum edit distance to consider a name as a valid suggestion.
/// Names with distance greater than this threshold are not suggested.
const MAX_EDIT_DISTANCE: usize = 3;

/// Minimum length for the input name to provide suggestions.
/// Very short names (1-2 chars) often produce too many false positives.
const MIN_NAME_LENGTH_FOR_SUGGESTIONS: usize = 2;

/// Compute the Levenshtein distance (edit distance) between two strings.
///
/// The Levenshtein distance is the minimum number of single-character edits
/// (insertions, deletions, or substitutions) needed to transform one string
/// into another.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(levenshtein_distance("print", "pirnt"), 2); // transposition
/// assert_eq!(levenshtein_distance("hello", "helo"), 1);  // deletion
/// assert_eq!(levenshtein_distance("cat", "cats"), 1);    // insertion
/// assert_eq!(levenshtein_distance("sit", "sat"), 1);     // substitution
/// ```
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    // Early termination for trivial cases
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }
    if a == b {
        return 0;
    }

    // Use only two rows for space efficiency: O(min(a_len, b_len))
    // We keep previous row and current row
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for (i, a_char) in a_chars.iter().enumerate() {
        curr_row[0] = i + 1;

        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };

            // Minimum of:
            // - deletion (prev_row[j+1] + 1)
            // - insertion (curr_row[j] + 1)
            // - substitution (prev_row[j] + cost)
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + cost);
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Find similar names from a list of valid names.
///
/// Returns a list of suggestions sorted by edit distance (closest first).
/// Only names within the `MAX_EDIT_DISTANCE` threshold are returned.
///
/// # Arguments
///
/// * `misspelled` - The name that wasn't found (possibly misspelled)
/// * `valid_names` - Iterator of valid names to search through
///
/// # Returns
///
/// A vector of suggestions, sorted by edit distance (best match first).
/// Returns an empty vector if no suitable suggestions are found.
///
/// # Examples
///
/// ```ignore
/// let valid = vec!["print", "println", "printf", "read"];
/// let suggestions = suggest_similar("pirnt", valid.iter().map(|s| *s));
/// assert_eq!(suggestions, vec!["print"]);
/// ```
pub fn suggest_similar<'a, I>(misspelled: &str, valid_names: I) -> Vec<String>
where
    I: Iterator<Item = &'a str>,
{
    // Don't suggest for very short names - too many false positives
    if misspelled.len() < MIN_NAME_LENGTH_FOR_SUGGESTIONS {
        return Vec::new();
    }

    let mut candidates: Vec<(String, usize)> = valid_names
        .filter_map(|name| {
            let distance = levenshtein_distance(misspelled, name);

            // Apply dynamic threshold based on name length
            // Longer names can tolerate more edits
            let threshold = compute_threshold(misspelled, name);

            if distance <= threshold && distance > 0 {
                Some((name.to_string(), distance))
            } else {
                None
            }
        })
        .collect();

    // Sort by distance (closest first), then alphabetically for determinism
    candidates.sort_by(|a, b| {
        a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0))
    });

    // Return at most 3 suggestions
    candidates.into_iter()
        .take(3)
        .map(|(name, _)| name)
        .collect()
}

/// Compute a dynamic threshold based on the lengths of both strings.
///
/// The threshold scales with string length:
/// - Very short strings (1-2 chars): threshold 1
/// - Short strings (3-4 chars): threshold 1
/// - Medium strings (5-7 chars): threshold 2
/// - Long strings (8+ chars): threshold 3
fn compute_threshold(a: &str, b: &str) -> usize {
    let max_len = a.len().max(b.len());
    let min_len = a.len().min(b.len());

    // If lengths differ significantly, reduce threshold
    let length_diff = max_len - min_len;
    if length_diff > 2 {
        return 1;
    }

    match max_len {
        0..=2 => 1,
        3..=4 => 1,
        5..=7 => 2,
        _ => MAX_EDIT_DISTANCE.min(3),
    }
}

/// Format suggestions as a help message.
///
/// # Examples
///
/// ```ignore
/// let suggestions = vec!["print".to_string()];
/// let msg = format_suggestions(&suggestions);
/// assert_eq!(msg, Some("did you mean `print`?".to_string()));
///
/// let suggestions = vec!["print".to_string(), "println".to_string()];
/// let msg = format_suggestions(&suggestions);
/// assert_eq!(msg, Some("did you mean `print` or `println`?".to_string()));
/// ```
pub fn format_suggestions(suggestions: &[String]) -> Option<String> {
    match suggestions.len() {
        0 => None,
        1 => Some(format!("did you mean `{}`?", suggestions[0])),
        2 => Some(format!(
            "did you mean `{}` or `{}`?",
            suggestions[0], suggestions[1]
        )),
        _ => {
            // SAFETY: This arm only matches when len >= 3, so last() always returns Some
            let last = suggestions.last().expect("len >= 3 in this match arm");
            let rest: Vec<_> = suggestions[..suggestions.len() - 1]
                .iter()
                .map(|s| format!("`{}`", s))
                .collect();
            Some(format!(
                "did you mean {}, or `{}`?",
                rest.join(", "),
                last
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("a", "a"), 0);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert_eq!(levenshtein_distance("", "world"), 5);
    }

    #[test]
    fn test_levenshtein_single_char_typos() {
        // Substitution
        assert_eq!(levenshtein_distance("cat", "hat"), 1);
        assert_eq!(levenshtein_distance("sit", "sat"), 1);

        // Insertion
        assert_eq!(levenshtein_distance("cat", "cats"), 1);
        assert_eq!(levenshtein_distance("helo", "hello"), 1);

        // Deletion
        assert_eq!(levenshtein_distance("hello", "helo"), 1);
        assert_eq!(levenshtein_distance("cats", "cat"), 1);
    }

    #[test]
    fn test_levenshtein_transposed_characters() {
        // Transposition requires 2 operations (delete + insert)
        assert_eq!(levenshtein_distance("pirnt", "print"), 2);
        assert_eq!(levenshtein_distance("teh", "the"), 2);
        assert_eq!(levenshtein_distance("recieve", "receive"), 2);
    }

    #[test]
    fn test_levenshtein_multiple_edits() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("saturday", "sunday"), 3);
    }

    #[test]
    fn test_suggest_similar_single_typo() {
        let valid = ["print", "println", "printf", "read", "write"];
        let suggestions = suggest_similar("pirnt", valid.iter().copied());
        assert!(suggestions.contains(&"print".to_string()));
    }

    #[test]
    fn test_suggest_similar_transposed() {
        // Test with longer words (short names are filtered out)
        let valid = ["print", "point", "paint"];
        let suggestions = suggest_similar("pirnt", valid.iter().copied());
        assert!(suggestions.contains(&"print".to_string()));

        // "reutrn" -> "return" (transposition)
        let valid = ["return", "retain", "retina"];
        let suggestions = suggest_similar("reutrn", valid.iter().copied());
        assert!(suggestions.contains(&"return".to_string()));
    }

    #[test]
    fn test_suggest_similar_no_close_matches() {
        let valid = ["alpha", "beta", "gamma"];
        let suggestions = suggest_similar("zzzzzz", valid.iter().copied());
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_suggest_similar_exact_match_not_suggested() {
        let valid = ["print", "println"];
        let suggestions = suggest_similar("print", valid.iter().copied());
        // Exact match should not appear in suggestions (distance 0)
        assert!(!suggestions.contains(&"print".to_string()));
    }

    #[test]
    fn test_suggest_similar_short_names() {
        // Very short names shouldn't produce suggestions
        let valid = ["a", "b", "c", "ab", "bc"];
        let suggestions = suggest_similar("x", valid.iter().copied());
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_suggest_similar_returns_sorted() {
        let valid = ["printz", "print", "prnt", "printf"];
        let suggestions = suggest_similar("prnt", valid.iter().copied());
        // "print" has distance 1, "prnt" is exact (filtered), "printz" has distance 2
        if suggestions.len() >= 2 {
            // First suggestion should be closer than second
            let d1 = levenshtein_distance("prnt", &suggestions[0]);
            let d2 = levenshtein_distance("prnt", &suggestions[1]);
            assert!(d1 <= d2);
        }
    }

    #[test]
    fn test_format_suggestions_empty() {
        assert_eq!(format_suggestions(&[]), None);
    }

    #[test]
    fn test_format_suggestions_single() {
        let suggestions = vec!["print".to_string()];
        assert_eq!(
            format_suggestions(&suggestions),
            Some("did you mean `print`?".to_string())
        );
    }

    #[test]
    fn test_format_suggestions_two() {
        let suggestions = vec!["print".to_string(), "println".to_string()];
        assert_eq!(
            format_suggestions(&suggestions),
            Some("did you mean `print` or `println`?".to_string())
        );
    }

    #[test]
    fn test_format_suggestions_multiple() {
        let suggestions = vec![
            "print".to_string(),
            "println".to_string(),
            "printf".to_string(),
        ];
        assert_eq!(
            format_suggestions(&suggestions),
            Some("did you mean `print`, `println`, or `printf`?".to_string())
        );
    }

    #[test]
    fn test_common_programming_typos() {
        // Common typos developers make
        let cases = [
            ("fucntion", ["function"], true),
            ("retrun", ["return"], true),
            ("calss", ["class"], true),
            ("improt", ["import"], true),
            ("pubilc", ["public"], true),
            ("priavte", ["private"], true),
            ("strign", ["string"], true),
            ("interger", ["integer"], true),
        ];

        for (typo, valid, should_suggest) in cases {
            let suggestions = suggest_similar(typo, valid.iter().copied());
            if should_suggest {
                assert!(
                    !suggestions.is_empty(),
                    "Expected suggestion for typo '{}' from {:?}",
                    typo,
                    valid
                );
            }
        }
    }
}
