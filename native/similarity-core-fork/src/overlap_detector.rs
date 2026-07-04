use crate::{
    function_extractor::{extract_functions, FunctionDefinition},
    parser::parse_and_convert_to_tree,
    subtree_fingerprint::{
        detect_partial_overlaps, generate_subtree_fingerprints, IndexedFunction, LineIndex,
        OverlapOptions, PartialOverlap,
    },
    tsed::{calculate_tsed, TSEDOptions},
};
use std::collections::HashMap;

/// Overlap result with file information
#[derive(Debug, Clone)]
pub struct PartialOverlapWithFiles {
    pub source_file: String,
    pub target_file: String,
    pub overlap: PartialOverlap,
}

/// Detailed overlap with exact similarity and code snippets
#[derive(Debug, Clone)]
pub struct DetailedOverlap {
    pub overlap: PartialOverlap,
    pub exact_similarity: f64,
    pub source_code: String,
    pub target_code: String,
}

/// Extract and index every function of one file exactly once — including
/// nested helpers, so a duplicated inner function copied into another
/// module still participates in cross-file comparisons. Functions that
/// cannot be parsed standalone (e.g. constructors) are skipped; files
/// that fail to parse produce an empty index.
fn index_file_functions(file_name: &str, source: &str) -> Vec<IndexedFunction> {
    let Ok(functions) = extract_functions(file_name, source) else {
        return Vec::new();
    };
    let lines: Vec<&str> = source.lines().collect();
    let mut indexed = Vec::with_capacity(functions.len());
    for func in &functions {
        if let Ok(entry) = index_function(func, &lines, file_name) {
            indexed.push(entry);
        }
    }
    indexed
}

/// Whether one same-file function's line range strictly contains the
/// other's. A nested function lives entirely inside its parent, so the
/// parent's subtree fingerprints trivially include the child and every
/// parent/child pair would "overlap" at similarity 1.0 — a containment
/// artifact, not duplication. Only these PAIRS are skipped; the nested
/// function itself stays indexed for every other comparison.
fn is_nested_pair(left: &IndexedFunction, right: &IndexedFunction) -> bool {
    let left_contains_right = left.start_line <= right.start_line
        && left.end_line >= right.end_line
        && (left.start_line, left.end_line) != (right.start_line, right.end_line);
    let right_contains_left = right.start_line <= left.start_line
        && right.end_line >= left.end_line
        && (left.start_line, left.end_line) != (right.start_line, right.end_line);
    left_contains_right || right_contains_left
}

/// Detect overlapping code fragments between two source texts.
///
/// # Errors
///
/// Currently infallible (unparseable files yield no overlaps); the
/// `Result` shape is kept for API stability.
pub fn find_function_overlaps(
    source_code: &str,
    target_code: &str,
    options: &OverlapOptions,
) -> Result<Vec<PartialOverlap>, String> {
    // Identify "same file" via pointer equality only — a content-equality
    // fallback would wrongly collapse two distinct files that happen to be
    // byte-identical (legitimate full-file duplication we want to detect).
    let same_file = std::ptr::eq(source_code, target_code);

    let source_indexed = index_file_functions("source.ts", source_code);
    let target_indexed = if same_file {
        Vec::new()
    } else {
        index_file_functions("target.ts", target_code)
    };
    let target_view: &[IndexedFunction] =
        if same_file { &source_indexed } else { &target_indexed };

    let mut all_overlaps = Vec::new();
    for (source_position, source_func) in source_indexed.iter().enumerate() {
        for (target_position, target_func) in target_view.iter().enumerate() {
            // In a same-file scan the (source, target) pair space would
            // otherwise visit every unordered pair twice and produce
            // (A, A) self-pairs.
            if same_file && target_position <= source_position {
                continue;
            }
            if same_file && is_nested_pair(source_func, target_func) {
                continue;
            }
            all_overlaps.extend(detect_partial_overlaps(source_func, target_func, options));
        }
    }

    Ok(all_overlaps)
}

/// Detect overlaps across multiple files.
///
/// Each file's functions are extracted, parsed, and fingerprinted exactly
/// once (the previous implementation re-did all of that inside every
/// file-pair iteration, i.e. O(files²) parses). File pairs are visited in
/// sorted-filename order so results are deterministic.
///
/// # Errors
///
/// Currently infallible; the `Result` shape is kept for API stability.
pub fn find_overlaps_across_files(
    file_contents: &HashMap<String, String>,
    options: &OverlapOptions,
) -> Result<Vec<PartialOverlapWithFiles>, String> {
    let mut files: Vec<(&String, &String)> = file_contents.iter().collect();
    files.sort_by_key(|(name, _)| name.as_str());

    let indexed_per_file: Vec<Vec<IndexedFunction>> = files
        .iter()
        .map(|(name, source)| index_file_functions(name, source))
        .collect();

    let mut all_overlaps = Vec::new();
    for i in 0..files.len() {
        // Same-file scan: ordered pairs only, skipping parent/child
        // containment pairs.
        for (source_position, source_func) in indexed_per_file[i].iter().enumerate() {
            for (target_position, target_func) in indexed_per_file[i].iter().enumerate() {
                if target_position <= source_position {
                    continue;
                }
                if is_nested_pair(source_func, target_func) {
                    continue;
                }
                for overlap in detect_partial_overlaps(source_func, target_func, options) {
                    all_overlaps.push(PartialOverlapWithFiles {
                        source_file: files[i].0.clone(),
                        target_file: files[i].0.clone(),
                        overlap,
                    });
                }
            }
        }
        // Cross-file pairs.
        for j in (i + 1)..files.len() {
            for source_func in &indexed_per_file[i] {
                for target_func in &indexed_per_file[j] {
                    for overlap in detect_partial_overlaps(source_func, target_func, options) {
                        all_overlaps.push(PartialOverlapWithFiles {
                            source_file: files[i].0.clone(),
                            target_file: files[j].0.clone(),
                            overlap,
                        });
                    }
                }
            }
        }
    }

    Ok(all_overlaps)
}

