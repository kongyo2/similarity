//! The analysis entry point behind the WebAssembly module: runs the
//! requested modes over a set of in-memory source files and reports the
//! similar pairs of each.

use serde::{Deserialize, Serialize};
use similarity_core::{
    extract_declarations, find_overlaps, find_similar_classes, find_similar_function_pairs,
    find_similar_types, DeclarationKinds, FileDeclarations, FunctionDefinition, FunctionSource,
    OverlapOptions, PairScope, TSEDOptions, TypeComparisonOptions, TypeDefinition, TypeKind,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub file_path: String,
    pub content: String,
}

/// One kind of analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Functions,
    Types,
    Classes,
    Overlap,
}

/// Which declarations type mode compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TypeFilter {
    #[default]
    All,
    Interface,
    Type,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeInput {
    pub files: Vec<ProjectFile>,
    pub modes: Vec<Mode>,
    pub threshold: f64,
    pub min_lines: Option<u32>,
    pub min_tokens: Option<u32>,
    pub size_penalty: Option<bool>,
    #[serde(default)]
    pub same_file_only: bool,
    #[serde(default)]
    pub cross_file_only: bool,
    #[serde(default)]
    pub types_only: TypeFilter,
    pub allow_cross_kind: Option<bool>,
    #[serde(default)]
    pub include_type_literals: bool,
    pub overlap_min_window: Option<u32>,
    pub overlap_max_window: Option<u32>,
    pub overlap_size_tolerance: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeOutput {
    pub analyzed_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub warnings: Vec<AnalyzeWarning>,
    /// Every pair of every mode, most similar first.
    pub results: Vec<SimilarityPair>,
    pub by_mode: ByMode,
    pub stats: AnalyzeStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeWarning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeStats {
    /// Files that were analyzed; skipped files do not count.
    pub file_count: usize,
    pub pair_count: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ByMode {
    pub functions: Vec<SimilarityPair>,
    pub types: Vec<SimilarityPair>,
    pub classes: Vec<SimilarityPair>,
    pub overlap: Vec<SimilarityPair>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimilarityPair {
    pub mode: Mode,
    pub similarity: f64,
    pub left: AnalyzerLocation,
    pub right: AnalyzerLocation,
    pub details: PairDetails,
}

/// What a mode reports about a pair besides its similarity.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum PairDetails {
    /// Functions: the lines a refactor could remove.
    Function { impact: u32 },
    /// Types and classes: the two terms of the similarity.
    #[serde(rename_all = "camelCase")]
    Shape { naming_similarity: f64, structural_similarity: f64 },
    /// Overlap: the size and root of the shared fragment.
    #[serde(rename_all = "camelCase")]
    Fragment { node_count: u32, node_type: &'static str },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzerLocation {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol_name: String,
    pub kind: &'static str,
}

/// Largest overlap window, in nodes. Overlap mode looks for fragments
/// *inside* functions; a window of thousands of nodes is a whole function,
/// which function mode compares, and every size up to the largest costs a
/// pass over every function.
pub const MAX_OVERLAP_WINDOW: u32 = 1000;

/// A source file that passed the checks, with what was extracted from it.
struct AnalyzedFile<'a> {
    file: &'a ProjectFile,
    declarations: FileDeclarations,
}

/// Run the analysis.
///
/// # Errors
///
/// Returns an error for options that are out of range or contradict each
/// other. A source file that cannot be analyzed is not an error: it is
/// reported in `skipped_files` and `warnings`, whatever the modes, and the
/// rest is analyzed without it.
pub fn analyze_project(input: &AnalyzeInput) -> Result<AnalyzeOutput, String> {
    let overlap_options = overlap_options(input)?;
    // `contains` is false for NaN.
    if !(0.0..=1.0).contains(&input.threshold) {
        return Err("threshold must be between 0 and 1".to_string());
    }
    let scope = match (input.same_file_only, input.cross_file_only) {
        (true, true) => {
            return Err("sameFileOnly and crossFileOnly cannot both be set".to_string());
        }
        (true, false) => PairScope::SameFileOnly,
        (false, true) => PairScope::CrossFileOnly,
        (false, false) => PairScope::All,
    };
    let wants = |mode: Mode| input.modes.contains(&mode);

    // Every file is parsed once, for all modes: what one mode cannot
    // analyze, none can, and it is reported once.
    let kinds = DeclarationKinds {
        functions: wants(Mode::Functions) || wants(Mode::Overlap),
        types: wants(Mode::Types),
        type_literals: wants(Mode::Types) && input.include_type_literals,
        classes: wants(Mode::Classes),
    };
    let mut analyzed: Vec<AnalyzedFile> = Vec::with_capacity(input.files.len());
    let mut skipped_files = Vec::new();
    let mut warnings = Vec::new();
    for file in &input.files {
        match extract_declarations(&file.file_path, &file.content, kinds) {
            Ok(declarations) => analyzed.push(AnalyzedFile { file, declarations }),
            Err(error) => {
                warnings.push(AnalyzeWarning {
                    file_path: Some(file.file_path.clone()),
                    message: format!("skipped: {error}"),
                });
                skipped_files.push(file.file_path.clone());
            }
        }
    }

    let mut by_mode = ByMode::default();
    if wants(Mode::Functions) {
        by_mode.functions = function_pairs(&analyzed, input, scope);
    }
    if wants(Mode::Types) {
        by_mode.types = type_pairs(&analyzed, input, scope);
    }
    if wants(Mode::Classes) {
        by_mode.classes = class_pairs(&analyzed, input, scope);
    }
    if wants(Mode::Overlap) {
        by_mode.overlap = overlap_pairs(&analyzed, &overlap_options, scope);
    }

    let mut results: Vec<SimilarityPair> = [
        by_mode.functions.as_slice(),
        by_mode.types.as_slice(),
        by_mode.classes.as_slice(),
        by_mode.overlap.as_slice(),
    ]
    .concat();
    results.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));

    Ok(AnalyzeOutput {
        analyzed_files: analyzed.iter().map(|entry| entry.file.file_path.clone()).collect(),
        skipped_files,
        warnings,
        stats: AnalyzeStats { file_count: analyzed.len(), pair_count: results.len() },
        results,
        by_mode,
    })
}

/// Whether a source file can be analyzed.
///
/// # Errors
///
/// Returns the reason it cannot: syntax errors, or code nested too deeply.
pub fn check_source(file_path: &str, content: &str) -> Result<(), String> {
    similarity_core::check_source(file_path, content)
}

/// [`analyze_project`] from and to JSON, the form the WebAssembly boundary
/// carries.
///
/// # Errors
///
/// Returns an error when the input is not a valid [`AnalyzeInput`] or its
/// options contradict each other.
pub fn analyze_project_json(input_json: &str) -> Result<String, String> {
    let input: AnalyzeInput =
        serde_json::from_str(input_json).map_err(|error| format!("invalid input: {error}"))?;
    let output = analyze_project(&input)?;
    serde_json::to_string(&output).map_err(|error| format!("serialization error: {error}"))
}

/// The functions of every analyzed file, the files in path order: which
/// function of a pair is reported on the left, and the order of pairs that
/// are equally similar, follow the order the files are compared in — and
/// must not follow the order they happened to be listed in.
fn function_sources<'a>(analyzed: &'a [AnalyzedFile<'a>]) -> Vec<FunctionSource<'a>> {
    let mut sources: Vec<FunctionSource> = analyzed
        .iter()
        .map(|entry| FunctionSource {
            path: &entry.file.file_path,
            source_text: &entry.file.content,
            functions: &entry.declarations.functions,
        })
        .collect();
    sources.sort_by_key(|source| source.path);
    sources
}

fn function_location(file_path: &str, function: FunctionDefinition) -> AnalyzerLocation {
    AnalyzerLocation {
        file_path: file_path.to_string(),
        start_line: function.start_line as usize,
        end_line: function.end_line as usize,
        symbol_name: function.name,
        kind: function.function_type.as_str(),
    }
}

fn function_pairs(
    analyzed: &[AnalyzedFile],
    input: &AnalyzeInput,
    scope: PairScope,
) -> Vec<SimilarityPair> {
    let mut options = TSEDOptions::default();
    if let Some(min_lines) = input.min_lines {
        options.min_lines = min_lines;
    }
    options.min_tokens = input.min_tokens;
    if let Some(size_penalty) = input.size_penalty {
        options.size_penalty = size_penalty;
    }

    let sources = function_sources(analyzed);
    let mut pairs: Vec<SimilarityPair> =
        find_similar_function_pairs(&sources, input.threshold, &options, scope)
            .into_iter()
            .map(|pair| SimilarityPair {
                mode: Mode::Functions,
                similarity: pair.result.similarity,
                left: function_location(sources[pair.left_file].path, pair.result.func1),
                right: function_location(sources[pair.right_file].path, pair.result.func2),
                details: PairDetails::Function { impact: pair.result.impact },
            })
            .collect();

    // Highest similarity first; ties keep the scan's impact-descending
    // order (this sort is stable).
    pairs.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    pairs
}

fn type_location(definition: TypeDefinition) -> AnalyzerLocation {
    AnalyzerLocation {
        file_path: definition.file_path,
        start_line: definition.start_line,
        end_line: definition.end_line,
        symbol_name: definition.name,
        kind: match definition.kind {
            TypeKind::Interface => "interface",
            TypeKind::TypeAlias => "type",
            TypeKind::TypeLiteral => "typeLiteral",
        },
    }
}

fn type_pairs(
    analyzed: &[AnalyzedFile],
    input: &AnalyzeInput,
    scope: PairScope,
) -> Vec<SimilarityPair> {
    let declared = analyzed.iter().flat_map(|entry| entry.declarations.types.iter().cloned());
    // Anonymous object types take part as shapes of their own kind.
    let literals =
        analyzed.iter().flat_map(|entry| &entry.declarations.type_literals).map(|literal| {
            TypeDefinition {
                name: literal.name.clone(),
                kind: TypeKind::TypeLiteral,
                properties: literal.properties.clone(),
                generics: Vec::new(),
                extends: Vec::new(),
                start_line: literal.start_line,
                end_line: literal.end_line,
                file_path: literal.file_path.clone(),
                has_ignore_directive: literal.has_ignore_directive,
            }
        });
    let mut types: Vec<TypeDefinition> = declared
        .chain(literals)
        .filter(|definition| match input.types_only {
            TypeFilter::All => true,
            TypeFilter::Interface => definition.kind == TypeKind::Interface,
            TypeFilter::Type => definition.kind == TypeKind::TypeAlias,
        })
        .collect();

    // The comparison order decides which side of a pair drives the greedy
    // rename-tolerant property matching, so it is pinned: the same input
    // always yields the same report, however the files were listed.
    types.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.start_line.cmp(&b.start_line))
            .then(a.name.cmp(&b.name))
    });

    let mut options = TypeComparisonOptions::default();
    if let Some(allow_cross_kind) = input.allow_cross_kind {
        options.allow_cross_kind_comparison = allow_cross_kind;
    }

    find_similar_types(&types, input.threshold, &options)
        .into_iter()
        .filter(|pair| scope.includes(pair.type1.file_path == pair.type2.file_path))
        .map(|pair| SimilarityPair {
            mode: Mode::Types,
            similarity: pair.result.similarity,
            left: type_location(pair.type1),
            right: type_location(pair.type2),
            details: PairDetails::Shape {
                naming_similarity: pair.result.naming_similarity,
                structural_similarity: pair.result.structural_similarity,
            },
        })
        .collect()
}

