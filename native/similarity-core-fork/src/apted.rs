use crate::tree::TreeNode;
use std::hash::{Hash, Hasher};
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

/// Largest pair of child lists — in cells of their cost matrix — that is
/// aligned exactly.
///
/// The exact alignment compares every child of one node with every child
/// of the other: time and memory grow with the product of the two list
/// lengths. That is nothing for the lists code has (statements of a block,
/// arguments of a call) and ruinous for the ones data has — two functions
/// that each return a table of ten thousand entries used to take minutes
/// and gigabytes, and brought the whole run down when the allocation
/// failed. Above this size the lists are aligned along the children they
/// share instead (see [`Alignment::align_wide`]).
const EXACT_ALIGNMENT_CELLS: usize = 1 << 20;

/// Most children two long lists may differ by — inserted plus deleted —
/// for the children they share to be looked for (see
/// [`common_subsequence`]). The search takes time and memory that grow
/// with this number; lists further apart than this are not alike.
const MAX_LIST_EDITS: usize = 2000;

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
fn is_identifier_like_kind(kind: &str) -> bool {
    matches!(kind, "Identifier" | "BindingIdentifier" | "PrivateIdentifier" | "Parameter")
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
///   * same kind, differing label, identifier-like leaf → `rename_cost`
///     (e.g. parameter or variable rename — neutral refactor), EXCEPT when
///     both labels are well-known builtin member names — swapping `.map`
///     for `.filter` is a behavioural rewrite and pays the within-kind cap
///   * same kind, differing label, non-identifier-like (operator, method
///     key, literal value, etc.) → `rename_cost * 2.0` (semantic change
///     within a syntactic category)
///   * differing kind → `rename_cost * 3.0` (subtree-shape
///     change — e.g. CallExpression vs BinaryExpression). Capped so that
///     APTED still prefers a rename over delete+insert when the rename
///     would otherwise be cheaper than delete + insert.
fn label_rename_cost(
    label1: &str,
    label2: &str,
    kind1: &str,
    kind2: &str,
    options: &APTEDOptions,
) -> f64 {
    let labels_match = label1 == label2;
    let kinds_match = kind1 == kind2;

    if labels_match && kinds_match {
        return 0.0;
    }

    if options.compare_values {
        // `compare_values: true` mode is used by the class/type
        // comparators where rich semantics on a single node matter less
        // than the surrounding declaration shape. Use the legacy flat
        // cost so those callers see the same behaviour they always did.
        return options.rename_cost;
    }

    if kinds_match {
        if is_identifier_like_kind(kind1) {
            if is_semantic_builtin_name(label1) && is_semantic_builtin_name(label2) {
                rename_cap(options)
            } else {
                options.rename_cost
            }
        } else {
            (options.rename_cost * 2.0).min(rename_cap(options))
        }
    } else {
        (options.rename_cost * 3.0).min(rename_cap(options))
    }
}

/// Maximum cost a rename may charge, whatever it renames.
///
/// The cap keeps the optimal alignment from ever preferring a
/// delete-then-insert over a straight rename: it stays strictly below
/// `delete + insert`. A fixed offset (`delete + insert - 0.01`) would
/// collapse to zero for callers that configure very small per-operation
/// costs, and a zero cap would make substitutions free, so the cap scales
/// with the configured costs instead.
fn rename_cap(options: &APTEDOptions) -> f64 {
    ((options.delete_cost + options.insert_cost) * 0.99).max(0.0)
}

// ---------------------------------------------------------------------------
// Flat tree representation
// ---------------------------------------------------------------------------
//
// The recursive `Rc<TreeNode>` shape is convenient to build but slow to
// compare. Each tree is flattened once per comparison into parallel arrays
// in pre-order, where a subtree is a contiguous run: the first child of
// node `i` is `i + 1`, and the sibling after a node `c` is `c + size(c)`.
// No child lists are stored at all.

struct FlatTree<'a> {
    labels: Vec<&'a str>,
    kinds: Vec<&'static str>,
    /// Number of nodes in the subtree rooted at each node.
    sizes: Vec<u32>,
}

