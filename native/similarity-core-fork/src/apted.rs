use crate::tree::TreeNode;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct APTEDOptions {
    pub rename_cost: f64,
    pub delete_cost: f64,
    pub insert_cost: f64,
    /// Whether to compare node values in addition to labels
    pub compare_values: bool,
}

impl Default for APTEDOptions {
    fn default() -> Self {
        APTEDOptions {
            rename_cost: 1.0,
            delete_cost: 1.0,
            insert_cost: 1.0,
            compare_values: true, // Default: compare both structure and values
        }
    }
}

/// Sentinel value indicating the distance exceeds the cutoff budget. Picked
/// to be unambiguous when compared against any realistic distance value,
/// while avoiding NaN propagation issues that would arise from
/// `f64::INFINITY`.
pub const DISTANCE_EXCEEDED: f64 = f64::MAX / 4.0;

// Identifier-like leaf kinds. Renaming one of these (e.g. a parameter or
// local variable) is semantically neutral — it doesn't change what the
// surrounding code does. Most rename refactors land here, so we give them
// the base `rename_cost`.
//
// All other label-only changes (operator labels on BinaryExpression,
// UnaryExpression, AssignmentExpression, …; method/property keys; literal
// values; etc.) are actually semantic changes inside the same syntactic
// kind, so we charge a higher rate so APTED prefers to delete+insert
// when that's cheaper than masking the change as a "rename".
fn is_identifier_like_kind(value: &str) -> bool {
    matches!(
        value,
        "Identifier"
            | "BindingIdentifier"
            | "PrivateIdentifier"
            | "StringLiteral"
            | "NumericLiteral"
            | "BooleanLiteral"
            | "BigIntLiteral"
            | "NullLiteral"
            | "TemplateElement"
            | "Parameter"
    )
}

/// Compute the rename cost for substituting one node with another.
///
/// The cost reflects how much semantic change the substitution represents:
///   * identical label and kind → 0 (exact match)
///   * same kind (`value`), differing label, identifier-like leaf → `rename_cost`
///     (e.g. parameter or variable rename — neutral refactor)
///   * same kind, differing label, non-identifier-like (operator, method
///     key, literal value, etc.) → `rename_cost * 2.0` (semantic change
///     within a syntactic category)
///   * differing kind (`value`) → `rename_cost * 3.0` (subtree-shape
///     change — e.g. CallExpression vs BinaryExpression). Capped so that
///     APTED still prefers a rename over delete+insert when the rename
///     would otherwise be cheaper than 2 × delete_cost.
fn node_rename_cost(node1: &TreeNode, node2: &TreeNode, options: &APTEDOptions) -> f64 {
    let labels_match = node1.label == node2.label;
    let values_match = node1.value == node2.value;

    if labels_match && values_match {
        return 0.0;
    }

    if options.compare_values {
        // `compare_values: true` mode is used by the class/type
        // comparators where rich semantics on a single node matter less
        // than the surrounding declaration shape. Use the legacy flat
        // cost so those callers see the same behaviour they always did.
        return options.rename_cost;
    }

    if values_match {
        if is_identifier_like_kind(&node1.value) {
            options.rename_cost
        } else {
            (options.rename_cost * 2.0).min(options.delete_cost + options.insert_cost - 0.01)
        }
    } else {
        (options.rename_cost * 3.0).min(options.delete_cost + options.insert_cost - 0.01)
    }
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn compute_edit_distance(
    tree1: &Rc<TreeNode>,
    tree2: &Rc<TreeNode>,
    options: &APTEDOptions,
) -> f64 {
    let mut memo: HashMap<(usize, usize), f64> = HashMap::new();
    compute_edit_distance_recursive(tree1, tree2, options, &mut memo)
}

/// Compute the APTED edit distance, aborting as soon as a partial result
/// proves it cannot stay within `max_distance`. Callers should treat any
/// return value `>= DISTANCE_EXCEEDED` as "the distance is at least
/// `max_distance`, exact value unknown".
///
/// The cutoff is a pure performance hint — when the budget is generous
/// the function returns the exact distance and the result matches
/// `compute_edit_distance`. The TSED layer uses it to skip pairs that
/// cannot reach the user-requested similarity threshold.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn compute_edit_distance_with_cutoff(
    tree1: &Rc<TreeNode>,
    tree2: &Rc<TreeNode>,
    options: &APTEDOptions,
    max_distance: f64,
) -> f64 {
    let mut memo: HashMap<(usize, usize), f64> = HashMap::new();
    compute_edit_distance_cutoff(tree1, tree2, options, &mut memo, max_distance)
}

