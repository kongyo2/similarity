//! Overlap mode: duplicated fragments *inside* functions.
//!
//! [`detect_partial_overlaps`](crate::subtree_fingerprint::detect_partial_overlaps)
//! defines what two functions share. Running it on every pair of functions
//! of a project is quadratic in the project and was by far the slowest
//! part of a default run, so the scan here answers the same questions from
//! two indexes over all functions at once — which functions hold a given
//! subtree, and which subtrees have a given child — and only ever looks at
//! functions that have something in common.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::function_comparator::{FunctionSource, PairScope};
use crate::function_extractor::{extract_functions, FunctionDefinition};
use crate::function_fragment::standalone_source;
use crate::parser::parse_and_convert_to_tree;
use crate::source::LineIndex;
use crate::subtree_fingerprint::{
    counterparts, is_incidental_match, pair_repeated_windows, rank_and_deduplicate,
    sizes_are_close, window_might_match, window_shape, window_similarity, IndexedFunction,
    OverlapOptions, PartialOverlap, SubtreeFingerprint, WindowMatch, EXACT_SIMILARITY,
    INCIDENTAL_MATCH_NODES, MULTIPLICITY_SIGNAL_MATCHES,
};
use crate::tree::TreeNode;

/// Overlap result with file information
#[derive(Debug, Clone)]
pub struct PartialOverlapWithFiles {
    pub source_file: String,
    pub target_file: String,
    pub overlap: PartialOverlap,
}

/// Detect overlapping fragments between the functions of two source texts.
/// A text that cannot be analyzed contributes no functions.
#[must_use]
pub fn find_function_overlaps(
    source_code: &str,
    target_code: &str,
    options: &OverlapOptions,
) -> Vec<PartialOverlap> {
    let files = [
        ("source.ts".to_string(), source_code.to_string()),
        ("target.ts".to_string(), target_code.to_string()),
    ];
    find_overlaps_across_files(&files, options, PairScope::CrossFileOnly)
        .into_iter()
        .map(|found| found.overlap)
        .collect()
}

/// [`find_overlaps`] over `(path, source text)` pairs: the functions are
/// extracted first, and a file that cannot be analyzed contributes none.
#[must_use]
pub fn find_overlaps_across_files(
    files: &[(String, String)],
    options: &OverlapOptions,
    scope: PairScope,
) -> Vec<PartialOverlapWithFiles> {
    let extracted: Vec<Vec<FunctionDefinition>> = files
        .iter()
        .map(|(path, source_text)| extract_functions(path, source_text).unwrap_or_default())
        .collect();
    let sources: Vec<FunctionSource> = files
        .iter()
        .zip(&extracted)
        .map(|((path, source_text), functions)| FunctionSource { path, source_text, functions })
        .collect();
    find_overlaps(&sources, options, scope)
}

/// Detect overlaps between the functions of a set of files: for every pair
/// of functions `scope` admits, except a function and one nested in it,
/// what [`detect_partial_overlaps`] finds with the earlier function as the
/// source.
///
/// Files are visited in path order — whatever order they are passed in —
/// and functions in the order they are declared, so the result does not
/// depend on how the files were listed: the pairs of a file with itself
/// come first, then its pairs with each later file in turn.
///
/// A fragment inside a nested function belongs to that function and to
/// every function around it, and is found for each of them. It is reported
/// once, for the innermost functions.
///
/// [`detect_partial_overlaps`]: crate::subtree_fingerprint::detect_partial_overlaps
#[must_use]
pub fn find_overlaps(
    sources: &[FunctionSource],
    options: &OverlapOptions,
    scope: PairScope,
) -> Vec<PartialOverlapWithFiles> {
    let scan = Scan::new(sources, options, scope);
    scan.innermost_only(scan.overlaps())
        .into_iter()
        .map(|found| PartialOverlapWithFiles {
            source_file: scan.path_of(found.source).to_string(),
            target_file: scan.path_of(found.target).to_string(),
            overlap: found.overlap,
        })
        .collect()
}

/// An overlap between the functions at `source` and `target` of the scan.
struct Found {
    source: usize,
    target: usize,
    overlap: PartialOverlap,
}

/// One function of the scan.
struct ScannedFunction<'a> {
    definition: &'a FunctionDefinition,
    indexed: IndexedFunction,
    /// Position of the function's file in path order.
    file: usize,
}

/// One subtree of one scanned function: the function's position in the
/// scan and the subtree's among the function's subtrees by position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Posting {
    function: usize,
    rank: usize,
}

/// The postings of functions that come after `function`. Posting lists
/// are ordered by function.
fn after(postings: &[Posting], function: usize) -> &[Posting] {
    &postings[postings.partition_point(|posting| posting.function <= function)..]
}

/// Where every function of the scan holds what.
struct ScanIndex {
    /// The subtrees of a size overlap mode reports, by their hash.
    by_hash: HashMap<u64, Vec<Posting>>,
    /// The subtrees of a size a window can be compared with, by the hash
    /// of each of their children and then by their own weight (ascending).
    /// A window only ever matches subtrees of about its own weight, and a
    /// child as common as a loop variable has parents of every weight.
    by_child_hash: HashMap<u64, Vec<(u32, Vec<Posting>)>>,
    /// The candidate sizes of each window size, the smallest window first.
    candidate_sizes: Vec<(u32, u32)>,
}

impl ScanIndex {
    fn new(functions: &[ScannedFunction], options: &OverlapOptions) -> Self {
        let reported = options.min_window_size..=options.max_window_size;
        let candidate_sizes: Vec<(u32, u32)> =
            reported.clone().map(|window_size| options.candidate_sizes(window_size)).collect();
        let compared = |weight: u32| {
            candidate_sizes
                .iter()
                .any(|&(lightest, heaviest)| (lightest..=heaviest).contains(&weight))
        };

        let mut by_hash: HashMap<u64, Vec<Posting>> = HashMap::new();
        let mut by_child_hash: HashMap<u64, Vec<(u32, Vec<Posting>)>> = HashMap::new();
        let mut child_hashes: Vec<u64> = Vec::new();
        for (function, scanned) in functions.iter().enumerate() {
            for (rank, subtree) in scanned.indexed.subtrees_by_position().enumerate() {
                let posting = Posting { function, rank };
                if reported.contains(&subtree.weight) {
                    by_hash.entry(subtree.hash).or_default().push(posting);
                }
                if compared(subtree.weight) {
                    child_hashes.clear();
                    child_hashes.extend_from_slice(&subtree.child_hashes);
                    child_hashes.sort_unstable();
                    child_hashes.dedup();
                    for &hash in &child_hashes {
                        let by_weight = by_child_hash.entry(hash).or_default();
                        let slot =
                            by_weight.partition_point(|(weight, _)| *weight < subtree.weight);
                        if by_weight.get(slot).is_none_or(|(weight, _)| *weight != subtree.weight) {
                            by_weight.insert(slot, (subtree.weight, Vec::new()));
                        }
                        by_weight[slot].1.push(posting);
                    }
                }
            }
        }
        Self { by_hash, by_child_hash, candidate_sizes }
    }
}

