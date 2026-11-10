//! Type comparison: property-by-property similarity of two type shapes,
//! and the pairwise scan.

use crate::type_extractor::TypeDefinition;
use crate::type_normalizer::{
    calculate_property_similarity, edit_distance_length, find_property_matches_with,
    is_discriminant_mismatch, is_nominal_atom, normalize_type, Computed, NormalizationOptions,
    NormalizedType, PropertyMatch, Remembered, Similarities,
};
use std::collections::{HashMap, HashSet};

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
    compare_normalized_types_with(normalized1, normalized2, options, &mut Computed)
}

/// [`compare_normalized_types`] with the similarities of annotations and
/// property names taken from `similarities`.
fn compare_normalized_types_with<'a>(
    normalized1: &'a NormalizedType,
    normalized2: &'a NormalizedType,
    options: &TypeComparisonOptions,
    similarities: &mut dyn Similarities<'a>,
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

    let property_matches = find_property_matches_with(normalized1, normalized2, similarities);

    // Calculate structural similarity. Heritage is part of the contract: an
    // interface that `extends` a base carries every member of that base, so
    // two declarations with identical own members but different (or
    // one-sided) `extends` clauses are different shapes — the compiler's
    // `NonNullExpression extends LeftHandSideExpressionBase` vs
    // `VoidExpression extends UnaryExpressionBase` twins used to score as
    // near-duplicates on their shared `expression` member alone.
    let heritage_factor = heritage_factor(normalized1, normalized2);
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

/// What two shapes with different `extends` clauses keep of their
/// structural similarity.
fn heritage_factor(normalized1: &NormalizedType, normalized2: &NormalizedType) -> f64 {
    if normalized1.extends == normalized2.extends {
        1.0
    } else {
        0.7
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

/// A type of the scan: its normalized shape, and its members numbered for
/// the cheap comparison that comes before the real one.
struct ScannedType<'a> {
    definition: &'a TypeDefinition,
    normalized: NormalizedType,
    /// `(property name, annotation)` of every member, each numbered across
    /// the scan, ordered by name.
    members: Vec<(u32, u32)>,
}

/// Numbers strings: equal strings get the same number.
#[derive(Default)]
struct Interner {
    numbers: HashMap<String, u32>,
}

impl Interner {
    /// The number of `text`, and whether it is new.
    fn number(&mut self, text: &str) -> (u32, bool) {
        if let Some(&number) = self.numbers.get(text) {
            return (number, false);
        }
        let number = u32::try_from(self.numbers.len()).expect("fewer than 2^32 strings");
        self.numbers.insert(text.to_string(), number);
        (number, true)
    }
}

/// The names and annotations of a scan, numbered.
#[derive(Default)]
struct MemberTable {
    names: Interner,
    annotations: Interner,
    /// Per annotation, [`edit_distance_length`].
    edit_distance_lengths: Vec<Option<usize>>,
}

impl MemberTable {
    /// `normalized` with its members numbered.
    fn scanned<'a>(
        &mut self,
        definition: &'a TypeDefinition,
        normalized: NormalizedType,
    ) -> ScannedType<'a> {
        let mut members: Vec<(u32, u32)> = normalized
            .properties
            .iter()
            .map(|(name, annotation)| {
                let (annotation_number, is_new) = self.annotations.number(annotation);
                if is_new {
                    self.edit_distance_lengths.push(edit_distance_length(annotation));
                }
                (self.names.number(name).0, annotation_number)
            })
            .collect();
        members.sort_unstable();
        ScannedType { definition, normalized, members }
    }

    /// An upper bound on the similarity of two annotations: one for equal
    /// ones, and for two that are compared by edit distance, what their
    /// lengths allow.
    fn annotation_similarity_bound(&self, annotation1: u32, annotation2: u32) -> f64 {
        if annotation1 == annotation2 {
            return 1.0;
        }
        match (
            self.edit_distance_lengths[annotation1 as usize],
            self.edit_distance_lengths[annotation2 as usize],
        ) {
            #[allow(clippy::cast_precision_loss)]
            (Some(length1), Some(length2)) if length1.max(length2) > 0 => {
                length1.min(length2) as f64 / length1.max(length2) as f64
            }
            _ => 1.0,
        }
    }
}

/// How far a bound on a similarity may fall short of the threshold before
/// the pair is given up: the bound and the similarity add up the same
/// terms in different orders.
const BOUND_SLACK: f64 = 1e-9;