impl<'a> FlatTree<'a> {
    fn build(root: &'a Rc<TreeNode>) -> FlatTree<'a> {
        let node_count = root.subtree_size();
        let mut tree = FlatTree {
            labels: Vec::with_capacity(node_count),
            kinds: Vec::with_capacity(node_count),
            sizes: Vec::with_capacity(node_count),
        };
        let mut stack: Vec<&'a TreeNode> = vec![root];
        while let Some(node) = stack.pop() {
            tree.labels.push(node.label.as_str());
            tree.kinds.push(node.kind);
            #[allow(clippy::cast_possible_truncation)]
            tree.sizes.push(node.subtree_size() as u32);
            stack.extend(node.children.iter().rev().map(Rc::as_ref));
        }
        tree
    }

    fn len(&self) -> usize {
        self.labels.len()
    }

    fn size(&self, index: usize) -> f64 {
        f64::from(self.sizes[index])
    }

    fn is_leaf(&self, index: usize) -> bool {
        self.sizes[index] == 1
    }

    /// Append the indices of `index`'s children to `out`, in order.
    fn push_children(&self, index: usize, out: &mut Vec<u32>) {
        let end = index + self.sizes[index] as usize;
        let mut child = index + 1;
        while child < end {
            #[allow(clippy::cast_possible_truncation)]
            out.push(child as u32);
            child += self.sizes[child] as usize;
        }
    }

    /// A hash of every subtree: equal for subtrees with the same labels,
    /// kinds and shape.
    fn subtree_hashes(&self) -> Vec<u64> {
        let mut hashes = vec![0u64; self.len()];
        // Children come after their parent in pre-order, so walking
        // backwards meets every child before its parent.
        for index in (0..self.len()).rev() {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.labels[index].hash(&mut hasher);
            self.kinds[index].hash(&mut hasher);
            let end = index + self.sizes[index] as usize;
            let mut child = index + 1;
            while child < end {
                hashes[child].hash(&mut hasher);
                child += self.sizes[child] as usize;
            }
            hashes[index] = hasher.finish();
        }
        hashes
    }
}

/// One edit-distance computation between two flattened trees.
///
/// Every pair of nodes is visited at most once — a pair is only ever
/// reached through the pair of its parents — so nothing is memoized. (A
/// dense `n1 × n2` memo table used to be allocated and filled per
/// comparison; nothing ever read an entry back, and for two large
/// functions the table alone ran to gigabytes.) The child-cost matrices
/// and alignment rows live on two scratch stacks that are reused across
/// the whole recursion, so a comparison allocates a handful of times, not
/// once per node pair.
struct Alignment<'t, 'a> {
    tree1: &'t FlatTree<'a>,
    tree2: &'t FlatTree<'a>,
    options: &'t APTEDOptions,
    /// Stack of child-cost matrices and alignment rows.
    costs: Vec<f64>,
    /// Stack of child index lists.
    children: Vec<u32>,
    /// Subtree hashes of both trees, computed when the first pair of
    /// child lists too long to align exactly is met.
    subtree_hashes: Option<(Vec<u64>, Vec<u64>)>,
    /// Largest cost matrix aligned exactly, in cells.
    exact_cells: usize,
}

/// A run of one node's children on the child stack: `start..end`.
type ChildRange = (usize, usize);

impl<'t, 'a> Alignment<'t, 'a> {
    fn new(tree1: &'t FlatTree<'a>, tree2: &'t FlatTree<'a>, options: &'t APTEDOptions) -> Self {
        Self {
            tree1,
            tree2,
            options,
            costs: Vec::new(),
            children: Vec::new(),
            subtree_hashes: None,
            exact_cells: EXACT_ALIGNMENT_CELLS,
        }
    }

    /// Exact distance between subtree `i` of `tree1` and subtree `j` of
    /// `tree2`: the cheaper of
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
    fn distance(&mut self, i: usize, j: usize) -> f64 {
        let rename_cost = label_rename_cost(
            self.tree1.labels[i],
            self.tree2.labels[j],
            self.tree1.kinds[i],
            self.tree2.kinds[j],
            self.options,
        );
        if self.tree1.is_leaf(i) && self.tree2.is_leaf(j) {
            return rename_cost;
        }
        let replace_cost = self.options.delete_cost * self.tree1.size(i)
            + self.options.insert_cost * self.tree2.size(j);
        let rename_plus_cost = rename_cost + self.align_children(i, j);
        replace_cost.min(rename_plus_cost)
    }

    /// Sequence alignment over the child lists of `tree1[i]` / `tree2[j]`:
    /// the minimum cost to turn one list into the other, where skipping a
    /// child costs deleting/inserting its whole subtree and matching a
    /// pair costs their recursive distance.
    fn align_children(&mut self, i: usize, j: usize) -> f64 {
        let children_base = self.children.len();
        self.tree1.push_children(i, &mut self.children);
        let left = (children_base, self.children.len());
        self.tree2.push_children(j, &mut self.children);
        let right = (left.1, self.children.len());

        let cost = if (left.1 - left.0).saturating_mul(right.1 - right.0) > self.exact_cells {
            self.align_wide(left, right)
        } else {
            self.align(left, right)
        };
        self.children.truncate(children_base);
        cost
    }

