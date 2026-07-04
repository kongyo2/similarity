use crate::apted::{
    compute_edit_distance, compute_edit_distance_with_cutoff, APTEDOptions, DISTANCE_EXCEEDED,
};
use crate::tree::TreeNode;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct TSEDOptions {
    pub apted_options: APTEDOptions,
    pub min_lines: u32, // Minimum number of lines for a function to be considered
    pub min_tokens: Option<u32>, // Minimum number of tokens (AST nodes) for a function to be considered
    pub size_penalty: bool,      // Apply penalty for short functions
    pub skip_test: bool,         // Skip test functions (language-specific)
}

impl Default for TSEDOptions {
    fn default() -> Self {
        TSEDOptions {
            apted_options: APTEDOptions {
                rename_cost: 0.3, // Default from the TypeScript implementation
                delete_cost: 1.0,
                insert_cost: 1.0,
                compare_values: false, // TypeScript default: structural comparison only
            },
            min_lines: 5,       // Increased default to better filter trivial matches
            min_tokens: None,   // No token limit by default
            size_penalty: true, // Enable size penalty by default
            skip_test: false,   // Don't skip test functions by default
        }
    }
}

/// Calculate TSED (Tree Structure Edit Distance) similarity between two trees
/// Returns a value between 0.0 and 1.0, where 1.0 means identical
///
/// # Penalty design
///
/// The raw `1 - distance / max_size` is a structural similarity, but for
/// refactoring use the raw number is misleading on short functions: two
/// trivial 1-liners differing only in operator will read as ~0.95 even
/// though they share no real refactoring opportunity. The penalty layer
/// below shapes the score so that:
///
/// * trivial trees (`max_size < 8`) with any structural distance get a
///   strong discount (case `() => a+b` vs `() => a-b` should not register
///   at default 0.85 threshold)
/// * short functions (10–30 nodes) get a softener that's mostly waived
///   when the edit distance is small (rename-only refactor between two
///   ~5-line helpers should still surface) and reapplied when distance
///   is meaningful (operator/shape change in a short loop body should
///   pull the score back down)
/// * functions ≥30 nodes pay no short-function penalty — APTED's
///   normalized distance is already well-calibrated there.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn calculate_tsed(tree1: &Rc<TreeNode>, tree2: &Rc<TreeNode>, options: &TSEDOptions) -> f64 {
    let distance = compute_edit_distance(tree1, tree2, &options.apted_options);
    let size1 = tree1.get_subtree_size() as f64;
    let size2 = tree2.get_subtree_size() as f64;
    finalize_tsed_similarity(distance, size1, size2, options)
}

/// Variant of [`calculate_tsed`] that aborts early when the maximum
/// achievable similarity is provably below `threshold`. Returns `0.0` in
/// that case. When the budget isn't exceeded the result is the same as
/// `calculate_tsed` to within floating-point noise — so this is purely a
/// performance hint with no accuracy impact.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn calculate_tsed_with_threshold(
    tree1: &Rc<TreeNode>,
    tree2: &Rc<TreeNode>,
    options: &TSEDOptions,
    threshold: f64,
) -> f64 {
    let size1 = tree1.get_subtree_size() as f64;
    let size2 = tree2.get_subtree_size() as f64;
    let max_size = size1.max(size2);
    if max_size == 0.0 {
        return 1.0;
    }
    // Maximum APTED distance that could still yield similarity ≥ threshold:
    //   similarity = 1 - distance / max_size ≥ threshold
    //   ⇒ distance ≤ max_size × (1 − threshold)
    // The penalty layer can only further reduce the similarity, so any
    // distance beyond this guarantees the final score will fall below
    // `threshold`. Add a tiny epsilon so the cutoff doesn't reject the
    // borderline case where distance is exactly on the boundary.
    let max_distance = (max_size * (1.0 - threshold) + 1e-9).max(0.0);

    let distance =
        compute_edit_distance_with_cutoff(tree1, tree2, &options.apted_options, max_distance);
    if distance >= DISTANCE_EXCEEDED {
        return 0.0;
    }
    finalize_tsed_similarity(distance, size1, size2, options)
}

