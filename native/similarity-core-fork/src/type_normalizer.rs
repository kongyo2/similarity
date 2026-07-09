use crate::type_extractor::{TypeDefinition, TypeKind};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct NormalizedType {
    pub properties: HashMap<String, String>, // プロパティ名 -> 型
    pub optional_properties: HashSet<String>,
    pub readonly_properties: HashSet<String>,
    pub signature: String, // 正規化された型シグネチャ
    pub original_name: String,
    pub kind: TypeKind,
}

#[derive(Debug, Clone)]
pub struct NormalizationOptions {
    pub ignore_property_order: bool,
    pub ignore_optional_modifiers: bool,
    pub ignore_readonly_modifiers: bool,
    pub normalize_type_names: bool,
}

impl Default for NormalizationOptions {
    fn default() -> Self {
        Self {
            ignore_property_order: true,
            ignore_optional_modifiers: false,
            ignore_readonly_modifiers: true,
            normalize_type_names: true,
        }
    }
}

/// Normalize a type definition for comparison
pub fn normalize_type(type_def: &TypeDefinition, options: &NormalizationOptions) -> NormalizedType {
    let mut properties = HashMap::new();
    let mut optional_properties = HashSet::new();
    let mut readonly_properties = HashSet::new();

    // Process each property. Names keep their case — lowercasing used to
    // merge `Name` and `name` into one map slot, silently dropping a
    // property.
    for prop in &type_def.properties {
        let normalized_prop_name = prop.name.trim().to_string();
        let mut normalized_type = if options.normalize_type_names {
            normalize_type_name(&prop.type_annotation)
        } else {
            prop.type_annotation.clone()
        };

        // `x: T | undefined` and `x?: T` are the same contract for
        // ordinary TypeScript configurations: strip a top-level
        // `undefined` union arm and record optionality instead, so both
        // spellings normalize identically.
        let mut effective_optional = prop.optional;
        if options.normalize_type_names {
            let arms = split_top_level(&normalized_type, '|');
            if arms.len() > 1 && arms.iter().any(|arm| arm.trim() == "undefined") {
                let kept: Vec<String> = arms
                    .into_iter()
                    .map(|arm| arm.trim().to_string())
                    .filter(|arm| arm != "undefined")
                    .collect();
                if !kept.is_empty() {
                    normalized_type = kept.join(" | ");
                    effective_optional = true;
                }
            }
        }

        properties.insert(normalized_prop_name.clone(), normalized_type);

        if effective_optional && !options.ignore_optional_modifiers {
            optional_properties.insert(normalized_prop_name.clone());
        }

        if prop.readonly && !options.ignore_readonly_modifiers {
            readonly_properties.insert(normalized_prop_name);
        }
    }

    // Generate normalized signature
    let signature = generate_type_signature(
        &properties,
        &optional_properties,
        &readonly_properties,
        options.ignore_property_order,
    );

    NormalizedType {
        properties,
        optional_properties,
        readonly_properties,
        signature,
        original_name: type_def.name.clone(),
        kind: type_def.kind.clone(),
    }
}

