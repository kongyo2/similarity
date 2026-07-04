use crate::tree::TreeNode;
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
// Literal kinds are intentionally NOT classified as identifier-like:
// changing `length > 100` to `length > 1000`, or `"info"` to `"warn"`,
// is a behavioural change rather than a rename refactor, so it pays the
// non-identifier-rename rate. Same goes for `TemplateElement` content —
// switching the literal portion of a template string changes what the
// code does, not just what its parts are called.
//
// All other label-only changes (operator labels on BinaryExpression,
// UnaryExpression, AssignmentExpression, …; method/property keys; etc.)
// are also semantic changes inside the same syntactic kind, so they too
// stay outside this allow-list and end up paying the higher within-kind
// rename cost.
fn is_identifier_like_kind(value: &str) -> bool {
    matches!(
        value,
        "Identifier" | "BindingIdentifier" | "PrivateIdentifier" | "Parameter"
    )
}

/// Built-in collection/utility member names whose identity IS the behavior
/// of the surrounding call. Substituting one for another (`.map` ⇔
/// `.filter`, `Math.max` ⇔ `Math.min`, `.push` ⇔ `.unshift`) rewrites what
/// the code computes even though the rest of the call shape is untouched,
/// so such a substitution pays the full within-kind cap instead of the
/// cheap identifier-rename rate. User code occasionally defines methods
/// with these names too — accepting that small mislabeling risk is worth
/// reliably keeping `xs.map(f)` and `xs.filter(f)` apart.
fn is_semantic_builtin_name(label: &str) -> bool {
    matches!(
        label,
        "map"
            | "filter"
            | "forEach"
            | "reduce"
            | "reduceRight"
            | "some"
            | "every"
            | "find"
            | "findIndex"
            | "findLast"
            | "findLastIndex"
            | "flatMap"
            | "push"
            | "pop"
            | "shift"
            | "unshift"
            | "slice"
            | "splice"
            | "concat"
            | "join"
            | "reverse"
            | "sort"
            | "includes"
            | "indexOf"
            | "lastIndexOf"
            | "keys"
            | "values"
            | "entries"
            | "min"
            | "max"
            | "floor"
            | "ceil"
            | "round"
            | "trunc"
            | "abs"
            | "parse"
            | "stringify"
            | "toUpperCase"
            | "toLowerCase"
            | "trim"
            | "trimStart"
            | "trimEnd"
            | "startsWith"
            | "endsWith"
            | "padStart"
            | "padEnd"
            | "get"
            | "set"
            | "has"
            | "delete"
            | "add"
    )
}

/// Compute the rename cost for substituting one node with another.
///
/// The cost reflects how much semantic change the substitution represents:
///   * identical label and kind → 0 (exact match)
///   * same kind (`value`), differing label, identifier-like leaf → `rename_cost`
///     (e.g. parameter or variable rename — neutral refactor), EXCEPT when
///     both labels are well-known builtin member names — swapping `.map`
///     for `.filter` is a behavioural rewrite and pays the within-kind cap
///   * same kind, differing label, non-identifier-like (operator, method
///     key, literal value, etc.) → `rename_cost * 2.0` (semantic change
///     within a syntactic category)
///   * differing kind (`value`) → `rename_cost * 3.0` (subtree-shape
///     change — e.g. CallExpression vs BinaryExpression). Capped so that
///     APTED still prefers a rename over delete+insert when the rename
///     would otherwise be cheaper than delete + insert.
fn label_rename_cost(
    label1: &str,
    label2: &str,
    value1: &str,
    value2: &str,
    options: &APTEDOptions,
) -> f64 {
    let labels_match = label1 == label2;
    let values_match = value1 == value2;

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
        if is_identifier_like_kind(value1) {
            if is_semantic_builtin_name(label1) && is_semantic_builtin_name(label2) {
                within_kind_rename_cap(options)
            } else {
                options.rename_cost
            }
        } else {
            (options.rename_cost * 2.0).min(within_kind_rename_cap(options))
        }
    } else {
        (options.rename_cost * 3.0).min(cross_kind_rename_cap(options))
    }
}


