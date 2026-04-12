use serde::{Deserialize, Serialize};
use similarity_core::{
    extract_type_literals_from_files, extract_types_from_files, find_overlaps_across_files,
    find_similar_classes_across_files, find_similar_functions_across_files,
    find_similar_functions_in_file, find_similar_types, OverlapOptions, TSEDOptions,
    TypeComparisonOptions, TypeKind,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeInput {
    pub files: Vec<ProjectFile>,
    pub modes: Vec<String>,
    pub threshold: f64,
    pub min_lines: Option<u32>,
    pub size_penalty: Option<bool>,
    pub same_file_only: Option<bool>,
    pub cross_file_only: Option<bool>,
    pub types_only: Option<String>,
    pub allow_cross_kind: Option<bool>,
    pub include_type_literals: Option<bool>,
    pub overlap_min_window: Option<u32>,
    pub overlap_max_window: Option<u32>,
    pub overlap_size_tolerance: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeOutput {
    pub analyzed_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub warnings: Vec<AnalyzeWarning>,
    pub results: Vec<SimilarityPair>,
    pub by_mode: ByMode,
    pub stats: AnalyzeStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeWarning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeStats {
    pub file_count: usize,
    pub pair_count: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByMode {
    pub functions: Vec<SimilarityPair>,
    pub types: Vec<SimilarityPair>,
    pub classes: Vec<SimilarityPair>,
    pub overlap: Vec<SimilarityPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityPair {
    pub mode: String,
    pub similarity: f64,
    pub left: AnalyzerLocation,
    pub right: AnalyzerLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzerLocation {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol_name: String,
    pub kind: String,
}

fn should_compare(same_file: bool, same_file_only: bool, cross_file_only: bool) -> bool {
    (!same_file_only || same_file) && (!cross_file_only || !same_file)
}

pub fn analyze_project(input: AnalyzeInput) -> AnalyzeOutput {
    // Note: elapsed_ms is always set to 0 here because std::time::Instant is
    // unsupported on wasm32-unknown-unknown. The JavaScript caller measures
    // the wall-clock elapsed time and overwrites this field before returning.
    let files: Vec<(String, String)> = input
        .files
        .iter()
        .map(|f| (f.file_path.clone(), f.content.clone()))
        .collect();

    let same_file_only = input.same_file_only.unwrap_or(false);
    let cross_file_only = input.cross_file_only.unwrap_or(false);

    let mut warnings = Vec::<AnalyzeWarning>::new();
    let mut by_mode = ByMode {
        functions: vec![],
        types: vec![],
        classes: vec![],
        overlap: vec![],
    };

    if input.modes.iter().any(|m| m == "functions") {
        let mut options = TSEDOptions::default();
        if let Some(min_lines) = input.min_lines {
            options.min_lines = min_lines;
        }
        if let Some(size_penalty) = input.size_penalty {
            options.size_penalty = size_penalty;
        }

        let mut function_pairs: Vec<SimilarityPair> = Vec::new();

        // Cross-file pairs. `find_similar_functions_across_files` intentionally
        // skips same-file pairs, so we only consult it when cross-file results
        // are still in scope.
        if !same_file_only {
            match find_similar_functions_across_files(&files, input.threshold, &options) {
                Ok(pairs) => {
                    for (left_file, res, right_file) in pairs {
                        if res.similarity < input.threshold {
                            continue;
                        }
                        function_pairs.push(SimilarityPair {
                            mode: "functions".to_string(),
                            similarity: res.similarity,
                            left: AnalyzerLocation {
                                file_path: left_file,
                                start_line: res.func1.start_line as usize,
                                end_line: res.func1.end_line as usize,
                                symbol_name: res.func1.name,
                                kind: format!("{:?}", res.func1.function_type).to_lowercase(),
                            },
                            right: AnalyzerLocation {
                                file_path: right_file,
                                start_line: res.func2.start_line as usize,
                                end_line: res.func2.end_line as usize,
                                symbol_name: res.func2.name,
                                kind: format!("{:?}", res.func2.function_type).to_lowercase(),
                            },
                            details: Some(serde_json::json!({ "impact": res.impact })),
                        });
                    }
                }
                Err(err) => warnings.push(AnalyzeWarning {
                    file_path: None,
                    message: err,
                }),
            }
        }

        // Same-file pairs. The cross-file helper above skips these entirely,
        // so we have to walk each file explicitly whenever same-file results
        // are in scope.
        if !cross_file_only {
            for (filename, source) in &files {
                match find_similar_functions_in_file(filename, source, input.threshold, &options) {
                    Ok(pairs) => {
                        for res in pairs {
                            if res.similarity < input.threshold {
                                continue;
                            }
                            function_pairs.push(SimilarityPair {
                                mode: "functions".to_string(),
                                similarity: res.similarity,
                                left: AnalyzerLocation {
                                    file_path: filename.clone(),
                                    start_line: res.func1.start_line as usize,
                                    end_line: res.func1.end_line as usize,
                                    symbol_name: res.func1.name,
                                    kind: format!("{:?}", res.func1.function_type).to_lowercase(),
                                },
                                right: AnalyzerLocation {
                                    file_path: filename.clone(),
                                    start_line: res.func2.start_line as usize,
                                    end_line: res.func2.end_line as usize,
                                    symbol_name: res.func2.name,
                                    kind: format!("{:?}", res.func2.function_type).to_lowercase(),
                                },
                                details: Some(serde_json::json!({ "impact": res.impact })),
                            });
                        }
                    }
                    Err(err) => warnings.push(AnalyzeWarning {
                        file_path: Some(filename.clone()),
                        message: err,
                    }),
                }
            }
        }

        // Cross-file and per-file scans each produce sorted output on their
        // own, but after merging we have to re-sort so callers observing
        // `byMode.functions` directly still see highest-similarity first.
        function_pairs.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        by_mode.functions = function_pairs;
    }

    if input.modes.iter().any(|m| m == "types") {
        let mut all_types: Vec<_> = extract_types_from_files(&files)
            .into_values()
            .flatten()
            .collect();

        if input.include_type_literals.unwrap_or(false) {
            all_types.extend(
                extract_type_literals_from_files(&files)
                    .into_values()
                    .flatten()
                    .map(|literal| similarity_core::TypeDefinition {
                        name: literal.name,
                        kind: TypeKind::TypeLiteral,
                        properties: literal.properties,
                        generics: vec![],
                        extends: vec![],
                        start_line: literal.start_line,
                        end_line: literal.end_line,
                        file_path: literal.file_path,
                        has_ignore_directive: false,
                    }),
            );
        }

        let mut options = TypeComparisonOptions::default();
        if let Some(allow_cross_kind) = input.allow_cross_kind {
            options.allow_cross_kind_comparison = allow_cross_kind;
        }

        let kinds = input.types_only.unwrap_or_else(|| "all".to_string());
        let filtered: Vec<_> = all_types
            .into_iter()
            .filter(|t| match kinds.as_str() {
                "interface" => t.kind == TypeKind::Interface,
                "type" => t.kind == TypeKind::TypeAlias,
                _ => true,
            })
            .collect();

        by_mode.types = find_similar_types(&filtered, input.threshold, &options)
            .into_iter()
            .filter(|pair| {
                should_compare(
                    pair.type1.file_path == pair.type2.file_path,
                    same_file_only,
                    cross_file_only,
                )
            })
            .map(|pair| SimilarityPair {
                mode: "types".to_string(),
                similarity: pair.result.similarity,
                left: AnalyzerLocation {
                    file_path: pair.type1.file_path,
                    start_line: pair.type1.start_line,
                    end_line: pair.type1.end_line,
                    symbol_name: pair.type1.name,
                    kind: match pair.type1.kind {
                        TypeKind::Interface => "interface".to_string(),
                        TypeKind::TypeAlias => "type".to_string(),
                        TypeKind::TypeLiteral => "typeLiteral".to_string(),
                    },
                },
                right: AnalyzerLocation {
                    file_path: pair.type2.file_path,
                    start_line: pair.type2.start_line,
                    end_line: pair.type2.end_line,
                    symbol_name: pair.type2.name,
                    kind: match pair.type2.kind {
                        TypeKind::Interface => "interface".to_string(),
                        TypeKind::TypeAlias => "type".to_string(),
                        TypeKind::TypeLiteral => "typeLiteral".to_string(),
                    },
                },
                details: Some(serde_json::json!({
                    "structuralSimilarity": pair.result.structural_similarity,
                    "namingSimilarity": pair.result.naming_similarity
                })),
            })
            .collect();

    }

    if input.modes.iter().any(|m| m == "classes") {
        by_mode.classes = find_similar_classes_across_files(&files, input.threshold)
            .into_iter()
            .filter(|pair| {
                should_compare(
                    pair.class1.file_path == pair.class2.file_path,
                    same_file_only,
                    cross_file_only,
                )
            })
            .map(|pair| SimilarityPair {
                mode: "classes".to_string(),
                similarity: pair.result.similarity,
                left: AnalyzerLocation {
                    file_path: pair.class1.file_path,
                    start_line: pair.class1.start_line,
                    end_line: pair.class1.end_line,
                    symbol_name: pair.class1.name,
                    kind: "class".to_string(),
                },
                right: AnalyzerLocation {
                    file_path: pair.class2.file_path,
                    start_line: pair.class2.start_line,
                    end_line: pair.class2.end_line,
                    symbol_name: pair.class2.name,
                    kind: "class".to_string(),
                },
                details: Some(serde_json::json!({
                    "structuralSimilarity": pair.result.structural_similarity,
                    "namingSimilarity": pair.result.naming_similarity
                })),
            })
            .collect();
    }

    if input.modes.iter().any(|m| m == "overlap") {
        let map: HashMap<String, String> = files.iter().cloned().collect();
        let options = OverlapOptions {
            min_window_size: input.overlap_min_window.unwrap_or(10),
            max_window_size: input.overlap_max_window.unwrap_or(100),
            threshold: input.threshold,
            size_tolerance: input.overlap_size_tolerance.unwrap_or(0.2),
        };

        match find_overlaps_across_files(&map, &options) {
            Ok(overlaps) => {
                by_mode.overlap = overlaps
                    .into_iter()
                    .filter(|entry| {
                        should_compare(
                            entry.source_file == entry.target_file,
                            same_file_only,
                            cross_file_only,
                        ) && entry.overlap.similarity >= input.threshold
                    })
                    .map(|entry| SimilarityPair {
                        mode: "overlap".to_string(),
                        similarity: entry.overlap.similarity,
                        left: AnalyzerLocation {
                            file_path: entry.source_file,
                            start_line: entry.overlap.source_lines.0 as usize,
                            end_line: entry.overlap.source_lines.1 as usize,
                            symbol_name: entry.overlap.source_function,
                            kind: "token-window".to_string(),
                        },
                        right: AnalyzerLocation {
                            file_path: entry.target_file,
                            start_line: entry.overlap.target_lines.0 as usize,
                            end_line: entry.overlap.target_lines.1 as usize,
                            symbol_name: entry.overlap.target_function,
                            kind: "token-window".to_string(),
                        },
                        details: Some(serde_json::json!({
                            "nodeCount": entry.overlap.node_count,
                            "nodeType": entry.overlap.node_type,
                        })),
                    })
                    .collect();
            }
            Err(err) => warnings.push(AnalyzeWarning {
                file_path: None,
                message: format!("overlap detection failed: {err}"),
            }),
        }
    }

    let mut results = Vec::new();
    results.extend(by_mode.functions.clone());
    results.extend(by_mode.types.clone());
    results.extend(by_mode.classes.clone());
    results.extend(by_mode.overlap.clone());
    results.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));

    AnalyzeOutput {
        analyzed_files: input.files.into_iter().map(|f| f.file_path).collect(),
        skipped_files: vec![],
        warnings,
        results: results.clone(),
        by_mode,
        stats: AnalyzeStats {
            file_count: files.len(),
            pair_count: results.len(),
            elapsed_ms: 0,
        },
    }
}