/// Split `input` at top-level occurrences of `separator`, respecting
/// nesting inside `<>`, `()`, `[]`, `{}` and quoted strings. Returns one
/// element (the whole input) when the separator never appears at the top
/// level.
pub(crate) fn split_top_level(input: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_string: Option<char> = None;
    let mut previous = '\0';
    let mut current = String::new();
    for ch in input.chars() {
        if let Some(quote) = in_string {
            current.push(ch);
            if ch == quote {
                in_string = None;
            }
            previous = ch;
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                in_string = Some(ch);
                current.push(ch);
            }
            '<' | '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            // The `>` of an arrow (`=>`) is an operator, not a bracket —
            // decrementing on it desynchronized the depth for every type
            // containing a function arm (`Result<() => string, Error>`).
            '>' if previous == '=' => {
                current.push(ch);
            }
            '>' | ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            _ if ch == separator && depth == 0 => {
                parts.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(ch),
        }
        previous = ch;
    }
    parts.push(current.trim().to_string());
    parts.retain(|part| !part.is_empty());
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

/// Normalize a rendered type string for consistent comparison:
///
/// * union / intersection arms are normalized recursively and sorted, at
///   any nesting depth and regardless of the original spacing
/// * `Array<T>` rewrites to `T[]` (recursively)
/// * generic argument lists are normalized element-wise
/// * boxed primitive names map to their primitive spelling as whole
///   tokens only — substring replacement used to corrupt identifiers
///   like `PhoneNumber` → `Phonenumber`
pub fn normalize_type_name(type_name: &str) -> String {
    let trimmed = type_name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Union arms: normalize each, sort, dedupe.
    let union_arms = split_top_level(trimmed, '|');
    if union_arms.len() > 1 {
        let mut arms: Vec<String> =
            union_arms.iter().map(|arm| normalize_type_name(arm)).collect();
        arms.sort();
        arms.dedup();
        return arms.join(" | ");
    }

    // Intersection arms.
    let intersection_arms = split_top_level(trimmed, '&');
    if intersection_arms.len() > 1 {
        let mut arms: Vec<String> =
            intersection_arms.iter().map(|arm| normalize_type_name(arm)).collect();
        arms.sort();
        arms.dedup();
        return arms.join(" & ");
    }

    // Function types: strip parameter NAMES (they are local to the
    // signature — `(request: string) => R` and `(url: string) => R` are
    // the same contract) and normalize parameter/return types.
    if let Some((params, return_type)) = split_function_type(trimmed) {
        let normalized_params: Vec<String> = split_top_level(&params, ',')
            .iter()
            .filter(|piece| !piece.is_empty())
            .map(|piece| {
                // `name: T` / `name?: T` → T; a piece without a colon is
                // already a bare type (or a rest param `...rest: T`).
                let piece = piece.trim_start_matches("...");
                match piece.find(':') {
                    Some(colon)
                        if piece[..colon]
                            .trim_end_matches('?')
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '$') =>
                    {
                        normalize_type_name(piece[colon + 1..].trim())
                    }
                    _ => normalize_type_name(piece),
                }
            })
            .collect();
        return format!("({}) => {}", normalized_params.join(", "), normalize_type_name(&return_type));
    }

    // `T[]` — normalize the element type, keep the suffix.
    if let Some(inner) = trimmed.strip_suffix("[]") {
        if !inner.is_empty() && is_balanced(inner) {
            return format!("{}[]", normalize_type_name(inner));
        }
    }

    // Generic references: `Array<T>` → `T[]`; other generics normalize
    // their arguments in place.
    if let Some(open) = trimmed.find('<') {
        if trimmed.ends_with('>') {
            let base = trimmed[..open].trim();
            let args_text = &trimmed[open + 1..trimmed.len() - 1];
            if is_balanced(args_text) && base.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let args: Vec<String> = split_top_level(args_text, ',')
                    .iter()
                    .map(|arg| normalize_type_name(arg))
                    .collect();
                if base == "Array" && args.len() == 1 {
                    let element = &args[0];
                    // Parenthesize compound elements so `Array<A | B>`
                    // round-trips as `(A | B)[]`.
                    if element.contains(" | ") || element.contains(" & ") || element.contains("=>")
                    {
                        return format!("({element})[]");
                    }
                    return format!("{element}[]");
                }
                return format!("{base}<{}>", args.join(", "));
            }
        }
    }

    // Boxed primitives — whole-token only.
    match trimmed {
        "String" => "string".to_string(),
        "Number" => "number".to_string(),
        "Boolean" => "boolean".to_string(),
        "Object" => "object".to_string(),
        _ => trimmed.to_string(),
    }
}

