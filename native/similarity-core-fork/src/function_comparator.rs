//! Function comparison: the score of one pair, and the pairwise scan.

use std::cell::OnceCell;
use std::rc::Rc;

use crate::function_atoms::{dependent_statements_permuted, AtomSets, BodyStatements};
use crate::function_extractor::{extract_functions, FunctionDefinition};
use crate::function_fragment::parse_function_for_comparison;
use crate::tree::TreeNode;
use crate::tsed::{calculate_tsed, calculate_tsed_with_threshold, TSEDOptions};

#[derive(Debug, Clone)]
pub struct SimilarityResult {
    pub func1: FunctionDefinition,
    pub func2: FunctionDefinition,
    pub similarity: f64,
    /// Lines a refactor could remove: the smaller function's line count.
    pub impact: u32,
}

impl SimilarityResult {
    #[must_use]
    pub fn new(func1: FunctionDefinition, func2: FunctionDefinition, similarity: f64) -> Self {
        let impact = func1.line_count().min(func2.line_count());
        SimilarityResult { func1, func2, similarity, impact }
    }
}

/// A function ready to be compared: its canonical tree, and what the
/// scoring layers read off that tree. Each of those is the same for every
/// pair the function takes part in, so it is computed once, the first time
/// a pair gets far enough to need it.
struct ComparableFunction<'a> {
    func: &'a FunctionDefinition,
    tree: Rc<TreeNode>,
    atoms: OnceCell<AtomSets>,
    body_statements: OnceCell<Option<BodyStatements>>,
    yields_promise: OnceCell<bool>,
}

impl<'a> ComparableFunction<'a> {
    fn parse(filename: &str, func: &'a FunctionDefinition, source: &str) -> Result<Self, String> {
        Ok(Self {
            func,
            tree: parse_function_for_comparison(filename, func, source)?,
            atoms: OnceCell::new(),
            body_statements: OnceCell::new(),
            yields_promise: OnceCell::new(),
        })
    }

    fn atoms(&self) -> &AtomSets {
        self.atoms.get_or_init(|| AtomSets::of(&self.tree))
    }

    fn body_statements(&self) -> Option<&BodyStatements> {
        self.body_statements.get_or_init(|| BodyStatements::of(&self.tree)).as_ref()
    }

    /// Whether the function visibly hands its callers a promise without
    /// being `async`: it declares `Promise<…>`/`PromiseLike<…>`, chains a
    /// promise the canonicalizer lowered to `await`, or returns
    /// `new Promise(…)` or a `Promise.*(…)` call.
    fn yields_promise(&self) -> bool {
        self.func.declares_promise_return
            || *self
                .yields_promise
                .get_or_init(|| fragment_awaits(&self.tree) || fragment_returns_promise(&self.tree))
    }
}

/// Compare similarity between two functions.
///
/// # Errors
///
/// Returns an error when either function cannot be parsed on its own.
pub fn compare_functions(
    func1: &FunctionDefinition,
    func2: &FunctionDefinition,
    source1: &str,
    source2: &str,
    options: &TSEDOptions,
) -> Result<f64, String> {
    compare_functions_with_threshold(func1, func2, source1, source2, options, 0.0)
}

/// Variant that allows callers to provide a similarity threshold so the
/// edit-distance computation can give up early when the result is provably
/// below `threshold`. For such pairs the returned similarity is `0.0`, so
/// callers should still apply their own `>= threshold` check before
/// recording the pair.
///
/// # Errors
///
/// Returns an error when either function cannot be parsed on its own.
pub fn compare_functions_with_threshold(
    func1: &FunctionDefinition,
    func2: &FunctionDefinition,
    source1: &str,
    source2: &str,
    options: &TSEDOptions,
    threshold: f64,
) -> Result<f64, String> {
    let left = ComparableFunction::parse("func1.ts", func1, source1)?;
    let right = ComparableFunction::parse("func2.ts", func2, source2)?;
    Ok(score_pair(&left, &right, options, threshold))
}