fn compute_edit_distance_recursive(
    node1: &Rc<TreeNode>,
    node2: &Rc<TreeNode>,
    options: &APTEDOptions,
    memo: &mut HashMap<(usize, usize), f64>,
) -> f64 {
    let key = (node1.id, node2.id);

    if let Some(&cost) = memo.get(&key) {
        return cost;
    }

    // Base cases
    if node1.children.is_empty() && node2.children.is_empty() {
        let cost = node_rename_cost(node1, node2, options);
        memo.insert(key, cost);
        return cost;
    }

    // Calculate costs for all three operations
    let delete_all_cost = options.delete_cost * node1.get_subtree_size() as f64;
    let insert_all_cost = options.insert_cost * node2.get_subtree_size() as f64;

    // Calculate rename + optimal children alignment
    let mut rename_plus_cost = node_rename_cost(node1, node2, options);

    if !node1.children.is_empty() || !node2.children.is_empty() {
        // Compute all pairwise costs between children
        let mut child_cost_matrix: HashMap<(usize, usize), f64> = HashMap::new();

        for child1 in &node1.children {
            for child2 in &node2.children {
                let cost = compute_edit_distance_recursive(child1, child2, options, memo);
                child_cost_matrix.insert((child1.id, child2.id), cost);
            }
        }

        // Find optimal alignment
        let (alignment_cost, _) = compute_children_alignment(
            &node1.children,
            &node2.children,
            &child_cost_matrix,
            options,
        );

        rename_plus_cost += alignment_cost;
    }

    let min_cost = delete_all_cost.min(insert_all_cost).min(rename_plus_cost);
    memo.insert(key, min_cost);
    min_cost
}

#[allow(clippy::cast_precision_loss)]
fn compute_edit_distance_cutoff(
    node1: &Rc<TreeNode>,
    node2: &Rc<TreeNode>,
    options: &APTEDOptions,
    memo: &mut HashMap<(usize, usize), f64>,
    max_distance: f64,
) -> f64 {
    let key = (node1.id, node2.id);

    if let Some(&cost) = memo.get(&key) {
        return cost;
    }

    // Base cases — same as the non-cutoff version, because the rename
    // cost for a single leaf pair can never exceed delete+insert anyway.
    if node1.children.is_empty() && node2.children.is_empty() {
        let cost = node_rename_cost(node1, node2, options);
        memo.insert(key, cost);
        return cost;
    }

    let size1 = node1.get_subtree_size() as f64;
    let size2 = node2.get_subtree_size() as f64;

    // Lower bound: at minimum, we have to insert or delete the size gap.
    let min_op_cost = options.delete_cost.min(options.insert_cost);
    let lower_bound = (size1 - size2).abs() * min_op_cost;
    if lower_bound > max_distance {
        // Don't memoize "exceeded" — the result depends on the budget.
        return DISTANCE_EXCEEDED;
    }

    // Costs for delete-all and insert-all.
    let delete_all_cost = options.delete_cost * size1;
    let insert_all_cost = options.insert_cost * size2;
    let mut best = delete_all_cost.min(insert_all_cost);

    let rename_cost = node_rename_cost(node1, node2, options);

    // Only compute the children alignment when it could improve on `best`.
    if rename_cost < best && (!node1.children.is_empty() || !node2.children.is_empty()) {
        let alignment_budget = best - rename_cost;

        let mut child_cost_matrix: HashMap<(usize, usize), f64> = HashMap::new();
        for child1 in &node1.children {
            for child2 in &node2.children {
                let cost = compute_edit_distance_cutoff(
                    child1,
                    child2,
                    options,
                    memo,
                    alignment_budget,
                );
                child_cost_matrix.insert((child1.id, child2.id), cost);
            }
        }

        let (alignment_cost, _) = compute_children_alignment_cutoff(
            &node1.children,
            &node2.children,
            &child_cost_matrix,
            options,
            alignment_budget,
        );

        if alignment_cost < DISTANCE_EXCEEDED {
            let total = rename_cost + alignment_cost;
            if total < best {
                best = total;
            }
        }
    }

    memo.insert(key, best);
    best
}