/// What was found between a source function and one target.
#[derive(Default)]
struct PairFindings {
    /// Equal subtrees, in the order the pairwise comparison finds them.
    exact: Vec<PartialOverlap>,
    /// Window matches.
    windowed: Vec<WindowMatch>,
}

/// A run of a function's subtrees in source order: the window of every
/// size in `sizes`.
struct Run<'a> {
    /// Position of the first subtree in source order.
    start: usize,
    weight: u32,
    member_hashes: &'a [u64],
    lines: (u32, u32),
    /// The smallest and the largest window size the run is the window of.
    sizes: (u32, u32),
}

struct Scan<'a> {
    /// The files in path order.
    files: Vec<&'a FunctionSource<'a>>,
    /// The functions of all files, file by file, in declaration order.
    functions: Vec<ScannedFunction<'a>>,
    index: ScanIndex,
    options: &'a OverlapOptions,
    scope: PairScope,
}

impl<'a> Scan<'a> {
    fn new(
        sources: &'a [FunctionSource<'a>],
        options: &'a OverlapOptions,
        scope: PairScope,
    ) -> Self {
        let mut files: Vec<&FunctionSource> = sources.iter().collect();
        files.sort_by_key(|source| source.path);
        let functions: Vec<ScannedFunction> = files
            .iter()
            .enumerate()
            .flat_map(|(file, source)| index_file_functions(source, file))
            .collect();
        Self { index: ScanIndex::new(&functions, options), files, functions, options, scope }
    }

    /// Path of the file the function at `function` is declared in.
    fn path_of(&self, function: usize) -> &str {
        self.files[self.functions[function].file].path
    }

    /// Every overlap of every pair of functions the scan compares.
    fn overlaps(&self) -> Vec<Found> {
        let mut found: Vec<Found> = Vec::new();
        // The pairs of one file's functions, keyed by where they come in
        // the result: by the target's file, then source-major.
        let mut file_pairs: Vec<((usize, usize, usize), Vec<PartialOverlap>)> = Vec::new();
        for (source_id, source) in self.functions.iter().enumerate() {
            for (target_id, overlaps) in self.overlaps_from(source_id) {
                file_pairs.push(((self.functions[target_id].file, source_id, target_id), overlaps));
            }
            let file_is_done =
                self.functions.get(source_id + 1).is_none_or(|next| next.file != source.file);
            if file_is_done {
                file_pairs.sort_by_key(|(order, _)| *order);
                for ((_, source, target), overlaps) in file_pairs.drain(..) {
                    found.extend(overlaps.into_iter().map(|overlap| Found {
                        source,
                        target,
                        overlap,
                    }));
                }
            }
        }
        found
    }

    /// Whether the scan compares `source` with the later function
    /// `target`.
    ///
    /// A nested function lives entirely inside its parent, so the
    /// parent's subtrees trivially include the child's and every
    /// parent/child pair would "overlap" at 1.0 — a containment artifact,
    /// not duplication. Only these PAIRS are skipped; the nested function
    /// takes part in every other comparison.
    fn compares(&self, source: &ScannedFunction, target: &ScannedFunction) -> bool {
        if source.file == target.file {
            self.scope.includes(true)
                && !source.definition.is_parent_child_relationship(target.definition)
        } else {
            self.scope.includes(false)
        }
    }

    /// Drop the overlaps that repeat another for functions around it.
    ///
    /// The subtrees of a nested function are subtrees of its parent too, so
    /// a fragment of `inner`, declared in `outer`, that also sits in `other`
    /// is found twice: for `inner` and `other`, and for `outer` and `other`
    /// — on the same lines. With nesting on both sides that makes four
    /// reports of one fragment. Of the overlaps that report the same lines
    /// between functions nested in one another, only the one between the
    /// innermost functions is kept (the most similar one, should they
    /// differ).
    fn innermost_only(&self, found: Vec<Found>) -> Vec<Found> {
        // Where an overlap is: the lines it reports in each file.
        let place = |found: &Found| {
            (
                self.functions[found.source].file,
                found.overlap.source_lines,
                self.functions[found.target].file,
                found.overlap.target_lines,
            )
        };
        let span_of = |function: usize| self.functions[function].definition.span;
        let extent = |found: &Found| span_of(found.source).size() + span_of(found.target).size();
        let is_better = |candidate: &Found, best: &Found| {
            candidate.overlap.similarity > best.overlap.similarity
                || (candidate.overlap.similarity == best.overlap.similarity
                    && extent(candidate) < extent(best))
        };
        // Two functions of one file are nested when the span of one holds
        // the other's.
        let nested = |left: usize, right: usize| {
            let (left, right) = (span_of(left), span_of(right));
            (left.start <= right.start && right.end <= left.end)
                || (right.start <= left.start && left.end <= right.end)
        };

        let mut best_at = HashMap::new();
        for (index, entry) in found.iter().enumerate() {
            best_at
                .entry(place(entry))
                .and_modify(|best: &mut usize| {
                    if is_better(entry, &found[*best]) {
                        *best = index;
                    }
                })
                .or_insert(index);
        }
        let repeats_a_better_one: Vec<bool> = found
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let best = best_at[&place(entry)];
                // Functions that merely share a line report different
                // fragments on the same lines.
                best != index
                    && nested(entry.source, found[best].source)
                    && nested(entry.target, found[best].target)
            })
            .collect();
        found
            .into_iter()
            .zip(repeats_a_better_one)
            .filter_map(|(entry, repeats)| (!repeats).then_some(entry))
            .collect()
    }

    /// The overlaps of the function at `source_id` with each later
    /// function it shares something with, in the order of the functions.
    fn overlaps_from(&self, source_id: usize) -> Vec<(usize, Vec<PartialOverlap>)> {
        let mut findings: HashMap<usize, PairFindings> = HashMap::new();
        self.equal_subtrees(source_id, &mut findings);
        self.window_matches(source_id, &mut findings);

        let mut pairs: Vec<(usize, Vec<PartialOverlap>)> = findings
            .into_iter()
            .map(|(target_id, found)| {
                let mut overlaps = found.exact;
                overlaps.extend(pair_repeated_windows(found.windowed));
                (target_id, rank_and_deduplicate(overlaps))
            })
            .collect();
        pairs.sort_by_key(|(target_id, _)| *target_id);
        pairs
    }

    fn overlap(
        &self,
        source_id: usize,
        source_lines: (u32, u32),
        matched: Posting,
        target_lines: impl FnOnce(&SubtreeFingerprint) -> (u32, u32),
        similarity: f64,
        node_count: u32,
    ) -> PartialOverlap {
        let target = &self.functions[matched.function].indexed;
        let subtree = target.subtree_by_position(matched.rank);
        PartialOverlap {
            source_function: self.functions[source_id].indexed.name.clone(),
            target_function: target.name.clone(),
            source_lines,
            target_lines: target_lines(subtree),
            similarity,
            node_count,
            node_type: subtree.node_type,
        }
    }

    /// The first pass of the pairwise comparison, against every later
    /// function at once: the subtrees of the source that other functions
    /// hold too.
    fn equal_subtrees(&self, source_id: usize, findings: &mut HashMap<usize, PairFindings>) {
        let source = &self.functions[source_id];
        let subtrees = source
            .indexed
            .subtrees_in_size_range(self.options.min_window_size, self.options.max_window_size);

        // Matches of fewer nodes than this are subject to the incidental
        // rule, which the number of distinct shapes two functions share
        // lifts; larger windows never ask.
        let mut shared_shapes: HashMap<usize, usize> = HashMap::new();
        if self.options.min_window_size < INCIDENTAL_MATCH_NODES {
            let distinct: HashSet<u64> = subtrees.iter().map(|subtree| subtree.hash).collect();
            for hash in distinct {
                let mut last_function = None;
                for posting in self.postings_after(&self.index.by_hash, hash, source_id) {
                    if last_function != Some(posting.function) {
                        *shared_shapes.entry(posting.function).or_default() += 1;
                        last_function = Some(posting.function);
                    }
                }
            }
        }

        // How often each shape occurs in the source, and how many of them
        // have been met.
        let mut occurrences: HashMap<u64, usize> = HashMap::new();
        for subtree in &subtrees {
            *occurrences.entry(subtree.hash).or_default() += 1;
        }
        let mut met: HashMap<u64, usize> = HashMap::new();

        for subtree in subtrees {
            let occurrence = {
                let met = met.entry(subtree.hash).or_default();
                *met += 1;
                *met - 1
            };
            // The postings of one function follow each other.
            let postings = self.postings_after(&self.index.by_hash, subtree.hash, source_id);
            for in_target in postings.chunk_by(|left, right| left.function == right.function) {
                let target_id = in_target[0].function;
                let target = &self.functions[target_id];
                if !self.compares(source, target) {
                    continue;
                }
                let strong_multiplicity_signal =
                    shared_shapes.get(&target_id).copied().unwrap_or(0)
                        >= MULTIPLICITY_SIGNAL_MATCHES;
                if !strong_multiplicity_signal
                    && is_incidental_match(
                        subtree.weight,
                        source.indexed.node_count(),
                        target.indexed.node_count(),
                    )
                {
                    continue;
                }
                let found = findings.entry(target_id).or_default();
                for &posting in counterparts(occurrence, occurrences[&subtree.hash], in_target) {
                    found.exact.push(self.overlap(
                        source_id,
                        (subtree.start_line, subtree.end_line),
                        posting,
                        |matched| (matched.start_line, matched.end_line),
                        1.0,
                        subtree.weight,
                    ));
                }
            }
        }
    }

    fn postings_after<'p>(
        &self,
        postings: &'p HashMap<u64, Vec<Posting>>,
        hash: u64,
        function: usize,
    ) -> &'p [Posting] {
        postings.get(&hash).map_or(&[], |postings| after(postings, function))
    }

    /// The second pass of the pairwise comparison, against every later
    /// function at once: the windows of the source and the subtrees whose
    /// children they resemble.
    fn window_matches(&self, source_id: usize, findings: &mut HashMap<usize, PairFindings>) {
        let (min_size, max_size) = (self.options.min_window_size, self.options.max_window_size);
        let in_source_order: Vec<&SubtreeFingerprint> =
            self.functions[source_id].indexed.subtrees_in_source_order().collect();

        // The window of size `n` that starts at a subtree is the shortest
        // run from it weighing `n` or more, so one run is the window of
        // every size above the weight of the run before it, up to its own.
        let mut member_hashes: Vec<u64> = Vec::new();
        for (start, first) in in_source_order.iter().enumerate() {
            member_hashes.clear();
            let mut weight = 0u32;
            let mut end_line = 0u32;
            for subtree in &in_source_order[start..] {
                let lighter = weight;
                weight += subtree.weight;
                end_line = end_line.max(subtree.end_line);
                member_hashes.push(subtree.hash);

                let smallest =
                    if member_hashes.len() == 1 { min_size } else { min_size.max(lighter + 1) };
                let largest = max_size.min(weight);
                if smallest <= largest {
                    let run = Run {
                        start,
                        weight,
                        member_hashes: &member_hashes,
                        lines: (first.start_line, end_line),
                        sizes: (smallest, largest),
                    };
                    self.run_matches(source_id, &run, findings);
                }
                if weight >= max_size {
                    break;
                }
            }
        }
    }

    /// The subtrees of later functions the windows of `run` match.
    fn run_matches(
        &self,
        source_id: usize,
        run: &Run,
        findings: &mut HashMap<usize, PairFindings>,
    ) {
        let source = &self.functions[source_id];
        for (posting, window_size) in self.candidates(source_id, run) {
            let target = &self.functions[posting.function];
            let candidate = target.indexed.subtree_by_position(posting.rank);
            if !self.compares(source, target)
                || !window_might_match(
                    run.weight,
                    run.member_hashes,
                    candidate,
                    self.options.size_tolerance,
                )
            {
                continue;
            }
            let similarity = window_similarity(run.member_hashes, candidate);
            if similarity < self.options.threshold {
                continue;
            }
            if similarity >= EXACT_SIMILARITY
                && is_incidental_match(
                    run.weight,
                    source.indexed.node_count(),
                    target.indexed.node_count(),
                )
            {
                continue;
            }
            findings.entry(posting.function).or_default().windowed.push(WindowMatch {
                window_size,
                start: run.start,
                rank: posting.rank,
                shape: (window_shape(run.member_hashes), candidate.hash),
                overlap: self.overlap(
                    source_id,
                    run.lines,
                    posting,
                    |matched| matched.children_lines,
                    similarity,
                    run.weight,
                ),
            });
        }
    }

    /// The smallest window size `run` stands for whose candidates include
    /// subtrees of `weight` nodes — where the pairwise comparison first
    /// meets the pair — if the run can match a subtree of that weight at
    /// all.
    fn window_size_for(&self, run: &Run, weight: u32) -> Option<u32> {
        if !sizes_are_close(run.weight, weight, self.options.size_tolerance) {
            return None;
        }
        (run.sizes.0..=run.sizes.1).find(|&size| {
            let (lightest, heaviest) =
                self.index.candidate_sizes[(size - self.options.min_window_size) as usize];
            (lightest..=heaviest).contains(&weight)
        })
    }

    /// Every subtree of a later function that may match a window of `run`,
    /// ordered by function and rank, each with the window size the
    /// pairwise comparison meets it at.
    ///
    /// A match shares enough members with the subtree's children for
    /// their Jaccard similarity to reach the threshold. The similarity is
    /// at most the share of the window's distinct members the subtree
    /// holds, so it holds at least `required` of them — and with that at
    /// least one of any `distinct - required + 1`. Looking up only that
    /// many, the rarest ones, and only among subtrees of a fitting weight,
    /// leaves most windows a handful of subtrees to compare with instead
    /// of every subtree of every function.
    fn candidates(&self, source_id: usize, run: &Run) -> Vec<(Posting, u32)> {
        let mut distinct: Vec<u64> = run.member_hashes.to_vec();
        distinct.sort_unstable();
        distinct.dedup();

        let share = |shared: usize| shared as f64 / distinct.len() as f64;
        let Some(required) =
            (1..=distinct.len()).find(|&shared| share(shared) >= self.options.threshold)
        else {
            // A threshold no similarity reaches.
            return Vec::new();
        };

        // Per member, the posting lists of the parents it has at a weight
        // the run can match, with the window size for that weight.
        let mut by_rarity: Vec<Vec<(&[Posting], u32)>> = distinct
            .iter()
            .map(|hash| {
                self.index
                    .by_child_hash
                    .get(hash)
                    .into_iter()
                    .flatten()
                    .filter_map(|(weight, postings)| {
                        let window_size = self.window_size_for(run, *weight)?;
                        Some((after(postings, source_id), window_size))
                    })
                    .collect()
            })
            .collect();
        by_rarity.sort_by_cached_key(|lists| {
            lists.iter().map(|(postings, _)| postings.len()).sum::<usize>()
        });

        let mut candidates: Vec<(Posting, u32)> = by_rarity
            .into_iter()
            .take(distinct.len() - required + 1)
            .flatten()
            .flat_map(|(postings, window_size)| {
                postings.iter().map(move |&posting| (posting, window_size))
            })
            .collect();
        // A subtree found through several of its children is one
        // candidate; its weight, and so its window size, is the same each
        // time.
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

/// Index every function of one file — including nested helpers, so a
/// duplicated inner function copied into another module still participates
/// in cross-file comparisons. Functions under a `similarity-ignore`
/// directive are left out, as they are in function mode.
fn index_file_functions<'a>(
    source: &'a FunctionSource<'a>,
    file: usize,
) -> Vec<ScannedFunction<'a>> {
    let lines = LineIndex::new(source.source_text);
    source
        .functions
        .iter()
        .filter(|definition| !definition.has_ignore_directive)
        .filter_map(|definition| {
            let indexed = index_function(definition, source, &lines)?;
            Some(ScannedFunction { definition, indexed, file })
        })
        .collect()
}