/// The similarity of two prepared functions.
#[allow(clippy::cast_precision_loss)]
fn score_pair(
    left: &ComparableFunction,
    right: &ComparableFunction,
    options: &TSEDOptions,
    threshold: f64,
) -> f64 {
    let size1 = left.tree.subtree_size() as f64;
    let size2 = right.tree.subtree_size() as f64;
    // Pre-filter by size ratio. The minimum edit distance is at least
    // `|size1 - size2| * min(delete_cost, insert_cost)` (one tree has to
    // consume the size gap via deletes or inserts at that per-op rate),
    // so the base TSED is bounded above by
    //   `1 - |size1 - size2| * min_op_cost / max_size`.
    // When `delete_cost = insert_cost = 1.0` (the only configuration the
    // WASM CLI ever sets and the documented default) this collapses to
    // `size_ratio`, which gives us a clean `size_ratio < threshold` gate.
    //
    // Direct callers can override `apted_options` though, and with
    // smaller per-op costs the bound is looser — pairs that the simple
    // `size_ratio` formula would discard might still legitimately clear
    // the threshold. Skip the prune entirely in that case rather than
    // attempt a more elaborate bound; the threshold-aware cutoff below
    // still keeps the worst case bounded.
    if threshold > 0.0 && size1 > 0.0 && size2 > 0.0 {
        let delete_cost = options.apted_options.delete_cost;
        let insert_cost = options.apted_options.insert_cost;
        if delete_cost >= 1.0 && insert_cost >= 1.0 {
            let size_ratio = size1.min(size2) / size1.max(size2);
            if size_ratio < threshold {
                return 0.0;
            }
        }
    }

    // Use the threshold-pruned distance when the caller actually supplied
    // a useful budget; otherwise fall through to the full computation so
    // callers that expect the exact score still get it.
    let mut similarity = if threshold > 0.0 {
        // Reserve some headroom for the line-count penalty that may
        // follow — be conservative so we never under-report a real
        // duplicate. Using `threshold * 0.7` keeps the budget loose
        // enough that the penalty layer can shave the score without
        // pushing it below an "exact-zero" sentinel.
        let apted_threshold = (threshold * 0.7).max(0.0);
        let sim = calculate_tsed_with_threshold(&left.tree, &right.tree, options, apted_threshold);
        if sim == 0.0 {
            return 0.0;
        }
        sim
    } else {
        calculate_tsed(&left.tree, &right.tree, options)
    };

    // Apply line-count size penalty for short functions if enabled. This is a
    // second, coarser guard on top of the node-count penalty in
    // `calculate_tsed` — the purpose is to filter trivial lookalikes like
    // `() => 0` vs `() => 1`. But the original formulation (`similarity *=
    // avg_lines / 10.0` for functions under 10 lines) was far too aggressive
    // for rename-only duplicates: a genuinely-duplicated 6-line helper with
    // renamed identifiers would have its real ~0.91 TSED reduced to 0.55
    // purely because it happened to be short, hiding obvious duplication from
    // the default 0.8 threshold. We still want to discount pairs where the
    // similarity signal is thin AND the function is short, but we should not
    // punish high-similarity matches on short functions just for being short.
    if options.size_penalty {
        let avg_lines = f64::from(left.func.line_count() + right.func.line_count()) / 2.0;
        if avg_lines < 10.0 {
            // Confidence softener: scale the base short-function factor back
            // toward 1.0 as the raw similarity approaches 1.0. At sim >= 0.92
            // (essentially identical bodies) we apply no further penalty;
            // below 0.6 we apply the full original short-function discount.
            // The previous curve still pulled identical 3-line functions
            // down by ~30% because the confidence range bottomed out at
            // similarity 0.6 — but identical bodies already register at the
            // node-count layer's softened score (~0.85) so the avg_lines
            // multiplier should let those through.
            let base_factor = (avg_lines / 10.0).max(0.1);
            let confidence = ((similarity - 0.6) / 0.32).clamp(0.0, 1.0);
            let effective_factor = base_factor + (1.0 - base_factor) * confidence;
            similarity *= effective_factor;
        }
    }

    // Behavioral-atom twin cap. Alpha-renaming makes true rename clones
    // exactly equal, so a pair that is near-identical EXCEPT for a couple
    // of behavior-carrying atoms — a different callee (`sendEmail` vs
    // `sendSms`), a flipped operator (`*` vs `/`, `&&` vs `||`), a
    // different builtin (`find` vs `findLast`), a for-of turned for-in —
    // is precisely the "twins that do different things" false-positive
    // shape. The direction of the difference matters:
    //
    //   * SUBSTITUTIONS (each side has atoms the other lacks) rewrite
    //     behavior and cap hard;
    //   * one-sided ADDITIONS (one function is the other plus extra
    //     work, e.g. an added logging line) are the classic
    //     copy-paste-and-extend duplicate and stay reportable;
    //   * any CONTROL-flow atom difference (loop kind, `break`,
    //     `continue`) caps hard even one-sided — an added `break`
    //     changes what the loop computes, unlike an added call.
    //
    // Plain string/number/boolean literals are deliberately NOT atoms:
    // twins differing only in data constants are parameterizable
    // duplicates (the corpus's F-P02 precedent).
    if similarity > 0.6 {
        let difference = left.atoms().difference(right.atoms());
        if difference.control > 0 {
            similarity = similarity.min(0.72);
        } else if difference.missing_in_left > 0 && difference.missing_in_right > 0 {
            let total = difference.missing_in_left + difference.missing_in_right;
            let cap = (0.75 - 0.03 * (total.saturating_sub(2)) as f64).max(0.55);
            similarity = similarity.min(cap);
        } else if difference.missing_in_left + difference.missing_in_right >= 4 {
            similarity = similarity.min(0.85);
        }
    }

    // Call-contract cap: an `async` function returns a promise and a
    // generator returns an iterator, so a plain twin with the same body is
    // a different contract for every caller (`getSymbol(): Promise<Symbol>`
    // in an async API vs `getSymbol(): Symbol` in its sync sibling). The
    // one equivalence is a sync function that chains the same promise —
    // `return fetch(u).then((r) => …)` vs `const r = await fetch(u); …` —
    // and the canonicalizer has already lowered that `.then` into the
    // `await` the async twin spells, so an awaiting sync side is not a
    // contract change. The marker used to be one relabelled node (or, for
    // static methods and accessors, nothing at all), which identical
    // bodies easily outweighed.
    if similarity > 0.6 && call_contract_differs(left, right) {
        similarity = similarity.min(0.72);
    }

    // Statement-order factor: two bodies made of the same statements in a
    // different order share all their atoms and most of their tree. When
    // the reordered statements touch the same identifiers, order is
    // behavior (`total += fee; total *= rate;` vs the reverse) and the
    // pair gets a distinctness multiplier; independent statements (two
    // unrelated `const`s swapped) reorder freely and stay untouched.
    if similarity >= 0.8
        && dependent_statements_permuted(left.body_statements(), right.body_statements())
    {
        similarity *= 0.85;
    }

    similarity
}