    /// Cost of deleting the left children in `range`.
    fn delete_cost_of(&self, range: ChildRange) -> f64 {
        self.children[range.0..range.1]
            .iter()
            .map(|&child| self.options.delete_cost * self.tree1.size(child as usize))
            .sum()
    }

    /// Cost of inserting the right children in `range`.
    fn insert_cost_of(&self, range: ChildRange) -> f64 {
        self.children[range.0..range.1]
            .iter()
            .map(|&child| self.options.insert_cost * self.tree2.size(child as usize))
            .sum()
    }

    /// The exact alignment of the left children in `left` with the right
    /// children in `right`.
    fn align(&mut self, left: ChildRange, right: ChildRange) -> f64 {
        let (m, n) = (left.1 - left.0, right.1 - right.0);
        if m == 0 {
            return self.insert_cost_of(right);
        }
        if n == 0 {
            return self.delete_cost_of(left);
        }

        // This call's slice of the cost stack: an `m × n` matrix of child
        // pair distances, then one alignment row of `n + 1` cells. The
        // recursion below pushes and pops above it, so it is addressed by
        // index, never held as a slice.
        let pair_base = self.costs.len();
        let row_base = pair_base + m * n;
        self.costs.resize(row_base + n + 1, 0.0);

        for a in 0..m {
            for b in 0..n {
                let child1 = self.children[left.0 + a] as usize;
                let child2 = self.children[right.0 + b] as usize;
                let cost = self.distance(child1, child2);
                self.costs[pair_base + a * n + b] = cost;
            }
        }

        let (delete_unit, insert_unit) = (self.options.delete_cost, self.options.insert_cost);
        let delete_cost_of = |this: &Self, a: usize| {
            delete_unit * this.tree1.size(this.children[left.0 + a] as usize)
        };
        let insert_cost_of = |this: &Self, b: usize| {
            insert_unit * this.tree2.size(this.children[right.0 + b] as usize)
        };

        // Single-row DP: `row[b]` holds the cost of aligning the first `a`
        // left children with the first `b` right children while `a` sweeps
        // upward.
        self.costs[row_base] = 0.0;
        for b in 1..=n {
            self.costs[row_base + b] = self.costs[row_base + b - 1] + insert_cost_of(self, b - 1);
        }
        for a in 1..=m {
            let delete_cost = delete_cost_of(self, a - 1);
            let mut diagonal = self.costs[row_base];
            self.costs[row_base] += delete_cost;
            for b in 1..=n {
                let insert_cost = insert_cost_of(self, b - 1);
                let match_cost = diagonal + self.costs[pair_base + (a - 1) * n + (b - 1)];
                diagonal = self.costs[row_base + b];
                self.costs[row_base + b] = (self.costs[row_base + b] + delete_cost)
                    .min(self.costs[row_base + b - 1] + insert_cost)
                    .min(match_cost);
            }
        }

        let cost = self.costs[row_base + n];
        self.costs.truncate(pair_base);
        cost
    }

    /// Alignment of two child lists too long to align exactly (see
    /// [`EXACT_ALIGNMENT_CELLS`]): the cost of one valid alignment, found
    /// along the children both lists share.
    ///
    /// Equal subtrees cost nothing to match. So the children the two lists
    /// have in common, in order — their longest common subsequence — are
    /// matched outright, and only what lies between two matches is aligned
    /// by cost: a child that changed against its counterpart, a few that
    /// were added against nothing. A stretch that is still too long and
    /// shares nothing is cut into pieces that are not.
    ///
    /// For lists that are mostly the same — the only ones a long list can
    /// be similar to — this finds the alignment the exact computation
    /// would, at a cost that grows with the lists and their differences
    /// instead of with their product. For others the result may exceed the
    /// exact distance; it never falls below it.
    fn align_wide(&mut self, left: ChildRange, right: ChildRange) -> f64 {
        if self.subtree_hashes.is_none() {
            self.subtree_hashes = Some((self.tree1.subtree_hashes(), self.tree2.subtree_hashes()));
        }

        let mut cost = 0.0;
        let mut pending: Vec<(ChildRange, ChildRange)> = vec![(left, right)];
        while let Some((left, right)) = pending.pop() {
            let (left, right) = self.trim_shared_ends(left, right);
            let (m, n) = (left.1 - left.0, right.1 - right.0);
            if m.saturating_mul(n) <= self.exact_cells {
                cost += self.align(left, right);
                continue;
            }
            let shared = self.shared_children(left, right);
            if shared.is_empty() {
                cost += self.align_in_pieces(left, right);
                continue;
            }
            // The stretches before, between and after the shared children.
            let (mut left_start, mut right_start) = (left.0, right.0);
            for (left_shared, right_shared) in shared {
                pending.push(((left_start, left_shared), (right_start, right_shared)));
                left_start = left_shared + 1;
                right_start = right_shared + 1;
            }
            pending.push(((left_start, left.1), (right_start, right.1)));
        }
        cost
    }