/// Maximum cost we allow a within-kind rename to charge.
///
/// The cap exists so APTED's optimal alignment never prefers a
/// delete-then-insert over a straight rename when the rename would
/// otherwise be cheaper — i.e. the cap stays strictly below
/// `delete + insert`. A naive `delete + insert - 0.01` offset would
/// collapse to zero for callers that configure very small per-op costs
/// (e.g. `delete = insert = 0.005`), and a zero cap would make many
/// non-identifier substitutions effectively free — distance would
/// collapse and similarity would inflate well above the intended
/// values. Use a multiplicative epsilon instead so the cap scales with
/// the configured cost and stays positive as long as the sum is
/// positive.
fn within_kind_rename_cap(options: &APTEDOptions) -> f64 {
    cap_below_delete_plus_insert(options)
}

/// Same idea for cross-kind substitutions. Kept as a separate function
/// so the intent reads clearly at the call sites.
fn cross_kind_rename_cap(options: &APTEDOptions) -> f64 {
    cap_below_delete_plus_insert(options)
}

fn cap_below_delete_plus_insert(options: &APTEDOptions) -> f64 {
    let sum = options.delete_cost + options.insert_cost;
    (sum * 0.99).max(0.0)
}

// ---------------------------------------------------------------------------
// Flat tree representation
// ---------------------------------------------------------------------------
//
// The recursive `Rc<TreeNode>` shape is convenient to build but slow to
// compare: the edit-distance DP visits O(n1·n2) node pairs and previously
// keyed its memo table by `(node.id, node.id)` in a `HashMap`, paying a
// hash + probe on every visit, plus a fresh child-cost `HashMap` per
// internal pair. Flattening each tree once per comparison into dense
// arrays lets the DP address everything by small integer index: the memo
// becomes a flat `Vec<f64>` and child costs a positional matrix. The
// flatten pass is O(n) and trivially amortized by the O(n1·n2) DP.

struct FlatTree<'a> {
    labels: Vec<&'a str>,
    values: Vec<&'a str>,
    sizes: Vec<u32>,
    /// Children of node `i` are `child_data[child_start[i]..child_start[i+1]]`.
    child_start: Vec<u32>,
    child_data: Vec<u32>,
}