/// Whether the two functions expose different call contracts: generator vs
/// plain, or `async` vs a sync function that cannot be shown to return a
/// promise (see [`ComparableFunction::yields_promise`]); without any such
/// evidence, nothing in the syntax says its callers get a promise.
fn call_contract_differs(left: &ComparableFunction, right: &ComparableFunction) -> bool {
    if left.func.is_generator != right.func.is_generator {
        return true;
    }
    if left.func.is_async == right.func.is_async {
        return false;
    }
    let sync_side = if left.func.is_async { right } else { left };
    !sync_side.yields_promise()
}

fn is_function_like_node(kind: &str) -> bool {
    matches!(
        kind,
        "FunctionDeclaration"
            | "FunctionExpression"
            | "ArrowFunctionExpression"
            | "MethodDefinition"
    )
}

/// Whether `matches` holds for a node of the comparison fragment's own
/// function — nodes inside nested functions belong to those functions,
/// not to the fragment.
fn fragment_has_own_node(tree: &TreeNode, matches: &dyn Fn(&TreeNode) -> bool) -> bool {
    fn walk(node: &TreeNode, inside_fragment: bool, matches: &dyn Fn(&TreeNode) -> bool) -> bool {
        if inside_fragment && matches(node) {
            return true;
        }
        node.children.iter().any(|child| {
            if is_function_like_node(child.kind) {
                // The first function-like node is the fragment itself;
                // any function inside it acts for its own sake.
                !inside_fragment && walk(child, true, matches)
            } else {
                walk(child, inside_fragment, matches)
            }
        })
    }
    walk(tree, false, matches)
}

