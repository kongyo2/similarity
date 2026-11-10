//! Subtree fingerprints: what overlap mode compares functions by.
//!
//! Every subtree of a function's tree is hashed bottom-up, so two fragments
//! with the same structure and labels hash alike wherever they sit. Two
//! passes compare a pair of functions: equal hashes between their subtrees
//! (the same fragment in both), and windows of adjacent source subtrees
//! against the children of target subtrees (a fragment that starts or ends
//! off a subtree boundary on the source side).
//!
//! This module defines the comparison of one pair,
//! [`detect_partial_overlaps`]. The scan over a whole project lives in
//! [`crate::overlap_detector`] and finds the same overlaps without visiting
//! every pair.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::tree::TreeNode;

/// Matches smaller than this many nodes are suppressed when they cover
/// little of either function (see [`is_incidental_match`]).
pub(crate) const INCIDENTAL_MATCH_NODES: u32 = 8;

/// Distinct matching shapes between two functions from which small exact
/// matches stop being suppressed as incidental.
pub(crate) const MULTIPLICITY_SIGNAL_MATCHES: usize = 3;

/// The similarity from which a window match counts as exact.
pub(crate) const EXACT_SIMILARITY: f64 = 0.999;

/// Fingerprint of one subtree of a function's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtreeFingerprint {
    /// Number of nodes in the subtree.
    pub weight: u32,
    /// Hash of the subtree's structure and labels.
    pub hash: u64,
    /// Hashes of the root's direct children.
    pub child_hashes: Vec<u64>,
    /// First source line of the subtree (1-based).
    pub start_line: u32,
    /// Last source line of the subtree (1-based, inclusive).
    pub end_line: u32,
    /// First and last source line of the root's direct children: the
    /// subtree without what the root itself writes around them (the braces
    /// of a block, the keyword of a statement). The subtree's own lines
    /// when the root is a leaf.
    pub children_lines: (u32, u32),
    /// Syntactic kind of the subtree's root (`IfStatement`,
    /// `CallExpression`, …).
    pub node_type: &'static str,
}

/// A run of subtrees that follow each other in the source, compared as one
/// fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// Position of the first subtree among the function's subtrees in
    /// source order.
    pub start: usize,
    /// Summed weight of the subtrees.
    pub weight: u32,
    /// Hashes of the subtrees, in order.
    pub member_hashes: Vec<u64>,
    /// First source line of the run.
    pub start_line: u32,
    /// Last source line any of its subtrees reaches.
    pub end_line: u32,
}

/// One hash for the members of a window, in order: equal for two runs of
/// the same subtrees.
pub(crate) fn window_shape(member_hashes: &[u64]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    member_hashes.hash(&mut hasher);
    hasher.finish()
}

/// A window of the source that matches a subtree of the target, before
/// repeated fragments are paired off (see [`pair_repeated_windows`]).
#[derive(Debug, Clone)]
pub(crate) struct WindowMatch {
    /// Smallest window size the match is found at.
    pub window_size: u32,
    /// Where the window starts (see [`Window::start`]).
    pub start: usize,
    /// Rank of the matched subtree among the target's subtrees by
    /// position.
    pub rank: usize,
    /// What the window holds (see [`window_shape`]) and the hash of the
    /// subtree it matches.
    pub shape: (u64, u64),
    pub overlap: PartialOverlap,
}

/// The overlaps of `matches`, in the order the pairwise comparison finds
/// them (by window size, then window, then subtree), with repeated
/// fragments paired in order.
///
/// Every window of a shape matches every subtree of a shape alike, so a
/// run of statements both functions repeat matches each with each. Like
/// equal subtrees (see [`counterparts`]), the occurrences are paired in
/// order instead.
pub(crate) fn pair_repeated_windows(mut matches: Vec<WindowMatch>) -> Vec<PartialOverlap> {
    matches.sort_by_key(|found| (found.window_size, found.start, found.rank));

    // Per shape, where its windows start and which subtrees they match:
    // the occurrences on either side, in order.
    let mut occurrences: HashMap<(u64, u64), (Vec<usize>, Vec<usize>)> = HashMap::new();
    for found in &matches {
        let (starts, ranks) = occurrences.entry(found.shape).or_default();
        starts.push(found.start);
        ranks.push(found.rank);
    }
    for (starts, ranks) in occurrences.values_mut() {
        starts.sort_unstable();
        starts.dedup();
        ranks.sort_unstable();
        ranks.dedup();
    }

    // A window that stands for several sizes is met once per size; the
    // first meeting counts.
    let mut met: HashSet<(usize, usize, (u64, u64))> = HashSet::new();
    matches
        .into_iter()
        .filter(|found| {
            let (starts, ranks) = &occurrences[&found.shape];
            let occurrence = starts.partition_point(|&start| start < found.start);
            counterparts(occurrence, starts.len(), ranks).contains(&found.rank)
                && met.insert((found.start, found.rank, found.shape))
        })
        .map(|found| found.overlap)
        .collect()
}