    /// The subtree hash of the left child at `position` of the child stack.
    fn left_hash(&self, position: usize) -> u64 {
        let (hashes, _) = self.subtree_hashes.as_ref().expect("hashes are computed first");
        hashes[self.children[position] as usize]
    }

    /// The subtree hash of the right child at `position` of the child
    /// stack.
    fn right_hash(&self, position: usize) -> u64 {
        let (_, hashes) = self.subtree_hashes.as_ref().expect("hashes are computed first");
        hashes[self.children[position] as usize]
    }

    /// `left` and `right` without the equal children they start and end
    /// with.
    fn trim_shared_ends(
        &self,
        mut left: ChildRange,
        mut right: ChildRange,
    ) -> (ChildRange, ChildRange) {
        while left.0 < left.1
            && right.0 < right.1
            && self.left_hash(left.0) == self.right_hash(right.0)
        {
            left.0 += 1;
            right.0 += 1;
        }
        while left.0 < left.1
            && right.0 < right.1
            && self.left_hash(left.1 - 1) == self.right_hash(right.1 - 1)
        {
            left.1 -= 1;
            right.1 -= 1;
        }
        (left, right)
    }

    /// The equal children of `left` and `right`, in order, as `(left
    /// position, right position)` pairs; none when the lists are too far
    /// apart to look (see [`MAX_LIST_EDITS`]).
    fn shared_children(&self, left: ChildRange, right: ChildRange) -> Vec<(usize, usize)> {
        let left_hashes: Vec<u64> =
            (left.0..left.1).map(|position| self.left_hash(position)).collect();
        let right_hashes: Vec<u64> =
            (right.0..right.1).map(|position| self.right_hash(position)).collect();
        common_subsequence(&left_hashes, &right_hashes, MAX_LIST_EDITS)
            .unwrap_or_default()
            .into_iter()
            .map(|(in_left, in_right)| (left.0 + in_left, right.0 + in_right))
            .collect()
    }

    /// Alignment of two long lists that share no anchor: both are cut into
    /// the same number of pieces, small enough to align exactly, and the
    /// pieces are aligned in order.
    fn align_in_pieces(&mut self, left: ChildRange, right: ChildRange) -> f64 {
        let (m, n) = (left.1 - left.0, right.1 - right.0);
        let mut pieces = 2usize;
        while m.div_ceil(pieces).saturating_mul(n.div_ceil(pieces)) > self.exact_cells.max(1) {
            pieces += 1;
        }
        let piece = |range: ChildRange, length: usize, index: usize| -> ChildRange {
            (range.0 + length * index / pieces, range.0 + length * (index + 1) / pieces)
        };
        (0..pieces).map(|index| self.align(piece(left, m, index), piece(right, n, index))).sum()
    }
}

/// A longest common subsequence of `left` and `right`, as pairs of
/// positions, or `None` when turning one list into the other takes more
/// than `max_edits` insertions and deletions.
///
/// Myers' greedy algorithm: for a growing number of edits, the furthest
/// point reachable on each diagonal of the edit graph. Time and memory
/// grow with the lists' length times their difference — not with the
/// product of their lengths.
fn common_subsequence(
    left: &[u64],
    right: &[u64],
    max_edits: usize,
) -> Option<Vec<(usize, usize)>> {
    let (left_len, right_len) = (left.len() as isize, right.len() as isize);
    let max_edits = max_edits.min(left.len() + right.len()) as isize;
    // `furthest[offset + k]`: the largest position in `left` reached on
    // diagonal `k` (position in `left` minus position in `right`).
    let offset = max_edits + 1;
    let mut furthest = vec![0isize; (2 * max_edits + 3) as usize];
    // The reachable diagonals after each number of edits, to walk back
    // through.
    let mut rounds: Vec<Vec<isize>> = Vec::new();

    for edits in 0..=max_edits {
        let mut diagonal = -edits;
        while diagonal <= edits {
            let below = furthest[(offset + diagonal - 1) as usize];
            let above = furthest[(offset + diagonal + 1) as usize];
            // Reach this diagonal by inserting from the one above, or by
            // deleting from the one below — whichever got further.
            let mut in_left = if diagonal == -edits || (diagonal != edits && below < above) {
                above
            } else {
                below + 1
            };
            let mut in_right = in_left - diagonal;
            while in_left < left_len
                && in_right < right_len
                && left[in_left as usize] == right[in_right as usize]
            {
                in_left += 1;
                in_right += 1;
            }
            furthest[(offset + diagonal) as usize] = in_left;
            if in_left >= left_len && in_right >= right_len {
                return Some(walk_back(&rounds, left_len, right_len));
            }
            diagonal += 2;
        }
        rounds.push(furthest[(offset - edits) as usize..=(offset + edits) as usize].to_vec());
    }
    None
}

/// The matches along the path [`common_subsequence`] found: back from the
/// ends of both lists, through the state after each number of edits.
/// `rounds[e]` holds the diagonals `-e..=e`.
fn walk_back(rounds: &[Vec<isize>], left_len: isize, right_len: isize) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let (mut in_left, mut in_right) = (left_len, right_len);
    for edits in (1..=rounds.len() as isize).rev() {
        let previous = &rounds[(edits - 1) as usize];
        // `previous` covers the diagonals `-(edits - 1)..=(edits - 1)`.
        let reached = |diagonal: isize| previous[(diagonal + edits - 1) as usize];
        let diagonal = in_left - in_right;
        let from = if diagonal == -edits
            || (diagonal != edits && reached(diagonal - 1) < reached(diagonal + 1))
        {
            diagonal + 1
        } else {
            diagonal - 1
        };
        let from_left = reached(from);
        let from_right = from_left - from;
        // The matches run back to where the edit landed.
        let (landed_left, landed_right) = if from == diagonal + 1 {
            (from_left, from_right + 1)
        } else {
            (from_left + 1, from_right)
        };
        while in_left > landed_left && in_right > landed_right {
            in_left -= 1;
            in_right -= 1;
            matches.push((in_left as usize, in_right as usize));
        }
        in_left = from_left;
        in_right = from_right;
    }
    // Before the first edit both lists run in step.
    while in_left > 0 && in_right > 0 {
        in_left -= 1;
        in_right -= 1;
        matches.push((in_left as usize, in_right as usize));
    }
    matches.reverse();
    matches
}