impl<'a> FlatTree<'a> {
    fn build(root: &'a Rc<TreeNode>) -> FlatTree<'a> {
        let node_count = root.get_subtree_size();
        let mut tree = FlatTree {
            labels: Vec::with_capacity(node_count),
            values: Vec::with_capacity(node_count),
            sizes: Vec::with_capacity(node_count),
            child_start: Vec::with_capacity(node_count + 1),
            child_data: Vec::with_capacity(node_count.saturating_sub(1)),
        };
        // First pass assigns indices in pre-order; child index data is
        // appended in a second pass over the same order so the offsets
        // stay contiguous per parent.
        let mut order: Vec<&'a Rc<TreeNode>> = Vec::with_capacity(node_count);
        let mut stack: Vec<&'a Rc<TreeNode>> = vec![root];
        let mut index_of = std::collections::HashMap::with_capacity(node_count);
        while let Some(node) = stack.pop() {
            index_of.insert(std::ptr::from_ref::<TreeNode>(node.as_ref()) as usize, order.len());
            order.push(node);
            for child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        for node in &order {
            tree.labels.push(node.label.as_str());
            tree.values.push(node.value.as_str());
            #[allow(clippy::cast_possible_truncation)]
            tree.sizes.push(node.get_subtree_size() as u32);
        }
        for node in &order {
            #[allow(clippy::cast_possible_truncation)]
            tree.child_start.push(tree.child_data.len() as u32);
            for child in &node.children {
                let key = std::ptr::from_ref::<TreeNode>(child.as_ref()) as usize;
                #[allow(clippy::cast_possible_truncation)]
                tree.child_data.push(index_of[&key] as u32);
            }
        }
        #[allow(clippy::cast_possible_truncation)]
        tree.child_start.push(tree.child_data.len() as u32);
        tree
    }

    fn len(&self) -> usize {
        self.labels.len()
    }

    fn children(&self, index: usize) -> &[u32] {
        let start = self.child_start[index] as usize;
        let end = self.child_start[index + 1] as usize;
        &self.child_data[start..end]
    }

    fn size(&self, index: usize) -> f64 {
        f64::from(self.sizes[index])
    }
}

/// Exact pairwise distance between subtree `i` of `tree1` and subtree `j`
/// of `tree2`. The distance is the cheaper of:
///   * replacing the whole subtree (delete every node of `i`, insert
///     every node of `j`), or
///   * renaming the root pair and optimally aligning the children.
///
/// The replace branch must charge `delete + insert`, not
/// `min(delete, insert)`: an earlier version used the min, which let a
/// large subtree "become" a completely different one at the cost of
/// only the smaller side. That both inflated similarity between
/// structurally unrelated size-mismatched trees and violated the
/// `distance ≥ |size1 − size2| · min_op_cost` lower bound the
/// threshold pruning in `tsed.rs`/`function_extractor.rs` relies on.
///
/// `memo` is indexed `i * tree2.len() + j`; NaN means "not computed yet".
fn subtree_distance(
    tree1: &FlatTree,
    tree2: &FlatTree,
    options: &APTEDOptions,
    memo: &mut [f64],
    i: usize,
    j: usize,
) -> f64 {
    let key = i * tree2.len() + j;
    let cached = memo[key];
    if !cached.is_nan() {
        return cached;
    }

    let children1 = tree1.children(i);
    let children2 = tree2.children(j);

    let rename_cost = label_rename_cost(
        tree1.labels[i],
        tree2.labels[j],
        tree1.values[i],
        tree2.values[j],
        options,
    );

    let cost = if children1.is_empty() && children2.is_empty() {
        rename_cost
    } else {
        let replace_cost =
            options.delete_cost * tree1.size(i) + options.insert_cost * tree2.size(j);
        let rename_plus_cost =
            rename_cost + children_alignment(tree1, tree2, options, memo, i, j);
        replace_cost.min(rename_plus_cost)
    };

    memo[key] = cost;
    cost
}

/// Sequence-alignment DP over the child lists of `tree1[i]` / `tree2[j]`.
/// `dp[a][b]` = minimum cost to align the first `a` children of node1 with
/// the first `b` children of node2, where skipping a child costs
/// deleting/inserting its whole subtree and matching a pair costs their
/// recursive distance.
fn children_alignment(
    tree1: &FlatTree,
    tree2: &FlatTree,
    options: &APTEDOptions,
    memo: &mut [f64],
    i: usize,
    j: usize,
) -> f64 {
    let m = tree1.children(i).len();
    let n = tree2.children(j).len();
    if m == 0 {
        return tree2
            .children(j)
            .iter()
            .map(|&c| options.insert_cost * tree2.size(c as usize))
            .sum();
    }
    if n == 0 {
        return tree1
            .children(i)
            .iter()
            .map(|&c| options.delete_cost * tree1.size(c as usize))
            .sum();
    }

    // Recursive child distances are computed up front so the DP loop
    // below is pure arithmetic over the memo-backed matrix. Child index
    // slices are re-fetched by position to keep the borrows disjoint from
    // the `&mut memo` recursion.
    let mut pair_cost = vec![0.0f64; m * n];
    for a in 0..m {
        for b in 0..n {
            let c1 = tree1.children(i)[a] as usize;
            let c2 = tree2.children(j)[b] as usize;
            pair_cost[a * n + b] = subtree_distance(tree1, tree2, options, memo, c1, c2);
        }
    }

    let children1 = tree1.children(i);
    let children2 = tree2.children(j);

    // Single-row DP: row[b] holds dp[a][b] while sweeping a upward.
    let mut row = vec![0.0f64; n + 1];
    for b in 1..=n {
        row[b] = row[b - 1] + options.insert_cost * tree2.size(children2[b - 1] as usize);
    }
    for a in 1..=m {
        let delete_cost = options.delete_cost * tree1.size(children1[a - 1] as usize);
        let mut diagonal = row[0];
        row[0] += delete_cost;
        for b in 1..=n {
            let insert_cost = options.insert_cost * tree2.size(children2[b - 1] as usize);
            let match_cost = diagonal + pair_cost[(a - 1) * n + (b - 1)];
            diagonal = row[b];
            row[b] = (row[b] + delete_cost).min(row[b - 1] + insert_cost).min(match_cost);
        }
    }
    row[n]
}

#[must_use]
pub fn compute_edit_distance(
    tree1: &Rc<TreeNode>,
    tree2: &Rc<TreeNode>,
    options: &APTEDOptions,
) -> f64 {
    let flat1 = FlatTree::build(tree1);
    let flat2 = FlatTree::build(tree2);
    let mut memo = vec![f64::NAN; flat1.len() * flat2.len()];
    subtree_distance(&flat1, &flat2, options, &mut memo, 0, 0)
}

/// Compute the edit distance, aborting as soon as a partial result proves
/// it cannot stay within `max_distance`. Callers should treat any return
/// value `>= DISTANCE_EXCEEDED` as "the distance is at least
/// `max_distance`, exact value unknown".
///
/// The cutoff is a pure performance hint — when the budget is generous
/// the function returns the exact distance and the result matches
/// `compute_edit_distance`. The TSED layer uses it to skip pairs that
/// cannot reach the user-requested similarity threshold.
#[must_use]
pub fn compute_edit_distance_with_cutoff(
    tree1: &Rc<TreeNode>,
    tree2: &Rc<TreeNode>,
    options: &APTEDOptions,
    max_distance: f64,
) -> f64 {
    let flat1 = FlatTree::build(tree1);
    let flat2 = FlatTree::build(tree2);

    // Sound global lower bound: every edit script has to bridge the size
    // gap one delete or insert at a time. (This holds because the replace
    // branch in `distance` charges delete + insert — see the comment
    // there.)
    let min_op_cost = options.delete_cost.min(options.insert_cost);
    let size_gap = (flat1.size(0) - flat2.size(0)).abs();
    if size_gap * min_op_cost > max_distance {
        return DISTANCE_EXCEEDED;
    }

    // Label-histogram lower bound. Any edit script maps nodes 1:1 (plus
    // deletes/inserts); a mapped pair costs 0 only when label AND kind
    // agree, so at most `overlap` node pairs are free and every one of
    // the remaining `max(n1, n2) - overlap` nodes on the larger side
    // costs at least the cheapest operation. This is O(n1 + n2) and
    // prunes the "similar size, unrelated content" pairs that the pure
    // size-gap bound cannot touch — the case where skipping the full
    // O(n1·n2) DP actually matters.
    let cheapest_edit = options
        .delete_cost
        .min(options.insert_cost)
        .min(options.rename_cost);
    if cheapest_edit > 0.0 {
        let mut labels: std::collections::HashMap<(&str, &str), i64> =
            std::collections::HashMap::with_capacity(flat1.len());
        for index in 0..flat1.len() {
            *labels.entry((flat1.labels[index], flat1.values[index])).or_insert(0) += 1;
        }
        let mut overlap = 0usize;
        for index in 0..flat2.len() {
            if let Some(count) = labels.get_mut(&(flat2.labels[index], flat2.values[index])) {
                if *count > 0 {
                    *count -= 1;
                    overlap += 1;
                }
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let histogram_bound =
            (flat1.len().max(flat2.len()) - overlap) as f64 * cheapest_edit;
        if histogram_bound > max_distance {
            return DISTANCE_EXCEEDED;
        }
    }

    let mut memo = vec![f64::NAN; flat1.len() * flat2.len()];
    let distance = subtree_distance(&flat1, &flat2, options, &mut memo, 0, 0);
    if distance > max_distance {
        DISTANCE_EXCEEDED
    } else {
        distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(label: &str, kind: &str, id: usize) -> Rc<TreeNode> {
        Rc::new(TreeNode::new(label.to_string(), kind.to_string(), id))
    }

    fn node(label: &str, kind: &str, id: usize, children: Vec<Rc<TreeNode>>) -> Rc<TreeNode> {
        let mut inner = TreeNode::new(label.to_string(), kind.to_string(), id);
        for child in children {
            inner.add_child(child);
        }
        Rc::new(inner)
    }

    fn unit_options() -> APTEDOptions {
        APTEDOptions {
            rename_cost: 1.0,
            delete_cost: 1.0,
            insert_cost: 1.0,
            compare_values: false,
        }
    }

    #[test]
    fn identical_trees_have_zero_distance() {
        let t1 = node("A", "K", 0, vec![leaf("x", "Identifier", 1), leaf("y", "Identifier", 2)]);
        let t2 = node("A", "K", 0, vec![leaf("x", "Identifier", 1), leaf("y", "Identifier", 2)]);
        assert_eq!(compute_edit_distance(&t1, &t2, &unit_options()), 0.0);
    }

    #[test]
    fn replace_branch_charges_delete_plus_insert() {
        // A single leaf vs a 3-node subtree with nothing in common: the
        // distance must cover deleting one node and inserting three (or
        // renaming + inserting two), never the old min(delete, insert).
        let t1 = leaf("a", "Identifier", 0);
        let t2 = node("B", "K", 0, vec![leaf("c", "Identifier", 1), leaf("d", "Identifier", 2)]);
        let distance = compute_edit_distance(&t1, &t2, &unit_options());
        // rename a→B is cross-kind (cost min(3, 1.98)) + insert 2 children = 3.98,
        // replace = 1 + 3 = 4. The rename path is cheaper.
        assert!(distance >= 3.0, "distance {distance} must not undercut the size gap");
    }

    #[test]
    fn distance_respects_size_gap_lower_bound() {
        // Unrelated subtrees where one side is much larger: distance must
        // be at least the size gap.
        let big = node(
            "Root",
            "K",
            0,
            (1..=10).map(|i| leaf(&format!("n{i}"), "Identifier", i)).collect(),
        );
        let small = node("Root", "K", 0, vec![leaf("n1", "Identifier", 1)]);
        let distance = compute_edit_distance(&big, &small, &unit_options());
        assert!(distance >= 9.0, "distance {distance} must cover the 9-node gap");
    }

    #[test]
    fn cutoff_matches_exact_distance_when_budget_allows() {
        let t1 = node(
            "F",
            "K",
            0,
            vec![leaf("a", "Identifier", 1), leaf("b", "Identifier", 2), leaf("c", "Identifier", 3)],
        );
        let t2 = node(
            "F",
            "K",
            0,
            vec![leaf("a", "Identifier", 1), leaf("x", "Identifier", 2), leaf("c", "Identifier", 3)],
        );
        let options = unit_options();
        let exact = compute_edit_distance(&t1, &t2, &options);
        let with_budget = compute_edit_distance_with_cutoff(&t1, &t2, &options, 100.0);
        assert!((exact - with_budget).abs() < 1e-12);
    }

    #[test]
    fn cutoff_flags_pairs_beyond_budget() {
        let t1 = node(
            "F",
            "K",
            0,
            (1..=12).map(|i| leaf(&format!("a{i}"), "Identifier", i)).collect(),
        );
        let t2 = node("G", "K", 0, vec![leaf("z", "Identifier", 1)]);
        let distance = compute_edit_distance_with_cutoff(&t1, &t2, &unit_options(), 1.0);
        assert!(distance >= DISTANCE_EXCEEDED);
    }

    #[test]
    fn cutoff_prunes_same_size_unrelated_trees() {
        // Equal sizes defeat the size-gap bound; the label-histogram bound
        // must still flag the pair without running the full DP.
        let t1 = node(
            "F",
            "K",
            0,
            (1..=12).map(|i| leaf(&format!("a{i}"), "Identifier", i)).collect(),
        );
        let t2 = node(
            "F",
            "K",
            0,
            (1..=12).map(|i| leaf(&format!("z{i}"), "Identifier", i)).collect(),
        );
        let options = APTEDOptions { rename_cost: 0.3, compare_values: false, ..unit_options() };
        let distance = compute_edit_distance_with_cutoff(&t1, &t2, &options, 1.0);
        assert!(distance >= DISTANCE_EXCEEDED);
    }

    #[test]
    fn builtin_method_swap_costs_more_than_identifier_rename() {
        let options = APTEDOptions { rename_cost: 0.3, compare_values: false, ..unit_options() };
        let map1 = leaf("map", "Identifier", 0);
        let filter = leaf("filter", "Identifier", 0);
        let local1 = leaf("total", "Identifier", 0);
        let local2 = leaf("sum", "Identifier", 0);
        let builtin_swap = compute_edit_distance(&map1, &filter, &options);
        let plain_rename = compute_edit_distance(&local1, &local2, &options);
        assert!(builtin_swap > plain_rename);
        assert!((plain_rename - 0.3).abs() < 1e-12);
    }
}