fn class_pairs(
    analyzed: &[AnalyzedFile],
    input: &AnalyzeInput,
    scope: PairScope,
) -> Vec<SimilarityPair> {
    let mut classes: Vec<_> =
        analyzed.iter().flat_map(|entry| entry.declarations.classes.iter().cloned()).collect();
    // Pinned like the order of the types: the same input always yields the
    // same report, however the files were listed.
    classes.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.start_line.cmp(&b.start_line))
            .then(a.name.cmp(&b.name))
    });
    find_similar_classes(&classes, input.threshold)
        .into_iter()
        .filter(|pair| scope.includes(pair.class1.file_path == pair.class2.file_path))
        .map(|pair| {
            let location = |class: similarity_core::ClassDefinition| AnalyzerLocation {
                file_path: class.file_path,
                start_line: class.start_line,
                end_line: class.end_line,
                symbol_name: class.name,
                kind: "class",
            };
            SimilarityPair {
                mode: Mode::Classes,
                similarity: pair.result.similarity,
                left: location(pair.class1),
                right: location(pair.class2),
                details: PairDetails::Shape {
                    naming_similarity: pair.result.naming_similarity,
                    structural_similarity: pair.result.structural_similarity,
                },
            }
        })
        .collect()
}

/// The overlap options of `input`, defaults filled in.
fn overlap_options(input: &AnalyzeInput) -> Result<OverlapOptions, String> {
    let defaults = OverlapOptions::default();
    let options = OverlapOptions {
        min_window_size: input.overlap_min_window.unwrap_or(defaults.min_window_size),
        max_window_size: input.overlap_max_window.unwrap_or(defaults.max_window_size),
        threshold: input.threshold,
        size_tolerance: input.overlap_size_tolerance.unwrap_or(defaults.size_tolerance),
    };
    if options.min_window_size == 0 {
        return Err("overlapMinWindow must be at least 1".to_string());
    }
    if options.max_window_size > MAX_OVERLAP_WINDOW {
        return Err(format!("overlapMaxWindow must be at most {MAX_OVERLAP_WINDOW}"));
    }
    if options.min_window_size > options.max_window_size {
        return Err("overlapMinWindow must be less than or equal to overlapMaxWindow".to_string());
    }
    if !(0.0..=1.0).contains(&options.size_tolerance) {
        return Err("overlapSizeTolerance must be between 0 and 1".to_string());
    }
    Ok(options)
}