#[must_use]
pub fn compute_edit_distance(
    tree1: &Rc<TreeNode>,
    tree2: &Rc<TreeNode>,
    options: &APTEDOptions,
) -> f64 {
    let flat1 = FlatTree::build(tree1);
    let flat2 = FlatTree::build(tree2);
    Alignment::new(&flat1, &flat2, options).distance(0, 0)
}

/// Compute the edit distance, giving up as soon as a lower bound proves
/// it cannot stay within `max_distance`. Callers should treat any return
/// value `>= DISTANCE_EXCEEDED` as "the distance is more than
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
    // branch in `Alignment::distance` charges delete + insert — see the
    // comment there.)
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
    // alignment actually matters.
    let cheapest_edit = options.delete_cost.min(options.insert_cost).min(options.rename_cost);
    if cheapest_edit > 0.0 {
        let mut labels: std::collections::HashMap<(&str, &str), i64> =
            std::collections::HashMap::with_capacity(flat1.len());
        for index in 0..flat1.len() {
            *labels.entry((flat1.labels[index], flat1.kinds[index])).or_insert(0) += 1;
        }
        let mut overlap = 0usize;
        for index in 0..flat2.len() {
            if let Some(count) = labels.get_mut(&(flat2.labels[index], flat2.kinds[index])) {
                if *count > 0 {
                    *count -= 1;
                    overlap += 1;
                }
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let histogram_bound = (flat1.len().max(flat2.len()) - overlap) as f64 * cheapest_edit;
        if histogram_bound > max_distance {
            return DISTANCE_EXCEEDED;
        }
    }

    let distance = Alignment::new(&flat1, &flat2, options).distance(0, 0);
    if distance > max_distance {
        DISTANCE_EXCEEDED
    } else {
        distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(label: &str, kind: &'static str) -> Rc<TreeNode> {
        Rc::new(TreeNode::new(label, kind))
    }

    fn node(label: &str, kind: &'static str, children: Vec<Rc<TreeNode>>) -> Rc<TreeNode> {
        let mut inner = TreeNode::new(label, kind);
        for child in children {
            inner.add_child(child);
        }
        Rc::new(inner)
    }

    fn unit_options() -> APTEDOptions {
        APTEDOptions { rename_cost: 1.0, delete_cost: 1.0, insert_cost: 1.0, compare_values: false }
    }

    #[test]
    fn identical_trees_have_zero_distance() {
        let t1 = node("A", "K", vec![leaf("x", "Identifier"), leaf("y", "Identifier")]);
        let t2 = node("A", "K", vec![leaf("x", "Identifier"), leaf("y", "Identifier")]);
        assert_eq!(compute_edit_distance(&t1, &t2, &unit_options()), 0.0);
    }

    #[test]
    fn replace_branch_charges_delete_plus_insert() {
        // A single leaf vs a 3-node subtree with nothing in common: the
        // distance must cover deleting one node and inserting three (or
        // renaming + inserting two), never the old min(delete, insert).
        let t1 = leaf("a", "Identifier");
        let t2 = node("B", "K", vec![leaf("c", "Identifier"), leaf("d", "Identifier")]);
        let distance = compute_edit_distance(&t1, &t2, &unit_options());
        // rename a→B is cross-kind (cost min(3, 1.98)) + insert 2 children = 3.98,
        // replace = 1 + 3 = 4. The rename path is cheaper.
        assert!(distance >= 3.0, "distance {distance} must not undercut the size gap");
    }

    #[test]
    fn distance_respects_size_gap_lower_bound() {
        // Unrelated subtrees where one side is much larger: distance must
        // be at least the size gap.
        let big =
            node("Root", "K", (1..=10).map(|i| leaf(&format!("n{i}"), "Identifier")).collect());
        let small = node("Root", "K", vec![leaf("n1", "Identifier")]);
        let distance = compute_edit_distance(&big, &small, &unit_options());
        assert!(distance >= 9.0, "distance {distance} must cover the 9-node gap");
    }

    #[test]
    fn cutoff_matches_exact_distance_when_budget_allows() {
        let t1 = node(
            "F",
            "K",
            vec![leaf("a", "Identifier"), leaf("b", "Identifier"), leaf("c", "Identifier")],
        );
        let t2 = node(
            "F",
            "K",
            vec![leaf("a", "Identifier"), leaf("x", "Identifier"), leaf("c", "Identifier")],
        );
        let options = unit_options();
        let exact = compute_edit_distance(&t1, &t2, &options);
        let with_budget = compute_edit_distance_with_cutoff(&t1, &t2, &options, 100.0);
        assert!((exact - with_budget).abs() < 1e-12);
    }

    #[test]
    fn cutoff_flags_pairs_beyond_budget() {
        let t1 = node("F", "K", (1..=12).map(|i| leaf(&format!("a{i}"), "Identifier")).collect());
        let t2 = node("G", "K", vec![leaf("z", "Identifier")]);
        let distance = compute_edit_distance_with_cutoff(&t1, &t2, &unit_options(), 1.0);
        assert!(distance >= DISTANCE_EXCEEDED);
    }

    #[test]
    fn cutoff_prunes_same_size_unrelated_trees() {
        // Equal sizes defeat the size-gap bound; the label-histogram bound
        // must still flag the pair without running the full DP.
        let t1 = node("F", "K", (1..=12).map(|i| leaf(&format!("a{i}"), "Identifier")).collect());
        let t2 = node("F", "K", (1..=12).map(|i| leaf(&format!("z{i}"), "Identifier")).collect());
        let options = APTEDOptions { rename_cost: 0.3, compare_values: false, ..unit_options() };
        let distance = compute_edit_distance_with_cutoff(&t1, &t2, &options, 1.0);
        assert!(distance >= DISTANCE_EXCEEDED);
    }

    #[test]
    fn builtin_method_swap_costs_more_than_identifier_rename() {
        let options = APTEDOptions { rename_cost: 0.3, compare_values: false, ..unit_options() };
        let map1 = leaf("map", "Identifier");
        let filter = leaf("filter", "Identifier");
        let local1 = leaf("total", "Identifier");
        let local2 = leaf("sum", "Identifier");
        let builtin_swap = compute_edit_distance(&map1, &filter, &options);
        let plain_rename = compute_edit_distance(&local1, &local2, &options);
        assert!(builtin_swap > plain_rename);
        assert!((plain_rename - 0.3).abs() < 1e-12);
    }

    /// The distance with child lists aligned exactly up to `exact_cells`
    /// cells.
    fn distance_with_limit(t1: &Rc<TreeNode>, t2: &Rc<TreeNode>, exact_cells: usize) -> f64 {
        let options = APTEDOptions { rename_cost: 0.3, compare_values: false, ..unit_options() };
        let (flat1, flat2) = (FlatTree::build(t1), FlatTree::build(t2));
        let mut alignment = Alignment::new(&flat1, &flat2, &options);
        alignment.exact_cells = exact_cells;
        alignment.distance(0, 0)
    }

    /// `log(<name>, <number>);`-like statement.
    fn statement(name: &str, number: usize) -> Rc<TreeNode> {
        node(
            "ExpressionStatement",
            "ExpressionStatement",
            vec![node(
                "CallExpression",
                "CallExpression",
                vec![
                    leaf("log", "Identifier"),
                    leaf(name, "Identifier"),
                    leaf(&number.to_string(), "NumericLiteral"),
                ],
            )],
        )
    }

    fn block(statements: Vec<Rc<TreeNode>>) -> Rc<TreeNode> {
        node("BlockStatement", "BlockStatement", statements)
    }

    /// Deterministic pseudo-random numbers.
    fn random_numbers(seed: u64) -> impl FnMut(usize) -> usize {
        let mut state = seed;
        move |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as usize) % bound
        }
    }

    /// The length of the longest common subsequence, by the textbook
    /// table.
    fn subsequence_length(left: &[u64], right: &[u64]) -> usize {
        let mut table = vec![vec![0usize; right.len() + 1]; left.len() + 1];
        for (i, a) in left.iter().enumerate() {
            for (j, b) in right.iter().enumerate() {
                table[i + 1][j + 1] =
                    if a == b { table[i][j] + 1 } else { table[i][j + 1].max(table[i + 1][j]) };
            }
        }
        table[left.len()][right.len()]
    }

    #[test]
    fn common_subsequences_are_found() {
        let found = |left: &[u64], right: &[u64]| common_subsequence(left, right, 100).unwrap();
        assert_eq!(found(&[], &[]), []);
        assert_eq!(found(&[1, 2, 3], &[]), []);
        assert_eq!(found(&[], &[1, 2, 3]), []);
        assert_eq!(found(&[1, 2, 3], &[1, 2, 3]), [(0, 0), (1, 1), (2, 2)]);
        assert_eq!(found(&[1, 2, 3], &[4, 5, 6]), []);
        // One changed, one inserted, one removed.
        assert_eq!(found(&[1, 2, 3], &[1, 9, 3]), [(0, 0), (2, 2)]);
        assert_eq!(found(&[1, 2, 3], &[1, 2, 9, 3]), [(0, 0), (1, 1), (2, 3)]);
        assert_eq!(found(&[1, 2, 9, 3], &[1, 2, 3]), [(0, 0), (1, 1), (3, 2)]);
        // Repeated values: the lone 7s sit at opposite ends and must not
        // be matched at the price of everything between them.
        let left = [7, 1, 2, 1, 2, 1, 2];
        let right = [1, 2, 1, 2, 1, 2, 7];
        assert_eq!(found(&left, &right).len(), 6);
    }

    #[test]
    fn common_subsequences_are_longest_and_in_order() {
        let mut next = random_numbers(11);
        for round in 0..300 {
            let alphabet = 1 + next(6);
            let left: Vec<u64> = (0..next(40)).map(|_| next(alphabet) as u64).collect();
            let right: Vec<u64> = (0..next(40)).map(|_| next(alphabet) as u64).collect();
            let found = common_subsequence(&left, &right, usize::MAX).unwrap();
            assert_eq!(found.len(), subsequence_length(&left, &right), "round {round}");
            for &(in_left, in_right) in &found {
                assert_eq!(left[in_left], right[in_right], "round {round}");
            }
            assert!(
                found.windows(2).all(|pair| pair[0].0 < pair[1].0 && pair[0].1 < pair[1].1),
                "round {round}: {found:?}"
            );
            // The edits it takes are the elements left unmatched.
            let edits = left.len() + right.len() - 2 * found.len();
            assert!(common_subsequence(&left, &right, edits).is_some(), "round {round}");
            if edits > 0 {
                assert!(common_subsequence(&left, &right, edits - 1).is_none(), "round {round}");
            }
        }
    }

    #[test]
    fn long_lists_that_are_mostly_the_same_align_as_the_exact_computation_does() {
        // 60 distinct statements; each variant applies a few edits. With
        // the limit at 16 cells every list here is "too long".
        let base: Vec<Rc<TreeNode>> = (0..60).map(|i| statement(&format!("v{i}"), i)).collect();
        let mut variants: Vec<Vec<Rc<TreeNode>>> = Vec::new();
        // One statement changed, inserted or removed, at the start, in
        // the middle and at the end.
        for position in [0, 1, 29, 30, 58, 59] {
            let mut changed = base.clone();
            changed[position] = statement("other", 1000 + position);
            variants.push(changed);
            let mut inserted = base.clone();
            inserted.insert(position, statement("extra", 2000 + position));
            variants.push(inserted);
            let mut removed = base.clone();
            removed.remove(position);
            variants.push(removed);
        }
        // Several edits spread over the list.
        let mut scattered = base.clone();
        scattered[5] = statement("a", 3001);
        scattered.remove(20);
        scattered.insert(40, statement("b", 3002));
        scattered[55] = statement("c", 3003);
        variants.push(scattered);

        let left = block(base.clone());
        assert_eq!(distance_with_limit(&left, &block(base.clone()), 16), 0.0);

        // A moved block is where the two part ways: the statements that
        // stayed are matched, so the block is deleted and inserted, while
        // the exact computation shifts every statement in between onto
        // its neighbour at the price of renames.
        let mut moved = base;
        let chunk: Vec<Rc<TreeNode>> = moved.drain(10..15).collect();
        moved.splice(35..35, chunk);
        let right = block(moved);
        let exact = distance_with_limit(&left, &right, usize::MAX);
        let wide = distance_with_limit(&left, &right, 16);
        assert!(exact < wide, "exact {exact}, wide {wide}");
        // Five statements of five nodes, deleted and inserted.
        assert!((wide - 50.0).abs() < 1e-9, "{wide}");

        for (index, variant) in variants.into_iter().enumerate() {
            let right = block(variant);
            let exact = distance_with_limit(&left, &right, usize::MAX);
            let wide = distance_with_limit(&left, &right, 16);
            assert!(exact > 0.0);
            assert!((wide - exact).abs() < 1e-9, "variant {index}: exact {exact}, wide {wide}");
            // And the other way around.
            let exact = distance_with_limit(&right, &left, usize::MAX);
            let wide = distance_with_limit(&right, &left, 16);
            assert!((wide - exact).abs() < 1e-9, "variant {index} reversed: {exact} vs {wide}");
        }
    }

    #[test]
    fn the_alignment_of_long_lists_never_undercuts_the_exact_distance() {
        // Random lists over a small alphabet: repeated statements, few
        // unique anchors, sometimes none.
        let mut next = random_numbers(7);
        for round in 0..60 {
            let alphabet = 2 + next(12);
            let mut list = |length: usize| -> Vec<Rc<TreeNode>> {
                (0..length)
                    .map(|_| {
                        let symbol = next(alphabet);
                        statement(&format!("v{symbol}"), symbol)
                    })
                    .collect()
            };
            let left = block(list(30 + round % 25));
            let right = block(list(30 + (round * 7) % 25));
            let exact = distance_with_limit(&left, &right, usize::MAX);
            for limit in [1, 16, 100] {
                let wide = distance_with_limit(&left, &right, limit);
                assert!(wide >= exact - 1e-9, "round {round}: exact {exact}, wide {wide}");
                // Replacing everything is always possible.
                let replace = (left.subtree_size() + right.subtree_size()) as f64;
                assert!(wide <= replace + 1e-9, "round {round}: {wide} > {replace}");
            }
        }
    }

    #[test]
    fn repeated_entries_do_not_mislead_the_alignment_of_long_lists() {
        // A table of repeating values in which each side changes one entry
        // to the same new value, far apart. Matching the two new entries —
        // the only ones that occur once on each side — would cost the
        // whole table in between.
        let table = |changed: usize| {
            let entries: Vec<Rc<TreeNode>> = (0..1500)
                .map(|i| {
                    let value = if i == changed { 9999 } else { i % 7 };
                    leaf(&value.to_string(), "NumericLiteral")
                })
                .collect();
            node("ArrayExpression", "ArrayExpression", entries)
        };
        let options = APTEDOptions { rename_cost: 0.3, compare_values: false, ..unit_options() };
        let distance = compute_edit_distance(&table(10), &table(1400), &options);
        // Two entries renamed.
        assert!((distance - 2.0 * 0.6).abs() < 1e-9, "{distance}");
    }

    #[test]
    fn lists_longer_than_the_exact_limit_are_compared() {
        // 1500 × 1500 children is past the limit. Two tables that differ
        // in three entries are three renames apart; aligning them exactly
        // would take a matrix of 2.25 million cells.
        let table = |changed: &[usize]| {
            let entries: Vec<Rc<TreeNode>> = (0..1500)
                .map(|i| {
                    let value = if changed.contains(&i) { i + 5000 } else { i };
                    leaf(&value.to_string(), "NumericLiteral")
                })
                .collect();
            node("ArrayExpression", "ArrayExpression", entries)
        };
        let options = APTEDOptions { rename_cost: 0.3, compare_values: false, ..unit_options() };
        let distance = compute_edit_distance(&table(&[]), &table(&[3, 700, 1499]), &options);
        assert!((distance - 3.0 * 0.6).abs() < 1e-9, "{distance}");
        assert_eq!(compute_edit_distance(&table(&[]), &table(&[]), &options), 0.0);
    }
}