/// Whether a window of `weight` nodes with these members may be the same
/// code as the children of `subtree`: the sizes are within
/// `size_tolerance` of each other and most of the smaller side's parts
/// appear on the other. A subtree without children has no parts to
/// compare a window with; the subtree a single member equals is an exact
/// match of the first pass.
pub(crate) fn window_might_match(
    weight: u32,
    member_hashes: &[u64],
    subtree: &SubtreeFingerprint,
    size_tolerance: f64,
) -> bool {
    if member_hashes.is_empty() || subtree.child_hashes.is_empty() {
        return false;
    }
    if !sizes_are_close(weight, subtree.weight, size_tolerance) {
        return false;
    }
    let shared = member_hashes.iter().filter(|hash| subtree.child_hashes.contains(hash)).count();
    let smaller = member_hashes.len().min(subtree.child_hashes.len());
    shared as f64 / smaller as f64 > 0.5
}

/// Whether a window of `window_weight` nodes and a subtree of
/// `subtree_weight` are within `size_tolerance` of each other.
pub(crate) fn sizes_are_close(
    window_weight: u32,
    subtree_weight: u32,
    size_tolerance: f64,
) -> bool {
    let size_ratio = f64::from(window_weight) / f64::from(subtree_weight);
    !(size_ratio < (1.0 - size_tolerance) || size_ratio > (1.0 + size_tolerance))
}

/// Jaccard similarity between a window's members and the direct children
/// of `subtree`, each taken as a set of hashes.
pub(crate) fn window_similarity(member_hashes: &[u64], subtree: &SubtreeFingerprint) -> f64 {
    let members: HashSet<u64> = member_hashes.iter().copied().collect();
    let children: HashSet<u64> = subtree.child_hashes.iter().copied().collect();
    let union = members.union(&children).count();
    if union == 0 {
        0.0
    } else {
        members.intersection(&children).count() as f64 / union as f64
    }
}

/// A function prepared for overlap detection: the fingerprint of every
/// subtree of its tree.
#[derive(Debug, Clone)]
pub struct IndexedFunction {
    /// Function name.
    pub name: String,
    /// The subtrees in pre-order; the first is the whole function.
    subtrees: Vec<SubtreeFingerprint>,
    /// The subtrees ordered by position, then size and hash.
    by_position: Vec<u32>,
    /// The subtrees ordered by the line they start on.
    in_source_order: Vec<u32>,
}

impl IndexedFunction {
    /// Index the function `name` whose tree is `root`.
    ///
    /// `lines_of` maps the `(start, end)` byte span of a node to the first
    /// and last source line the report shows for it. A node without a span
    /// reports the lines of its nearest spanned ancestor, and a node with
    /// no spanned ancestor reports `fallback_lines`.
    #[must_use]
    pub fn new(
        name: String,
        root: &TreeNode,
        lines_of: &dyn Fn((u32, u32)) -> (u32, u32),
        fallback_lines: (u32, u32),
    ) -> Self {
        let mut subtrees = Vec::with_capacity(root.subtree_size());
        fingerprint_into(root, lines_of, fallback_lines, &mut subtrees);

        // Both orders are stable sorts of the pre-order, so subtrees that
        // tie keep it and the orders are the same on every run.
        #[allow(clippy::cast_possible_truncation)]
        let pre_order = || (0..subtrees.len() as u32).collect::<Vec<u32>>();
        let mut by_position = pre_order();
        by_position.sort_by_key(|&index| {
            let subtree = &subtrees[index as usize];
            (subtree.start_line, subtree.end_line, subtree.weight, subtree.hash)
        });
        let mut in_source_order = pre_order();
        in_source_order.sort_by_key(|&index| subtrees[index as usize].start_line);

        Self { name, subtrees, by_position, in_source_order }
    }

    /// Number of nodes in the function's tree.
    #[must_use]
    pub fn node_count(&self) -> u32 {
        self.subtrees.first().map_or(0, |root| root.weight)
    }

    /// Every subtree, the whole function first, then its descendants in
    /// pre-order.
    #[must_use]
    pub fn subtrees(&self) -> &[SubtreeFingerprint] {
        &self.subtrees
    }