fn overlap_pairs(
    analyzed: &[AnalyzedFile],
    options: &OverlapOptions,
    scope: PairScope,
) -> Vec<SimilarityPair> {
    find_overlaps(&function_sources(analyzed), options, scope)
        .into_iter()
        .map(|found| {
            let location =
                |file_path: String, function: String, lines: (u32, u32)| AnalyzerLocation {
                    file_path,
                    start_line: lines.0 as usize,
                    end_line: lines.1 as usize,
                    symbol_name: function,
                    kind: "token-window",
                };
            let overlap = found.overlap;
            SimilarityPair {
                mode: Mode::Overlap,
                similarity: overlap.similarity,
                left: location(found.source_file, overlap.source_function, overlap.source_lines),
                right: location(found.target_file, overlap.target_function, overlap.target_lines),
                details: PairDetails::Fragment {
                    node_count: overlap.node_count,
                    node_type: overlap.node_type,
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUPLICATED: &str = r"
export function collectActive(rows: Row[]): string[] {
    const names = [];
    for (const row of rows) {
        if (row.id > 0 && row.name.length > 0) {
            names.push(row.name.trim().toLowerCase());
        }
    }
    return names;
}

export interface Account {
    id: string;
    owner: string;
    balance: number;
}

export class Ledger {
    private entries: Entry[] = [];
    add(entry: Entry): void { this.entries.push(entry); }
    total(): number { return this.entries.reduce((sum, entry) => sum + entry.amount, 0); }
    clear(): void { this.entries = []; }
}
";

    fn input(files: &[(&str, &str)], options: serde_json::Value) -> AnalyzeInput {
        let mut value = serde_json::json!({
            "files": files
                .iter()
                .map(|(path, content)| serde_json::json!({ "filePath": path, "content": content }))
                .collect::<Vec<_>>(),
            "modes": ["functions", "types", "classes", "overlap"],
            "threshold": 0.8,
            "minLines": 3,
        });
        for (key, option) in options.as_object().expect("options are an object") {
            value[key] = option.clone();
        }
        serde_json::from_value(value).expect("a valid input")
    }

    fn twin_files() -> [(&'static str, &'static str); 2] {
        [("src/b.ts", DUPLICATED), ("src/a.ts", DUPLICATED)]
    }

    #[test]
    fn every_mode_reports_its_pairs() {
        let output = analyze_project(&input(&twin_files(), serde_json::json!({}))).unwrap();
        assert_eq!(output.analyzed_files, ["src/b.ts", "src/a.ts"]);
        assert!(output.skipped_files.is_empty() && output.warnings.is_empty());
        assert_eq!(output.by_mode.functions.len(), 1, "{:?}", output.by_mode.functions);
        assert_eq!(output.by_mode.types.len(), 1);
        assert_eq!(output.by_mode.classes.len(), 1);
        assert!(!output.by_mode.overlap.is_empty());
        assert_eq!(output.stats.file_count, 2);
        assert_eq!(output.stats.pair_count, output.results.len());
        assert_eq!(
            output.results.len(),
            3 + output.by_mode.overlap.len(),
            "the combined list holds every mode's pairs"
        );
        // Most similar first.
        assert!(output.results.windows(2).all(|pair| pair[0].similarity >= pair[1].similarity));

        let function = &output.by_mode.functions[0];
        assert_eq!(function.mode, Mode::Functions);
        assert_eq!(
            (function.left.file_path.as_str(), function.right.file_path.as_str()),
            ("src/a.ts", "src/b.ts")
        );
        assert_eq!((function.left.start_line, function.left.end_line), (2, 10));
        assert_eq!(function.left.symbol_name, "collectActive");
        assert_eq!(function.left.kind, "function");
        assert_eq!(output.by_mode.types[0].left.kind, "interface");
        assert_eq!(output.by_mode.classes[0].left.kind, "class");
        assert_eq!(output.by_mode.overlap[0].left.kind, "token-window");
    }

    #[test]
    fn the_report_is_camel_cased_json() {
        let json = serde_json::to_string(&input_json()).unwrap();
        let report: serde_json::Value =
            serde_json::from_str(&analyze_project_json(&json).unwrap()).unwrap();
        let keys = |value: &serde_json::Value| -> Vec<String> {
            value.as_object().unwrap().keys().cloned().collect()
        };
        assert_eq!(
            keys(&report),
            ["analyzedFiles", "byMode", "results", "skippedFiles", "stats", "warnings"]
        );
        assert_eq!(keys(&report["stats"]), ["fileCount", "pairCount"]);
        assert_eq!(keys(&report["byMode"]), ["classes", "functions", "overlap", "types"]);
        let pair = &report["byMode"]["functions"][0];
        assert_eq!(keys(pair), ["details", "left", "mode", "right", "similarity"]);
        assert_eq!(pair["mode"], "functions");
        assert_eq!(keys(&pair["left"]), ["endLine", "filePath", "kind", "startLine", "symbolName"]);
        // What each mode says about a pair besides its similarity.
        assert_eq!(keys(&pair["details"]), ["impact"]);
        assert_eq!(
            keys(&report["byMode"]["types"][0]["details"]),
            ["namingSimilarity", "structuralSimilarity"]
        );
        assert_eq!(
            keys(&report["byMode"]["classes"][0]["details"]),
            ["namingSimilarity", "structuralSimilarity"]
        );
        assert_eq!(keys(&report["byMode"]["overlap"][0]["details"]), ["nodeCount", "nodeType"]);
    }

    fn input_json() -> serde_json::Value {
        serde_json::json!({
            "files": twin_files()
                .iter()
                .map(|(path, content)| serde_json::json!({ "filePath": path, "content": content }))
                .collect::<Vec<_>>(),
            "modes": ["functions", "types", "classes", "overlap"],
            "threshold": 0.8,
            "minLines": 3,
        })
    }

    #[test]
    fn only_the_requested_modes_run() {
        let output =
            analyze_project(&input(&twin_files(), serde_json::json!({ "modes": ["types"] })))
                .unwrap();
        assert_eq!(output.by_mode.types.len(), 1);
        assert!(output.by_mode.functions.is_empty());
        assert!(output.by_mode.classes.is_empty());
        assert!(output.by_mode.overlap.is_empty());
        assert_eq!(output.results.len(), 1);
    }

    #[test]
    fn the_scope_filters_every_mode() {
        let same_file =
            analyze_project(&input(&twin_files(), serde_json::json!({ "sameFileOnly": true })))
                .unwrap();
        assert!(same_file.results.is_empty(), "{:?}", same_file.results);

        let cross_file =
            analyze_project(&input(&twin_files(), serde_json::json!({ "crossFileOnly": true })))
                .unwrap();
        assert!(cross_file.results.len() >= 4);
        assert!(cross_file.results.iter().all(|pair| pair.left.file_path != pair.right.file_path));
    }

    #[test]
    fn files_that_cannot_be_analyzed_are_skipped_in_every_mode() {
        let deep = format!("export const value = {}1{};\n", "[".repeat(400), "]".repeat(400));
        let files = [
            ("src/a.ts", DUPLICATED),
            ("src/broken.ts", "export class {{{ nope"),
            ("src/deep.ts", deep.as_str()),
            ("src/b.ts", DUPLICATED),
        ];
        for modes in [
            serde_json::json!(["functions"]),
            serde_json::json!(["types"]),
            serde_json::json!(["classes"]),
            serde_json::json!(["overlap"]),
        ] {
            let output =
                analyze_project(&input(&files, serde_json::json!({ "modes": modes }))).unwrap();
            assert_eq!(output.analyzed_files, ["src/a.ts", "src/b.ts"], "{modes}");
            assert_eq!(output.skipped_files, ["src/broken.ts", "src/deep.ts"], "{modes}");
            assert_eq!(output.stats.file_count, 2);
            let warnings: Vec<(Option<&str>, &str)> = output
                .warnings
                .iter()
                .map(|warning| (warning.file_path.as_deref(), warning.message.as_str()))
                .collect();
            assert_eq!(warnings.len(), 2);
            assert_eq!(warnings[0].0, Some("src/broken.ts"));
            assert!(warnings[0].1.starts_with("skipped: Parse errors: "), "{}", warnings[0].1);
            assert_eq!(
                warnings[1],
                (Some("src/deep.ts"), "skipped: brackets are nested more than 256 levels deep")
            );
            assert!(!output.results.is_empty(), "{modes}");
        }
    }

    #[test]
    fn type_filters_and_literals_are_honoured() {
        let source = r"
export interface Left { id: string; name: string; total: number }
export type Right = { id: string; name: string; total: number };
export function take(options: { id: string; name: string; total: number }) { return options; }
";
        let files = [("src/a.ts", source)];
        let pairs = |options: serde_json::Value| -> Vec<(String, String)> {
            let mut merged = serde_json::json!({ "modes": ["types"] });
            for (key, option) in options.as_object().unwrap() {
                merged[key] = option.clone();
            }
            analyze_project(&input(&files, merged))
                .unwrap()
                .results
                .into_iter()
                .map(|pair| (pair.left.kind.to_string(), pair.right.kind.to_string()))
                .collect()
        };
        let kinds = |left: &str, right: &str| (left.to_string(), right.to_string());
        assert_eq!(pairs(serde_json::json!({})), [kinds("interface", "type")]);
        assert!(pairs(serde_json::json!({ "allowCrossKind": false })).is_empty());
        assert!(pairs(serde_json::json!({ "typesOnly": "interface" })).is_empty());
        assert_eq!(pairs(serde_json::json!({ "includeTypeLiterals": true })).len(), 3);
    }

    #[test]
    fn options_out_of_range_are_errors() {
        let error = |options: serde_json::Value| {
            analyze_project(&input(&twin_files(), options)).unwrap_err()
        };
        assert_eq!(
            error(serde_json::json!({ "sameFileOnly": true, "crossFileOnly": true })),
            "sameFileOnly and crossFileOnly cannot both be set"
        );
        assert_eq!(
            error(serde_json::json!({ "threshold": 1.5 })),
            "threshold must be between 0 and 1"
        );
        assert_eq!(
            error(serde_json::json!({ "threshold": -0.1 })),
            "threshold must be between 0 and 1"
        );
        assert_eq!(
            error(serde_json::json!({ "overlapMinWindow": 0 })),
            "overlapMinWindow must be at least 1"
        );
        assert_eq!(
            error(serde_json::json!({ "overlapMinWindow": 40 })),
            "overlapMinWindow must be less than or equal to overlapMaxWindow"
        );
        assert_eq!(
            error(serde_json::json!({ "overlapMaxWindow": 1_000_000 })),
            "overlapMaxWindow must be at most 1000"
        );
        assert_eq!(
            error(serde_json::json!({ "overlapSizeTolerance": 2.0 })),
            "overlapSizeTolerance must be between 0 and 1"
        );
        // The limits themselves are fine.
        for options in [
            serde_json::json!({ "threshold": 0.0 }),
            serde_json::json!({ "threshold": 1.0 }),
            serde_json::json!({ "overlapMinWindow": 1, "overlapMaxWindow": 1 }),
            serde_json::json!({ "overlapSizeTolerance": 0.0 }),
            serde_json::json!({ "overlapSizeTolerance": 1.0 }),
        ] {
            assert!(analyze_project(&input(&twin_files(), options.clone())).is_ok(), "{options}");
        }
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert!(analyze_project_json("{").unwrap_err().starts_with("invalid input: "));
        let unknown_mode =
            serde_json::json!({ "files": [], "modes": ["everything"], "threshold": 0.8 });
        assert!(analyze_project_json(&unknown_mode.to_string())
            .unwrap_err()
            .starts_with("invalid input: "));
        let no_threshold = serde_json::json!({ "files": [], "modes": ["types"] });
        assert!(analyze_project_json(&no_threshold.to_string())
            .unwrap_err()
            .starts_with("invalid input: "));
    }

    #[test]
    fn sources_are_checked_one_at_a_time() {
        assert_eq!(check_source("a.ts", "export const answer = 42;"), Ok(()));
        assert!(check_source("a.ts", "export class {{{")
            .unwrap_err()
            .starts_with("Parse errors: "));
        let chain = vec!["a"; 600].join(" + ");
        assert_eq!(
            check_source("a.ts", &format!("export const total = {chain};")),
            Err("code is nested more than 512 levels deep".to_string())
        );
    }

    #[test]
    fn the_report_does_not_depend_on_the_order_of_the_files() {
        let other =
            DUPLICATED.replace("collectActive", "gatherActive").replace("Ledger", "Journal");
        let files =
            [("src/a.ts", DUPLICATED), ("src/b.ts", other.as_str()), ("lib/c.ts", DUPLICATED)];
        let listed = |order: [usize; 3]| -> Vec<String> {
            let ordered: Vec<(&str, &str)> = order.iter().map(|&index| files[index]).collect();
            let output = analyze_project(&input(&ordered, serde_json::json!({}))).unwrap();
            output
                .results
                .iter()
                .map(|pair| {
                    format!(
                        "{:?} {:.6} {}:{}-{} {} <> {}:{}-{} {}",
                        pair.mode,
                        pair.similarity,
                        pair.left.file_path,
                        pair.left.start_line,
                        pair.left.end_line,
                        pair.left.symbol_name,
                        pair.right.file_path,
                        pair.right.start_line,
                        pair.right.end_line,
                        pair.right.symbol_name
                    )
                })
                .collect()
        };
        let reference = listed([0, 1, 2]);
        assert!(reference.len() > 6);
        for order in [[0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            assert_eq!(listed(order), reference, "{order:?}");
        }
    }
}
