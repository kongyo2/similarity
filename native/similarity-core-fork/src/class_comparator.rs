//! Class comparison: member-by-member structural similarity of two class
//! shapes, and the pairwise scan.

use std::collections::{HashMap, HashSet};

use crate::class_extractor::{ClassDefinition, ClassMethod, ClassProperty};
use crate::module_scope::unqualified_name;
use crate::text::levenshtein_similarity;
use crate::type_normalizer::split_top_level;

/// A class prepared for comparison: its members keyed for lookup, with the
/// spellings the matching passes compare computed once.
#[derive(Debug, Clone)]
pub struct NormalizedClass {
    pub name: String,
    /// Members keyed by [`member_key`]: a static member and an instance
    /// member may share a name, and keying by the bare name let one
    /// silently overwrite the other.
    pub properties: HashMap<String, NormalizedProperty>,
    pub methods: HashMap<String, NormalizedMethod>,
    pub constructor_signature: String,
    pub constructor_wiring: Vec<(String, String)>,
    pub extends: Option<String>,
    pub implements: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NormalizedProperty {
    pub property: ClassProperty,
    /// The annotation with class-shaped names replaced by a placeholder
    /// (see [`replace_camelcase_identifiers`]).
    rename_tolerant_type: String,
}

#[derive(Debug, Clone)]
pub struct NormalizedMethod {
    pub method: ClassMethod,
    /// `(params) => return type`, as written.
    signature: String,
    /// The signature without parameter names and class-shaped names (see
    /// [`rename_tolerant_signature`]).
    rename_tolerant_signature: String,
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

/// Map key of a class member: the bare name for instance members,
/// `static <name>` for static ones.
fn member_key(name: &str, is_static: bool) -> String {
    if is_static {
        format!("static {name}")
    } else {
        name.to_string()
    }
}

/// The same member name on the other runtime surface (`static x` ⇔ `x`).
fn opposite_surface_key(key: &str) -> String {
    match key.strip_prefix("static ") {
        Some(name) => name.to_string(),
        None => format!("static {key}"),
    }
}

#[must_use]
pub fn normalize_class(class: &ClassDefinition) -> NormalizedClass {
    // Member annotations arrive normalized by the extractor (type mode's
    // renderer plus `normalize_type_name`).
    let properties = class
        .properties
        .iter()
        .map(|property| {
            (
                member_key(&property.name, property.is_static),
                NormalizedProperty {
                    rename_tolerant_type: replace_camelcase_identifiers(&property.type_annotation),
                    property: property.clone(),
                },
            )
        })
        .collect();

    let methods = class
        .methods
        .iter()
        .map(|method| {
            (
                member_key(&method.name, method.is_static),
                NormalizedMethod {
                    signature: format!(
                        "({}) => {}",
                        method.parameters.join(", "),
                        method.return_type
                    ),
                    rename_tolerant_signature: rename_tolerant_signature(method),
                    method: method.clone(),
                },
            )
        })
        .collect();

    let constructor_signature = if class.constructor_params.is_empty() {
        "()".to_string()
    } else {
        format!("({})", class.constructor_params.join(", "))
    };

    // `implements A, B` and `implements B, A` promise the same contract.
    let mut implements = class.implements.clone();
    implements.sort();
    implements.dedup();

    NormalizedClass {
        name: class.name.clone(),
        properties,
        methods,
        constructor_signature,
        constructor_wiring: class.constructor_wiring.clone(),
        extends: class.extends.clone(),
        implements,
    }
}

/// Strip parameter names and class-shaped tokens so two methods that have
/// the same structural signature but sit in renamed classes can still be
/// recognised as matching.
///
/// `ClassMethod.parameters` is populated by the class extractor as a
/// one-element `Vec<String>` containing the whole comma-separated
/// parameter list (e.g. `["a: A, b: B"]`). Splitting it requires
/// brackets-aware handling because parameter types legitimately contain
/// commas (`Map<string, number>`, tuples `[a, b]`, object literals,
/// function types `(a, b) => c`).
///
/// Class-shaped tokens (CamelCase identifiers) get a placeholder both in
/// parameter types AND in the return type — a method whose body
/// references the surrounding class also exposes that class in its
/// parameter signature, and we want renamed-class methods to match on
/// either side.
fn rename_tolerant_signature(method: &ClassMethod) -> String {
    let joined = method.parameters.join(", ");
    let stripped_params: Vec<String> = if joined.trim().is_empty() {
        Vec::new()
    } else {
        split_top_level(&joined, ',')
            .iter()
            .filter(|piece| !piece.is_empty())
            .map(|piece| {
                // Drop the parameter NAME but keep what it said about the
                // parameter: `x?: T` is optional and `...xs: T[]` is the
                // rest parameter, both part of the call contract.
                let (head, without_name) = match piece.find(':') {
                    Some(idx) => (piece[..idx].trim(), piece[idx + 1..].trim()),
                    None => ("", piece.as_str()),
                };
                let marker = if head.starts_with("...") {
                    "..."
                } else if head.ends_with('?') {
                    "?"
                } else {
                    ""
                };
                format!("{marker}{}", replace_camelcase_identifiers(without_name))
            })
            .collect()
    };
    let normalized_return = replace_camelcase_identifiers(&method.return_type);
    format!("({}) => {}", stripped_params.join(", "), normalized_return)
}

/// Replace every class-shaped (CamelCase) identifier with one placeholder,
/// so `repo: InvoiceRepo` and `repo: ReceiptRepo` — the same dependency
/// slot in two renamed class families — spell the same.
fn replace_camelcase_identifiers(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let is_token_char = |ch: char| ch.is_ascii_alphabetic() || ch == '_' || ch.is_ascii_digit();
    // A `.` joining two path segments, as opposed to a spread `...`.
    let is_qualifier_dot = |position: usize| {
        chars.get(position) == Some(&'.')
            && (position == 0 || chars[position - 1] != '.')
            && chars.get(position + 1) != Some(&'.')
    };
    let mut out = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
        if !is_token_char(chars[index]) {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && is_token_char(chars[index]) {
            index += 1;
        }
        let token: String = chars[start..index].iter().collect();
        // Segments of a qualified path keep their spelling: an enum member
        // (`SyntaxKind.Block` vs `SyntaxKind.Identifier`) or a namespace
        // (`React.ComponentType` vs `Preact.ComponentType`) is an identity,
        // not a class renamed along with its family, and placeholdering
        // every segment made all qualified references compare equal.
        let qualified = (start > 0 && is_qualifier_dot(start - 1)) || is_qualifier_dot(index);
        // Primitive type names (string, number, boolean, void, …) are
        // lowercase and stay intact.
        let is_camel_class = token.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && token.chars().any(|c| c.is_ascii_lowercase());
        if is_camel_class && !qualified {
            out.push_str("#T");
        } else {
            out.push_str(&token);
        }
    }
    out
}

/// Rough Jaccard on alphabetic tokens. Good enough to give a small bonus
/// when two normalised signatures share a few tokens but aren't identical
/// (e.g. partially overlapping parameter types).
#[allow(clippy::cast_precision_loss)]
fn normalized_signature_overlap(sig1: &str, sig2: &str) -> f64 {
    let tokens = |signature: &'_ str| -> HashSet<String> {
        signature
            .split(|c: char| !c.is_ascii_alphabetic())
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect()
    };
    let (tokens1, tokens2) = (tokens(sig1), tokens(sig2));
    if tokens1.is_empty() && tokens2.is_empty() {
        return 1.0;
    }
    let intersection = tokens1.intersection(&tokens2).count();
    let union = tokens1.union(&tokens2).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

#[must_use]
pub fn compare_classes(
    class1: &ClassDefinition,
    class2: &ClassDefinition,
) -> ClassComparisonResult {
    compare_normalized_classes(&normalize_class(class1), &normalize_class(class2))
}

/// [`compare_classes`] for classes that are already normalized — a scan
/// normalizes each class once, not once per pair it takes part in.
#[must_use]
pub fn compare_normalized_classes(
    class1: &NormalizedClass,
    class2: &NormalizedClass,
) -> ClassComparisonResult {
    // Namespace qualifiers stay out of name similarity: `Legacy.Pager` vs
    // `Pager` is the same name, and two unrelated classes in the same
    // namespace must not read as lexically related because they share its
    // prefix.
    let naming_similarity =
        calculate_name_similarity(unqualified_name(&class1.name), unqualified_name(&class2.name));

    let (structural_similarity, differences) = calculate_structural_similarity(class1, class2);

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
    levenshtein_similarity(name1, name2)
}

/// Multiplier applied to a matched method pair's credit reflecting how
/// much the two methods actually agree beyond their names/signatures:
///
/// * canonical body fingerprints disagree → bodies compute different
///   things (`0.55`) — signatures alone used to hide this entirely;
/// * `static` mismatch → different runtime surface (`0.4`);
/// * accessor-kind mismatch (getter/setter vs plain method) → different
///   call contract (`0.5`);
/// * optionality mismatch (`f?(): T` vs `f(): T`) → one side may lack the
///   method (`0.5`, the type comparator's optionality discount).
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
    if method1.is_optional != method2.is_optional {
        factor *= 0.5;
    }
    factor
}

/// Discount for a matched property pair that lives on different runtime
/// surfaces (`static` vs instance, `0.4` — the method pass applies the same
/// rule through its agreement factor) or disagrees on optionality (`x?: T`
/// vs `x: T`, `0.5` — the type comparator's optionality discount).
fn property_modifier_factor(prop1: &ClassProperty, prop2: &ClassProperty) -> f64 {
    let mut factor = 1.0;
    if prop1.is_static != prop2.is_static {
        factor *= 0.4;
    }
    if prop1.is_optional != prop2.is_optional {
        factor *= 0.5;
    }
    factor
}

/// Keys of `class1`'s members matched by key (pass 1), then by the same
/// name on the other runtime surface (pass 2), as `(key1, key2)` pairs in
/// a deterministic order. Everything left over goes to the fuzzy passes.
fn match_member_keys<'a, T>(
    members1: &'a HashMap<String, T>,
    members2: &'a HashMap<String, T>,
) -> Vec<(&'a str, &'a str)> {
    let mut keys1: Vec<&String> = members1.keys().collect();
    keys1.sort();
    let mut matched2: HashSet<&str> = HashSet::new();
    let mut pairs = Vec::new();
    let mut leftovers = Vec::new();
    for key in keys1 {
        if let Some((key2, _)) = members2.get_key_value(key) {
            matched2.insert(key2.as_str());
            pairs.push((key.as_str(), key2.as_str()));
        } else {
            leftovers.push(key);
        }
    }
    for key in leftovers {
        let opposite = opposite_surface_key(key);
        if let Some((key2, _)) = members2.get_key_value(&opposite) {
            if matched2.insert(key2.as_str()) {
                pairs.push((key.as_str(), key2.as_str()));
            }
        }
    }
    pairs
}