/// Whether the fragment's own function body contains an `await` (or
/// `for await`).
fn fragment_awaits(tree: &TreeNode) -> bool {
    fragment_has_own_node(tree, &|node| {
        node.kind == "AwaitExpression" || node.label == "ForAwaitOfStatement"
    })
}

/// Whether one of the fragment's own `return`s yields `new Promise(…)` or
/// a `Promise.resolve(…)`-style call.
fn fragment_returns_promise(tree: &TreeNode) -> bool {
    fn is_promise_global(node: &TreeNode) -> bool {
        node.kind == "Identifier" && node.label == "Promise"
    }
    fn returns_promise(node: &TreeNode) -> bool {
        let Some(argument) = node.children.first() else {
            return false;
        };
        let Some(callee) = argument.children.first() else {
            return false;
        };
        match argument.kind {
            "NewExpression" => is_promise_global(callee),
            "CallExpression" => {
                callee.kind == "StaticMemberExpression"
                    && callee.children.first().is_some_and(|object| is_promise_global(object))
            }
            _ => false,
        }
    }
    fragment_has_own_node(tree, &|node| node.kind == "ReturnStatement" && returns_promise(node))
}

/// Pair-generation scope for [`find_similar_function_pairs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairScope {
    All,
    SameFileOnly,
    CrossFileOnly,
}

impl PairScope {
    /// Whether a pair of declarations from the same file (`true`) or from
    /// two files (`false`) is in scope.
    #[must_use]
    pub fn includes(self, same_file: bool) -> bool {
        match self {
            PairScope::All => true,
            PairScope::SameFileOnly => same_file,
            PairScope::CrossFileOnly => !same_file,
        }
    }
}

/// One source file and the functions extracted from it.
#[derive(Debug, Clone, Copy)]
pub struct FunctionSource<'a> {
    pub path: &'a str,
    pub source_text: &'a str,
    pub functions: &'a [FunctionDefinition],
}

/// A reported pair of functions, with the positions of their files in the
/// scanned slice.
#[derive(Debug, Clone)]
pub struct SimilarFunctionPair {
    pub left_file: usize,
    pub right_file: usize,
    pub result: SimilarityResult,
}