fn is_balanced(text: &str) -> bool {
    let mut depth = 0i32;
    let mut previous = '\0';
    for ch in text.chars() {
        match ch {
            '<' | '(' | '[' | '{' => depth += 1,
            // Skip the `>` of `=>` — see `split_top_level`.
            '>' if previous == '=' => {}
            '>' | ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
        previous = ch;
    }
    depth == 0
}

/// Generate a normalized signature for the type
fn generate_type_signature(
    properties: &HashMap<String, String>,
    optional_properties: &HashSet<String>,
    readonly_properties: &HashSet<String>,
    ignore_order: bool,
) -> String {
    let mut prop_entries: Vec<(&String, &String)> = properties.iter().collect();

    if ignore_order {
        prop_entries.sort_by(|a, b| a.0.cmp(b.0));
    }

    let prop_strings: Vec<String> = prop_entries
        .iter()
        .map(|(name, type_annotation)| {
            let mut prop_str = String::new();

            if readonly_properties.contains(*name) {
                prop_str.push_str("readonly ");
            }

            prop_str.push_str(name);

            if optional_properties.contains(*name) {
                prop_str.push('?');
            }

            prop_str.push_str(": ");
            prop_str.push_str(type_annotation);

            prop_str
        })
        .collect();

    format!("{{ {} }}", prop_strings.join("; "))
}

/// Calculate similarity between two property names using Levenshtein distance
pub fn calculate_property_similarity(prop1: &str, prop2: &str) -> f64 {
    if prop1 == prop2 {
        return 1.0;
    }

    let normalized1 = prop1.to_lowercase();
    let normalized2 = prop2.to_lowercase();
    let normalized1 = normalized1.trim();
    let normalized2 = normalized2.trim();

    if normalized1 == normalized2 {
        return 0.95;
    }

    let max_length = normalized1.len().max(normalized2.len());
    if max_length == 0 {
        return 1.0;
    }

    let distance = levenshtein_distance(normalized1, normalized2);
    (1.0 - (distance as f64 / max_length as f64)).max(0.0)
}

/// Calculate similarity between two type strings
pub fn calculate_type_similarity(type1: &str, type2: &str) -> f64 {
    let normalized1 = normalize_type_name(type1);
    let normalized2 = normalize_type_name(type2);

    if normalized1 == normalized2 {
        return 1.0;
    }

    // Handle union types specially
    if split_top_level(&normalized1, '|').len() > 1
        || split_top_level(&normalized2, '|').len() > 1
    {
        return calculate_union_type_similarity(&normalized1, &normalized2);
    }

    // Handle intersection types specially
    if split_top_level(&normalized1, '&').len() > 1
        || split_top_level(&normalized2, '&').len() > 1
    {
        return calculate_intersection_type_similarity(&normalized1, &normalized2);
    }

    // Function types compare structurally: the return type carries most
    // of the contract, parameters the rest. This keeps `(string) =>
    // Promise<User>` and `(string) => Promise<Order>` far apart where
    // plain edit distance saw a near-match.
    match (split_function_type(&normalized1), split_function_type(&normalized2)) {
        (Some((params1, return1)), Some((params2, return2))) => {
            let params_similarity = if params1 == params2 {
                1.0
            } else {
                let arms1 = split_top_level(&params1, ',');
                let arms2 = split_top_level(&params2, ',');
                if arms1.len() == arms2.len() {
                    let total: f64 = arms1
                        .iter()
                        .zip(arms2.iter())
                        .map(|(a, b)| calculate_type_similarity(a, b))
                        .sum();
                    #[allow(clippy::cast_precision_loss)]
                    let count = arms1.len().max(1) as f64;
                    total / count
                } else {
                    0.3
                }
            };
            let return_similarity = calculate_type_similarity(&return1, &return2);
            return (0.35 * params_similarity + 0.65 * return_similarity).min(1.0);
        }
        (Some(_), None) | (None, Some(_)) => return 0.15,
        (None, None) => {}
    }

    // Array types compare element-wise: `ShopUser[]` vs `ShopOrder[]` is
    // the payload contrast itself, not the near-match plain edit distance
    // saw in the bracketed names, and an array vs a non-array is a shape
    // mismatch outright.
    let element1 =
        normalized1.strip_suffix("[]").filter(|inner| !inner.is_empty() && is_balanced(inner));
    let element2 =
        normalized2.strip_suffix("[]").filter(|inner| !inner.is_empty() && is_balanced(inner));
    match (element1, element2) {
        (Some(element1), Some(element2)) => {
            return calculate_type_similarity(element1, element2);
        }
        (Some(_), None) | (None, Some(_)) => return 0.2,
        (None, None) => {}
    }

    // Generic references compare structurally: same container with
    // different payloads (`Promise<User>` vs `Promise<Order>`) is a REAL
    // contract difference, not the near-match plain edit distance would
    // report from the shared container name.
    match (parse_generic_reference(&normalized1), parse_generic_reference(&normalized2)) {
        (Some((base1, args1)), Some((base2, args2))) => {
            if base1 == base2 && args1.len() == args2.len() {
                // Same container, same argument multiset, different
                // positions — `Map<string, number>` vs `Map<number,
                // string>` is a key/value swap, i.e. an inverted
                // contract, not a near-duplicate.
                if args1.len() >= 2 && args1 != args2 {
                    let mut sorted1 = args1.clone();
                    let mut sorted2 = args2.clone();
                    sorted1.sort();
                    sorted2.sort();
                    if sorted1 == sorted2 {
                        return 0.25;
                    }
                }
                let total: f64 = args1
                    .iter()
                    .zip(args2.iter())
                    .map(|(arg1, arg2)| calculate_type_similarity(arg1, arg2))
                    .sum();
                #[allow(clippy::cast_precision_loss)]
                let average = total / args1.len().max(1) as f64;
                return 0.35 + 0.4 * average;
            }
            return 0.2;
        }
        (Some(_), None) | (None, Some(_)) => return 0.2,
        (None, None) => {}
    }

    // Bare type references are nominal: `ShopUser` vs `ShopOrder` are
    // unrelated contracts no matter how many characters they share, so
    // don't let Levenshtein closeness of the NAMES manufacture type
    // similarity.
    if is_bare_type_reference(&normalized1) && is_bare_type_reference(&normalized2) {
        return if normalized1 == normalized2 { 1.0 } else { 0.2 };
    }

    // For other (structured) types, use string similarity
    let max_length = normalized1.len().max(normalized2.len());
    if max_length == 0 {
        return 1.0;
    }

    let distance = levenshtein_distance(&normalized1, &normalized2);
    (1.0 - (distance as f64 / max_length as f64)).max(0.0)
}

/// Split a top-level `(params) => return` function type. Returns `None`
/// unless the string starts with a balanced parameter list followed by a
/// top-level arrow.
fn split_function_type(type_name: &str) -> Option<(String, String)> {
    if !type_name.starts_with('(') {
        return None;
    }
    let mut depth = 0i32;
    let mut params_end = None;
    for (index, ch) in type_name.char_indices() {
        match ch {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' | '>' | ']' | '}' => {
                depth -= 1;
                if depth == 0 && ch == ')' {
                    params_end = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let params_end = params_end?;
    let rest = type_name[params_end + 1..].trim_start();
    let return_type = rest.strip_prefix("=>")?;
    Some((type_name[1..params_end].to_string(), return_type.trim().to_string()))
}

/// `Base<Arg, …>` splitter for already-normalized type strings.
fn parse_generic_reference(type_name: &str) -> Option<(&str, Vec<String>)> {
    let open = type_name.find('<')?;
    if !type_name.ends_with('>') {
        return None;
    }
    let base = &type_name[..open];
    if base.is_empty() || !base.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let args = split_top_level(&type_name[open + 1..type_name.len() - 1], ',');
    Some((base, args))
}

/// A single identifier-shaped type token (`User`, `string`, `#0`).
fn is_bare_type_reference(type_name: &str) -> bool {
    !type_name.is_empty()
        && type_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '#' || c == '$')
}

/// Calculate similarity between union types
fn calculate_union_type_similarity(type1: &str, type2: &str) -> f64 {
    let union1 = split_top_level(type1, '|');
    let union2 = split_top_level(type2, '|');

    let common = union1.iter().filter(|arm| union2.contains(arm)).count();

    if union1.is_empty() && union2.is_empty() {
        1.0
    } else {
        // Squash partial overlaps: a union that shares two of three arms
        // still admits values the other rejects, which is a contract
        // difference — the raw Dice coefficient overstated it.
        let overlap = (common * 2) as f64 / (union1.len() + union2.len()) as f64;
        if overlap >= 1.0 {
            1.0
        } else {
            overlap * 0.6
        }
    }
}

/// Calculate similarity between intersection types
fn calculate_intersection_type_similarity(type1: &str, type2: &str) -> f64 {
    let intersection1 = split_top_level(type1, '&');
    let intersection2 = split_top_level(type2, '&');

    let common = intersection1.iter().filter(|arm| intersection2.contains(arm)).count();

    if intersection1.is_empty() && intersection2.is_empty() {
        1.0
    } else {
        (common * 2) as f64 / (intersection1.len() + intersection2.len()) as f64
    }
}

#[derive(Debug, Clone)]
pub struct PropertyMatch {
    pub prop1: String,
    pub prop2: String,
    pub name_similarity: f64,
    pub type_similarity: f64,
    pub overall_similarity: f64,
}

/// Find the best property matches between two normalized types.
///
/// Two phases:
///   1. exact-name matches, scored by type similarity;
///   2. leftover properties pair up when their normalized TYPES are
///      identical — that's the consistently-renamed-property case
///      (`street/city/zip` vs `line/town/postal`, all `string`), which
///      exact-name matching scored as zero overlap.
///
/// Optionality disagreement discounts a match's quality: `x: T` and
/// `x?: T` are different contracts.
pub fn find_property_matches(
    type1: &NormalizedType,
    type2: &NormalizedType,
    _threshold: f64, // Keep for API compatibility but not used
) -> Vec<PropertyMatch> {
    let mut matches = Vec::new();

    let optionality_factor = |name1: &String, name2: &String| -> f64 {
        let optional1 = type1.optional_properties.contains(name1);
        let optional2 = type2.optional_properties.contains(name2);
        if optional1 == optional2 {
            1.0
        } else {
            0.5
        }
    };

    let mut matched1: HashSet<&String> = HashSet::new();
    let mut matched2: HashSet<&String> = HashSet::new();

    // Phase 1: exact property names.
    let mut names1: Vec<&String> = type1.properties.keys().collect();
    names1.sort();
    for prop1 in names1 {
        let type1_annotation = &type1.properties[prop1];
        if let Some(type2_annotation) = type2.properties.get(prop1) {
            let type_similarity = calculate_type_similarity(type1_annotation, type2_annotation);
            let overall_similarity = type_similarity * optionality_factor(prop1, prop1);
            matched1.insert(prop1);
            matched2.insert(prop1);
            matches.push(PropertyMatch {
                prop1: prop1.clone(),
                prop2: prop1.clone(),
                name_similarity: 1.0,
                type_similarity,
                overall_similarity,
            });
        }
    }

    // Phase 2: renamed properties — identical normalized types among the
    // leftovers, greedily paired in sorted order for determinism. The
    // 0.95 factor keeps a full-rename match slightly below an exact-name
    // match of the same shape.
    //
    // Index signatures are not renameable properties: `[index: string]:
    // boolean` admits every string key while `enabled: boolean` admits
    // exactly one, and `[index: number]` is a different key domain than
    // `[index: string]` even when the value types agree. So index
    // signatures never enter the rename-tolerant phase at all — two index
    // signatures with the same key type share their extractor-assigned
    // name and already matched in phase 1.
    let is_index_signature = |name: &str| name.starts_with('[');
    let mut leftovers1: Vec<&String> =
        type1.properties.keys().filter(|name| !matched1.contains(*name)).collect();
    let mut leftovers2: Vec<&String> =
        type2.properties.keys().filter(|name| !matched2.contains(*name)).collect();
    leftovers1.sort();
    leftovers2.sort();

    for prop1 in leftovers1 {
        if is_index_signature(prop1) {
            continue;
        }
        let type1_annotation = &type1.properties[prop1];
        let mut best: Option<(&String, f64)> = None;
        for prop2 in &leftovers2 {
            if matched2.contains(*prop2) {
                continue;
            }
            if is_index_signature(prop2) {
                continue;
            }
            let type2_annotation = &type2.properties[*prop2];
            if type1_annotation == type2_annotation {
                let name_similarity = calculate_property_similarity(prop1, prop2);
                if best.is_none_or(|(_, current)| name_similarity > current) {
                    best = Some((prop2, name_similarity));
                }
            }
        }
        if let Some((prop2, name_similarity)) = best {
            matched2.insert(prop2);
            let overall_similarity = 0.95 * optionality_factor(prop1, prop2);
            matches.push(PropertyMatch {
                prop1: prop1.clone(),
                prop2: prop2.clone(),
                name_similarity,
                type_similarity: 1.0,
                overall_similarity,
            });
        }
    }

    // Sort by overall similarity (descending), NaN-safe.
    matches.sort_by(|a, b| {
        b.overall_similarity
            .total_cmp(&a.overall_similarity)
            .then_with(|| a.prop1.cmp(&b.prop1))
    });

    matches
}

/// Calculate Levenshtein distance between two strings
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    // Initialize first row and column
    for (i, row) in matrix.iter_mut().enumerate().take(len1 + 1) {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate().take(len2 + 1) {
        *cell = j;
    }

    let chars1: Vec<char> = s1.chars().collect();
    let chars2: Vec<char> = s2.chars().collect();

    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = if chars1[i - 1] == chars2[j - 1] { 0 } else { 1 };

            matrix[i][j] = (matrix[i - 1][j] + 1) // deletion
                .min(matrix[i][j - 1] + 1) // insertion
                .min(matrix[i - 1][j - 1] + cost); // substitution
        }
    }

    matrix[len1][len2]
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
    fn test_normalize_type() {
        let type_def = create_test_type(
            "User",
            vec![
                ("id", "string", false, false),
                ("name", "string", false, false),
                ("age", "number", true, false),
                ("email", "string", false, true),
            ],
        );

        let options = NormalizationOptions::default();
        let normalized = normalize_type(&type_def, &options);

        assert_eq!(normalized.original_name, "User");
        assert_eq!(normalized.properties.len(), 4);
        assert_eq!(normalized.optional_properties.len(), 1); // "age" is optional, and ignore_optional_modifiers is false by default
        assert!(normalized.readonly_properties.is_empty()); // ignore_readonly_modifiers is true by default
    }

    #[test]
    fn test_normalize_type_name() {
        assert_eq!(normalize_type_name("String"), "string");
        assert_eq!(normalize_type_name("Array<string>"), "string[]");
        assert_eq!(normalize_type_name("Array<number>"), "number[]");
        assert_eq!(normalize_type_name("number | string"), "number | string");
        assert_eq!(normalize_type_name("string | number"), "number | string"); // sorted
    }

    #[test]
    fn test_calculate_property_similarity() {
        assert_eq!(calculate_property_similarity("name", "name"), 1.0);
        assert_eq!(calculate_property_similarity("name", "Name"), 0.95);
        assert!(calculate_property_similarity("name", "fullName") > 0.0);
        assert!(calculate_property_similarity("name", "fullName") < 1.0);
    }

    #[test]
    fn test_calculate_type_similarity() {
        assert_eq!(calculate_type_similarity("string", "string"), 1.0);
        assert_eq!(calculate_type_similarity("String", "string"), 1.0);
        assert!(calculate_type_similarity("string", "number") < 1.0);
    }

    #[test]
    fn test_union_type_similarity() {
        assert_eq!(calculate_union_type_similarity("string | number", "number | string"), 1.0);
        assert!(calculate_union_type_similarity("string | number", "string | boolean") > 0.0);
        assert!(calculate_union_type_similarity("string | number", "string | boolean") < 1.0);
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "ab"), 1);
        assert_eq!(levenshtein_distance("abc", "def"), 3);
    }

    #[test]
    fn array_types_compare_by_element() {
        // `ShopUser[]` vs `ShopOrder[]` is the nominal payload contrast,
        // not a Levenshtein near-match of the bracketed spellings.
        assert_eq!(calculate_type_similarity("ShopUser[]", "ShopOrder[]"), 0.2);
        assert_eq!(calculate_type_similarity("ShopUser[]", "ShopUser[]"), 1.0);
        // Array vs non-array is a shape mismatch outright.
        assert_eq!(calculate_type_similarity("string[]", "string"), 0.2);
        assert_eq!(calculate_type_similarity("Set<string>", "string[]"), 0.2);
    }

    #[test]
    fn permuted_generic_arguments_are_a_swapped_contract() {
        // Key/value swap admits (and returns) entirely different values.
        assert_eq!(
            calculate_type_similarity("Map<string, number>", "Map<number, string>"),
            0.25
        );
        // Identical argument lists still compare as equal…
        assert_eq!(
            calculate_type_similarity("Map<string, number>", "Map<string, number>"),
            1.0
        );
        // …and non-permutation argument differences keep the graded
        // same-container score.
        let graded = calculate_type_similarity("Map<string, number>", "Map<string, boolean>");
        assert!(graded > 0.25 && graded < 1.0, "got {graded}");
    }

    #[test]
    fn index_signatures_never_pair_with_concrete_properties() {
        // XT-N12 shape: an open string-keyed map vs a single concrete
        // boolean member. The rename-tolerant phase must not unify them.
        let flags = create_test_type("FeatureFlags", vec![("[index: string]", "boolean", false, false)]);
        let toggle = create_test_type("FeatureToggle", vec![("enabled", "boolean", false, false)]);
        let options = NormalizationOptions::default();
        let matches = find_property_matches(
            &normalize_type(&flags, &options),
            &normalize_type(&toggle, &options),
            0.7,
        );
        assert!(matches.is_empty(), "index signature must not match a concrete property");

        // Two identically-keyed index signatures still match (phase 1).
        let toggles = create_test_type("ToggleMap", vec![("[index: string]", "boolean", false, false)]);
        let matches = find_property_matches(
            &normalize_type(&flags, &options),
            &normalize_type(&toggles, &options),
            0.7,
        );
        assert_eq!(matches.len(), 1);

        // Different key domains are different contracts even when the
        // value types agree: `[index: string]` admits every string key,
        // `[index: number]` only numeric ones — the rename-tolerant
        // phase must not unify them either.
        let numeric_keys =
            create_test_type("SlotMap", vec![("[index: number]", "boolean", false, false)]);
        let matches = find_property_matches(
            &normalize_type(&flags, &options),
            &normalize_type(&numeric_keys, &options),
            0.7,
        );
        assert!(matches.is_empty(), "string-keyed and number-keyed index signatures must not pair");
    }

    #[test]
    fn split_top_level_ignores_arrow_operators() {
        assert_eq!(
            split_top_level("(a) => b, c", ','),
            vec!["(a) => b".to_string(), "c".to_string()]
        );
        assert_eq!(
            split_top_level("(() => User) | null", '|'),
            vec!["(() => User)".to_string(), "null".to_string()]
        );
    }

    #[test]
    fn function_types_with_arrows_normalize_argument_lists() {
        // The `>` in `=>` must not desynchronize bracket depth: both
        // arguments of the generic are still seen.
        let normalized = normalize_type_name("Result<() => string, Error>");
        assert_eq!(normalized, "Result<() => string, Error>");
    }
}
