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
            body_fingerprint: method.body_fingerprint,
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

    // Combined similarity (weighted average). Structural evidence
    // dominates, mirroring the type comparator's 0.85/0.15 rebalance: a
    // fully-renamed twin (class, fields, AND method names changed) keeps
    // no lexical overlap, and the old 0.3 naming weight pushed such pairs
    // below threshold even when every canonical method body matched. The
    // agreement factors inside the structural score already discount
    // same-name/different-body lookalikes, so naming adds little there.
    let similarity = 0.15 * naming_similarity + 0.85 * structural_similarity;

    ClassComparisonResult { similarity, structural_similarity, naming_similarity, differences }
}

fn calculate_name_similarity(name1: &str, name2: &str) -> f64 {
    if name1 == name2 {
        return 1.0;
    }

    // Calculate Levenshtein distance (char-based, like the distance).
    let distance = levenshtein_distance(name1, name2);
    let max_len = name1.chars().count().max(name2.chars().count()) as f64;

    if max_len > 0.0 {
        1.0 - (distance as f64 / max_len)
    } else {
        1.0
    }
}

/// Multiplier applied to a matched method pair's credit reflecting how
/// much the two methods actually agree beyond their names/signatures:
///
/// * canonical body fingerprints disagree → bodies compute different
///   things (`0.55`) — signatures alone used to hide this entirely;
/// * `static` mismatch → different runtime surface (`0.4`);
/// * accessor-kind mismatch (getter/setter vs plain method) → different
///   call contract (`0.5`).
fn method_agreement_factor(method1: &ClassMethod, method2: &ClassMethod) -> f64 {
    let mut factor = 1.0;
    if let (Some(fp1), Some(fp2)) = (method1.body_fingerprint, method2.body_fingerprint) {
        if fp1 != fp2 {
            factor *= 0.55;
        }
    }
    if method1.is_static != method2.is_static {
        factor *= 0.4;
    }
    if method1.kind != method2.kind {
        factor *= 0.5;
    }
    factor
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
            // A `static` field and an instance field live on different
            // runtime surfaces even when name and type agree — same
            // discount the method pass applies via its agreement factor.
            let modifier_factor = if prop1.is_static == prop2.is_static { 1.0 } else { 0.4 };
            // Class-reference types tolerate renames: `repo: InvoiceRepo`
            // and `repo: ReceiptRepo` are the same dependency slot in two
            // renamed class families (the same placeholdering the fuzzy
            // method matcher applies to signatures). Primitive types are
            // untouched — `count: number` vs `count: string` stays a
            // genuine mismatch.
            if prop1.type_annotation == prop2.type_annotation
                || replace_camelcase_identifiers(&prop1.type_annotation)
                    == replace_camelcase_identifiers(&prop2.type_annotation)
            {
                // Strict match — credit both class1 and class2 sides.
                property_score += 2.0 * modifier_factor;
            } else {
                // Same name, different type — partial match.
                property_score += 1.4 * modifier_factor;
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
            let mut score = 0.8 * type_match + 0.2 * name_sim;
            // Same runtime-surface rule as the strict pass: a static
            // field is not a rename of an instance field.
            if prop1.is_static != prop2.is_static {
                score *= 0.4;
            }
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
            let agreement = method_agreement_factor(method1, method2);
            if sig1 == sig2
                || normalize_signature_for_fuzzy(method1) == normalize_signature_for_fuzzy(method2)
            {
                method_score += 2.0 * agreement;
                if agreement < 1.0 {
                    method_signature_mismatches.push(MethodMismatch {
                        name: (*name).to_string(),
                        signature1: sig1,
                        signature2: sig2,
                    });
                }
            } else {
                method_score += 1.4 * agreement;
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
            let mut score = 0.6 * sig_match + 0.4 * name_sim;
            // Identical canonical body fingerprints are the strongest
            // duplicate signal there is: a full method rename
            // (`addSample` → `recordPoint`) leaves no lexical overlap for
            // the name term, but two methods that parse to the same
            // canonical tree DO the same thing. Let the body evidence
            // dominate the name evidence instead of averaging with it —
            // but only when the signatures agree (exactly or after the
            // fuzzy name-stripping): the fingerprint tree carries no
            // TypeScript type annotations, so `(input: string) => string`
            // and `(input: number) => string` around the same body hash
            // identically while exposing different public contracts.
            let fingerprints_match = matches!(
                (method1.body_fingerprint, method2.body_fingerprint),
                (Some(fp1), Some(fp2)) if fp1 == fp2
            );
            if fingerprints_match && sig_match >= 0.85 {
                score = score.max(0.95);
            }
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
                let method2 = &class2.methods[name2];
                method_score += 2.0 * score * method_agreement_factor(method1, method2);
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

    let structural_similarity = if total_elements > 0.0 {
        (matched_elements / total_elements).min(1.0)
    } else {
        // Two member-less classes: everything they DO comes from their
        // heritage, so "identical" is only justified when the heritage
        // matches (`class NotFoundError extends HttpError {}` vs
        // `class RateLimitError extends OtherBase {}` share nothing).
        if class1.extends == class2.extends && class1.implements == class2.implements {
            0.85
        } else {
            0.3
        }
    };

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
    // Operate on CHAR counts throughout. Sizing the matrix by byte length
    // while filling it by char iteration used to leave the bottom-right
    // cell untouched for multibyte names, so any two non-ASCII class
    // names compared as distance 0 (similarity 1.0).
    let chars1: Vec<char> = s1.chars().collect();
    let chars2: Vec<char> = s2.chars().collect();
    let len1 = chars1.len();
    let len2 = chars2.len();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut matrix = vec![vec![0usize; len2 + 1]; len1 + 1];
    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate() {
        *cell = j;
    }

    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = usize::from(chars1[i - 1] != chars2[j - 1]);
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class_extractor::extract_classes_from_code;

    fn compare_sources(source1: &str, source2: &str) -> ClassComparisonResult {
        let classes1 = extract_classes_from_code(source1, "a.ts").unwrap();
        let classes2 = extract_classes_from_code(source2, "b.ts").unwrap();
        compare_classes(&classes1[0], &classes2[0])
    }

    #[test]
    fn renamed_method_names_do_not_hide_equal_bodies() {
        // XC-P04 shape: class, field, AND method names all renamed, but
        // every signature and canonical method body matches.
        let result = compare_sources(
            r"
export class SampleWindow {
  private values: number[] = [];
  addSample(value: number): void {
    this.values.push(value);
    if (this.values.length > 50) this.values.shift();
  }
  averageOf(): number {
    if (this.values.length === 0) return 0;
    return this.values.reduce((acc, v) => acc + v, 0) / this.values.length;
  }
}
",
            r"
export class ReadingWindow {
  private points: number[] = [];
  recordPoint(point: number): void {
    this.points.push(point);
    if (this.points.length > 50) this.points.shift();
  }
  meanOf(): number {
    if (this.points.length === 0) return 0;
    return this.points.reduce((acc, p) => acc + p, 0) / this.points.length;
  }
}
",
        );
        assert!(
            result.similarity >= 0.8,
            "fully-renamed twin with equal bodies must clear the default threshold, got {}",
            result.similarity
        );
    }

    #[test]
    fn fingerprint_boost_requires_compatible_signatures() {
        // The fingerprint tree carries no TypeScript type annotations, so
        // two same-body methods over DIFFERENT parameter types hash
        // identically — the boost must not fire when the (annotation-
        // aware) signatures disagree, because the public contracts
        // differ.
        let result = compare_sources(
            r"
export class TextNormalizer {
  private count: number = 0;
  read(input: string): string {
    this.count += 1;
    return String(input);
  }
}
",
            r"
export class CodeFormatter {
  private total: number = 0;
  parse(input: number): string {
    this.total += 1;
    return String(input);
  }
}
",
        );
        assert!(
            result.similarity < 0.8,
            "same-body methods with incompatible signatures must stay below threshold, got {}",
            result.similarity
        );
    }

    #[test]
    fn static_and_instance_fields_are_different_surfaces() {
        // Same field name and type, but one lives on the class and the
        // other on instances — with unrelated class names this must not
        // report as a duplicate on the property signal alone.
        let result = compare_sources(
            r"
export class GlobalRegistry {
  static entries: Map<string, string> = new Map();
}
",
            r"
export class SessionScratch {
  entries: Map<string, string> = new Map();
}
",
        );
        assert!(
            result.similarity < 0.8,
            "static vs instance field lookalikes must stay below threshold, got {}",
            result.similarity
        );
    }

    #[test]
    fn renamed_methods_with_different_bodies_get_no_fingerprint_boost() {
        // Same member counts and signature shapes, but the bodies compute
        // different things — the fingerprint fast-path must not fire.
        let result = compare_sources(
            r"
export class RollingMax {
  private values: number[] = [];
  record(value: number): void {
    this.values.push(value);
    if (this.values.length > 50) this.values.shift();
  }
  currentPeak(): number {
    if (this.values.length === 0) return 0;
    return Math.max(...this.values);
  }
}
",
            r"
export class GapTracker {
  private points: number[] = [];
  observe(point: number): void {
    if (this.points.length >= 50) this.points.pop();
    this.points.unshift(point);
  }
  widestGap(): number {
    if (this.points.length < 2) return 0;
    return Math.max(...this.points) - Math.min(...this.points);
  }
}
",
        );
        assert!(
            result.similarity < 0.8,
            "different-body lookalikes must stay below the default threshold, got {}",
            result.similarity
        );
    }
}