    /// Every subtree ordered by position (then size and hash, so the order
    /// — and with it which of two equal overlaps survives deduplication —
    /// is the same on every run).
    pub fn subtrees_by_position(&self) -> impl Iterator<Item = &SubtreeFingerprint> + '_ {
        self.by_position.iter().map(|&index| &self.subtrees[index as usize])
    }

    /// The subtree at `rank` in [`Self::subtrees_by_position`].
    ///
    /// # Panics
    ///
    /// Panics when the function has no subtree of that rank.
    #[must_use]
    pub fn subtree_by_position(&self, rank: usize) -> &SubtreeFingerprint {
        &self.subtrees[self.by_position[rank] as usize]
    }

    /// Every subtree ordered by the line it starts on; subtrees starting
    /// on the same line keep their pre-order.
    pub fn subtrees_in_source_order(&self) -> impl Iterator<Item = &SubtreeFingerprint> + '_ {
        self.in_source_order.iter().map(|&index| &self.subtrees[index as usize])
    }

    /// The subtrees of `min_size..=max_size` nodes, ordered by position.
    #[must_use]
    pub fn subtrees_in_size_range(&self, min_size: u32, max_size: u32) -> Vec<&SubtreeFingerprint> {
        self.subtrees_by_position()
            .filter(|subtree| (min_size..=max_size).contains(&subtree.weight))
            .collect()
    }

    /// The windows of at least `window_size` nodes: for every subtree, the
    /// shortest run of subtrees starting at it — in source order — whose
    /// weights add up to `window_size`.
    #[must_use]
    pub fn sliding_windows(&self, window_size: u32) -> Vec<Window> {
        let in_source_order: Vec<&SubtreeFingerprint> = self.subtrees_in_source_order().collect();
        let mut windows = Vec::new();
        for (first, start) in in_source_order.iter().enumerate() {
            let mut weight = 0;
            // The window ends on the last line any of its subtrees reaches,
            // which need not be the last subtree's.
            let mut end_line = 0;
            for (offset, subtree) in in_source_order[first..].iter().enumerate() {
                weight += subtree.weight;
                end_line = end_line.max(subtree.end_line);
                if weight >= window_size {
                    windows.push(Window {
                        start: first,
                        weight,
                        member_hashes: in_source_order[first..=first + offset]
                            .iter()
                            .map(|member| member.hash)
                            .collect(),
                        start_line: start.start_line,
                        end_line,
                    });
                    break;
                }
            }
        }
        windows
    }
}

/// Append the fingerprint of `node` and of every subtree below it to
/// `out`, in pre-order, and return the position of `node`'s own.
fn fingerprint_into(
    node: &TreeNode,
    lines_of: &dyn Fn((u32, u32)) -> (u32, u32),
    inherited_lines: (u32, u32),
    out: &mut Vec<SubtreeFingerprint>,
) -> usize {
    let (start_line, end_line) =
        if node.has_source_span() { lines_of(node.source_span) } else { inherited_lines };

    // The node's slot comes before its descendants'; the hash and weight
    // that fill it are only known once they are done.
    let position = out.len();
    out.push(SubtreeFingerprint {
        weight: 1,
        hash: 0,
        child_hashes: Vec::new(),
        start_line,
        end_line,
        children_lines: (start_line, end_line),
        node_type: node.kind,
    });

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node.label.hash(&mut hasher);
    let mut weight = 1u32;
    let mut child_hashes = Vec::with_capacity(node.children.len());
    let mut children_lines: Option<(u32, u32)> = None;
    for child in &node.children {
        let child_position = fingerprint_into(child, lines_of, (start_line, end_line), out);
        let child = &out[child_position];
        child.hash.hash(&mut hasher);
        weight += child.weight;
        child_hashes.push(child.hash);
        children_lines = Some(match children_lines {
            Some((first, last)) => (first.min(child.start_line), last.max(child.end_line)),
            None => (child.start_line, child.end_line),
        });
    }
    node.kind.hash(&mut hasher);

    let fingerprint = &mut out[position];
    fingerprint.weight = weight;
    fingerprint.hash = hasher.finish();
    fingerprint.child_hashes = child_hashes;
    if let Some(lines) = children_lines {
        fingerprint.children_lines = lines;
    }
    position
}

/// Result of partial overlap detection
#[derive(Debug, Clone, PartialEq)]
pub struct PartialOverlap {
    /// Source function name
    pub source_function: String,
    /// Target function name
    pub target_function: String,
    /// Line range in source function
    pub source_lines: (u32, u32),
    /// Line range in target function
    pub target_lines: (u32, u32),
    /// Similarity score (0.0 to 1.0)
    pub similarity: f64,
    /// Number of nodes in the overlapping region
    pub node_count: u32,
    /// Syntactic kind of the root node of the overlapping subtree
    pub node_type: &'static str,
}

/// Options for overlap detection
#[derive(Debug, Clone)]
pub struct OverlapOptions {
    /// Minimum window size (in number of nodes)
    pub min_window_size: u32,
    /// Maximum window size (in number of nodes)
    pub max_window_size: u32,
    /// Similarity threshold (0.0 to 1.0)
    pub threshold: f64,
    /// Size tolerance for quick filtering (e.g., 0.2 for 20% tolerance)
    pub size_tolerance: f64,
}

