use crate::class_extractor::{ClassDefinition, ClassMethod, ClassProperty};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct NormalizedClass {
    pub name: String,
    pub properties: HashMap<String, ClassProperty>,
    pub methods: HashMap<String, ClassMethod>,
    pub constructor_signature: String,
    pub extends: Option<String>,
    pub implements: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClassComparisonResult {
    pub similarity: f64,
    pub structural_similarity: f64,
    pub naming_similarity: f64,
    pub differences: ClassDifferences,
}

#[derive(Debug, Clone)]
pub struct ClassDifferences {
    pub missing_properties: Vec<String>,
    pub extra_properties: Vec<String>,
    pub missing_methods: Vec<String>,
    pub extra_methods: Vec<String>,
    pub property_type_mismatches: Vec<PropertyMismatch>,
    pub method_signature_mismatches: Vec<MethodMismatch>,
}

#[derive(Debug, Clone)]
pub struct PropertyMismatch {
    pub name: String,
    pub type1: String,
    pub type2: String,
}

#[derive(Debug, Clone)]
pub struct MethodMismatch {
    pub name: String,
    pub signature1: String,
    pub signature2: String,
}

#[derive(Debug, Clone)]
pub struct SimilarClassPair {
    pub class1: ClassDefinition,
    pub class2: ClassDefinition,
    pub result: ClassComparisonResult,
}

pub fn normalize_class(class: &ClassDefinition) -> NormalizedClass {
    let mut properties = HashMap::new();
    for prop in &class.properties {
        properties.insert(prop.name.clone(), prop.clone());
    }

    let mut methods = HashMap::new();
    for method in &class.methods {
        // Normalize method signature
        let normalized_method = ClassMethod {
            name: method.name.clone(),
            parameters: normalize_parameters(&method.parameters),
            return_type: normalize_type(&method.return_type),
            is_static: method.is_static,
            is_private: method.is_private,
            is_async: method.is_async,
            is_generator: method.is_generator,
            kind: method.kind.clone(),
        };
        methods.insert(method.name.clone(), normalized_method);
    }

    let constructor_signature = if class.constructor_params.is_empty() {
        "()".to_string()
    } else {
        format!("({})", class.constructor_params.join(", "))
    };

    NormalizedClass {
        name: class.name.clone(),
        properties,
        methods,
        constructor_signature,
        extends: class.extends.clone(),
        implements: class.implements.clone(),
    }
}

fn normalize_parameters(params: &[String]) -> Vec<String> {
    params.iter().map(|p| normalize_type(p)).collect()
}

fn normalize_type(type_str: &str) -> String {
    // Basic normalization - can be expanded
    type_str.replace("Array<", "[").replace(">", "]").replace(" ", "").trim().to_string()
}

/// Strip parameter names and class-shaped tokens so two methods that have
/// the same structural signature but sit in renamed classes can still be
/// recognised as matching. Used only by the fuzzy method-pairing pass.
///
/// `ClassMethod.parameters` is populated by the class extractor as a
/// one-element `Vec<String>` containing the whole comma-separated
/// parameter list (e.g. `["a: A, b: B"]`). Splitting it requires
/// brackets-aware handling because parameter types legitimately contain
/// commas (`Map<string, number>`, tuples `[a, b]`, object literals,
/// function types `(a, b) => c`); a naive `split(',')` would otherwise
/// shred those types and break the rename-detection fast path.
///
/// Class-shaped tokens (CamelCase identifiers) get a placeholder both in
/// parameter types AND in the return type — a method whose body
/// references the surrounding class also exposes that class in its
/// parameter signature, and we want renamed-class methods to match on
/// either side.
fn normalize_signature_for_fuzzy(method: &ClassMethod) -> String {
    let joined = method.parameters.join(", ");
    let stripped_params: Vec<String> = if joined.trim().is_empty() {
        Vec::new()
    } else {
        split_parameter_list(&joined)
            .into_iter()
            .map(|piece| {
                let without_name = match piece.find(':') {
                    Some(idx) => piece[idx + 1..].trim(),
                    None => piece,
                };
                replace_camelcase_identifiers(without_name)
            })
            .collect()
    };
    let normalized_return = replace_camelcase_identifiers(&method.return_type);
    format!("({}) => {}", stripped_params.join(", "), normalized_return)
}

/// Split a parameter list on top-level commas, ignoring commas that sit
/// inside `<>`, `()`, `[]` or `{}`. Returned slices are trimmed.
fn split_parameter_list(input: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    for (idx, ch) in input.char_indices() {
        match ch {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth = (depth - 1).max(0),
            ',' if depth == 0 => {
                let piece = input[start..idx].trim();
                if !piece.is_empty() {
                    parts.push(piece);
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = input[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn replace_camelcase_identifiers(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut current = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphabetic() || ch == '_' || ch.is_ascii_digit() {
            current.push(ch);
        } else {
            push_identifier_token(&current, &mut out);
            current.clear();
            out.push(ch);
        }
    }
    push_identifier_token(&current, &mut out);
    out
}

fn push_identifier_token(token: &str, out: &mut String) {
    if token.is_empty() {
        return;
    }
    let is_camel_class = token.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && token.chars().any(|c| c.is_ascii_lowercase());
    if is_camel_class {
        // Substitute a fixed placeholder so two class names of the same
        // shape collapse together. Primitive type names (string, number,
        // boolean, void, etc.) stay intact since they're lowercase.
        out.push_str("#T");
    } else {
        out.push_str(token);
    }
}

fn normalized_signature_overlap(sig1: &str, sig2: &str) -> f64 {
    // Rough Jaccard on alphabetic tokens. Good enough to give a small
    // bonus when two normalised signatures share a few tokens but aren't
    // identical (e.g. partially overlapping parameter types).
    let tokens1: std::collections::HashSet<&str> =
        sig1.split(|c: char| !c.is_ascii_alphabetic()).filter(|s| !s.is_empty()).collect();
    let tokens2: std::collections::HashSet<&str> =
        sig2.split(|c: char| !c.is_ascii_alphabetic()).filter(|s| !s.is_empty()).collect();
    if tokens1.is_empty() && tokens2.is_empty() {
        return 1.0;
    }
    let intersection = tokens1.intersection(&tokens2).count();
    let union = tokens1.union(&tokens2).count();
    if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
}

pub fn compare_classes(
    class1: &ClassDefinition,
    class2: &ClassDefinition,
) -> ClassComparisonResult {
    let norm1 = normalize_class(class1);
    let norm2 = normalize_class(class2);

    // Calculate naming similarity
    let naming_similarity = calculate_name_similarity(&class1.name, &class2.name);

    // Calculate structural similarity
    let (structural_similarity, differences) = calculate_structural_similarity(&norm1, &norm2);

    // Combined similarity (weighted average)
    let similarity = 0.3 * naming_similarity + 0.7 * structural_similarity;

    ClassComparisonResult { similarity, structural_similarity, naming_similarity, differences }
}

fn calculate_name_similarity(name1: &str, name2: &str) -> f64 {
    if name1 == name2 {
        return 1.0;
    }

    // Calculate Levenshtein distance
    let distance = levenshtein_distance(name1, name2);
    let max_len = name1.len().max(name2.len()) as f64;

    if max_len > 0.0 {
        1.0 - (distance as f64 / max_len)
    } else {
        1.0
    }
}

fn calculate_structural_similarity(
    class1: &NormalizedClass,
    class2: &NormalizedClass,
) -> (f64, ClassDifferences) {
    let mut missing_properties = Vec::new();
    let mut extra_properties = Vec::new();
    let mut property_type_mismatches = Vec::new();

    // Properties: two-phase matching.
    //
    // Phase 1 — strict by name. A property keeps its name across most
    // refactors so this is the primary signal.
    //
    // Phase 2 — fuzzy by type for the leftovers. A common refactor pattern
    // is renaming a private storage field (`store` → `items` between two
    // cache classes); strict-name matching scores those as a full mismatch
    // which then drowns out an otherwise-identical method surface. Pairing
    // unmatched properties by type — and crediting them as partial matches
    // — recovers that signal without falsely matching properties of
    // unrelated types.
    let mut property_score = 0.0;
    let property_total_count = (class1.properties.len() + class2.properties.len()) as f64;
    let mut matched_in_class2: std::collections::HashSet<String> = Default::default();

    let mut leftovers_class1: Vec<&str> = Vec::new();

    // Iterate properties in name-sorted order so the strict pass behaves
    // deterministically (the HashMap iteration order is randomised in
    // Rust, and the greedy phase 2 below would otherwise produce
    // different pairings on different runs of the same input).
    let mut class1_property_names: Vec<&String> = class1.properties.keys().collect();
    class1_property_names.sort();
    for name in &class1_property_names {
        let prop1 = &class1.properties[*name];
        if let Some(prop2) = class2.properties.get(*name) {
            if prop1.type_annotation == prop2.type_annotation {
                // Strict match — credit both class1 and class2 sides.
                property_score += 2.0;
            } else {
                // Same name, different type — partial match.
                property_score += 1.4;
                property_type_mismatches.push(PropertyMismatch {
                    name: (*name).to_string(),
                    type1: prop1.type_annotation.clone(),
                    type2: prop2.type_annotation.clone(),
                });
            }
            matched_in_class2.insert((*name).to_string());
        } else {
            leftovers_class1.push(name.as_str());
        }
    }

    let mut leftovers_class2: Vec<&str> = class2
        .properties
        .keys()
        .filter(|n| !matched_in_class2.contains(n.as_str()))
        .map(|n| n.as_str())
        .collect();
    // Sorting both leftover lists makes the greedy match below produce
    // the same pairings on every run for a given input.
    leftovers_class2.sort();

    // Greedy best-match by type for the leftovers. This is N×M but class
    // properties are typically a handful so the quadratic factor is fine.
    let mut leftover_consumed_class2: std::collections::HashSet<String> = Default::default();
    for name1 in &leftovers_class1 {
        let prop1 = &class1.properties[*name1];
        let mut best_match: Option<(&str, f64)> = None;
        for name2 in &leftovers_class2 {
            if leftover_consumed_class2.contains(*name2) {
                continue;
            }
            let prop2 = &class2.properties[*name2];
            // Score: type match dominates, name similarity tie-breaks.
            let type_match = if prop1.type_annotation == prop2.type_annotation {
                1.0
            } else if !prop1.type_annotation.is_empty() && !prop2.type_annotation.is_empty() {
                // Both annotated but mismatched: weak partial credit if the
                // annotation strings share a non-trivial prefix (e.g.
                // `Map<string, A>` vs `Map<string, B>`).
                let shared_prefix = prop1
                    .type_annotation
                    .chars()
                    .zip(prop2.type_annotation.chars())
                    .take_while(|(a, b)| a == b)
                    .count();
                let max_len =
                    prop1.type_annotation.len().max(prop2.type_annotation.len()).max(1) as f64;
                (shared_prefix as f64 / max_len).min(0.6)
            } else {
                0.4
            };
            let name_sim = calculate_name_similarity(name1, name2);
            let score = 0.8 * type_match + 0.2 * name_sim;
            if best_match.is_none_or(|(_, b)| score > b) {
                best_match = Some((name2, score));
            }
        }
        if let Some((name2, score)) = best_match {
            if score >= 0.7 {
                // Credit it as a rename match — both sides count.
                property_score += 2.0 * score;
                leftover_consumed_class2.insert(name2.to_string());
                continue;
            }
        }
        missing_properties.push((*name1).to_string());
    }
    for name2 in &leftovers_class2 {
        if !leftover_consumed_class2.contains(*name2) {
            extra_properties.push((*name2).to_string());
        }
    }

    // Check methods — same two-phase matching strategy so renamed methods
    // (e.g. `findById` → `lookupById`) still register as related.
    let mut missing_methods = Vec::new();
    let mut extra_methods = Vec::new();
    let mut method_signature_mismatches = Vec::new();

    let method_total_count = (class1.methods.len() + class2.methods.len()) as f64;
    let mut method_score = 0.0;
    let mut method_matched_in_class2: std::collections::HashSet<String> = Default::default();
    let mut method_leftovers_class1: Vec<&str> = Vec::new();

    // Same deterministic ordering as the property pass — see the comment
    // there for why.
    let mut class1_method_names: Vec<&String> = class1.methods.keys().collect();
    class1_method_names.sort();
    for name in &class1_method_names {
        let method1 = &class1.methods[*name];
        if let Some(method2) = class2.methods.get(*name) {
            let sig1 = format!("({}) => {}", method1.parameters.join(", "), method1.return_type);
            let sig2 = format!("({}) => {}", method2.parameters.join(", "), method2.return_type);
            // Parameter renames (`findById(id)` vs `findById(key)`) are
            // neutral refactors — when the name matches and the
            // name-stripped signatures agree, credit a full match instead
            // of the partial-mismatch rate.
            if sig1 == sig2
                || normalize_signature_for_fuzzy(method1) == normalize_signature_for_fuzzy(method2)
            {
                method_score += 2.0;
            } else {
                method_score += 1.4;
                method_signature_mismatches.push(MethodMismatch {
                    name: (*name).to_string(),
                    signature1: sig1,
                    signature2: sig2,
                });
            }
            method_matched_in_class2.insert((*name).to_string());
        } else {
            method_leftovers_class1.push(name.as_str());
        }
    }

    let mut method_leftovers_class2: Vec<&str> = class2
        .methods
        .keys()
        .filter(|n| !method_matched_in_class2.contains(n.as_str()))
        .map(|n| n.as_str())
        .collect();
    method_leftovers_class2.sort();

    let mut method_leftover_consumed: std::collections::HashSet<String> = Default::default();
    for name1 in &method_leftovers_class1 {
        let method1 = &class1.methods[*name1];
        let sig1 = format!("({}) => {}", method1.parameters.join(", "), method1.return_type);
        let normalized_sig1 = normalize_signature_for_fuzzy(method1);
        let mut best_match: Option<(&str, f64)> = None;
        for name2 in &method_leftovers_class2 {
            if method_leftover_consumed.contains(*name2) {
                continue;
            }
            let method2 = &class2.methods[*name2];
            let sig2 = format!("({}) => {}", method2.parameters.join(", "), method2.return_type);
            let normalized_sig2 = normalize_signature_for_fuzzy(method2);
            // Tiered signature match: exact > param-shape-only > nothing.
            // The "param-shape" tier strips identifier names from
            // parameters and the surrounding-class name from the return
            // type, so methods that genuinely have the same shape but
            // sit in renamed classes (UserBuilder.withName vs
            // CustomerBuilder.withLabel) still register as similar.
            let sig_match = if sig1 == sig2 {
                1.0
            } else if normalized_sig1 == normalized_sig2 {
                0.85
            } else {
                let name_overlap = normalized_signature_overlap(&normalized_sig1, &normalized_sig2);
                0.4 + 0.4 * name_overlap
            };
            let name_sim = calculate_name_similarity(name1, name2);
            let score = 0.6 * sig_match + 0.4 * name_sim;
            if best_match.is_none_or(|(_, b)| score > b) {
                best_match = Some((name2, score));
            }
        }
        if let Some((name2, score)) = best_match {
            // Threshold tuned so a same-shape method whose name has
            // some lexical overlap with the original (`withName` vs
            // `withLabel` share a `with` prefix) registers, but a
            // structurally different method with a tangentially similar
            // name does not.
            if score >= 0.65 {
                method_score += 2.0 * score;
                method_leftover_consumed.insert(name2.to_string());
                continue;
            }
        }
        missing_methods.push((*name1).to_string());
    }
    for name2 in &method_leftovers_class2 {
        if !method_leftover_consumed.contains(*name2) {
            extra_methods.push((*name2).to_string());
        }
    }

    // Calculate overall structural similarity. Each side counts once, so
    // a "complete match" pair contributes 2 to the score and 2 to the
    // denominator — leaving the ratio at 1.0 for an exact match.
    let total_elements = property_total_count + method_total_count;
    let matched_elements = property_score + method_score;

    let structural_similarity =
        if total_elements > 0.0 { (matched_elements / total_elements).min(1.0) } else { 1.0 };

    let differences = ClassDifferences {
        missing_properties,
        extra_properties,
        missing_methods,
        extra_methods,
        property_type_mismatches,
        method_signature_mismatches,
    };

    (structural_similarity, differences)
}

fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.len();
    let len2 = s2.len();
    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    #[allow(clippy::needless_range_loop)]
    for i in 0..=len1 {
        matrix[i][0] = i;
    }

    #[allow(clippy::needless_range_loop)]
    for j in 0..=len2 {
        matrix[0][j] = j;
    }

    for (i, c1) in s1.chars().enumerate() {
        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            matrix[i + 1][j + 1] = std::cmp::min(
                std::cmp::min(matrix[i][j + 1] + 1, matrix[i + 1][j] + 1),
                matrix[i][j] + cost,
            );
        }
    }

    matrix[len1][len2]
}

pub fn find_similar_classes(classes: &[ClassDefinition], threshold: f64) -> Vec<SimilarClassPair> {
    let mut similar_pairs = Vec::new();

    for i in 0..classes.len() {
        for j in i + 1..classes.len() {
            let result = compare_classes(&classes[i], &classes[j]);

            if result.similarity >= threshold {
                similar_pairs.push(SimilarClassPair {
                    class1: classes[i].clone(),
                    class2: classes[j].clone(),
                    result,
                });
            }
        }
    }

    // Sort by similarity (highest first)
    similar_pairs.sort_by(|a, b| {
        b.result.similarity.partial_cmp(&a.result.similarity).unwrap_or(std::cmp::Ordering::Equal)
    });

    similar_pairs
}

pub fn find_similar_classes_across_files(
    files: &[(String, String)],
    threshold: f64,
) -> Vec<SimilarClassPair> {
    let mut all_classes = Vec::new();

    for (file_path, content) in files {
        if let Ok(classes) = crate::class_extractor::extract_classes_from_code(content, file_path) {
            all_classes.extend(classes);
        }
    }

    find_similar_classes(&all_classes, threshold)
}