/// Shared core of the size/penalty layer used by both `calculate_tsed` and
/// `calculate_tsed_with_threshold`. Keeping this single source of truth
/// matters because the penalty profile is calibrated against the test
/// suite and any drift between the two paths would silently corrupt the
/// threshold-based fast path.
#[must_use]
#[allow(clippy::cast_precision_loss)]
fn finalize_tsed_similarity(
    distance: f64,
    size1: f64,
    size2: f64,
    options: &TSEDOptions,
) -> f64 {
    let max_size = size1.max(size2);
    let tsed_similarity =
        if max_size > 0.0 { (1.0 - distance / max_size).max(0.0) } else { 1.0 };

    let tsed_similarity = if distance == 0.0 && size1 != size2 {
        let size_ratio = size1.min(size2) / size1.max(size2);
        let size_diff = (size1 - size2).abs();
        if size_diff > 10.0 {
            tsed_similarity * 0.5
        } else if size_ratio < 0.95 || size_diff > 3.0 {
            tsed_similarity * size_ratio.powf(0.5)
        } else {
            tsed_similarity
        }
    } else {
        tsed_similarity
    };

    let mut similarity = tsed_similarity;
    let size_ratio = size1.min(size2) / size1.max(size2);
    let min_size = size1.min(size2);
    let normalized_distance = if max_size > 0.0 { distance / max_size } else { 0.0 };

    if options.size_penalty {
        if max_size < 8.0 && distance > 0.0 {
            similarity *= 0.55;
        } else if max_size < 16.0 && distance > 0.0 {
            // Soften the short-tree penalty when the structural difference
            // is tiny — a single-rename match between two ~11-node helpers
            // should land far above the default 0.8 threshold, not at
            // ~0.84 where the prior flat 0.92 multiplier put it. The
            // softener tilts the factor toward 1.0 as `normalized_distance`
            // approaches 0 and toward 0.92 as it grows.
            let softener = (1.0 - normalized_distance * 8.0).clamp(0.0, 1.0);
            let effective = 0.92 + (1.0 - 0.92) * softener;
            similarity *= effective;
        }

        // Soften the short-function penalties when the trees are exactly
        // identical (distance == 0). Two 3-line helpers that happen to be
        // byte-equivalent are clearly refactoring candidates regardless of
        // their size; previously the compounded `< 10` and `>0.04` factors
        // crushed them to ~0.13 even at distance zero, which sat far below
        // every reasonable threshold. The penalty curve still applies its
        // full discount once the bodies diverge.
        if distance > 0.0 {
            if min_size < 10.0 {
                let base_factor = (min_size / 10.0).powf(0.7).max(0.25);
                similarity *= base_factor;
                if normalized_distance > 0.04 {
                    similarity *= 0.6;
                }
            } else if min_size < 20.0 {
                let softener = (1.0 - normalized_distance * 4.0).clamp(0.0, 1.0);
                let base_factor = 0.92;
                let effective_factor = base_factor + (1.0 - base_factor) * softener;
                similarity *= effective_factor;
                if normalized_distance > 0.18 {
                    similarity *= 0.85;
                }
            } else if min_size < 30.0 {
                let softener = (1.0 - normalized_distance * 4.0).clamp(0.0, 1.0);
                let base_factor = 0.95;
                let effective_factor = base_factor + (1.0 - base_factor) * softener;
                similarity *= effective_factor;
                if normalized_distance > 0.13 {
                    similarity *= 0.88;
                }
            }
        } else if min_size < 8.0 {
            // Identical trees below ~8 nodes still get a mild discount so a
            // 1-token `() => 0` vs `() => 0` doesn't dominate the report,
            // but the discount is bounded. Larger identical trees take no
            // discount at all: the canonicalizer legitimately SHRINKS
            // equivalent bodies (temp-return elimination, single-use
            // inlining), and an exact structural match is the strongest
            // duplicate signal there is — its score should not depend on
            // how compact the canonical form happens to be.
            let base_factor = ((min_size + 4.0) / 12.0).clamp(0.7, 0.95);
            similarity *= base_factor;
        }

        if normalized_distance > 0.16 {
            let excess = normalized_distance - 0.16;
            let penalty_factor = (1.0 - excess * 4.0).max(0.3);
            similarity *= penalty_factor;
        }

        if size_ratio < 0.5 {
            similarity *= size_ratio.powf(0.5);
        }
    }

    similarity
}

/// Calculate TSED from TypeScript code strings
///
/// # Errors
///
/// Returns an error if parsing fails for either code string
pub fn calculate_tsed_from_code(
    code1: &str,
    code2: &str,
    filename1: &str,
    filename2: &str,
    options: &TSEDOptions,
) -> Result<f64, String> {
    use crate::parser::parse_and_convert_to_tree;

    let tree1 = parse_and_convert_to_tree(filename1, code1)?;
    let tree2 = parse_and_convert_to_tree(filename2, code2)?;

    Ok(calculate_tsed(&tree1, &tree2, options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_code() {
        let code = "function add(a: number, b: number) { return a + b; }";
        let options = TSEDOptions {
            size_penalty: false, // Disable for small test functions
            ..Default::default()
        };

        let similarity =
            calculate_tsed_from_code(code, code, "test1.ts", "test2.ts", &options).unwrap();
        assert!((similarity - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_renamed_function() {
        let code1 = "function add(a: number, b: number) { return a + b; }";
        let code2 = "function sum(x: number, y: number) { return x + y; }";
        let options = TSEDOptions {
            size_penalty: false, // Disable for small test functions
            ..Default::default()
        };

        let similarity =
            calculate_tsed_from_code(code1, code2, "test1.ts", "test2.ts", &options).unwrap();
        // Should have high similarity due to low rename cost
        assert!(similarity > 0.8);
    }

    #[test]
    fn test_different_structure() {
        let code1 = "function test() { return 1; }";
        let code2 = "class Test { method() { return 1; } }";
        let options = TSEDOptions::default();

        let similarity =
            calculate_tsed_from_code(code1, code2, "test1.ts", "test2.ts", &options).unwrap();
        // Should have lower similarity due to structural differences
        assert!(similarity < 0.7);
    }
}