impl Default for OverlapOptions {
    fn default() -> Self {
        // Defaults intentionally mirror similarity-ts's TypeScript defaults
        // (DEFAULT_OVERLAP_MIN_WINDOW=8, DEFAULT_OVERLAP_MAX_WINDOW=30) so a
        // direct Rust caller and the WASM-driven CLI behave the same way.
        Self { min_window_size: 8, max_window_size: 30, threshold: 0.8, size_tolerance: 0.2 }
    }
}

impl OverlapOptions {
    /// The sizes of the subtrees a window of `window_size` nodes is
    /// compared with: those within the size tolerance of it.
    #[must_use]
    pub fn candidate_sizes(&self, window_size: u32) -> (u32, u32) {
        let size = f64::from(window_size);
        // Truncation (and saturation at zero) is the definition.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bounds = (
            (size * (1.0 - self.size_tolerance)) as u32,
            (size * (1.0 + self.size_tolerance)) as u32,
        );
        bounds
    }
}

/// Whether an exact match of `weight` nodes between two functions of
/// `source_nodes` and `target_nodes` nodes is too small to mean anything.
/// Very small subtrees (a 5-node `obj.method(arg)` call) routinely hash
/// identically across unrelated functions — there are only so many ways
/// to shape a handful of nodes — and would flood the report with
/// similarity-1.0 "overlaps" that are no duplication. They are kept only
/// when they cover a quarter of the smaller function, or when both
/// functions are small.
pub(crate) fn is_incidental_match(weight: u32, source_nodes: u32, target_nodes: u32) -> bool {
    if weight >= INCIDENTAL_MATCH_NODES {
        return false;
    }
    let smaller_total = source_nodes.min(target_nodes).max(1);
    smaller_total >= 30 && f64::from(weight) / f64::from(smaller_total) < 0.25
}

/// The occurrences on the other side that the `occurrence`-th of `total`
/// equal subtrees pairs with.
///
/// A fragment that occurs several times in both functions is paired in
/// order — the first occurrence with the first, the second with the
/// second — and the occurrences one side has more of with the last of the
/// other, so every occurrence is reported once. Pairing each with each
/// reported a statement that both functions repeat a dozen times as a
/// hundred and forty-four overlaps.
pub(crate) fn counterparts<T>(occurrence: usize, total: usize, others: &[T]) -> &[T] {
    let Some(last) = others.len().checked_sub(1) else {
        return others;
    };
    let first = occurrence.min(last);
    if occurrence + 1 >= total {
        &others[first..]
    } else {
        &others[first..=first]
    }
}