/// Index a function for overlap detection. `lines` is the pre-split line
/// table of the whole file (shared across the file's functions).
fn index_function(
    func: &FunctionDefinition,
    lines: &[&str],
    file_name: &str,
) -> Result<IndexedFunction, String> {
    let start_line = (func.start_line as usize).saturating_sub(1);
    let end_line = func.end_line as usize;

    if start_line >= lines.len() || end_line > lines.len() {
        return Err("Function line numbers out of bounds".to_string());
    }

    let func_code = lines[start_line..end_line].join("\n");

    // Parse the function. Some shapes (class methods, constructors) are
    // not valid standalone snippets — the caller skips those.
    let tree = parse_and_convert_to_tree(file_name, &func_code)?;

    // Generate fingerprints for all subtrees, with a line index so each
    // fingerprint reports the real source range it covers.
    let line_index = LineIndex::new(&func_code);
    let (root_fp, subtrees) =
        generate_subtree_fingerprints(&tree, 0, func.start_line, Some(&line_index));

    let mut indexed = IndexedFunction::new(
        func.name.clone(),
        file_name.to_string(),
        func.start_line,
        func.end_line,
        root_fp,
    );

    for subtree in subtrees {
        indexed.add_subtree(subtree);
    }

    Ok(indexed)
}

/// Find overlaps with detailed similarity calculation.
///
/// # Errors
///
/// Returns an error when an overlapping segment cannot be re-parsed for
/// the exact-similarity pass.
pub fn find_overlaps_with_similarity(
    source_code: &str,
    target_code: &str,
    options: &OverlapOptions,
    tsed_options: &TSEDOptions,
) -> Result<Vec<DetailedOverlap>, String> {
    let overlaps = find_function_overlaps(source_code, target_code, options)?;
    let mut detailed_overlaps = Vec::new();

    for overlap in overlaps {
        // For high-similarity overlaps, calculate exact TSED similarity
        if overlap.similarity > 0.9 {
            let source_segment =
                extract_code_segment(source_code, overlap.source_lines.0, overlap.source_lines.1)?;
            let target_segment =
                extract_code_segment(target_code, overlap.target_lines.0, overlap.target_lines.1)?;

            // The segments are line-aligned slices and may not parse in
            // isolation (e.g. a block cut mid-statement); fall back to the
            // fingerprint similarity when that happens.
            let exact_similarity = match (
                parse_and_convert_to_tree("source.ts", &source_segment),
                parse_and_convert_to_tree("target.ts", &target_segment),
            ) {
                (Ok(source_tree), Ok(target_tree)) => {
                    calculate_tsed(&source_tree, &target_tree, tsed_options)
                }
                _ => overlap.similarity,
            };

            detailed_overlaps.push(DetailedOverlap {
                overlap: overlap.clone(),
                exact_similarity,
                source_code: source_segment,
                target_code: target_segment,
            });
        } else {
            detailed_overlaps.push(DetailedOverlap {
                overlap: overlap.clone(),
                exact_similarity: overlap.similarity,
                source_code: String::new(),
                target_code: String::new(),
            });
        }
    }

    Ok(detailed_overlaps)
}

/// Extract code segment by line numbers
fn extract_code_segment(code: &str, start_line: u32, end_line: u32) -> Result<String, String> {
    let lines: Vec<_> = code.lines().collect();

    if start_line as usize > lines.len() || end_line as usize > lines.len() {
        return Err("Line numbers out of bounds".to_string());
    }

    let start = (start_line as usize).saturating_sub(1);
    let end = (end_line as usize).min(lines.len());

    Ok(lines[start..end].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let overlaps = find_function_overlaps(source_code, target_code, &options).unwrap();
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
        let overlaps = find_function_overlaps(source_code, target_code, &options).unwrap();
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
    fn test_extract_code_segment() {
        let code = "line1\nline2\nline3\nline4\nline5";

        let segment = extract_code_segment(code, 2, 4).unwrap();
        assert_eq!(segment, "line2\nline3\nline4");

        let segment = extract_code_segment(code, 1, 5).unwrap();
        assert_eq!(segment, "line1\nline2\nline3\nline4\nline5");
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
        let mut files = HashMap::new();
        files.insert("a.ts".to_string(), file_a.to_string());
        files.insert("b.ts".to_string(), file_b.to_string());
        let options = OverlapOptions {
            min_window_size: 5,
            max_window_size: 30,
            threshold: 0.8,
            size_tolerance: 0.25,
        };
        let overlaps = find_overlaps_across_files(&files, &options).unwrap();
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
}