/// Similar-function scan across a set of files.
///
/// Every function is parsed for comparison exactly once, then a single
/// O(N²) pairwise loop runs over the pairs in `scope`. Functions under a
/// `similarity-ignore` directive, functions below the size gate
/// (`min_tokens` when set, `min_lines` otherwise) and functions that do not
/// parse on their own take no part.
///
/// Returns the pairs at or above `threshold`, ordered by impact
/// (descending), then similarity (descending).
#[must_use]
pub fn find_similar_function_pairs(
    sources: &[FunctionSource],
    threshold: f64,
    options: &TSEDOptions,
    scope: PairScope,
) -> Vec<SimilarFunctionPair> {
    struct Candidate<'a> {
        file_index: usize,
        function: ComparableFunction<'a>,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (file_index, source) in sources.iter().enumerate() {
        for func in source.functions {
            if func.has_ignore_directive {
                continue;
            }
            // The line gate needs no tree, so it runs before the parse —
            // parsing dominates the cost of preparing a function.
            if options.min_tokens.is_none() && func.line_count() < options.min_lines {
                continue;
            }
            let Ok(function) = ComparableFunction::parse(source.path, func, source.source_text)
            else {
                continue;
            };
            // The token gate is measured on the tree that gets compared,
            // so the gate and the score agree.
            if let Some(min_tokens) = options.min_tokens {
                let tree_size = u32::try_from(function.tree.subtree_size()).unwrap_or(u32::MAX);
                if tree_size < min_tokens {
                    continue;
                }
            }
            candidates.push(Candidate { file_index, function });
        }
    }

    let mut similar_pairs = Vec::new();
    for (i, left) in candidates.iter().enumerate() {
        for right in &candidates[i + 1..] {
            let same_file = left.file_index == right.file_index;
            if !scope.includes(same_file) {
                continue;
            }
            // Nested-function containment is only meaningful within one
            // file — across files the span/line comparison is between
            // unrelated coordinate spaces and used to spuriously drop
            // pairs whose ranges happened to nest.
            if same_file && left.function.func.is_parent_child_relationship(right.function.func) {
                continue;
            }

            let similarity = score_pair(&left.function, &right.function, options, threshold);
            if similarity >= threshold {
                similar_pairs.push(SimilarFunctionPair {
                    left_file: left.file_index,
                    right_file: right.file_index,
                    result: SimilarityResult::new(
                        left.function.func.clone(),
                        right.function.func.clone(),
                        similarity,
                    ),
                });
            }
        }
    }

    // Sort by impact (descending), then by similarity (descending).
    similar_pairs.sort_by(|a, b| {
        b.result
            .impact
            .cmp(&a.result.impact)
            .then(b.result.similarity.total_cmp(&a.result.similarity))
    });

    similar_pairs
}

/// A file that could not be analyzed, with the reason.
pub type SkippedFile = (String, String);

/// [`find_similar_function_pairs`] over `(path, source text)` pairs: the
/// functions are extracted first, and the files that cannot be analyzed
/// are returned next to the pairs. Pairs carry their two file paths.
#[must_use]
pub fn find_similar_functions_in_files(
    files: &[(String, String)],
    threshold: f64,
    options: &TSEDOptions,
    scope: PairScope,
) -> (Vec<(String, SimilarityResult, String)>, Vec<SkippedFile>) {
    let mut skipped = Vec::new();
    let mut extracted: Vec<(usize, Vec<FunctionDefinition>)> = Vec::new();
    for (index, (path, source_text)) in files.iter().enumerate() {
        match extract_functions(path, source_text) {
            Ok(functions) => extracted.push((index, functions)),
            Err(error) => skipped.push((path.clone(), error)),
        }
    }
    let sources: Vec<FunctionSource> = extracted
        .iter()
        .map(|(index, functions)| FunctionSource {
            path: &files[*index].0,
            source_text: &files[*index].1,
            functions,
        })
        .collect();
    let pairs = find_similar_function_pairs(&sources, threshold, options, scope)
        .into_iter()
        .map(|pair| {
            (
                sources[pair.left_file].path.to_string(),
                pair.result,
                sources[pair.right_file].path.to_string(),
            )
        })
        .collect();
    (pairs, skipped)
}

/// Find similar functions within one file.
///
/// # Errors
///
/// Returns an error when the file cannot be analyzed.
pub fn find_similar_functions_in_file(
    filename: &str,
    source_text: &str,
    threshold: f64,
    options: &TSEDOptions,
) -> Result<Vec<SimilarityResult>, String> {
    let files = [(filename.to_string(), source_text.to_string())];
    let (pairs, skipped) =
        find_similar_functions_in_files(&files, threshold, options, PairScope::SameFileOnly);
    if let Some((_, error)) = skipped.into_iter().next() {
        return Err(error);
    }
    Ok(pairs.into_iter().map(|(_, result, _)| result).collect())
}

/// Find similar functions across files (cross-file pairs only; same-file
/// pairs are the domain of [`find_similar_functions_in_file`]). Files that
/// cannot be analyzed are skipped.
#[must_use]
pub fn find_similar_functions_across_files(
    files: &[(String, String)],
    threshold: f64,
    options: &TSEDOptions,
) -> Vec<(String, SimilarityResult, String)> {
    find_similar_functions_in_files(files, threshold, options, PairScope::CrossFileOnly).0
}

#[cfg(test)]
mod tests;