/// Detect partial overlaps between two functions.
///
/// This is the definition of what overlap mode reports for a pair. It
/// compares every window of the source with every subtree of the target;
/// a project scan gets the same overlaps from an index (see
/// [`crate::overlap_detector::find_overlaps`]).
#[must_use]
pub fn detect_partial_overlaps(
    source: &IndexedFunction,
    target: &IndexedFunction,
    options: &OverlapOptions,
) -> Vec<PartialOverlap> {
    let overlap = |source_lines: (u32, u32),
                   target_lines: (u32, u32),
                   matched: &SubtreeFingerprint,
                   similarity: f64,
                   node_count: u32| PartialOverlap {
        source_function: source.name.clone(),
        target_function: target.name.clone(),
        source_lines,
        target_lines,
        similarity,
        node_count,
        node_type: matched.node_type,
    };
    let is_incidental =
        |weight: u32| is_incidental_match(weight, source.node_count(), target.node_count());
    let mut overlaps = Vec::new();

    // Pass 1: subtree × subtree.
    //
    // The window pass below finds matches that are not aligned to a
    // subtree on the source side, but it never scores an identical
    // fragment as identical: a window is compared by the hashes of its
    // members, a subtree by the hashes of its children, and a Jaccard
    // estimate over those rarely tops 0.7 on small blocks. So equal
    // subtree hashes are matched first — the path that reports "the same
    // `if (!x) throw` block appears in both functions" at 1.0.
    let source_subtrees =
        source.subtrees_in_size_range(options.min_window_size, options.max_window_size);
    let mut target_by_hash: HashMap<u64, Vec<&SubtreeFingerprint>> = HashMap::new();
    for subtree in target.subtrees_in_size_range(options.min_window_size, options.max_window_size) {
        target_by_hash.entry(subtree.hash).or_default().push(subtree);
    }

    // Multiplicity signal: a single small match between two unrelated
    // functions is likely coincidence, but several distinct matching
    // shapes between the same pair is itself a duplication signal — e.g.
    // three different validation blocks recurring across two endpoints'
    // preludes. Distinct hashes are counted, not occurrences: a function
    // repeating one trivial shape 50 times must not lift the suppression.
    let distinct_matches: HashSet<u64> = source_subtrees
        .iter()
        .map(|subtree| subtree.hash)
        .filter(|hash| target_by_hash.contains_key(hash))
        .collect();
    let strong_multiplicity_signal = distinct_matches.len() >= MULTIPLICITY_SIGNAL_MATCHES;

    // How often each shape occurs in the source, and how many of them
    // have been met.
    let mut occurrences: HashMap<u64, usize> = HashMap::new();
    for subtree in &source_subtrees {
        *occurrences.entry(subtree.hash).or_default() += 1;
    }
    let mut met: HashMap<u64, usize> = HashMap::new();

    for subtree in &source_subtrees {
        let occurrence = {
            let met = met.entry(subtree.hash).or_default();
            *met += 1;
            *met - 1
        };
        let Some(matches) = target_by_hash.get(&subtree.hash) else {
            continue;
        };
        if !strong_multiplicity_signal && is_incidental(subtree.weight) {
            continue;
        }
        for matched in counterparts(occurrence, occurrences[&subtree.hash], matches) {
            overlaps.push(overlap(
                (subtree.start_line, subtree.end_line),
                (matched.start_line, matched.end_line),
                matched,
                1.0,
                subtree.weight,
            ));
        }
    }

    // Pass 2: window × subtree, per window size. This catches matches
    // where the duplicated stretch does not start at a subtree boundary on
    // the source side. A window is compared with the children of the
    // candidate, so those are the lines reported for the target: the
    // candidate's own lines would add what its root writes around them
    // (the brace lines of a block), which the window has no part of.
    let mut window_matches: Vec<WindowMatch> = Vec::new();
    for window_size in options.min_window_size..=options.max_window_size {
        let (size_min, size_max) = options.candidate_sizes(window_size);
        let candidates: Vec<(usize, &SubtreeFingerprint)> = target
            .subtrees_by_position()
            .enumerate()
            .filter(|(_, subtree)| (size_min..=size_max).contains(&subtree.weight))
            .collect();
        for window in source.sliding_windows(window_size) {
            for &(rank, candidate) in &candidates {
                if !window_might_match(
                    window.weight,
                    &window.member_hashes,
                    candidate,
                    options.size_tolerance,
                ) {
                    continue;
                }
                let similarity = window_similarity(&window.member_hashes, candidate);
                if similarity < options.threshold {
                    continue;
                }
                // Members that are exactly the candidate's children make a
                // 1.0 as coincidental as an equal hash does.
                if similarity >= EXACT_SIMILARITY && is_incidental(window.weight) {
                    continue;
                }
                window_matches.push(WindowMatch {
                    window_size,
                    start: window.start,
                    rank,
                    shape: (window_shape(&window.member_hashes), candidate.hash),
                    overlap: overlap(
                        (window.start_line, window.end_line),
                        candidate.children_lines,
                        candidate,
                        similarity,
                        window.weight,
                    ),
                });
            }
        }
    }
    overlaps.extend(pair_repeated_windows(window_matches));

    rank_and_deduplicate(overlaps)
}

