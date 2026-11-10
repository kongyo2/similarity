//! Type comparison: property-by-property similarity of two type shapes,
//! and the pairwise scan.

use crate::type_extractor::TypeDefinition;
use crate::type_normalizer::{
    calculate_property_similarity, find_property_matches, is_discriminant_mismatch,
    is_nominal_atom, normalize_type, NormalizationOptions, NormalizedType, PropertyMatch,
};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct TypeComparisonResult {
    pub similarity: f64,
    pub structural_similarity: f64,
    pub naming_similarity: f64,
    pub differences: TypeDifferences,
    pub matched_properties: Vec<MatchedProperty>,
}

#[derive(Debug, Clone, Default)]
pub struct TypeDifferences {
    pub missing_properties: Vec<String>,
    pub extra_properties: Vec<String>,
    pub type_mismatches: Vec<TypeMismatch>,
    pub optionality_differences: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TypeMismatch {
    pub property: String,
    pub type1: String,
    pub type2: String,
}

#[derive(Debug, Clone)]
pub struct MatchedProperty {
    pub prop1: String,
    pub prop2: String,
    pub similarity: f64,
}

#[derive(Debug, Clone)]
pub struct TypeComparisonOptions {
    /// Weight of the structural (member-by-member) similarity.
    pub structural_weight: f64,
    /// Weight of the naming similarity.
    pub naming_weight: f64,
    /// Whether an `interface` may pair with a `type` alias.
    pub allow_cross_kind_comparison: bool,
    pub normalization_options: NormalizationOptions,
}

impl Default for TypeComparisonOptions {
    fn default() -> Self {
        Self {
            // Structural evidence dominates: naming overlap is redundant
            // with the exact-name matching phase and used to prop up
            // coincidental matches (any shared property name guaranteed a
            // 0.8+ naming term). 0.85/0.15 keeps full-rename skeleton
            // matches above the default threshold while same-name/
            // different-type lookalikes fall well below it.
            structural_weight: 0.85,
            naming_weight: 0.15,
            allow_cross_kind_comparison: true,
            normalization_options: NormalizationOptions::default(),
        }
    }
}

/// Compare two type definitions and calculate their similarity.
#[must_use]
pub fn compare_types(
    type1: &TypeDefinition,
    type2: &TypeDefinition,
    options: &TypeComparisonOptions,
) -> TypeComparisonResult {
    compare_normalized_types(
        &normalize_type(type1, &options.normalization_options),
        &normalize_type(type2, &options.normalization_options),
        options,
    )
}

/// [`compare_types`] for types that are already normalized — a scan
/// normalizes each type once, not once per pair it takes part in.
#[must_use]
pub fn compare_normalized_types(
    normalized1: &NormalizedType,
    normalized2: &NormalizedType,
    options: &TypeComparisonOptions,
) -> TypeComparisonResult {
    if !options.allow_cross_kind_comparison && normalized1.kind != normalized2.kind {
        return TypeComparisonResult {
            similarity: 0.0,
            structural_similarity: 0.0,
            naming_similarity: 0.0,
            differences: TypeDifferences::default(),
            matched_properties: Vec::new(),
        };
    }

    let property_matches = find_property_matches(normalized1, normalized2);

    // Calculate structural similarity. Heritage is part of the contract: an
    // interface that `extends` a base carries every member of that base, so
    // two declarations with identical own members but different (or
    // one-sided) `extends` clauses are different shapes — the compiler's
    // `NonNullExpression extends LeftHandSideExpressionBase` vs
    // `VoidExpression extends UnaryExpressionBase` twins used to score as
    // near-duplicates on their shared `expression` member alone.
    let heritage_factor = if normalized1.extends == normalized2.extends { 1.0 } else { 0.7 };
    let mut structural_similarity =
        calculate_structural_similarity(normalized1, normalized2, &property_matches)
            * heritage_factor;

    // An alias whose body is a single atom — a primitive, a literal, a
    // bare reference (`type GLsizei = number`, `type SchemaVersion = "1"`,
    // `type Handler = EventHandler`) — has no structure of its own to
    // duplicate: it exists to give the atom a name. Two such aliases with
    // the same body are the same declaration only when they also name it
    // the same way (the same alias copied into two files, `UserId` vs
    // `UserID`); otherwise they are distinct nominal labels, and equal
    // bodies used to report every pair of them — the DOM library's
    // `GLint`/`GLsizei`/`GPUSize32` numbers and a contract file's version
    // singletons by the hundreds. The structural evidence is the name
    // agreement, cubed so that only near-identical names (similarity above
    // ~0.91: the same name, a case variant) clear the default threshold
    // while one-letter label variants (`GLint64` vs `GLuint64`) do not.
    if is_atomic_alias(normalized1) && is_atomic_alias(normalized2) {
        let name_similarity =
            calculate_property_similarity(&normalized1.original_name, &normalized2.original_name);
        structural_similarity *= name_similarity.powi(3);
    }

    let naming_similarity =
        calculate_naming_similarity(normalized1, normalized2, &property_matches);

    let similarity = (structural_similarity * options.structural_weight)
        + (naming_similarity * options.naming_weight);

    let differences = identify_differences(normalized1, normalized2, &property_matches);

    let matched_properties = property_matches
        .iter()
        .map(|m| MatchedProperty {
            prop1: m.prop1.clone(),
            prop2: m.prop2.clone(),
            similarity: m.overall_similarity,
        })
        .collect();

    TypeComparisonResult {
        similarity,
        structural_similarity,
        naming_similarity,
        differences,
        matched_properties,
    }
}

/// A type alias whose whole body is one nominal atom (see the call site).
fn is_atomic_alias(normalized: &NormalizedType) -> bool {
    normalized.properties.len() == 1
        && normalized.properties.get("<type-body>").is_some_and(|body| is_nominal_atom(body))
}

/// Calculate structural similarity between two normalized types.
#[allow(clippy::cast_precision_loss)]
fn calculate_structural_similarity(
    type1: &NormalizedType,
    type2: &NormalizedType,
    matches: &[PropertyMatch],
) -> f64 {
    let total_props1 = type1.properties.len();
    let total_props2 = type2.properties.len();

    if total_props1 == 0 && total_props2 == 0 {
        // Two property-less shapes carry no structural evidence at all —
        // a flat 1.0 here would make every pair of empty marker types
        // score ≥ structural_weight and flood the report. Require naming
        // agreement to stand in for the missing structure: identically
        // named empties (the interface ⇔ alias cross-kind case) still
        // reach ~1.0, related names (EmptyA/EmptyB) stay above the
        // default threshold, unrelated names fall well below it.
        let name_similarity =
            calculate_property_similarity(&type1.original_name, &type2.original_name);
        return 0.6 + 0.4 * name_similarity;
    }

    if total_props1 == 0 || total_props2 == 0 {
        return 0.0; // One empty, one not
    }

    // Use the best matches (avoid double counting)
    let mut used_props1: HashSet<&str> = HashSet::new();
    let mut used_props2: HashSet<&str> = HashSet::new();
    let mut matched_count = 0;
    let mut total_match_score = 0.0;

    for property_match in matches {
        if !used_props1.contains(property_match.prop1.as_str())
            && !used_props2.contains(property_match.prop2.as_str())
        {
            used_props1.insert(&property_match.prop1);
            used_props2.insert(&property_match.prop2);
            matched_count += 1;
            total_match_score += property_match.overall_similarity;
        }
    }

    if matched_count == 0 {
        return 0.0;
    }

    // Calculate similarity based on matched properties
    let average_match_quality = total_match_score / matched_count as f64;
    let coverage_ratio = (matched_count * 2) as f64 / (total_props1 + total_props2) as f64;
    let mut structural = average_match_quality * coverage_ratio;

    // Discriminant mismatch: a same-named member typed with a literal (or
    // with members of the same enum) on both sides but with DIFFERENT
    // values is the discriminated-union pattern — `kind:
    // SyntaxKind.ForInStatement` vs `kind: SyntaxKind.ForOfStatement`.
    // Such variants are disjoint types by construction however many other
    // members they share (the compiler's AST interfaces share five), so
    // the structural evidence is halved rather than diluted one member's
    // worth.
    let discriminant_mismatch = matches.iter().any(|property_match| {
        property_match.prop1 == property_match.prop2
            && is_discriminant_mismatch(
                &type1.properties[&property_match.prop1],
                &type2.properties[&property_match.prop2],
            )
    });
    if discriminant_mismatch {
        structural *= 0.5;
    }

    structural
}

/// Calculate naming similarity between two normalized types.
#[allow(clippy::cast_precision_loss)]
fn calculate_naming_similarity(
    type1: &NormalizedType,
    type2: &NormalizedType,
    matches: &[PropertyMatch],
) -> f64 {
    // Type-name similarity is always meaningful (and the only signal we have
    // for property-less types like `type StatusA = "a" | "b"`).
    let type_name_similarity =
        calculate_property_similarity(&type1.original_name, &type2.original_name);

    if matches.is_empty() {
        // Without property matches the only remaining naming signal is the
        // type alias / interface name itself.
        return type_name_similarity;
    }

    let average_naming_similarity =
        matches.iter().map(|m| m.name_similarity).sum::<f64>() / matches.len() as f64;

    // Weight property naming more heavily than type naming
    (average_naming_similarity * 0.8) + (type_name_similarity * 0.2)
}

/// Identify differences between two normalized types.
fn identify_differences(
    type1: &NormalizedType,
    type2: &NormalizedType,
    matches: &[PropertyMatch],
) -> TypeDifferences {
    let matched_props1: HashSet<&str> = matches.iter().map(|m| m.prop1.as_str()).collect();
    let matched_props2: HashSet<&str> = matches.iter().map(|m| m.prop2.as_str()).collect();

    // Sorted: the properties live in hash maps, whose order changes from
    // run to run.
    let unmatched = |normalized: &NormalizedType, matched: &HashSet<&str>| -> Vec<String> {
        let mut names: Vec<String> = normalized
            .properties
            .keys()
            .filter(|name| !matched.contains(name.as_str()))
            .cloned()
            .collect();
        names.sort();
        names
    };
    let missing_properties = unmatched(type1, &matched_props1);
    let extra_properties = unmatched(type2, &matched_props2);

    let mut type_mismatches = Vec::new();
    let mut optionality_differences = Vec::new();

    for property_match in matches {
        let type1_type = &type1.properties[&property_match.prop1];
        let type2_type = &type2.properties[&property_match.prop2];
        if type1_type != type2_type {
            type_mismatches.push(TypeMismatch {
                property: format!("{} -> {}", property_match.prop1, property_match.prop2),
                type1: type1_type.clone(),
                type2: type2_type.clone(),
            });
        }

        let is_optional1 = type1.optional_properties.contains(&property_match.prop1);
        let is_optional2 = type2.optional_properties.contains(&property_match.prop2);
        if is_optional1 != is_optional2 {
            optionality_differences
                .push(format!("{} -> {}", property_match.prop1, property_match.prop2));
        }
    }

    TypeDifferences {
        missing_properties,
        extra_properties,
        type_mismatches,
        optionality_differences,
    }
}

#[derive(Debug, Clone)]
pub struct SimilarTypePair {
    pub type1: TypeDefinition,
    pub type2: TypeDefinition,
    pub result: TypeComparisonResult,
}

/// Compare every pair of types and return those at or above `threshold`,
/// most similar first. Types under a `// similarity-ignore` directive take
/// no part, and two declarations of one name in one file (declaration
/// merging) are not a pair.
#[must_use]
pub fn find_similar_types(
    types: &[TypeDefinition],
    threshold: f64,
    options: &TypeComparisonOptions,
) -> Vec<SimilarTypePair> {
    let candidates: Vec<(&TypeDefinition, NormalizedType)> = types
        .iter()
        .filter(|type_def| !type_def.has_ignore_directive)
        .map(|type_def| (type_def, normalize_type(type_def, &options.normalization_options)))
        .collect();

    let mut similar_pairs = Vec::new();
    for (i, (type1, normalized1)) in candidates.iter().enumerate() {
        for (type2, normalized2) in &candidates[i + 1..] {
            if type1.name == type2.name && type1.file_path == type2.file_path {
                continue;
            }
            let result = compare_normalized_types(normalized1, normalized2, options);
            if result.similarity >= threshold {
                similar_pairs.push(SimilarTypePair {
                    type1: (*type1).clone(),
                    type2: (*type2).clone(),
                    result,
                });
            }
        }
    }

    similar_pairs.sort_by(|a, b| b.result.similarity.total_cmp(&a.result.similarity));
    similar_pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_extractor::{PropertyDefinition, TypeDefinition, TypeKind};

    fn create_test_type(name: &str, properties: Vec<(&str, &str, bool, bool)>) -> TypeDefinition {
        TypeDefinition {
            name: name.to_string(),
            kind: TypeKind::Interface,
            properties: properties
                .into_iter()
                .map(|(name, type_annotation, optional, readonly)| PropertyDefinition {
                    name: name.to_string(),
                    type_annotation: type_annotation.to_string(),
                    optional,
                    readonly,
                })
                .collect(),
            generics: Vec::new(),
            extends: Vec::new(),
            start_line: 1,
            end_line: 10,
            file_path: "test.ts".to_string(),
            has_ignore_directive: false,
        }
    }

    #[test]
    fn test_compare_identical_types() {
        let type1 = create_test_type(
            "User",
            vec![("id", "string", false, false), ("name", "string", false, false)],
        );
        let type2 = create_test_type(
            "Person",
            vec![("id", "string", false, false), ("name", "string", false, false)],
        );

        let options = TypeComparisonOptions::default();
        let result = compare_types(&type1, &type2, &options);

        assert!(result.similarity > 0.9);
        assert_eq!(result.matched_properties.len(), 2);
    }

    #[test]
    fn test_compare_similar_types() {
        let type1 = create_test_type(
            "User",
            vec![("id", "string", false, false), ("name", "string", false, false)],
        );
        let type2 = create_test_type(
            "Person",
            vec![("id", "string", false, false), ("fullName", "string", false, false)],
        );

        let options = TypeComparisonOptions::default();
        let result = compare_types(&type1, &type2, &options);

        // `name` and `fullName` share the exact same type, so the
        // rename-tolerant phase pairs them — at a small discount, keeping
        // the renamed pair strictly below a byte-identical pair.
        assert!(result.similarity > 0.5);
        assert_eq!(result.matched_properties.len(), 2);

        let identical = create_test_type(
            "Account",
            vec![("id", "string", false, false), ("name", "string", false, false)],
        );
        let identical_result = compare_types(&type1, &identical, &options);
        assert!(identical_result.similarity > result.similarity);
    }

    #[test]
    fn test_compare_different_types() {
        let type1 = create_test_type(
            "User",
            vec![("id", "string", false, false), ("name", "string", false, false)],
        );
        let type2 = create_test_type(
            "Product",
            vec![("sku", "string", false, false), ("price", "number", false, false)],
        );

        let options = TypeComparisonOptions::default();
        let result = compare_types(&type1, &type2, &options);

        assert!(result.similarity < 0.5);
    }

    #[test]
    fn test_find_similar_types() {
        let types = vec![
            create_test_type(
                "User",
                vec![("id", "string", false, false), ("name", "string", false, false)],
            ),
            create_test_type(
                "Person",
                vec![("id", "string", false, false), ("name", "string", false, false)],
            ),
            create_test_type(
                "Product",
                vec![("sku", "string", false, false), ("price", "number", false, false)],
            ),
        ];

        let options = TypeComparisonOptions::default();
        let similar_pairs = find_similar_types(&types, 0.7, &options);

        assert_eq!(similar_pairs.len(), 1);
        assert_eq!(similar_pairs[0].type1.name, "User");
        assert_eq!(similar_pairs[0].type2.name, "Person");
    }

    #[test]
    fn test_find_similar_types_skips_similarity_ignore_directives() {
        let active = create_test_type(
            "User",
            vec![("id", "string", false, false), ("email", "string", false, false)],
        );
        let mut ignored = create_test_type(
            "IgnoredUser",
            vec![("id", "string", false, false), ("email", "string", false, false)],
        );
        ignored.has_ignore_directive = true;

        let options = TypeComparisonOptions::default();
        let similar_pairs = find_similar_types(&[active, ignored], 0.7, &options);

        assert!(similar_pairs.is_empty());
    }

    #[test]
    fn unrelated_empty_types_stay_below_threshold() {
        let type1 = create_test_type("FeatureGate", vec![]);
        let type2 = create_test_type("RetryBudget", vec![]);
        let options = TypeComparisonOptions::default();
        let result = compare_types(&type1, &type2, &options);
        assert!(
            result.similarity < 0.8,
            "unrelated empty markers must not auto-match, got {}",
            result.similarity
        );

        // Same-named empties (the interface ⇔ alias cross-kind case)
        // still clear a high threshold.
        let type3 = create_test_type("Empty", vec![]);
        let type4 = create_test_type("Empty", vec![]);
        let same = compare_types(&type3, &type4, &options);
        assert!(
            same.similarity >= 0.9,
            "identical empty names must match, got {}",
            same.similarity
        );
    }

    #[test]
    fn heritage_is_part_of_the_contract() {
        // XT-N14 shape: identical own members, one side extends a base.
        let options = TypeComparisonOptions::default();
        fn members(names: [&str; 3]) -> Vec<(&str, &str, bool, bool)> {
            names.iter().map(|name| (*name, "string", false, false)).collect()
        }
        let mut with_base =
            create_test_type("AuditEvent", members(["actorId", "action", "targetId"]));
        with_base.extends = vec!["EventBase".to_string()];
        let without_base =
            create_test_type("AuditRecord", members(["actorId", "action", "targetId"]));
        let result = compare_types(&with_base, &without_base, &options);
        assert!(
            result.similarity < 0.8,
            "one-sided extends must stay below threshold, got {}",
            result.similarity
        );

        let mut same_base =
            create_test_type("ChargeEvent", members(["actorId", "action", "targetId"]));
        same_base.extends = vec!["EventBase".to_string()];
        let same = compare_types(&with_base, &same_base, &options);
        assert!(
            same.similarity > 0.95,
            "shared heritage must not be penalized, got {}",
            same.similarity
        );
    }

    #[test]
    fn atomic_aliases_need_name_agreement() {
        let options = TypeComparisonOptions::default();
        let alias = |name: &str, body: &str| TypeDefinition {
            kind: TypeKind::TypeAlias,
            ..create_test_type(name, vec![("<type-body>", body, false, false)])
        };
        // Version singletons and primitive labels: equal bodies, distinct
        // nominal aliases.
        for (left, right, body) in [
            ("SimilarCodeSchemaVersion", "MutationPlanSchemaVersion", "\"1\""),
            ("SimilarCodeSchemaVersion", "SimilarCodeReviewSchemaVersion", "\"1\""),
            ("GLsizei", "GPUSize32", "number"),
            ("GLint64", "GLuint64", "number"),
            ("RequestHandler", "ResponseHandler", "EventHandler"),
        ] {
            let result = compare_types(&alias(left, body), &alias(right, body), &options);
            assert!(
                result.similarity < 0.8,
                "{left} vs {right} must not pair on an atomic body, got {}",
                result.similarity
            );
        }
        // The same alias declared twice (copied into two files), or a
        // case variant of its name, is still one declaration.
        let copied =
            compare_types(&alias("UserId", "string"), &alias("UserId", "string"), &options);
        assert!(copied.similarity > 0.99, "got {}", copied.similarity);
        let variant =
            compare_types(&alias("UserId", "string"), &alias("UserID", "string"), &options);
        assert!(variant.similarity >= 0.8, "got {}", variant.similarity);
        // Structured bodies are unaffected: literal unions and generic
        // instantiations still pair under unrelated names (T-P05, T-P09).
        let union = "\"debug\" | \"error\" | \"info\" | \"warn\"";
        let levels = compare_types(&alias("LogLevel", union), &alias("Severity", union), &options);
        assert!(levels.similarity >= 0.8, "got {}", levels.similarity);
        let record = "Record<string, number>";
        let totals =
            compare_types(&alias("CountsByDay", record), &alias("DailyTotals", record), &options);
        assert!(totals.similarity >= 0.8, "got {}", totals.similarity);
    }

    #[test]
    fn single_quoted_atomic_aliases_need_name_agreement() {
        // An alias body is its source text; `'1'` must still read as the
        // literal atom `"1"` and take the name check.
        let options = TypeComparisonOptions::default();
        let code = r#"
export type SchemaVersion = '1';
export type CacheFormat = '1';
export type ConfigVersion = "1";
export type UserId = 'user';
export type UserID = "user";
"#;
        let types = crate::type_extractor::extract_types_from_code(code, "test.ts").unwrap();
        let find = |name: &str| types.iter().find(|t| t.name == name).unwrap();
        for (left, right) in [("SchemaVersion", "CacheFormat"), ("CacheFormat", "ConfigVersion")] {
            let result = compare_types(find(left), find(right), &options);
            assert!(
                result.similarity < 0.8,
                "{left} vs {right} must not pair on an atomic body, got {}",
                result.similarity
            );
        }
        // The same alias in either quote style is still one declaration.
        let variant = compare_types(find("UserId"), find("UserID"), &options);
        assert!(variant.similarity >= 0.8, "got {}", variant.similarity);
    }

    #[test]
    fn discriminant_mismatch_halves_structural_evidence() {
        // The compiler's `ForInStatement` / `ForOfStatement` interfaces:
        // five shared members, one enum-member discriminant apart.
        let options = TypeComparisonOptions::default();
        let variant = |name: &str, kind: &str| {
            create_test_type(
                name,
                vec![
                    ("kind", kind, false, false),
                    ("awaitModifier", "AwaitKeyword", true, false),
                    ("initializer", "ForInitializer", false, false),
                    ("expression", "Expression", false, false),
                    ("statement", "Statement", false, false),
                ],
            )
        };
        let for_in = variant("ForInStatement", "SyntaxKind.ForInStatement");
        let for_of = variant("ForOfStatement", "SyntaxKind.ForOfStatement");
        let result = compare_types(&for_in, &for_of, &options);
        assert!(
            result.similarity < 0.7,
            "distinct discriminants must stay well below threshold, got {}",
            result.similarity
        );
        let twin = variant("ForInLoop", "SyntaxKind.ForInStatement");
        let same = compare_types(&for_in, &twin, &options);
        assert!(
            same.similarity > 0.95,
            "same discriminant is a plain twin, got {}",
            same.similarity
        );

        // String-literal discriminants behave the same way.
        let click = create_test_type(
            "ClickEvt",
            vec![
                ("kind", "\"click\"", false, false),
                ("x", "number", false, false),
                ("y", "number", false, false),
            ],
        );
        let tap = create_test_type(
            "TapEvt",
            vec![
                ("kind", "\"tap\"", false, false),
                ("x", "number", false, false),
                ("y", "number", false, false),
            ],
        );
        let literal = compare_types(&click, &tap, &options);
        assert!(literal.similarity < 0.7, "got {}", literal.similarity);
    }

    fn extracted(code: &str) -> Vec<TypeDefinition> {
        crate::type_extractor::extract_types_from_code(code, "test.ts").unwrap()
    }

    fn similarity_of(code: &str) -> f64 {
        let types = extracted(code);
        assert_eq!(types.len(), 2, "{types:?}");
        compare_types(&types[0], &types[1], &TypeComparisonOptions::default()).similarity
    }

    #[test]
    fn signature_only_interfaces_compare_by_their_signatures() {
        // Call and construct signatures used to be dropped: all of these
        // were empty shapes that compared on their names alone.
        let same = similarity_of(
            "interface Render { (content: string, head: Head): Response }
             interface Renderer { (body: string, head: Head): Response }",
        );
        assert!(same > 0.95, "{same}");

        // An empty interface and one with a call signature under the same
        // name used to be a 1.0 pair.
        let empty_and_callable = similarity_of(
            "interface ContextRenderer {}
             namespace Augmented { export interface ContextRenderer { (content: string): Response } }",
        );
        assert!(empty_and_callable < 0.5, "{empty_and_callable}");

        // Calling is not constructing.
        let call_and_construct = similarity_of(
            "interface ErrorMap { (issue: Issue): Message }
             interface ErrorClass { new (issue: Issue): Message }",
        );
        assert!(call_and_construct < 0.5, "{call_and_construct}");

        let different_contracts = similarity_of(
            "interface Get { (key: string): Value }
             interface Set { (key: string, value: Value): void }",
        );
        assert!(different_contracts < 0.8, "{different_contracts}");
    }

    #[test]
    fn overloads_are_compared_one_by_one() {
        // Only the last of several signatures of one name used to count:
        // these two were identical on their shared `(): string`.
        let one_shared_overload = similarity_of(
            "interface Transition {
                 (strings: TemplateStringsArray, ...values: Variable[]): string
                 (content: string): string
                 (): string
             }
             interface Transition2 {
                 (first: number, second: number): Promise<number>
                 (list: number[]): Promise<number>
                 (): string
             }",
        );
        assert!(one_shared_overload < 0.8, "{one_shared_overload}");

        let same_overloads = similarity_of(
            "interface Emitter {
                 on(event: \"open\", listener: () => void): void;
                 on(event: \"close\", listener: (code: number) => void): void;
             }
             interface Channel {
                 on(event: \"open\", listener: () => void): void;
                 on(event: \"close\", listener: (code: number) => void): void;
             }",
        );
        assert!(same_overloads > 0.95, "{same_overloads}");
    }

    #[test]
    fn numerically_keyed_members_take_part() {
        // Members with numeric keys used to be dropped, so two generated
        // response maps compared as two empty shapes.
        let different_payloads = similarity_of(
            "type UserListResponses = { 200: { result: { data: Array<User> } } };
             type UserCreateResponses = { 200: { result: { data: { name: string; id: string; createdAt: Date } } } };",
        );
        assert!(different_payloads < 0.8, "{different_payloads}");
        let same_payloads = similarity_of(
            "type UserListResponses = { 200: { result: User[] }; 404: { error: string } };
             type OrderListResponses = { 200: { result: User[] }; 404: { error: string } };",
        );
        assert!(same_payloads > 0.9, "{same_payloads}");
    }
}
