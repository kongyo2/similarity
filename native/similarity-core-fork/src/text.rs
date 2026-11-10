//! String metrics shared by the name and type comparisons.

/// Levenshtein distance between two strings, counted in characters.
pub(crate) fn levenshtein_distance(left: &str, right: &str) -> usize {
    let right_chars: Vec<char> = right.chars().collect();
    if right_chars.is_empty() {
        return left.chars().count();
    }

    // One row of the edit matrix at a time: `row[j]` is the distance
    // between the characters of `left` consumed so far and the first `j`
    // characters of `right`.
    let mut row: Vec<usize> = (0..=right_chars.len()).collect();
    for (i, left_char) in left.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right_char) in right_chars.iter().enumerate() {
            let substitution = diagonal + usize::from(left_char != *right_char);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(diagonal + 1).min(row[j] + 1);
        }
    }
    row[right_chars.len()]
}

/// `1 − distance / length of the longer string`, in characters: 1.0 for
/// equal strings (two empty ones included), 0.0 for strings with nothing
/// in common.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn levenshtein_similarity(left: &str, right: &str) -> f64 {
    let longest = left.chars().count().max(right.chars().count());
    if longest == 0 {
        return 1.0;
    }
    (1.0 - levenshtein_distance(left, right) as f64 / longest as f64).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_counts_single_character_edits() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "ab"), 1);
        assert_eq!(levenshtein_distance("abc", "def"), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("flaw", "lawn"), 2);
    }

    #[test]
    fn distance_and_similarity_count_characters_not_bytes() {
        // Each of these names is one edit apart; in UTF-8 they are three
        // bytes per character.
        assert_eq!(levenshtein_distance("注文", "注釈"), 1);
        assert!((levenshtein_similarity("注文", "注釈") - 0.5).abs() < 1e-12);
        assert!((levenshtein_similarity("naïve", "naive") - 0.8).abs() < 1e-12);
        assert!((levenshtein_similarity("", "") - 1.0).abs() < 1e-12);
        assert!(levenshtein_similarity("abc", "xyz").abs() < 1e-12);
    }
}