/// Index a function for overlap detection: parse its own source text —
/// exactly the declaration's span, wrapped in whatever makes that parse by
/// itself — and fingerprint the declaration's subtrees. `file_lines` is
/// the line index of the whole file. `None` when the text does not parse
/// on its own.
///
/// The function used to be cut out of the file by whole lines, which only
/// parses when the declaration starts and ends on lines of its own and is
/// a statement as it stands: every class method, every object member and
/// every function sharing a line with other code was left out.
fn index_function(
    func: &FunctionDefinition,
    source: &FunctionSource,
    file_lines: &LineIndex,
) -> Option<IndexedFunction> {
    let fragment = standalone_source(func, source.source_text)?;
    let tree = parse_and_convert_to_tree(source.path, &fragment.text).ok()?;

    // The fragment's tree holds the wrapper too; the declaration is the
    // node converted from exactly its text.
    let declaration_span = (fragment.offset, fragment.offset + func.span.size());
    let declaration = node_converted_from(&tree, declaration_span).unwrap_or(&tree);

    // A span of the fragment is a span of the file, shifted by where the
    // declaration starts in each.
    let last_offset = func.span.end.saturating_sub(1);
    let file_line = |offset: u32| {
        let in_file = func.span.start + offset.saturating_sub(fragment.offset);
        file_lines.line_of(in_file.min(last_offset))
    };
    let lines_of = |(start, end): (u32, u32)| (file_line(start), file_line(end.saturating_sub(1)));

    Some(IndexedFunction::new(
        func.name.clone(),
        declaration,
        &lines_of,
        (func.start_line, func.end_line),
    ))
}

