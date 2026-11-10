//! String metrics shared by the name and type comparisons.

/// Levenshtein distance between two strings, counted in characters.
pub(crate) fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left.is_ascii() && right.is_ascii() {
        // One byte is one character.
        distance_between(left.as_bytes(), right.as_bytes())
    } else {
        let left: Vec<char> = left.chars().collect();
        let right: Vec<char> = right.chars().collect();
        distance_between(&left, &right)
    }
}

fn distance_between<T: PartialEq>(mut left: &[T], mut right: &[T]) -> usize {
    // What two sequences start and end with alike takes no edit: the
    // names and annotations compared here often differ in a few
    // characters in the middle.
    let prefix = left.iter().zip(right).take_while(|(a, b)| a == b).count();
    (left, right) = (&left[prefix..], &right[prefix..]);
    let suffix = left.iter().rev().zip(right.iter().rev()).take_while(|(a, b)| a == b).count();
    (left, right) = (&left[..left.len() - suffix], &right[..right.len() - suffix]);
    if right.is_empty() {
        return left.len();
    }

    // One row of the edit matrix at a time: `row[j]` is the distance
    // between the elements of `left` consumed so far and the first `j`
    // elements of `right`.
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (i, left_element) in left.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right_element) in right.iter().enumerate() {
            let substitution = diagonal + usize::from(left_element != right_element);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(diagonal + 1).min(row[j] + 1);
        }
    }
    row[right.len()]
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
    fn shared_ends_take_no_edit() {
        assert_eq!(levenshtein_distance("prefix-abc-suffix", "prefix-xbz-suffix"), 2);
        assert_eq!(levenshtein_distance("same", "same-and-more"), 9);
        assert_eq!(levenshtein_distance("more-and-same", "same"), 9);
        assert_eq!(levenshtein_distance("aaaa", "aa"), 2);
        assert_eq!(levenshtein_distance("abab", "ab"), 2);
        assert_eq!(levenshtein_distance("注文の番号", "注文の名前"), 2);
        // The overlap of a shared start and a shared end is counted once.
        assert_eq!(levenshtein_distance("aXa", "a"), 2);
        assert_eq!(levenshtein_distance("a", "aXa"), 2);
    }

    #[test]
    fn distance_agrees_with_the_full_matrix() {
        fn by_matrix(left: &str, right: &str) -> usize {
            let (left, right): (Vec<char>, Vec<char>) =
                (left.chars().collect(), right.chars().collect());
            let mut matrix = vec![vec![0usize; right.len() + 1]; left.len() + 1];
            for (i, row) in matrix.iter_mut().enumerate() {
                row[0] = i;
            }
            for (j, cell) in matrix[0].iter_mut().enumerate() {
                *cell = j;
            }
            for i in 1..=left.len() {
                for j in 1..=right.len() {
                    let substitution =
                        matrix[i - 1][j - 1] + usize::from(left[i - 1] != right[j - 1]);
                    matrix[i][j] = substitution.min(matrix[i - 1][j] + 1).min(matrix[i][j - 1] + 1);
                }
            }
            matrix[left.len()][right.len()]
        }
        let mut state = 5u64;
        let mut next = |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as usize) % bound
        };
        // Words over ASCII alone and over a mixed alphabet.
        for alphabet in ["abc", "abcé文"] {
            let alphabet: Vec<char> = alphabet.chars().collect();
            for _ in 0..400 {
                let mut word = |max: usize| -> String {
                    (0..next(max)).map(|_| alphabet[next(alphabet.len())]).collect()
                };
                let (left, right) = (word(12), word(12));
                assert_eq!(
                    levenshtein_distance(&left, &right),
                    by_matrix(&left, &right),
                    "{left} / {right}"
                );
            }
        }
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