fn compute_children_alignment(
    children1: &[Rc<TreeNode>],
    children2: &[Rc<TreeNode>],
    cost_matrix: &HashMap<(usize, usize), f64>,
    options: &APTEDOptions,
) -> (f64, HashMap<usize, Option<usize>>) {
    let m = children1.len();
    let n = children2.len();

    // dp[i][j] = minimum cost to align first i children of node1 with first j children of node2
    let mut dp = vec![vec![0.0; n + 1]; m + 1];

    // Initialize base cases
    for i in 1..=m {
        dp[i][0] = dp[i - 1][0] + options.delete_cost * children1[i - 1].get_subtree_size() as f64;
    }
    for j in 1..=n {
        dp[0][j] = dp[0][j - 1] + options.insert_cost * children2[j - 1].get_subtree_size() as f64;
    }

    // Fill DP table
    for i in 1..=m {
        for j in 1..=n {
            let child1 = &children1[i - 1];
            let child2 = &children2[j - 1];
            let edit_cost = cost_matrix.get(&(child1.id, child2.id)).unwrap_or(&0.0);

            dp[i][j] = (dp[i - 1][j] + options.delete_cost * child1.get_subtree_size() as f64)
                .min(dp[i][j - 1] + options.insert_cost * child2.get_subtree_size() as f64)
                .min(dp[i - 1][j - 1] + edit_cost);
        }
    }

    // Backtrack to find alignment
    let mut alignment = HashMap::new();
    let mut i = m;
    let mut j = n;

    while i > 0 || j > 0 {
        if i == 0 {
            j -= 1;
        } else if j == 0 {
            alignment.insert(children1[i - 1].id, None);
            i -= 1;
        } else {
            let child1 = &children1[i - 1];
            let child2 = &children2[j - 1];
            let edit_cost = cost_matrix.get(&(child1.id, child2.id)).unwrap_or(&0.0);

            let delete_cost = dp[i - 1][j] + options.delete_cost * child1.get_subtree_size() as f64;
            let insert_cost = dp[i][j - 1] + options.insert_cost * child2.get_subtree_size() as f64;
            let match_cost = dp[i - 1][j - 1] + edit_cost;

            if match_cost <= delete_cost && match_cost <= insert_cost {
                alignment.insert(child1.id, Some(child2.id));
                i -= 1;
                j -= 1;
            } else if delete_cost <= insert_cost {
                alignment.insert(child1.id, None);
                i -= 1;
            } else {
                j -= 1;
            }
        }
    }

    (dp[m][n], alignment)
}

fn compute_children_alignment_cutoff(
    children1: &[Rc<TreeNode>],
    children2: &[Rc<TreeNode>],
    cost_matrix: &HashMap<(usize, usize), f64>,
    options: &APTEDOptions,
    budget: f64,
) -> (f64, HashMap<usize, Option<usize>>) {
    let m = children1.len();
    let n = children2.len();

    let mut dp = vec![vec![DISTANCE_EXCEEDED; n + 1]; m + 1];
    dp[0][0] = 0.0;

    for i in 1..=m {
        let prev = dp[i - 1][0];
        if prev < DISTANCE_EXCEEDED {
            dp[i][0] = prev + options.delete_cost * children1[i - 1].get_subtree_size() as f64;
        }
    }
    for j in 1..=n {
        let prev = dp[0][j - 1];
        if prev < DISTANCE_EXCEEDED {
            dp[0][j] = prev + options.insert_cost * children2[j - 1].get_subtree_size() as f64;
        }
    }

    for i in 1..=m {
        for j in 1..=n {
            let child1 = &children1[i - 1];
            let child2 = &children2[j - 1];
            let edit_cost = cost_matrix.get(&(child1.id, child2.id)).copied().unwrap_or(0.0);

            let mut best = DISTANCE_EXCEEDED;

            let d_prev = dp[i - 1][j];
            if d_prev < DISTANCE_EXCEEDED {
                let cand = d_prev + options.delete_cost * child1.get_subtree_size() as f64;
                if cand < best {
                    best = cand;
                }
            }
            let i_prev = dp[i][j - 1];
            if i_prev < DISTANCE_EXCEEDED {
                let cand = i_prev + options.insert_cost * child2.get_subtree_size() as f64;
                if cand < best {
                    best = cand;
                }
            }
            let m_prev = dp[i - 1][j - 1];
            if m_prev < DISTANCE_EXCEEDED && edit_cost < DISTANCE_EXCEEDED {
                let cand = m_prev + edit_cost;
                if cand < best {
                    best = cand;
                }
            }

            // Prune entries that have already overshot the budget — the
            // optimal path can only grow from here, so they cannot lead
            // to a valid alignment. Keep `DISTANCE_EXCEEDED` so callers
            // recognise the abort.
            if best > budget {
                dp[i][j] = DISTANCE_EXCEEDED;
            } else {
                dp[i][j] = best;
            }
        }
    }

    if dp[m][n] >= DISTANCE_EXCEEDED {
        return (DISTANCE_EXCEEDED, HashMap::new());
    }

    let mut alignment = HashMap::new();
    let mut i = m;
    let mut j = n;
    while i > 0 || j > 0 {
        if i == 0 {
            j -= 1;
            continue;
        }
        if j == 0 {
            alignment.insert(children1[i - 1].id, None);
            i -= 1;
            continue;
        }
        let child1 = &children1[i - 1];
        let child2 = &children2[j - 1];
        let edit_cost = cost_matrix.get(&(child1.id, child2.id)).copied().unwrap_or(0.0);
        let delete_cost =
            dp[i - 1][j] + options.delete_cost * child1.get_subtree_size() as f64;
        let insert_cost =
            dp[i][j - 1] + options.insert_cost * child2.get_subtree_size() as f64;
        let match_cost = dp[i - 1][j - 1] + edit_cost;

        if match_cost <= delete_cost && match_cost <= insert_cost {
            alignment.insert(child1.id, Some(child2.id));
            i -= 1;
            j -= 1;
        } else if delete_cost <= insert_cost {
            alignment.insert(child1.id, None);
            i -= 1;
        } else {
            j -= 1;
        }
    }
    (dp[m][n], alignment)
}