/// An upper bound on the similarity of two types, from the names and
/// annotations of their members alone.
///
/// The structural term is the summed quality of the matched members over
/// the mean member count. A member matches by name at a quality of at most
/// the similarity of the two annotations (see
/// [`MemberTable::annotation_similarity_bound`]), or — renamed — a member
/// with the very same annotation at no more than `0.95 × rename
/// confidence`; the naming term is at most one. So two types that share
/// few names and few annotations cannot be similar, whatever their
/// annotations turn out to have in common — and finding that out is the
/// expensive part: it normalizes, splits and measures strings for every
/// pair of members.
fn similarity_upper_bound(
    type1: &ScannedType,
    type2: &ScannedType,
    table: &MemberTable,
    options: &TypeComparisonOptions,
    spare: &mut (Vec<u32>, Vec<u32>),
) -> f64 {
    let (count1, count2) = (type1.members.len(), type2.members.len());
    if count1 == 0 && count2 == 0 {
        // Two empty shapes compare by their names alone.
        return f64::INFINITY;
    }
    if count1 == 0 || count2 == 0 {
        // An empty shape and one with members have no structure in common.
        return options.naming_weight;
    }

    // Members with a name both types have, and the annotations of the
    // others.
    let (spare1, spare2) = spare;
    spare1.clear();
    spare2.clear();
    let mut by_name = 0.0;
    let (mut i, mut j) = (0, 0);
    while i < count1 && j < count2 {
        let ((name1, annotation1), (name2, annotation2)) = (type1.members[i], type2.members[j]);
        match name1.cmp(&name2) {
            std::cmp::Ordering::Equal => {
                by_name += table.annotation_similarity_bound(annotation1, annotation2);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                spare1.push(annotation1);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                spare2.push(annotation2);
                j += 1;
            }
        }
    }
    spare1.extend(type1.members[i..].iter().map(|&(_, annotation)| annotation));
    spare2.extend(type2.members[j..].iter().map(|&(_, annotation)| annotation));

    // Renamed members pair up one to one among equal annotations (see
    // `find_property_matches`).
    #[allow(clippy::cast_precision_loss)]
    let rename_confidence = ((count1.min(count2) as f64 - 1.0) / 2.0).clamp(0.0, 1.0);
    let mut renamed = 0usize;
    if rename_confidence > 0.0 {
        spare1.sort_unstable();
        spare2.sort_unstable();
        let (mut i, mut j) = (0, 0);
        while i < spare1.len() && j < spare2.len() {
            match spare1[i].cmp(&spare2[j]) {
                std::cmp::Ordering::Equal => {
                    renamed += 1;
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let structural = 2.0 * (by_name + 0.95 * rename_confidence * renamed as f64)
        / (count1 + count2) as f64
        * heritage_factor(&type1.normalized, &type2.normalized);
    structural * options.structural_weight + options.naming_weight
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
    let mut table = MemberTable::default();
    let candidates: Vec<ScannedType> = types
        .iter()
        .filter(|type_def| !type_def.has_ignore_directive)
        .map(|definition| {
            table.scanned(definition, normalize_type(definition, &options.normalization_options))
        })
        .collect();

    let mut similarities = Remembered::default();
    let mut spare = (Vec::new(), Vec::new());
    let mut similar_pairs = Vec::new();
    for (i, type1) in candidates.iter().enumerate() {
        for type2 in &candidates[i + 1..] {
            if type1.definition.name == type2.definition.name
                && type1.definition.file_path == type2.definition.file_path
            {
                continue;
            }
            if similarity_upper_bound(type1, type2, &table, options, &mut spare)
                < threshold - BOUND_SLACK
            {
                continue;
            }
            let result = compare_normalized_types_with(
                &type1.normalized,
                &type2.normalized,
                options,
                &mut similarities,
            );
            if result.similarity >= threshold {
                similar_pairs.push(SimilarTypePair {
                    type1: type1.definition.clone(),
                    type2: type2.definition.clone(),
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

    /// Types of every kind the comparison treats specially.
    const SHAPES: &str = r#"
export interface User { id: string; name: string; email: string; active: boolean }
export interface Person { id: string; name: string; email: string; active: boolean }
export interface Customer { id: string; name: string; email?: string; active: boolean }
export type Member = { id: string; name: string; email: string; active: boolean };
// Renamed twins, and types that only share annotations.
export interface Address { street: string; city: string; zip: string }
export interface Location { line: string; town: string; postal: string }
export interface Size { width: number; height: number; depth: number }
// Same names, different contracts.
export interface Loader { load(id: string): Promise<User>; size: number }
export interface SyncLoader { load(id: string): User; size: number }
export interface OrderLoader { load(id: string): Promise<Order>; size: number }
// Heritage.
export interface Admin extends User { level: number; scope: string }
export interface Auditor extends Person { level: number; scope: string }
export interface Operator extends User { level: number; scope: string }
// Discriminated variants.
export interface Created { kind: "created"; at: Date; by: string }
export interface Deleted { kind: "deleted"; at: Date; by: string }
// Empty shapes, atoms, unions, index signatures, call signatures.
export interface EmptyA {}
export interface EmptyB {}
export interface Marker {}
export type UserId = string;
export type UserID = string;
export type OrderId = string;
export type Status = "open" | "closed" | "pending";
export type State = "open" | "closed" | "pending";
export type Phase = "open" | "closed";
export interface Dictionary { [key: string]: number }
export interface Lookup { [key: string]: number }
export interface Flags { [key: string]: boolean }
export interface Handler { (request: Request): Response }
export interface Listener { (request: Request): Response }
export interface Factory { new (options: Options): Widget }
// One member, two members.
export interface Versioned { version: string }
export interface Acted { resultingAction: string }
export interface Point { x: number; y: number }
export interface Offset { left: number; top: number }
// Shapes without a rule of their own: compared by edit distance.
export type Pair = [first: string, second: number];
export type Couple = [first: string, second: number];
export type Triple = [first: string, second: number, third: boolean, fourth: Date, fifth: RegExp];
export type Flag<T> = T extends string ? "text" : T extends number ? "count" : never;
export type Mark<T> = T extends string ? "text" : T extends number ? "total" : never;
export type Keys<T> = { [K in keyof T]: K };
export type Values<T> = { [K in keyof T]: T[K] };
export type Getters<T> = { [K in keyof T as `get${Capitalize<string & K>}`]: () => T[K] };
export interface Holder { pair: [first: string, second: number]; flag: keyof Size }
export interface Keeper { pair: [first: string, second: number, third: boolean, fourth: Date]; flag: keyof Size }
export interface Wide {
    a: string; b: string; c: string; d: string; e: string; f: string; g: string; h: string;
    i: number; j: number; k: number; l: number; m: boolean; n: boolean; o: Date; p: Date;
}
export interface Wider {
    a: string; b: string; c: string; d: string; e: string; f: string; g: string; h: string;
    i: number; j: number; k: number; l: number; m: boolean; n: boolean; o: Date; q: Date;
}
"#;

    #[test]
    fn the_upper_bound_never_falls_below_the_similarity() {
        let types = extracted(SHAPES);
        assert!(types.len() > 35, "{}", types.len());
        let option_sets = [
            TypeComparisonOptions::default(),
            TypeComparisonOptions { allow_cross_kind_comparison: false, ..Default::default() },
            TypeComparisonOptions {
                structural_weight: 0.5,
                naming_weight: 0.5,
                ..Default::default()
            },
        ];
        for options in &option_sets {
            let mut table = MemberTable::default();
            let scanned: Vec<ScannedType> = types
                .iter()
                .map(|definition| {
                    table.scanned(
                        definition,
                        normalize_type(definition, &options.normalization_options),
                    )
                })
                .collect();
            let mut spare = (Vec::new(), Vec::new());
            let mut tight = 0;
            for type1 in &scanned {
                for type2 in &scanned {
                    let bound = similarity_upper_bound(type1, type2, &table, options, &mut spare);
                    let similarity =
                        compare_normalized_types(&type1.normalized, &type2.normalized, options)
                            .similarity;
                    assert!(
                        bound >= similarity - 1e-12,
                        "{} vs {}: bound {bound} < similarity {similarity}",
                        type1.definition.name,
                        type2.definition.name
                    );
                    if (bound - similarity).abs() < 1e-9 {
                        tight += 1;
                    }
                }
            }
            // Not a trivial bound: it is reached, and it rules pairs out.
            assert!(tight >= scanned.len() / 2, "{tight}");
            let ruled_out = scanned
                .iter()
                .flat_map(|type1| scanned.iter().map(move |type2| (type1, type2)))
                .filter(|(type1, type2)| {
                    similarity_upper_bound(type1, type2, &table, options, &mut spare) < 0.8
                })
                .count();
            assert!(ruled_out > scanned.len() * scanned.len() / 2, "{ruled_out}");
        }
    }

    #[test]
    fn the_scan_reports_what_comparing_every_pair_reports() {
        let types = extracted(SHAPES);
        for options in [
            TypeComparisonOptions::default(),
            TypeComparisonOptions { allow_cross_kind_comparison: false, ..Default::default() },
        ] {
            for threshold in [0.0, 0.3, 0.5, 0.8, 0.95, 1.0] {
                let mut expected: Vec<(String, String, f64, f64, f64)> = Vec::new();
                for (i, type1) in types.iter().enumerate() {
                    for type2 in &types[i + 1..] {
                        let result = compare_types(type1, type2, &options);
                        if result.similarity >= threshold {
                            expected.push((
                                type1.name.clone(),
                                type2.name.clone(),
                                result.similarity,
                                result.structural_similarity,
                                result.naming_similarity,
                            ));
                        }
                    }
                }
                expected.sort_by(|a, b| b.2.total_cmp(&a.2));
                let scanned: Vec<(String, String, f64, f64, f64)> =
                    find_similar_types(&types, threshold, &options)
                        .into_iter()
                        .map(|pair| {
                            (
                                pair.type1.name,
                                pair.type2.name,
                                pair.result.similarity,
                                pair.result.structural_similarity,
                                pair.result.naming_similarity,
                            )
                        })
                        .collect();
                assert_eq!(scanned, expected, "threshold {threshold}");
            }
        }
        // The fixtures do hold duplicates.
        assert!(find_similar_types(&types, 0.8, &TypeComparisonOptions::default()).len() > 8);
    }
}