/// The outermost node of `tree` converted from exactly `span`.
fn node_converted_from(tree: &Rc<TreeNode>, span: (u32, u32)) -> Option<&Rc<TreeNode>> {
    if tree.source_span == span {
        return Some(tree);
    }
    tree.children
        .iter()
        // A node's descendants lie inside its span.
        .filter(|child| {
            !child.has_source_span()
                || (child.source_span.0 <= span.0 && span.1 <= child.source_span.1)
        })
        .find_map(|child| node_converted_from(child, span))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtree_fingerprint::{detect_partial_overlaps, Window};

    fn index_source(path: &str, source_text: &str) -> Vec<IndexedFunction> {
        let functions = extract_functions(path, source_text).unwrap();
        index_file_functions(&FunctionSource { path, source_text, functions: &functions }, 0)
            .into_iter()
            .map(|scanned| scanned.indexed)
            .collect()
    }

    /// The scan's overlaps before repeated fragments are attributed to the
    /// innermost functions.
    fn scan_every_pair(
        files: &[(String, String)],
        options: &OverlapOptions,
        scope: PairScope,
    ) -> Vec<(String, String, PartialOverlap)> {
        let extracted: Vec<Vec<FunctionDefinition>> =
            files.iter().map(|(path, text)| extract_functions(path, text).unwrap()).collect();
        let sources: Vec<FunctionSource> = files
            .iter()
            .zip(&extracted)
            .map(|((path, source_text), functions)| FunctionSource { path, source_text, functions })
            .collect();
        let scan = Scan::new(&sources, options, scope);
        scan.overlaps()
            .into_iter()
            .map(|found| {
                (
                    scan.path_of(found.source).to_string(),
                    scan.path_of(found.target).to_string(),
                    found.overlap,
                )
            })
            .collect()
    }

    /// What the scan is defined to find before it attributes repeated
    /// fragments to the innermost functions: the pairwise comparison of
    /// every pair it compares, in its order.
    fn compare_every_pair(
        files: &[(String, String)],
        options: &OverlapOptions,
        scope: PairScope,
    ) -> Vec<(String, String, PartialOverlap)> {
        let mut files: Vec<&(String, String)> = files.iter().collect();
        files.sort_by_key(|(path, _)| path.as_str());
        let extracted: Vec<Vec<FunctionDefinition>> =
            files.iter().map(|(path, text)| extract_functions(path, text).unwrap()).collect();
        let sources: Vec<FunctionSource> = files
            .iter()
            .zip(&extracted)
            .map(|((path, source_text), functions)| FunctionSource { path, source_text, functions })
            .collect();
        let scanned: Vec<Vec<ScannedFunction>> = sources
            .iter()
            .enumerate()
            .map(|(file, source)| index_file_functions(source, file))
            .collect();

        let mut found = Vec::new();
        for (i, sources) in scanned.iter().enumerate() {
            let mut compare = |source: &ScannedFunction, target: &ScannedFunction, j: usize| {
                for overlap in detect_partial_overlaps(&source.indexed, &target.indexed, options) {
                    found.push((files[i].0.clone(), files[j].0.clone(), overlap));
                }
            };
            if scope.includes(true) {
                for (position, source) in sources.iter().enumerate() {
                    for target in &sources[position + 1..] {
                        if !source.definition.is_parent_child_relationship(target.definition) {
                            compare(source, target, i);
                        }
                    }
                }
            }
            if scope.includes(false) {
                for (j, targets) in scanned.iter().enumerate().skip(i + 1) {
                    for source in sources {
                        for target in targets {
                            compare(source, target, j);
                        }
                    }
                }
            }
        }
        found
    }

    fn overlaps_within(source_text: &str, options: &OverlapOptions) -> Vec<PartialOverlap> {
        let files = [("a.ts".to_string(), source_text.to_string())];
        find_overlaps_across_files(&files, options, PairScope::All)
            .into_iter()
            .map(|found| found.overlap)
            .collect()
    }

    const OPTIONS: OverlapOptions = OverlapOptions {
        min_window_size: 8,
        max_window_size: 30,
        threshold: 0.8,
        size_tolerance: 0.2,
    };

    /// `(source function, source lines, target function, target lines)`.
    type Located<'a> = (&'a str, (u32, u32), &'a str, (u32, u32));

    fn located(overlaps: &[PartialOverlap]) -> Vec<Located<'_>> {
        overlaps
            .iter()
            .map(|overlap| {
                (
                    overlap.source_function.as_str(),
                    overlap.source_lines,
                    overlap.target_function.as_str(),
                    overlap.target_lines,
                )
            })
            .collect()
    }

    #[test]
    fn every_declaration_form_is_indexed_from_its_own_node() {
        let source = r#"
function declared(items: number[]) { return items.map((item) => item * 2); }
const arrow = (items: number[]) => { return items.filter((item) => item > 2); };
const expression = function (items: number[]) { return items.length; };
class Box {
    method(items: number[]) { return items.slice(1); }
    field = (items: number[]) => { return items.slice(2); };
    get size() { return this.items.length; }
    constructor(private items: number[]) { this.items = items; }
}
const table = {
    member(items: number[]) { return items.concat(items); },
    valued: function (items: number[]) { return items.reverse(); },
    arrowed: (items: number[]) => items.join(","),
};
"#;
        let indexed = index_source("a.ts", source);
        let roots: Vec<(&str, &str, u32, u32)> = indexed
            .iter()
            .map(|function| {
                let root = &function.subtrees()[0];
                (function.name.as_str(), root.node_type, root.start_line, root.end_line)
            })
            .collect();
        assert_eq!(
            roots,
            [
                ("declared", "FunctionDeclaration", 2, 2),
                ("arrow", "ArrowFunctionExpression", 3, 3),
                ("expression", "FunctionExpression", 4, 4),
                ("Box.method", "MethodDefinition", 6, 6),
                ("Box.field", "ArrowFunctionExpression", 7, 7),
                ("Box.size", "MethodDefinition", 8, 8),
                ("Box.constructor", "MethodDefinition", 9, 9),
                ("table.member", "ObjectProperty", 12, 12),
                ("table.valued", "ObjectProperty", 13, 13),
                ("table.arrowed", "ArrowFunctionExpression", 14, 14),
            ]
        );
    }

    #[test]
    fn class_methods_and_object_members_take_part() {
        // Cut out of the file by whole lines, a method is no statement and
        // never parsed: overlap mode saw functions and nothing else.
        let source = r#"
class Importer {
    load(rows: Row[]) {
        const valid = [];
        for (const row of rows) {
            if (row.id > 0 && row.name.length > 0) {
                valid.push({ id: row.id, name: row.name.trim() });
            }
        }
        return this.store(valid);
    }
}

const exporter = {
    dump(entries: Row[]) {
        const valid = [];
        for (const row of entries) {
            if (row.id > 0 && row.name.length > 0) {
                valid.push({ id: row.id, name: row.name.trim() });
            }
        }
        return JSON.stringify(valid);
    },
};
"#;
        let overlaps = overlaps_within(source, &OPTIONS);
        // The shared `if` block, at its own lines on both sides — and only
        // it: the condition and the `push` inside it are the same code
        // again, not further overlaps.
        assert_eq!(
            located(&overlaps),
            [("Importer.load", (6, 8), "exporter.dump", (18, 20))],
            "{overlaps:#?}"
        );
    }

    #[test]
    fn functions_sharing_a_line_with_other_code_take_part() {
        let source = r#"
export const first = wrap(1), collect = (rows: Row[]) => {
    const names = [];
    for (const row of rows) {
        if (row.id > 0 && row.name.length > 0) { names.push(row.name.trim().toLowerCase()); }
    }
    return names;
}, last = wrap(2);

function gather(entries: Row[]) {
    const found = [];
    for (const row of entries) {
        if (row.id > 0 && row.name.length > 0) { found.push(row.name.trim().toLowerCase()); }
    }
    return found;
}
"#;
        let overlaps = overlaps_within(source, &OPTIONS);
        assert!(
            located(&overlaps).contains(&("collect", (5, 5), "gather", (13, 13))),
            "{overlaps:#?}"
        );
    }

    #[test]
    fn a_function_and_one_nested_on_the_same_lines_are_not_compared() {
        // The pair shares its line range, so line containment alone cannot
        // tell that one is inside the other.
        let source = "const outer = (rows: Row[]) => { const inner = (row: Row) => { return row.id > 0 && row.name.length > 0 ? row.name.trim() : String(row.id); }; return rows.map(inner); };\n";
        assert_eq!(index_source("a.ts", source).len(), 2);
        let overlaps = overlaps_within(source, &OPTIONS);
        assert!(overlaps.is_empty(), "{overlaps:#?}");
    }

    #[test]
    fn functions_that_only_share_a_line_are_compared() {
        // `second` starts on the line `first` ends on; neither contains
        // the other.
        let source = r#"
const first = (row: Row) => {
    return row.id > 0 && row.name.length > 0 ? row.name.trim() : String(row.id);
}, second = (row: Row) => { return row.id > 0 && row.name.length > 0 ? row.name.trim() : String(row.id); };
"#;
        let overlaps = overlaps_within(source, &OPTIONS);
        assert!(
            located(&overlaps).iter().any(|found| found.0 == "first" && found.2 == "second"),
            "{overlaps:#?}"
        );
    }

    #[test]
    fn ignored_functions_take_no_part() {
        let block = r#"{
    const valid = [];
    for (const row of rows) {
        if (row.id > 0 && row.name.length > 0) {
            valid.push({ id: row.id, name: row.name.trim() });
        }
    }
    return valid;
}"#;
        let plain = format!(
            "function first(rows: Row[]) {block}\n\nfunction second(rows: Row[]) {block}\n"
        );
        assert!(!overlaps_within(&plain, &OPTIONS).is_empty());
        let ignored = format!(
            "function first(rows: Row[]) {block}\n\n// similarity-ignore\nfunction second(rows: Row[]) {block}\n"
        );
        assert!(overlaps_within(&ignored, &OPTIONS).is_empty());
    }

    #[test]
    fn the_scope_limits_which_files_are_paired() {
        let block = r#"{
    const valid = [];
    for (const row of rows) {
        if (row.id > 0 && row.name.length > 0) {
            valid.push({ id: row.id, name: row.name.trim() });
        }
    }
    return valid;
}"#;
        let file = |first: &str, second: &str| {
            format!(
                "function {first}(rows: Row[]) {block}\n\nfunction {second}(rows: Row[]) {block}\n"
            )
        };
        let files =
            [("a.ts".to_string(), file("one", "two")), ("b.ts".to_string(), file("three", "four"))];
        let pairs = |scope: PairScope| -> Vec<(String, String)> {
            let mut pairs: Vec<(String, String)> =
                find_overlaps_across_files(&files, &OPTIONS, scope)
                    .into_iter()
                    .map(|found| (found.overlap.source_function, found.overlap.target_function))
                    .collect();
            pairs.sort();
            pairs.dedup();
            pairs
        };
        let name = |left: &str, right: &str| (left.to_string(), right.to_string());
        assert_eq!(pairs(PairScope::SameFileOnly), [name("one", "two"), name("three", "four")]);
        assert_eq!(
            pairs(PairScope::CrossFileOnly),
            [name("one", "four"), name("one", "three"), name("two", "four"), name("two", "three")]
        );
        assert_eq!(pairs(PairScope::All).len(), 6);
    }

    #[test]
    fn test_find_function_overlaps() {
        let source_code = r#"
function processData(items) {
    const results = [];
    for (let i = 0; i < items.length; i++) {
        if (items[i].value > 10) {
            results.push(items[i].value * 2);
        }
    }
    return results;
}

function helperFunction() {
    const data = [];
    for (let i = 0; i < 10; i++) {
        data.push(i * 2);
    }
    return data;
}
"#;

        let target_code = r#"
function transformData(elements) {
    const output = [];
    // Similar loop structure
    for (let j = 0; j < elements.length; j++) {
        if (elements[j].val > 10) {
            output.push(elements[j].val * 2);
        }
    }
    return output;
}

function utilityFunction() {
    const numbers = [];
    // Exact same loop as helperFunction
    for (let i = 0; i < 10; i++) {
        numbers.push(i * 2);
    }
    return numbers;
}
"#;

        let options = OverlapOptions {
            min_window_size: 3,
            max_window_size: 20,
            threshold: 0.5,      // Lower threshold
            size_tolerance: 0.5, // Higher tolerance
        };

        let overlaps = find_function_overlaps(source_code, target_code, &options);
        assert!(!overlaps.is_empty());
    }

    #[test]
    fn multiple_distinct_shared_blocks_survive_dedup() {
        // Two functions sharing TWO different duplicated regions must
        // report at least two overlaps — the old dedup collapsed every
        // overlap for a pair into one because all line ranges were the
        // whole function.
        let source_code = r#"
function alpha(items: number[]) {
    const evens = [];
    for (const item of items) {
        if (item % 2 === 0) {
            evens.push(item * 10);
        }
    }
    let total = 0;
    for (const even of evens) {
        total += even * even + 7;
    }
    return total;
}
"#;
        let target_code = r#"
function beta(values: number[]) {
    const evens = [];
    for (const item of values) {
        if (item % 2 === 0) {
            evens.push(item * 10);
        }
    }
    let sum = 1;
    for (const even of evens) {
        sum += even * even + 7;
    }
    return sum;
}
"#;
        let options = OverlapOptions {
            min_window_size: 5,
            max_window_size: 30,
            threshold: 0.8,
            size_tolerance: 0.25,
        };
        let overlaps = find_function_overlaps(source_code, target_code, &options);
        assert!(
            overlaps.len() >= 2,
            "expected at least two distinct overlap regions, got {}: {:?}",
            overlaps.len(),
            overlaps
        );
        // And the reported ranges must be sub-ranges, not the whole functions.
        assert!(
            overlaps.iter().any(|o| o.source_lines.1 - o.source_lines.0 < 10),
            "expected a narrow overlap range, got {:?}",
            overlaps
        );
    }

    #[test]
    fn nested_helpers_participate_in_cross_file_overlaps() {
        // The inner helper of `outer` is duplicated as a top-level
        // function in another file: it must stay indexed (only
        // parent/child PAIRS are skipped) so the cross-file overlap is
        // found.
        let file_a = r"
function outer(rows: number[][]) {
    function normalizeRow(row: number[]): number[] {
        const scaled = [];
        for (const cell of row) {
            if (cell > 0) {
                scaled.push(cell * 100 + 7);
            }
        }
        return scaled;
    }
    return rows.map(normalizeRow);
}
";
        let file_b = r"
export function normalizeVector(row: number[]): number[] {
    const scaled = [];
    for (const cell of row) {
        if (cell > 0) {
            scaled.push(cell * 100 + 7);
        }
    }
    return scaled;
}
";
        let files =
            [("a.ts".to_string(), file_a.to_string()), ("b.ts".to_string(), file_b.to_string())];
        let options = OverlapOptions {
            min_window_size: 5,
            max_window_size: 30,
            threshold: 0.8,
            size_tolerance: 0.25,
        };
        let overlaps = find_overlaps_across_files(&files, &options, PairScope::All);
        let involves_inner_helper = overlaps.iter().any(|o| {
            (o.overlap.source_function == "normalizeRow"
                || o.overlap.target_function == "normalizeRow")
                && (o.overlap.source_function == "normalizeVector"
                    || o.overlap.target_function == "normalizeVector")
        });
        assert!(
            involves_inner_helper,
            "expected the nested helper to overlap its cross-file copy: {overlaps:?}"
        );
    }

    #[test]
    fn sliding_windows_do_not_depend_on_hash_map_order() {
        // Every index gets its own randomly seeded hash maps; windows used
        // to inherit their tie order (subtrees starting on the same line),
        // so two indexes of one function produced different windows.
        let source = r#"
function settle(items: Item[], limit: number): number {
    let total = 0; let seen = 0; const skipped: string[] = [];
    for (const item of items) { if (item.price > limit) { skipped.push(item.id); continue; } total += item.price * item.qty; seen++; }
    if (seen === 0) { throw new Error("nothing to settle: " + skipped.join(",")); }
    return Math.round(total * 100) / 100;
}
"#;
        let signature = |indexed: &IndexedFunction| -> Vec<Vec<Window>> {
            (5..=30).map(|size| indexed.sliding_windows(size)).collect()
        };
        let first = index_source("a.ts", source);
        let reference = signature(&first[0]);
        for _ in 0..8 {
            let again = index_source("a.ts", source);
            assert_eq!(signature(&again[0]), reference);
        }
    }

    #[test]
    fn a_fragment_of_a_nested_function_is_reported_once() {
        // `collect` is declared inside `build` on both sides, so the loop
        // is a fragment of four pairs of functions: build/build,
        // build/collect, collect/build and collect/collect.
        let file = |outer: &str| {
            format!(
                r#"
export function {outer}(rows: Row[]) {{
    const collect = (entries: Row[]) => {{
        const names = [];
        for (const row of entries) {{
            if (row.id > 0 && row.name.length > 0) {{
                names.push(row.name.trim().toLowerCase());
            }}
        }}
        return names;
    }};
    return collect(rows);
}}
"#
            )
        };
        let files =
            [("a.ts".to_string(), file("buildLeft")), ("b.ts".to_string(), file("buildRight"))];

        let every_pair = scan_every_pair(&files, &OPTIONS, PairScope::All);
        let pairs_of = |found: &[(String, String, PartialOverlap)]| -> Vec<(String, String)> {
            let mut pairs: Vec<(String, String)> = found
                .iter()
                .map(|(_, _, overlap)| {
                    (overlap.source_function.clone(), overlap.target_function.clone())
                })
                .collect();
            pairs.sort();
            pairs.dedup();
            pairs
        };
        assert_eq!(pairs_of(&every_pair).len(), 4, "{every_pair:#?}");

        // The body of the loop inside `collect` is reported for `collect`
        // alone.
        let reported: Vec<PartialOverlap> =
            find_overlaps_across_files(&files, &OPTIONS, PairScope::All)
                .into_iter()
                .map(|found| found.overlap)
                .collect();
        assert_eq!(located(&reported), [("collect", (5, 9), "collect", (5, 9))], "{reported:#?}");
    }

    #[test]
    fn the_scan_finds_what_comparing_every_pair_finds() {
        // Functions, methods, object members and nested helpers that share
        // whole blocks, parts of blocks, runs of small statements and
        // nothing at all, over three files.
        let orders = r#"
export class OrderService {
    constructor(private readonly store: Store, private readonly log: Logger) {}

    validate(order: Order): string[] {
        const errors: string[] = [];
        if (!order.id) { errors.push("id"); }
        if (!order.customer) { errors.push("customer"); }
        if (order.total < 0) { errors.push("total"); }
        for (const line of order.lines) {
            if (line.quantity <= 0 || line.price < 0) {
                errors.push(`line ${line.sku}`);
            }
        }
        return errors;
    }

    async save(order: Order): Promise<void> {
        const errors = this.validate(order);
        if (errors.length > 0) {
            this.log.warn("rejected", { id: order.id, errors });
            throw new ValidationError(errors.join(", "));
        }
        await this.store.put(order.id, order);
        this.log.info("saved", { id: order.id, total: order.total });
    }

    total(orders: Order[]): number {
        let sum = 0;
        for (const order of orders) {
            if (order.status === "paid" && order.total > 0) {
                sum += order.total * (1 - order.discount);
            }
        }
        return Math.round(sum * 100) / 100;
    }
}

export function summarize(orders: Order[]): Summary {
    const byCustomer = new Map<string, number>();
    for (const order of orders) {
        const key = order.customer.trim().toLowerCase();
        byCustomer.set(key, (byCustomer.get(key) ?? 0) + order.total);
    }
    const sorted = [...byCustomer.entries()].sort((a, b) => b[1] - a[1]);
    return { customers: sorted.length, top: sorted.slice(0, 3), codes: [1, 2, 3, 5, 8, 13, 21, 34, 55] };
}
"#;
        let invoices = r#"
export class InvoiceService {
    constructor(private readonly store: Store, private readonly log: Logger) {}

    check(invoice: Invoice): string[] {
        const errors: string[] = [];
        if (!invoice.id) { errors.push("id"); }
        if (!invoice.customer) { errors.push("customer"); }
        if (invoice.total < 0) { errors.push("total"); }
        for (const line of invoice.lines) {
            if (line.quantity <= 0 || line.price < 0) {
                errors.push(`line ${line.sku}`);
            }
        }
        return errors;
    }

    async persist(invoice: Invoice): Promise<void> {
        const errors = this.check(invoice);
        if (errors.length > 0) {
            this.log.warn("rejected", { id: invoice.id, errors });
            throw new ValidationError(errors.join(", "));
        }
        await this.store.put(invoice.id, invoice);
    }
}

export const reports = {
    revenue(orders: Order[]): number {
        let sum = 0;
        for (const order of orders) {
            if (order.status === "paid" && order.total > 0) {
                sum += order.total * (1 - order.discount);
            }
        }
        return sum;
    },
    perCustomer: (orders: Order[]) => {
        const byCustomer = new Map<string, number>();
        for (const order of orders) {
            const key = order.customer.trim().toLowerCase();
            byCustomer.set(key, (byCustomer.get(key) ?? 0) + order.total);
        }
        return byCustomer;
    },
    codes: () => [1, 2, 3, 5, 8, 13, 21, 34, 89],
};
"#;
        let misc = r#"
export function outer(rows: number[][]) {
    function normalizeRow(row: number[]): number[] {
        const scaled = [];
        for (const cell of row) {
            if (cell > 0) { scaled.push(cell * 100 + 7); }
        }
        return scaled;
    }
    const first = (row: number[]) => { const scaled = []; for (const cell of row) { if (cell > 0) { scaled.push(cell * 100 + 7); } } return scaled; };
    return rows.map(normalizeRow).concat(rows.map(first));
}

export function unrelated(text: string): string {
    return text.split("\n").map((line) => line.trimEnd()).filter(Boolean).join("\n");
}

// similarity-ignore
export function ignored(orders: Order[]): number {
    let sum = 0;
    for (const order of orders) {
        if (order.status === "paid" && order.total > 0) {
            sum += order.total * (1 - order.discount);
        }
    }
    return sum;
}
"#;
        // Listed out of path order: the scan sorts them.
        let files = [
            ("src/orders.ts".to_string(), orders.to_string()),
            ("src/misc.ts".to_string(), misc.to_string()),
            ("src/invoices.ts".to_string(), invoices.to_string()),
        ];
        let option_sets = [
            OPTIONS,
            // Small windows: the incidental rule and the multiplicity
            // signal apply.
            OverlapOptions { min_window_size: 3, max_window_size: 12, ..OPTIONS },
            OverlapOptions { min_window_size: 1, max_window_size: 6, threshold: 0.5, ..OPTIONS },
            // Windows that stand for a single size, and sizes without
            // tolerance.
            OverlapOptions { min_window_size: 10, max_window_size: 10, ..OPTIONS },
            OverlapOptions { size_tolerance: 0.0, ..OPTIONS },
            // Low and unreachable thresholds, a wide tolerance, large
            // windows.
            OverlapOptions { threshold: 0.3, size_tolerance: 0.5, ..OPTIONS },
            OverlapOptions { threshold: 0.0, max_window_size: 12, ..OPTIONS },
            OverlapOptions { threshold: 1.0, ..OPTIONS },
            OverlapOptions { threshold: 1.5, ..OPTIONS },
            OverlapOptions { min_window_size: 20, max_window_size: 80, ..OPTIONS },
            // No window at all.
            OverlapOptions { min_window_size: 9, max_window_size: 8, ..OPTIONS },
        ];
        let mut compared = 0;
        for options in &option_sets {
            for scope in [PairScope::All, PairScope::SameFileOnly, PairScope::CrossFileOnly] {
                let expected = compare_every_pair(&files, options, scope);
                let scanned = scan_every_pair(&files, options, scope);
                assert_eq!(scanned, expected, "{options:?} {scope:?}");
                compared += expected.len();
            }
        }
        // The fixtures do overlap: the comparison is not of empty lists.
        assert!(compared > 100, "{compared}");
        assert!(!compare_every_pair(&files, &OPTIONS, PairScope::All).is_empty());
        assert!(compare_every_pair(&files, &option_sets[8], PairScope::All)
            .iter()
            .all(|(_, _, overlap)| overlap.similarity >= 1.0));
    }
}