/// The keys of `members` that are not in `matched`, sorted. Sorting makes
/// the greedy fuzzy passes produce the same pairings on every run (hash
/// map iteration order is randomised).
fn sorted_leftovers<'a, T>(
    members: &'a HashMap<String, T>,
    matched: &HashSet<&str>,
) -> Vec<&'a str> {
    let mut leftovers: Vec<&str> =
        members.keys().map(String::as_str).filter(|key| !matched.contains(key)).collect();
    leftovers.sort_unstable();
    leftovers
}

/// Constructor-wiring positions on which the two classes disagree: the
/// same parameter stored into fields the property pass did not pair, or
/// into different `super(…)` slots. Counted only when both constructors
/// wire something — a class without a constructor is not "missing" one.
fn wiring_mismatches(
    wiring1: &[(String, String)],
    wiring2: &[(String, String)],
    field_pairing: &HashMap<String, String>,
) -> usize {
    if wiring1.is_empty() || wiring2.is_empty() {
        return 0;
    }
    let targets = |wiring: &[(String, String)]| -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (parameter, target) in wiring {
            map.entry(parameter.clone()).or_default().push(target.clone());
        }
        for list in map.values_mut() {
            list.sort();
        }
        map
    };
    let targets1 = targets(wiring1);
    let targets2 = targets(wiring2);
    targets1
        .iter()
        .filter(|(parameter, list1)| {
            // A parameter only one constructor wires is an extra (or
            // missing) parameter or field, which the member tally already
            // counts; only a parameter both sides store somewhere
            // DIFFERENT is miswired.
            let Some(list2) = targets2.get(*parameter) else {
                return false;
            };
            let mut translated: Vec<String> = list1
                .iter()
                .map(|target| {
                    if target.starts_with("super[") {
                        target.clone()
                    } else {
                        field_pairing.get(target).cloned().unwrap_or_else(|| target.clone())
                    }
                })
                .collect();
            translated.sort();
            translated != *list2
        })
        .count()
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn calculate_structural_similarity(
    class1: &NormalizedClass,
    class2: &NormalizedClass,
) -> (f64, ClassDifferences) {
    let mut missing_properties = Vec::new();
    let mut extra_properties = Vec::new();
    let mut property_type_mismatches = Vec::new();
    // class1 field → class2 field, for the constructor-wiring check.
    let mut field_pairing: HashMap<String, String> = HashMap::new();

    // Properties: two-phase matching.
    //
    // Phase 1 — strict by name. A property keeps its name across most
    // refactors so this is the primary signal. Members are keyed by name
    // and runtime surface; a same-named static/instance pair still matches
    // here, at the surface discount.
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
    let mut matched_in_class1: HashSet<&str> = HashSet::new();
    let mut matched_in_class2: HashSet<&str> = HashSet::new();

    for (key1, key2) in match_member_keys(&class1.properties, &class2.properties) {
        let normalized1 = &class1.properties[key1];
        let normalized2 = &class2.properties[key2];
        let (prop1, prop2) = (&normalized1.property, &normalized2.property);
        let modifier_factor = property_modifier_factor(prop1, prop2);
        // Class-reference types tolerate renames: `repo: InvoiceRepo`
        // and `repo: ReceiptRepo` are the same dependency slot in two
        // renamed class families (the same placeholdering the fuzzy
        // method matcher applies to signatures). Primitive types are
        // untouched — `count: number` vs `count: string` stays a
        // genuine mismatch.
        if prop1.type_annotation == prop2.type_annotation
            || normalized1.rename_tolerant_type == normalized2.rename_tolerant_type
        {
            // Strict match — credit both class1 and class2 sides.
            property_score += 2.0 * modifier_factor;
        } else {
            // Same name, different type — partial match.
            property_score += 1.4 * modifier_factor;
            property_type_mismatches.push(PropertyMismatch {
                name: prop1.name.clone(),
                type1: prop1.type_annotation.clone(),
                type2: prop2.type_annotation.clone(),
            });
        }
        if !prop1.is_static && !prop2.is_static {
            field_pairing.insert(prop1.name.clone(), prop2.name.clone());
        }
        matched_in_class1.insert(key1);
        matched_in_class2.insert(key2);
    }

    let leftovers_class1 = sorted_leftovers(&class1.properties, &matched_in_class1);
    let leftovers_class2 = sorted_leftovers(&class2.properties, &matched_in_class2);

    // Greedy best-match by type for the leftovers. This is N×M but class
    // properties are typically a handful so the quadratic factor is fine.
    let mut leftover_consumed_class2: HashSet<&str> = HashSet::new();
    for name1 in &leftovers_class1 {
        let prop1 = &class1.properties[*name1].property;
        let mut best_match: Option<(&str, f64)> = None;
        for name2 in &leftovers_class2 {
            if leftover_consumed_class2.contains(name2) {
                continue;
            }
            let prop2 = &class2.properties[*name2].property;
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
                let max_len = prop1
                    .type_annotation
                    .chars()
                    .count()
                    .max(prop2.type_annotation.chars().count())
                    .max(1) as f64;
                (shared_prefix as f64 / max_len).min(0.6)
            } else {
                0.4
            };
            let name_sim = calculate_name_similarity(&prop1.name, &prop2.name);
            // Same runtime-surface and optionality rules as the strict
            // pass: a static field is not a rename of an instance field.
            let score =
                (0.8 * type_match + 0.2 * name_sim) * property_modifier_factor(prop1, prop2);
            if best_match.is_none_or(|(_, b)| score > b) {
                best_match = Some((name2, score));
            }
        }
        if let Some((name2, score)) = best_match {
            if score >= 0.7 {
                // Credit it as a rename match — both sides count.
                property_score += 2.0 * score;
                leftover_consumed_class2.insert(name2);
                let prop2 = &class2.properties[name2].property;
                if !prop1.is_static && !prop2.is_static {
                    field_pairing.insert(prop1.name.clone(), prop2.name.clone());
                }
                continue;
            }
        }
        missing_properties.push((*name1).to_string());
    }
    for name2 in &leftovers_class2 {
        if !leftover_consumed_class2.contains(name2) {
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
    let mut method_matched_in_class1: HashSet<&str> = HashSet::new();
    let mut method_matched_in_class2: HashSet<&str> = HashSet::new();

    for (key1, key2) in match_member_keys(&class1.methods, &class2.methods) {
        let normalized1 = &class1.methods[key1];
        let normalized2 = &class2.methods[key2];
        // Parameter renames (`findById(id)` vs `findById(key)`) are
        // neutral refactors — when the name matches and the
        // name-stripped signatures agree, credit a full match instead
        // of the partial-mismatch rate.
        let agreement = method_agreement_factor(&normalized1.method, &normalized2.method);
        let signatures_agree = normalized1.signature == normalized2.signature
            || normalized1.rename_tolerant_signature == normalized2.rename_tolerant_signature;
        let credit = if signatures_agree { 2.0 } else { 1.4 };
        method_score += credit * agreement;
        if !signatures_agree || agreement < 1.0 {
            method_signature_mismatches.push(MethodMismatch {
                name: normalized1.method.name.clone(),
                signature1: normalized1.signature.clone(),
                signature2: normalized2.signature.clone(),
            });
        }
        method_matched_in_class1.insert(key1);
        method_matched_in_class2.insert(key2);
    }

    let method_leftovers_class1 = sorted_leftovers(&class1.methods, &method_matched_in_class1);
    let method_leftovers_class2 = sorted_leftovers(&class2.methods, &method_matched_in_class2);

    let mut method_leftover_consumed: HashSet<&str> = HashSet::new();
    for name1 in &method_leftovers_class1 {
        let normalized1 = &class1.methods[*name1];
        let method1 = &normalized1.method;
        let mut best_match: Option<(&str, f64)> = None;
        for name2 in &method_leftovers_class2 {
            if method_leftover_consumed.contains(name2) {
                continue;
            }
            let normalized2 = &class2.methods[*name2];
            let method2 = &normalized2.method;
            // Tiered signature match: exact > param-shape-only > nothing.
            // The "param-shape" tier strips identifier names from
            // parameters and the surrounding-class name from the return
            // type, so methods that genuinely have the same shape but
            // sit in renamed classes (UserBuilder.withName vs
            // CustomerBuilder.withLabel) still register as similar.
            let sig_match = if normalized1.signature == normalized2.signature {
                1.0
            } else if normalized1.rename_tolerant_signature == normalized2.rename_tolerant_signature
            {
                0.85
            } else {
                let name_overlap = normalized_signature_overlap(
                    &normalized1.rename_tolerant_signature,
                    &normalized2.rename_tolerant_signature,
                );
                0.4 + 0.4 * name_overlap
            };
            let name_sim = calculate_name_similarity(&method1.name, &method2.name);
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
                let method2 = &class2.methods[name2].method;
                method_score += 2.0 * score * method_agreement_factor(method1, method2);
                method_leftover_consumed.insert(name2);
                continue;
            }
        }
        missing_methods.push((*name1).to_string());
    }
    for name2 in &method_leftovers_class2 {
        if !method_leftover_consumed.contains(name2) {
            extra_methods.push((*name2).to_string());
        }
    }

    // `implements` clauses are declared elements of the class shape: an
    // interface one side promises and the other does not is one edit's
    // worth of difference, so unmatched clauses join the denominator.
    // Matched clauses add nothing — sharing an interface is what unrelated
    // implementations of it do, not evidence of duplication (two one-method
    // `implements I` classes with different bodies must not pass on the
    // strength of the clause).
    let unmatched_implements =
        class1.implements.iter().filter(|name| !class2.implements.contains(name)).count()
            + class2.implements.iter().filter(|name| !class1.implements.contains(name)).count();

    // Constructor wiring: the same fields filled from different parameters
    // (`this.x = y; this.y = x` vs `this.x = x; this.y = y`) initialize
    // differently, and wiring-only constructors are not members, so nothing
    // else would notice. A miswired parameter is wired on both sides, both
    // entries disagree, and it counts like a member present on both sides
    // with nothing matching: two unmatched elements.
    let miswired =
        wiring_mismatches(&class1.constructor_wiring, &class2.constructor_wiring, &field_pairing);

    // Calculate overall structural similarity. Each side counts once, so
    // a "complete match" pair contributes 2 to the score and 2 to the
    // denominator — leaving the ratio at 1.0 for an exact match.
    let member_count = property_total_count + method_total_count;
    let total_elements = member_count + unmatched_implements as f64 + 2.0 * miswired as f64;
    let matched_elements = property_score + method_score;

    let structural_similarity = if total_elements > 0.0 {
        let mut ratio = (matched_elements / total_elements).min(1.0);
        // A single member carries about as much evidence as none: `class
        // Foo { x = 1 }` and `class Bar { x = 1 }` agree on one field and
        // nothing else, so — like the member-less case below — a tiny shape
        // needs naming agreement to clear the default threshold.
        if member_count <= 2.0 {
            ratio = ratio.min(0.85);
        }
        // The base class is a contract the other side lacks or draws from
        // elsewhere (inherited members, `super` dispatch, `instanceof`
        // identity): `class AuditLog extends EventEmitter { … }` is not a
        // duplicate of a standalone class with the same own members, nor
        // of one that extends `Readable`. Only the same base — as written,
        // type arguments included — leaves the score alone.
        if class1.extends != class2.extends {
            ratio *= 0.7;
        }
        ratio
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

/// Compare every pair of classes and return those at or above `threshold`,
/// most similar first. Classes under a `// similarity-ignore` directive
/// take no part.
#[must_use]
pub fn find_similar_classes(classes: &[ClassDefinition], threshold: f64) -> Vec<SimilarClassPair> {
    let candidates: Vec<(&ClassDefinition, NormalizedClass)> = classes
        .iter()
        .filter(|class| !class.has_ignore_directive)
        .map(|class| (class, normalize_class(class)))
        .collect();

    let mut similar_pairs = Vec::new();
    for (i, (class1, normalized1)) in candidates.iter().enumerate() {
        for (class2, normalized2) in &candidates[i + 1..] {
            let result = compare_normalized_classes(normalized1, normalized2);
            if result.similarity >= threshold {
                similar_pairs.push(SimilarClassPair {
                    class1: (*class1).clone(),
                    class2: (*class2).clone(),
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

    #[test]
    fn constructor_work_counts() {
        // XC-N07 shape: same field and method, but one constructor also
        // schedules a refill timer.
        let result = compare_sources(
            r"
export class RetryBudget {
  remaining: number;
  constructor(limit: number) {
    this.remaining = limit;
  }
  spend(): boolean {
    if (this.remaining <= 0) return false;
    this.remaining -= 1;
    return true;
  }
}
",
            r"
export class TokenBucket {
  remaining: number;
  constructor(limit: number, refillMs: number) {
    this.remaining = limit;
    setInterval(() => { this.remaining = limit; }, refillMs);
  }
  spend(): boolean {
    if (this.remaining <= 0) return false;
    this.remaining -= 1;
    return true;
  }
}
",
        );
        assert!(
            result.similarity < 0.8,
            "divergent constructors must stay below threshold, got {}",
            result.similarity
        );
    }

    #[test]
    fn one_sided_heritage_is_discounted() {
        // XC-N08 shape.
        let result = compare_sources(
            r"
export class AuditLog extends EventEmitter {
  private entries: string[] = [];
  record(entry: string): void { this.entries.push(entry); }
  snapshot(): string[] { return [...this.entries]; }
}
",
            r"
export class TraceLog {
  private entries: string[] = [];
  record(entry: string): void { this.entries.push(entry); }
  snapshot(): string[] { return [...this.entries]; }
}
",
        );
        assert!(
            result.similarity < 0.8,
            "one-sided extends must stay below threshold, got {}",
            result.similarity
        );
        let same_base = compare_sources(
            r"
export class AuditLog extends EventEmitter {
  private entries: string[] = [];
  record(entry: string): void { this.entries.push(entry); }
}
",
            r"
export class TraceLog extends EventEmitter {
  private entries: string[] = [];
  record(entry: string): void { this.entries.push(entry); }
}
",
        );
        assert!(
            same_base.similarity >= 0.9,
            "shared heritage must not be penalized, got {}",
            same_base.similarity
        );
    }

    #[test]
    fn heritage_values_are_compared() {
        // Same own members, different bases: different contracts.
        let different_bases = compare_sources(
            r"
export class CsvExporter extends BaseExporter {
  private rows: string[] = [];
  push(row: string): void { this.rows.push(row); }
  flush(): string { return this.rows.join('\n'); }
}
",
            r"
export class JsonExporter extends StreamWriter {
  private rows: string[] = [];
  push(row: string): void { this.rows.push(row); }
  flush(): string { return this.rows.join('\n'); }
}
",
        );
        assert!(
            different_bases.similarity < 0.8,
            "different bases must stay below threshold, got {}",
            different_bases.similarity
        );

        // A qualified base is still a base.
        let qualified = compare_sources(
            r"
export class Widget extends React.Component {
  private rows: string[] = [];
  push(row: string): void { this.rows.push(row); }
  flush(): string { return this.rows.join('\n'); }
}
",
            r"
export class Panel {
  private rows: string[] = [];
  push(row: string): void { this.rows.push(row); }
  flush(): string { return this.rows.join('\n'); }
}
",
        );
        assert!(
            qualified.similarity < 0.8,
            "a qualified base is heritage too, got {}",
            qualified.similarity
        );
    }

    #[test]
    fn implements_clauses_join_the_member_tally() {
        let shared = compare_sources(
            r"
export class FileCache implements Cache, Disposable {
  private items = new Map<string, string>();
  get(key: string): string | undefined { return this.items.get(key); }
  set(key: string, value: string): void { this.items.set(key, value); }
  dispose(): void { this.items.clear(); }
}
",
            r"
export class MemoryStore implements Disposable, Cache {
  private items = new Map<string, string>();
  get(key: string): string | undefined { return this.items.get(key); }
  set(key: string, value: string): void { this.items.set(key, value); }
  dispose(): void { this.items.clear(); }
}
",
        );
        assert!(
            (shared.structural_similarity - 1.0).abs() < 1e-9,
            "the same clauses in any order are a full match, got {}",
            shared.structural_similarity
        );

        let one_sided = compare_sources(
            r"
export class FileCache implements Cache, Disposable {
  private items = new Map<string, string>();
  get(key: string): string | undefined { return this.items.get(key); }
  set(key: string, value: string): void { this.items.set(key, value); }
  dispose(): void { this.items.clear(); }
}
",
            r"
export class MemoryStore {
  private items = new Map<string, string>();
  get(key: string): string | undefined { return this.items.get(key); }
  set(key: string, value: string): void { this.items.set(key, value); }
  dispose(): void { this.items.clear(); }
}
",
        );
        // Four members matched on both sides: 8 of 10 elements.
        assert!(
            (one_sided.structural_similarity - 0.8).abs() < 1e-9,
            "two unmatched clauses are two edits, got {}",
            one_sided.structural_similarity
        );

        // Sharing an interface is not evidence of duplication: two
        // one-method implementations of `I` with different bodies (the
        // conformance suite's `symbolProperty23`/`symbolProperty24`) stay
        // apart.
        let same_interface = compare_sources(
            "export class C implements I { [Symbol.toPrimitive]() { return true; } }",
            "export class C implements I { [Symbol.toPrimitive]() { return ''; } }",
        );
        assert!(
            same_interface.similarity < 0.8,
            "a shared clause must not lift different bodies over the threshold, got {}",
            same_interface.similarity
        );
    }

    #[test]
    fn ignored_classes_are_not_reported() {
        let classes = extract_classes_from_code(
            r"
export class ActiveCache {
  private items = new Map<string, string>();
  get(key: string): string | undefined { return this.items.get(key); }
  set(key: string, value: string): void { this.items.set(key, value); }
}

// similarity-ignore
export class IgnoredCache {
  private items = new Map<string, string>();
  get(key: string): string | undefined { return this.items.get(key); }
  set(key: string, value: string): void { this.items.set(key, value); }
}
",
            "a.ts",
        )
        .unwrap();
        assert!(find_similar_classes(&classes, 0.5).is_empty());
    }

    #[test]
    fn member_types_keep_their_identity() {
        // Literal, enum-member, qualified-generic and payload types used to
        // collapse onto `any`/`unknown`/bare names in class mode.
        let result = compare_sources(
            r#"
export class Alpha {
  status: "active" | "inactive" = "active";
  kind: SyntaxKind.Block;
  render: React.ComponentType<Props>;
  load(): Promise<User> { return this.repo.load(); }
  ids: readonly string[] = [];
}
"#,
            r#"
export class Beta {
  status: "open" | "closed" = "open";
  kind: SyntaxKind.Identifier;
  render: Preact.ComponentType<Props>;
  load(): Promise<Order> { return this.repo.load(); }
  ids: Map<string, number>[] = [];
}
"#,
        );
        assert!(result.similarity < 0.8, "got {}", result.similarity);
        // A renamed class reference in an otherwise equal family still
        // matches (`repo: InvoiceRepo` vs `repo: ReceiptRepo`).
        let family = compare_sources(
            "export class InvoiceService { repo: InvoiceRepo; load(id: string): Invoice { return this.repo.find(id); } total(): number { return 0; } }",
            "export class ReceiptService { repo: ReceiptRepo; load(id: string): Receipt { return this.repo.find(id); } total(): number { return 0; } }",
        );
        assert!(family.structural_similarity > 0.99, "got {}", family.structural_similarity);
    }

    #[test]
    fn generic_heritage_renames_are_neutral() {
        let result = compare_sources(
            r"
export class Alpha<TItem> extends Base<TItem> {
  private items: TItem[] = [];
  add(item: TItem): void { this.items.push(item); }
  all(): TItem[] { return [...this.items]; }
}
",
            r"
export class Beta<U> extends Base<U> {
  private items: U[] = [];
  add(item: U): void { this.items.push(item); }
  all(): U[] { return [...this.items]; }
}
",
        );
        assert!(
            (result.structural_similarity - 1.0).abs() < 1e-9,
            "got {}",
            result.structural_similarity
        );
        // A concrete instantiation is still a different base.
        let concrete = compare_sources(
            "export class A extends Base<string> { add(item: string): void { this.items.push(item); } all(): string[] { return [...this.items]; } }",
            "export class B extends Base<number> { add(item: string): void { this.items.push(item); } all(): string[] { return [...this.items]; } }",
        );
        assert!(concrete.structural_similarity < 0.8, "got {}", concrete.structural_similarity);
    }

    #[test]
    fn reordered_type_parameters_keep_compound_annotations_equal() {
        // `T | U` is `#0 | #1` in `Alpha<T, U>` and `#1 | #0` in
        // `Beta<U, T>` until it is normalized again after substitution.
        let result = compare_sources(
            r"
export class Alpha<T, U> extends Base<T | U> {
  x: T | U;
  y: T & U;
  pick<A, B>(left: A | B, right: T | U): A | B { return left; }
}
",
            r"
export class Beta<U, T> extends Base<T | U> {
  x: T | U;
  y: T & U;
  pick<B, A>(left: A | B, right: T | U): A | B { return left; }
}
",
        );
        assert!(
            (result.structural_similarity - 1.0).abs() < 1e-9,
            "got {}",
            result.structural_similarity
        );
        // Positions still matter: `T` is the first parameter of `Alpha<T, U>`
        // but the second of `Beta<U, T>`, so a field typed `T` alone differs.
        let asymmetric = compare_sources(
            "export class Alpha<T, U> { x: T | U; first: T; }",
            "export class Beta<U, T> { x: T | U; first: T; }",
        );
        assert!(
            asymmetric.structural_similarity < 1.0 - 1e-9,
            "got {}",
            asymmetric.structural_similarity
        );
        // Heritage type arguments are normalized like annotations.
        let heritage = compare_sources(
            "export class A extends Base<string | number> { x: string; y: number; }",
            "export class B extends Base<number | string> { x: string; y: number; }",
        );
        assert!(
            (heritage.structural_similarity - 1.0).abs() < 1e-9,
            "got {}",
            heritage.structural_similarity
        );
    }

    #[test]
    fn swapped_constructor_wiring_is_a_difference() {
        let swapped = r"
export class PointA {
  x: number;
  y: number;
  constructor(x: number, y: number) { this.x = y; this.y = x; }
  sum(): number { return this.x + this.y; }
  scale(k: number): number { return this.x * k; }
}
";
        let straight = r"
export class PointB {
  x: number;
  y: number;
  constructor(x: number, y: number) { this.x = x; this.y = y; }
  sum(): number { return this.x + this.y; }
  scale(k: number): number { return this.x * k; }
}
";
        let shorthand = r"
export class PointC {
  constructor(public x: number, public y: number) {}
  sum(): number { return this.x + this.y; }
  scale(k: number): number { return this.x * k; }
}
";
        assert!(compare_sources(swapped, straight).similarity < 0.8);
        assert!(compare_sources(swapped, shorthand).similarity < 0.8);
        let same = compare_sources(straight, shorthand);
        assert!(
            (same.structural_similarity - 1.0).abs() < 1e-9,
            "got {}",
            same.structural_similarity
        );

        // Forwarding to `super` in a different order is miswiring too.
        let forward = compare_sources(
            "export class NamedA extends Entity { label = ''; constructor(name: string, id: string) { super(id, name); } describe(): string { return this.label; } }",
            "export class NamedB extends Entity { label = ''; constructor(name: string, id: string) { super(name, id); } describe(): string { return this.label; } }",
        );
        assert!(forward.structural_similarity < 0.8, "got {}", forward.structural_similarity);

        // A parameter only one side wires is an extra field, counted once
        // by the member tally, not miswiring.
        let extended = compare_sources(
            "export class Pair { a: string; b: string; constructor(a: string, b: string) { this.a = a; this.b = b; } join(): string { return this.a + this.b; } }",
            "export class Triple { a: string; b: string; c: string; constructor(a: string, b: string, c: string) { this.a = a; this.b = b; this.c = c; } join(): string { return this.a + this.b; } }",
        );
        assert!(
            (extended.structural_similarity - 6.0 / 7.0).abs() < 1e-9,
            "got {}",
            extended.structural_similarity
        );
    }

    #[test]
    fn static_and_instance_members_sharing_a_name_both_count() {
        // Keyed by bare name, the static `current`/`create` overwrote the
        // instance ones and this pair scored structural 1.0.
        let result = compare_sources(
            r"
export class SessionStore {
  static current: SessionStore | null = null;
  current: string | null = null;
  static create(): SessionStore { return new SessionStore(); }
  create(id: string): void { this.current = id; }
  clear(): void { this.current = null; }
}
",
            r"
export class DraftStore {
  current: string | null = null;
  create(id: string): void { this.current = id; }
  clear(): void { this.current = null; }
}
",
        );
        assert!(result.similarity < 0.8, "got {}", result.similarity);
        assert_eq!(result.differences.missing_properties, vec!["static current"]);
        assert_eq!(result.differences.missing_methods, vec!["static create"]);
    }

    #[test]
    fn optional_members_differ_from_required_ones() {
        let result = compare_sources(
            r"
export class DraftStore {
  current: string | null = null;
  create(id: string): void { this.current = id; }
  clear(): void { this.current = null; }
}
",
            r"
export declare class OptionalStore {
  current?: string | null;
  create?(id: string): void;
  clear?(): void;
}
",
        );
        assert!(result.similarity < 0.8, "got {}", result.similarity);
    }

    #[test]
    fn single_member_classes_need_naming_agreement() {
        // XC-N09 shape.
        let unrelated =
            compare_sources("export class Marker { id = 0; }", "export class Slot { id = 0; }");
        assert!(
            unrelated.similarity < 0.8,
            "one shared field is not a duplicate class, got {}",
            unrelated.similarity
        );
        let related = compare_sources(
            "export class FeatureFlag { enabled = false; }",
            "export class FeatureFlagCopy { enabled = false; }",
        );
        assert!(
            related.similarity >= 0.8,
            "matching names plus the same member still pass, got {}",
            related.similarity
        );
    }
}