/// Order the overlaps found between two functions and drop every one
/// whose line ranges lie, on both sides, inside those of an overlap kept
/// before it.
///
/// Most similar first and, among equally similar overlaps, the one
/// reaching over the most lines first: an overlap never comes after one it
/// encloses, so the enclosing overlap is the one kept, wherever the two
/// start. The sort is stable — of two overlaps that agree on all of this,
/// the one found first stays ahead (the exact subtree match of the first
/// pass, before a window over the same lines).
pub(crate) fn rank_and_deduplicate(mut overlaps: Vec<PartialOverlap>) -> Vec<PartialOverlap> {
    let line_counts = |overlap: &PartialOverlap| {
        (
            overlap.source_lines.1.saturating_sub(overlap.source_lines.0),
            overlap.target_lines.1.saturating_sub(overlap.target_lines.0),
        )
    };
    overlaps.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| line_counts(b).cmp(&line_counts(a)))
            .then_with(|| b.node_count.cmp(&a.node_count))
            .then_with(|| a.source_lines.cmp(&b.source_lines))
            .then_with(|| a.target_lines.cmp(&b.target_lines))
    });

    let within = |inner: (u32, u32), outer: (u32, u32)| inner.0 >= outer.0 && inner.1 <= outer.1;
    let mut kept: Vec<PartialOverlap> = Vec::new();
    for overlap in overlaps {
        let is_duplicate = kept.iter().any(|existing| {
            within(overlap.source_lines, existing.source_lines)
                && within(overlap.target_lines, existing.target_lines)
        });
        if !is_duplicate {
            kept.push(overlap);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn node(label: &str, kind: &'static str, children: Vec<Rc<TreeNode>>) -> Rc<TreeNode> {
        let mut node = TreeNode::new(label, kind);
        for child in children {
            node.add_child(child);
        }
        Rc::new(node)
    }

    fn spanned(start: u32, end: u32, tree: Rc<TreeNode>) -> Rc<TreeNode> {
        let mut tree = (*tree).clone();
        tree.set_source_span(start, end);
        Rc::new(tree)
    }

    /// One line per ten bytes.
    fn lines_of((start, end): (u32, u32)) -> (u32, u32) {
        (start / 10 + 1, (end - 1) / 10 + 1)
    }

    fn call(callee: &str, argument: &str) -> Rc<TreeNode> {
        node(
            "CallExpression",
            "CallExpression",
            vec![node(callee, "Identifier", vec![]), node(argument, "Identifier", vec![])],
        )
    }

    fn subtree(weight: u32, child_hashes: Vec<u64>) -> SubtreeFingerprint {
        SubtreeFingerprint {
            weight,
            hash: 99,
            child_hashes,
            start_line: 30,
            end_line: 40,
            children_lines: (31, 39),
            node_type: "IfStatement",
        }
    }

    #[test]
    fn subtrees_are_listed_in_pre_order_with_the_root_first() {
        let tree = node("Block", "BlockStatement", vec![call("log", "a"), call("log", "b")]);
        let indexed = IndexedFunction::new("f".to_string(), &tree, &lines_of, (1, 1));
        let kinds: Vec<&str> = indexed.subtrees().iter().map(|subtree| subtree.node_type).collect();
        assert_eq!(
            kinds,
            [
                "BlockStatement",
                "CallExpression",
                "Identifier",
                "Identifier",
                "CallExpression",
                "Identifier",
                "Identifier"
            ]
        );
        assert_eq!(indexed.node_count(), 7);
        let weights: Vec<u32> = indexed.subtrees().iter().map(|subtree| subtree.weight).collect();
        assert_eq!(weights, [7, 3, 1, 1, 3, 1, 1]);
        // The root lists its two children.
        let root = &indexed.subtrees()[0];
        assert_eq!(root.child_hashes, [indexed.subtrees()[1].hash, indexed.subtrees()[4].hash]);
    }

    #[test]
    fn equal_subtrees_hash_alike_and_labels_tell_them_apart() {
        let tree = node(
            "Block",
            "BlockStatement",
            vec![call("log", "a"), call("log", "a"), call("warn", "a")],
        );
        let indexed = IndexedFunction::new("f".to_string(), &tree, &lines_of, (1, 1));
        let calls: Vec<&SubtreeFingerprint> = indexed.subtrees_in_size_range(3, 3);
        assert_eq!(calls.len(), 3);
        let hashes: HashSet<u64> = calls.iter().map(|subtree| subtree.hash).collect();
        assert_eq!(hashes.len(), 2);
    }

    #[test]
    fn a_node_without_a_span_reports_the_lines_of_its_nearest_spanned_ancestor() {
        let inner = spanned(25, 38, call("log", "a"));
        let tree = node(
            "Block",
            "BlockStatement",
            vec![spanned(12, 48, node("If", "IfStatement", vec![inner]))],
        );
        let indexed = IndexedFunction::new("f".to_string(), &tree, &lines_of, (7, 9));
        let lines: Vec<(&str, u32, u32)> = indexed
            .subtrees()
            .iter()
            .map(|subtree| (subtree.node_type, subtree.start_line, subtree.end_line))
            .collect();
        assert_eq!(
            lines,
            [
                // No span and no spanned ancestor: the fallback.
                ("BlockStatement", 7, 9),
                ("IfStatement", 2, 5),
                ("CallExpression", 3, 4),
                // The identifiers carry no span of their own.
                ("Identifier", 3, 4),
                ("Identifier", 3, 4),
            ]
        );
        // The children of the `if` are the call alone; a leaf has none and
        // keeps its own lines.
        assert_eq!(indexed.subtrees()[1].children_lines, (3, 4));
        assert_eq!(indexed.subtrees()[3].children_lines, (3, 4));
        assert_eq!(indexed.subtrees()[0].children_lines, (2, 5));
    }

    #[test]
    fn a_window_ends_on_the_last_line_any_of_its_subtrees_reaches() {
        // The call spans lines 1-4 and so do its two leaves, which carry
        // no span of their own; the leaf after it sits on line 2.
        let long = spanned(0, 40, call("log", "a"));
        let short = spanned(10, 15, node("b", "Identifier", vec![]));
        let tree = node("Block", "BlockStatement", vec![long, short]);
        let indexed = IndexedFunction::new("f".to_string(), &tree, &lines_of, (1, 4));

        let windows = indexed.sliding_windows(3);
        let shapes: Vec<(u32, usize, u32, u32)> = windows
            .iter()
            .map(|window| {
                (window.weight, window.member_hashes.len(), window.start_line, window.end_line)
            })
            .collect();
        assert_eq!(
            shapes,
            [
                // The block alone, then the call alone.
                (5, 1, 1, 4),
                (3, 1, 1, 4),
                // `log`, `a` and `b`: the run ends with the subtree on
                // line 2, but its first two reach line 4.
                (3, 3, 1, 4),
                // No run starting at `a` or `b` adds up to three nodes.
            ]
        );
    }

    #[test]
    fn windows_match_subtrees_of_similar_size_that_share_most_parts() {
        let members = [1, 2, 3];
        assert!(window_might_match(10, &members, &subtree(11, vec![1, 2, 4]), 0.2));
        // Too different in size.
        assert!(!window_might_match(10, &members, &subtree(20, vec![1, 2, 3]), 0.2));
        // Too few shared parts.
        assert!(!window_might_match(10, &members, &subtree(10, vec![5, 6, 7]), 0.2));
        // A leaf has no parts to share.
        assert!(!window_might_match(1, &[1], &subtree(1, vec![]), 0.2));

        let similarity = |children: Vec<u64>| window_similarity(&members, &subtree(10, children));
        assert!((similarity(vec![1, 2, 3]) - 1.0).abs() < f64::EPSILON);
        assert!((similarity(vec![3, 3, 2, 1]) - 1.0).abs() < f64::EPSILON);
        assert!((similarity(vec![1, 2, 4]) - 0.5).abs() < f64::EPSILON);
        assert!(similarity(vec![]) < f64::EPSILON);
    }

    #[test]
    fn candidate_sizes_follow_the_size_tolerance() {
        let options = OverlapOptions::default();
        assert_eq!(options.candidate_sizes(8), (6, 9));
        assert_eq!(options.candidate_sizes(30), (24, 36));
        let exact = OverlapOptions { size_tolerance: 0.0, ..OverlapOptions::default() };
        assert_eq!(exact.candidate_sizes(8), (8, 8));
    }

    #[test]
    fn the_same_subtree_in_two_functions_is_an_exact_overlap() {
        let left = IndexedFunction::new("left".to_string(), &call("log", "a"), &lines_of, (1, 1));
        let same = IndexedFunction::new("same".to_string(), &call("log", "a"), &lines_of, (1, 1));
        let options = OverlapOptions { min_window_size: 3, ..OverlapOptions::default() };
        let overlaps = detect_partial_overlaps(&left, &same, &options);
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].node_type, "CallExpression");
        assert_eq!(overlaps[0].node_count, 3);
        assert!((overlaps[0].similarity - 1.0).abs() < f64::EPSILON);

        let other =
            IndexedFunction::new("other".to_string(), &call("warn", "b"), &lines_of, (1, 1));
        assert!(detect_partial_overlaps(&left, &other, &options).is_empty());
    }

    #[test]
    fn a_window_never_matches_a_leaf() {
        // A single node used to count as half similar to any window of one
        // node: with a threshold of 0.5 every leaf "overlapped" every other.
        let left = IndexedFunction::new("left".to_string(), &call("log", "a"), &lines_of, (1, 1));
        let other =
            IndexedFunction::new("other".to_string(), &call("warn", "b"), &lines_of, (1, 1));
        let options = OverlapOptions {
            min_window_size: 1,
            max_window_size: 4,
            threshold: 0.5,
            size_tolerance: 0.2,
        };
        assert!(detect_partial_overlaps(&left, &other, &options).is_empty());
    }

    #[test]
    fn small_matches_in_large_functions_are_incidental() {
        assert!(is_incidental_match(5, 40, 60));
        // A quarter of the smaller function is no accident.
        assert!(!is_incidental_match(5, 20, 60));
        assert!(!is_incidental_match(7, 28, 28));
        // From eight nodes on a match always counts.
        assert!(!is_incidental_match(8, 400, 600));
    }

    #[test]
    fn deduplication_keeps_the_enclosing_overlap() {
        let overlap = |source_lines, target_lines, node_type| PartialOverlap {
            source_function: "a".to_string(),
            target_function: "b".to_string(),
            source_lines,
            target_lines,
            similarity: 1.0,
            node_count: 10,
            node_type,
        };
        let kept = rank_and_deduplicate(vec![
            // Inside the third on both sides, though it starts where that
            // one does.
            overlap((1, 4), (20, 23), "LogicalExpression"),
            // Inside the third on the source side only.
            overlap((2, 4), (40, 42), "CallExpression"),
            overlap((1, 10), (20, 30), "IfStatement"),
            // The same lines as the third: the first found stays.
            overlap((1, 10), (20, 30), "BlockStatement"),
        ]);
        let kinds: Vec<&str> = kept.iter().map(|overlap| overlap.node_type).collect();
        assert_eq!(kinds, ["IfStatement", "CallExpression"]);
    }

    #[test]
    fn repeated_fragments_pair_in_order() {
        let others = ["a", "b", "c"];
        // One occurrence pairs with every counterpart.
        assert_eq!(counterparts(0, 1, &others), ["a", "b", "c"]);
        // Three and three: each with its own.
        assert_eq!(counterparts(0, 3, &others), ["a"]);
        assert_eq!(counterparts(1, 3, &others), ["b"]);
        assert_eq!(counterparts(2, 3, &others), ["c"]);
        // Two against three: the last takes what is left.
        assert_eq!(counterparts(0, 2, &others), ["a"]);
        assert_eq!(counterparts(1, 2, &others), ["b", "c"]);
        // Five against three: the extra ones pair with the last.
        assert_eq!(counterparts(3, 5, &others), ["c"]);
        assert_eq!(counterparts(4, 5, &others), ["c"]);
        assert!(counterparts::<&str>(0, 1, &[]).is_empty());
    }

    #[test]
    fn a_fragment_repeated_on_both_sides_is_reported_once_per_occurrence() {
        // Four calls on four lines in one function, three in the other.
        let repeated = |count: u32| {
            let calls: Vec<Rc<TreeNode>> = (0..count)
                .map(|line| spanned(line * 10, line * 10 + 9, call("log", "a")))
                .collect();
            node("Block", "BlockStatement", calls)
        };
        let source = IndexedFunction::new("left".to_string(), &repeated(4), &lines_of, (1, 1));
        let target = IndexedFunction::new("right".to_string(), &repeated(3), &lines_of, (1, 1));
        let options =
            OverlapOptions { min_window_size: 3, max_window_size: 3, ..OverlapOptions::default() };
        let overlaps = detect_partial_overlaps(&source, &target, &options);
        let mut lines: Vec<(u32, u32)> = overlaps
            .iter()
            .filter(|overlap| overlap.node_type == "CallExpression")
            .map(|overlap| (overlap.source_lines.0, overlap.target_lines.0))
            .collect();
        lines.sort_unstable();
        // Not the twelve combinations.
        assert_eq!(lines, [(1, 1), (2, 2), (3, 3), (4, 3)]);
    }

    #[test]
    fn a_window_repeated_on_both_sides_is_reported_once_per_occurrence() {
        // Each function wraps the same call in a statement several times.
        // The call is too large to be reported as a subtree here, so only
        // the window pass finds it: the call alone against the statement
        // around it.
        let repeated = |count: u32| {
            let statements: Vec<Rc<TreeNode>> = (0..count)
                .map(|line| {
                    let wrapped = node("Statement", "ExpressionStatement", vec![call("log", "a")]);
                    spanned(line * 10, line * 10 + 9, wrapped)
                })
                .collect();
            node("Block", "BlockStatement", statements)
        };
        let source = IndexedFunction::new("left".to_string(), &repeated(4), &lines_of, (1, 1));
        let target = IndexedFunction::new("right".to_string(), &repeated(3), &lines_of, (1, 1));
        // Windows of three nodes — the call — against subtrees of four:
        // the statement. Subtrees of exactly three nodes are reported by
        // the first pass, on the same lines.
        let options = OverlapOptions {
            min_window_size: 3,
            max_window_size: 3,
            threshold: 0.8,
            size_tolerance: 0.4,
        };
        let overlaps = detect_partial_overlaps(&source, &target, &options);
        let mut lines: Vec<(u32, u32)> = overlaps
            .iter()
            .map(|overlap| (overlap.source_lines.0, overlap.target_lines.0))
            .collect();
        lines.sort_unstable();
        assert_eq!(lines, [(1, 1), (2, 2), (3, 3), (4, 3)], "{overlaps:#?}");
    }

    #[test]
    fn repeated_window_matches_keep_the_first_meeting() {
        let found = |window_size: u32, start: usize, rank: usize| WindowMatch {
            window_size,
            start,
            rank,
            shape: (7, 9),
            overlap: PartialOverlap {
                source_function: "a".to_string(),
                target_function: "b".to_string(),
                source_lines: (start as u32, start as u32),
                target_lines: (rank as u32, rank as u32),
                similarity: 1.0,
                node_count: window_size,
                node_type: "IfStatement",
            },
        };
        // Windows at 10 and 20 against subtrees 3 and 5, each pair met at
        // two window sizes, listed out of order.
        let paired = pair_repeated_windows(vec![
            found(9, 20, 5),
            found(8, 20, 5),
            found(8, 10, 5),
            found(9, 10, 3),
            found(8, 20, 3),
            found(8, 10, 3),
        ]);
        let places: Vec<(u32, u32, u32)> = paired
            .iter()
            .map(|overlap| (overlap.node_count, overlap.source_lines.0, overlap.target_lines.0))
            .collect();
        assert_eq!(places, [(8, 10, 3), (8, 20, 5)]);
    }
}
