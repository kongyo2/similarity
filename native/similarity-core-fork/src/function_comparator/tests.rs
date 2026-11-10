//! Function extraction, fragment and scan tests.

use super::*;
use crate::function_atoms::AtomSets;
use crate::function_extractor::{FunctionType, MethodKind};
use crate::function_fragment::build_normalized_fragment;

#[test]
fn test_extract_functions() {
    let code = r"
            function add(a: number, b: number): number {
                return a + b;
            }
            
            const multiply = (x: number, y: number) => x * y;
            
            class Calculator {
                constructor(private initial: number) {}
                
                add(value: number): number {
                    return this.initial + value;
                }
                
                subtract(value: number): number {
                    return this.initial - value;
                }
            }
            
            export function divide(a: number, b: number): number {
                return a / b;
            }
        ";

    let functions = extract_functions("test.ts", code).unwrap();

    assert_eq!(functions.len(), 6);

    // Check function names
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"add"));
    assert!(names.contains(&"multiply"));
    assert!(names.contains(&"constructor"));
    assert!(names.contains(&"subtract"));
    assert!(names.contains(&"divide"));

    // Check function types
    let add_func = functions.iter().find(|f| f.name == "add" && f.class_name.is_none()).unwrap();
    assert_eq!(add_func.function_type, FunctionType::Function);
    assert_eq!(add_func.parameters, vec!["a", "b"]);

    let multiply_func = functions.iter().find(|f| f.name == "multiply").unwrap();
    assert_eq!(multiply_func.function_type, FunctionType::Arrow);

    let constructor = functions.iter().find(|f| f.name == "constructor").unwrap();
    assert_eq!(constructor.function_type, FunctionType::Constructor);
    assert_eq!(constructor.class_name, Some("Calculator".to_string()));
}

#[test]
fn min_tokens_gate_uses_comparison_tree_size() {
    let code = r"
            function tinyA() {
                return 42;
            }

            function tinyB() {
                return 42;
            }
        ";
    let mut options = TSEDOptions { min_lines: 1, size_penalty: false, ..Default::default() };

    // Without a token gate the identical pair is reported…
    let (pairs, _) = find_similar_functions_in_files(
        &[("test.ts".to_string(), code.to_string())],
        0.9,
        &options,
        PairScope::SameFileOnly,
    );
    assert_eq!(pairs.len(), 1);

    // …and a high min_tokens threshold filters it out.
    options.min_tokens = Some(500);
    let (pairs, _) = find_similar_functions_in_files(
        &[("test.ts".to_string(), code.to_string())],
        0.9,
        &options,
        PairScope::SameFileOnly,
    );
    assert!(pairs.is_empty());
}

#[test]
fn test_find_similar_functions_in_file() {
    let code = r"
            function calculateSum(a: number, b: number): number {
                return a + b;
            }
            
            function addNumbers(x: number, y: number): number {
                return x + y;
            }
            
            function multiply(a: number, b: number): number {
                return a * b;
            }
            
            function computeSum(first: number, second: number): number {
                return first + second;
            }
        ";

    let mut options = TSEDOptions::default();
    options.apted_options.rename_cost = 0.3; // Lower rename cost for better similarity detection
    options.size_penalty = false; // Disable for test with small functions
    options.min_lines = 1; // Allow small functions in test

    let similar_pairs = find_similar_functions_in_file("test.ts", code, 0.7, &options).unwrap();

    // Should find that calculateSum, addNumbers, and computeSum are similar
    assert!(
        similar_pairs.len() >= 2,
        "Expected at least 2 similar pairs, found {}",
        similar_pairs.len()
    );

    // Note: multiply IS similar to others because they all have the same structure
    // (two parameters, single return statement). This is expected behavior.
    // Let's check that we found the expected similar pairs
    let sum_pairs = similar_pairs
        .iter()
        .filter(|result| {
            (result.func1.name.contains("Sum") || result.func2.name.contains("Sum"))
                || (result.func1.name == "addNumbers" || result.func2.name == "addNumbers")
        })
        .count();
    assert!(sum_pairs >= 3, "Expected at least 3 pairs involving sum functions");
}

#[test]
fn test_find_similar_functions_across_files() {
    let file1 = (
        "file1.ts".to_string(),
        r#"
            export function processUser(user: User): void {
                validateUser(user);
                saveUser(user);
                notifyUser(user);
            }
            
            function validateUser(user: User): boolean {
                return user.name.length > 0 && user.email.includes('@');
            }
        "#
        .to_string(),
    );

    let file2 = (
        "file2.ts".to_string(),
        r#"
            export function handleUser(u: User): void {
                checkUser(u);
                storeUser(u);
                alertUser(u);
            }
            
            function checkUser(u: User): boolean {
                return u.name.length > 0 && u.email.includes('@');
            }
        "#
        .to_string(),
    );

    let mut options = TSEDOptions::default();
    options.apted_options.rename_cost = 0.3;
    options.size_penalty = false; // Disable for test with small functions
    options.min_lines = 1; // Allow small functions in test

    let similar_pairs = find_similar_functions_across_files(&[file1, file2], 0.7, &options);

    // validateUser/checkUser are genuine rename duplicates (same free
    // atoms, renamed locals) and must be reported.
    let validate_check = similar_pairs.iter().find(|(_, result, _)| {
        (result.func1.name == "validateUser" && result.func2.name == "checkUser")
            || (result.func1.name == "checkUser" && result.func2.name == "validateUser")
    });
    assert!(validate_check.is_some());

    // processUser/handleUser share only the three-call skeleton — the
    // callees are entirely different sets, which the behavioral-atom
    // cap treats as different work (mirroring the corpus's skeleton
    // negatives), so the pair stays below the 0.7 threshold.
    let process_handle = similar_pairs.iter().find(|(_, result, _)| {
        (result.func1.name == "processUser" && result.func2.name == "handleUser")
            || (result.func1.name == "handleUser" && result.func2.name == "processUser")
    });
    assert!(process_handle.is_none());
}

#[test]
fn boundary_index_twins_stay_distinct() {
    // XF-N15 shape: `.at(-1)` picks the newest entry, `.at(0)` the
    // oldest — one literal apart, but reading different elements.
    let code = r#"
export function newestSnapshot(snapshots: string[]): string {
  const picked = snapshots.at(-1);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}

export function oldestSnapshot(snapshots: string[]): string {
  const picked = snapshots.at(0);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "boundary-index twins must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn slice_arity_twins_stay_distinct() {
    // XF-N17 shape: `slice(0, cut)` keeps the head that `slice(cut)`
    // drops — complementary halves from the same call shape.
    let code = r"
export function takeTopJobs(jobs: string[], cut: number): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice(0, cut);
}

export function dropTopJobs(jobs: string[], cut: number): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice(cut);
}
";
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "slice head/tail twins must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn fold_direction_twins_stay_distinct() {
    // XF-N40 shape: appending builds a/b/c/ while prepending builds
    // c/b/a/ — string + is not commutative.
    let code = r#"
export function buildTrailForward(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = trail + segment + "/";
  }
  return trail;
}

export function buildTrailReversed(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = segment + "/" + trail;
  }
  return trail;
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "append vs prepend folds must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn splice_inserted_value_twins_stay_duplicates() {
    // Only `start` and `deleteCount` are boundary positions on
    // `splice`; arguments from position 2 onward are inserted VALUES,
    // and twins differing only there are parameterizable data-literal
    // duplicates.
    let code = r"
export function resetLeadingFlag(flags: number[], cursor: number): number[] {
  if (flags.length === 0) {
    return flags;
  }
  flags.splice(cursor, 1, 0);
  return flags;
}

export function raiseLeadingFlag(flags: number[], cursor: number): number[] {
  if (flags.length === 0) {
    return flags;
  }
  flags.splice(cursor, 1, 1);
  return flags;
}
";
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(
        pairs.len(),
        1,
        "splice twins differing only in the inserted value must stay reportable"
    );
}

#[test]
fn member_target_fold_direction_twins_stay_distinct() {
    // `this.trail` / `state.trail` accumulators carry the same
    // append-vs-prepend behavioral difference as local ones.
    let code = r#"
export function extendAuditTrail(state: { trail: string }, segments: string[]): void {
  for (const segment of segments) {
    state.trail = state.trail + segment + "/";
  }
}

export function rewindAuditTrail(state: { trail: string }, segments: string[]): void {
  for (const segment of segments) {
    state.trail = segment + "/" + state.trail;
  }
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "member-target append vs prepend folds must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn boundary_atoms_count_occurrences() {
    // One of TWO `.at()` reads changes boundary: `at[0]=0` survives on
    // both sides through the other call site, so set semantics would
    // misread the change as a one-sided addition. Occurrence counting
    // must flag it as a substitution and keep the pair distinct.
    let code = r#"
export function firstOfBoth(alpha: string[], beta: string[]): string {
  const left = alpha.at(0) ?? "none";
  const right = beta.at(0) ?? "none";
  return left + ":" + right;
}

export function firstAndLast(alpha: string[], beta: string[]): string {
  const left = alpha.at(0) ?? "none";
  const right = beta.at(-1) ?? "none";
  return left + ":" + right;
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "a substituted boundary among repeated calls must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn bracket_notation_boundary_twins_stay_distinct() {
    // `xs["at"](…)` is the same property access as `xs.at(…)` spelled
    // with brackets — boundary atoms must fire for it too.
    let code = r#"
export function newestEntry(entries: string[]): string {
  const picked = entries["at"](-1);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}

export function oldestEntry(entries: string[]): string {
  const picked = entries["at"](0);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "bracket-notation boundary twins must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn fold_atoms_count_occurrences() {
    // Two accumulators: both append on one side; append + prepend on
    // the other. The shared `fold:head` must not mask the flipped
    // second accumulator.
    let code = r#"
export function buildBothForward(items: string[]): [string, string] {
  let names = "";
  let codes = "";
  for (const item of items) {
    names = names + item + ",";
    codes = codes + item + ";";
  }
  return [names, codes];
}

export function buildSecondReversed(items: string[]): [string, string] {
  let names = "";
  let codes = "";
  for (const item of items) {
    names = names + item + ",";
    codes = item + ";" + codes;
  }
  return [names, codes];
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "a flipped second accumulator must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn omitted_zero_defaults_are_not_boundary_twins() {
    // `xs.slice()` and `xs.slice(0)` copy the same array — one side
    // spelling the default argument must not trip the boundary cap.
    let code = r"
export function cloneQueue(jobs: string[]): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice();
}

export function copyQueue(tasks: string[]): string[] {
  if (tasks.length === 0) {
    return [];
  }
  return tasks.slice(0);
}
";
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1, "slice() vs slice(0) twins must stay reportable duplicates");
}

#[test]
fn substring_boundary_literals_are_position_insensitive() {
    // JavaScript swaps substring's arguments after clamping, so
    // `s.substring(0, cut)` and `s.substring(cut, 0)` select the same
    // range and must not be capped apart.
    let code = r#"
export function headOfLabel(label: string, cut: number): string {
  if (label.length === 0) {
    return "";
  }
  return label.substring(0, cut);
}

export function labelPrefix(title: string, cut: number): string {
  if (title.length === 0) {
    return "";
  }
  return title.substring(cut, 0);
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(
        pairs.len(),
        1,
        "substring(0, cut) vs substring(cut, 0) must stay reportable duplicates"
    );
}

#[test]
fn extra_boundary_arguments_are_ignored_like_javascript() {
    // `xs.at(0, traceId)` reads the same element as `xs.at(0)` — the
    // ignored extra argument must not change the recorded arity.
    let code = r#"
export function firstJob(jobs: string[], traceId: string): string {
  const picked = jobs.at(0, traceId);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}

export function headJob(tasks: string[], traceId: string): string {
  const picked = tasks.at(0);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1, "at(0) vs at(0, ignored) twins must stay reportable duplicates");
}

#[test]
fn string_initialized_locals_prove_fold_direction() {
    // No literal in the chain, but `let trail = ""` proves the
    // accumulator is a string — append vs prepend must stay distinct.
    let code = r#"
export function joinForward(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = trail + segment;
  }
  return trail;
}

export function joinReversed(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = segment + trail;
  }
  return trail;
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "string-initialized fold reversals must stay below threshold, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn trailing_undefined_boundary_defaults_are_omitted_args() {
    // `xs.slice(0, undefined)` copies to the end exactly like
    // `xs.slice(0)` — the explicit default must not trip the cap.
    let code = r"
export function copyBatch(jobs: string[]): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice(0);
}

export function cloneBatch(tasks: string[]): string[] {
  if (tasks.length === 0) {
    return [];
  }
  return tasks.slice(0, undefined);
}
";
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(
        pairs.len(),
        1,
        "slice(0) vs slice(0, undefined) twins must stay reportable duplicates"
    );
}

#[test]
fn negative_substring_literals_clamp_to_zero() {
    // JavaScript clamps substring's negative arguments to 0, so
    // `substring(-1, cut)` selects the same text as `substring(0, cut)`.
    let code = r#"
export function frontOfCode(code: string, cut: number): string {
  if (code.length === 0) {
    return "";
  }
  return code.substring(-1, cut);
}

export function codePrefix(text: string, cut: number): string {
  if (text.length === 0) {
    return "";
  }
  return text.substring(0, cut);
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(
        pairs.len(),
        1,
        "substring(-1, cut) vs substring(0, cut) twins must stay reportable duplicates"
    );
}

#[test]
fn folds_without_string_evidence_get_no_direction_atoms() {
    // `+` over numbers is commutative: `sum += n` vs `sum = n + sum`
    // is not a directional rewrite the analyzer can prove, so without
    // a string literal in the chain the fold-direction atoms must
    // stay silent (the pair keeps whatever score the tree distance
    // gives it, uncapped). With a string literal in the chain the
    // direction IS provable and the atoms fire.
    use crate::parser::parse_and_convert_to_tree_canonical;

    let fold_atoms = |source: &str| -> Vec<String> {
        let tree = parse_and_convert_to_tree_canonical("probe.ts", source).unwrap();
        AtomSets::of(&tree)
            .synthetic_atom_names()
            .into_iter()
            .filter(|atom| atom.starts_with("fold:"))
            .collect()
    };

    let numeric_append =
        "function f(xs: number[]) { let sum = 0; for (const x of xs) { sum += x; } return sum; }";
    let numeric_prepend =
        "function f(xs: number[]) { let sum = 0; for (const x of xs) { sum = x + sum; } return sum; }";
    assert!(fold_atoms(numeric_append).is_empty(), "numeric append must not emit fold atoms");
    assert!(fold_atoms(numeric_prepend).is_empty(), "numeric prepend must not emit fold atoms");

    let string_append = r#"function f(xs: string[]) { let out = ""; for (const x of xs) { out = out + x + "/"; } return out; }"#;
    let string_prepend = r#"function f(xs: string[]) { let out = ""; for (const x of xs) { out = x + "/" + out; } return out; }"#;
    assert_eq!(fold_atoms(string_append), vec!["fold:head".to_string()]);
    assert_eq!(fold_atoms(string_prepend), vec!["fold:tail".to_string()]);
}

#[test]
fn same_direction_folds_still_match() {
    // Control for the fold-direction atom: two append folds with
    // renamed identifiers mark the same accumulator position, so the
    // new atoms must not push a true rename-duplicate below
    // threshold.
    let code = r#"
export function joinSegments(pieces: string[]): string {
  let joined = "";
  for (const piece of pieces) {
    joined += piece + "/";
  }
  return joined;
}

export function gluePath(parts: string[]): string {
  let glued = "";
  for (const part of parts) {
    glued += part + "/";
  }
  return glued;
}
"#;
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1, "same-direction folds must still report as duplicates");
}

#[test]
fn test_extract_functions_marks_similarity_ignore_directives() {
    let code = r#"
function keepMe() {
    return 1;
}

// similarity-ignore
function ignoreMe() {
    return 2;
}

/**
 * Keep this duplicated export local for now.
 */
// similarity-ignore
export function ignoredExport() {
    return 3;
}
"#;

    let functions = extract_functions("test.ts", code).unwrap();

    let keep = functions.iter().find(|f| f.name == "keepMe").unwrap();
    assert!(!keep.has_ignore_directive);

    let ignored = functions.iter().find(|f| f.name == "ignoreMe").unwrap();
    assert!(ignored.has_ignore_directive);

    let ignored_export = functions.iter().find(|f| f.name == "ignoredExport").unwrap();
    assert!(ignored_export.has_ignore_directive);
}

/// `(name, has_ignore_directive)` of every extracted function.
fn ignored_flags(code: &str) -> Vec<(String, bool)> {
    extract_functions("test.ts", code)
        .unwrap()
        .into_iter()
        .map(|function| (function.name, function.has_ignore_directive))
        .collect()
}

fn flag(name: &str, ignored: bool) -> (String, bool) {
    (name.to_string(), ignored)
}

#[test]
fn a_directive_above_a_statement_covers_what_the_statement_declares() {
    // The arrow starts lines below the comment: the directive used to be
    // looked for right above the arrow's own line only.
    let code = r"
// similarity-ignore
export const handler: Handler<APIGatewayProxyEvent, APIGatewayProxyResult> =
    async (event) => {
        return respond(event);
    };

// similarity-ignore
const table = {
    first(row: Row) { return row.id; },
    second: (row: Row) => row.name,
    nested: { third(row: Row) { return row.total; } },
};

export const kept = async (event: Event) => {
    return respond(event);
};
";
    assert_eq!(
        ignored_flags(code),
        [
            flag("handler", true),
            flag("table.first", true),
            flag("table.second", true),
            flag("table.nested.third", true),
            flag("kept", false),
        ]
    );
}

#[test]
fn a_directive_covers_everything_declared_inside() {
    let code = r"
// similarity-ignore
@Injectable()
export class Legacy {
    load(id: string) { return this.rows.get(id); }
    save = (row: Row) => { this.rows.set(row.id, row); };
}

@Injectable()
export class Current {
    load(id: string) { return this.rows.get(id); }
    // similarity-ignore
    generated(id: string) { return this.rows.get(id); }
    after(id: string) { return this.rows.get(id); }
}

// similarity-ignore
namespace Generated {
    export function parse(text: string) {
        const helper = (line: string) => line.trim();
        return text.split('\n').map(helper);
    }
}

// similarity-ignore
function outer(rows: Row[]) {
    function inner(row: Row) { return row.id; }
    return rows.map(inner);
}

const handlers = {
    // similarity-ignore
    generated(row: Row) { return row.id; },
    written(row: Row) { return row.id; },
};
";
    assert_eq!(
        ignored_flags(code),
        [
            // The directive sits above the decorator of the class.
            flag("load", true),
            flag("save", true),
            // A directive above one member is that member's alone.
            flag("load", false),
            flag("generated", true),
            flag("after", false),
            flag("parse", true),
            flag("helper", true),
            flag("outer", true),
            flag("inner", true),
            flag("handlers.generated", true),
            flag("handlers.written", false),
        ]
    );
}

#[test]
fn ignored_functions_are_not_reported() {
    let body = r"{
    const names = [];
    for (const row of rows) {
        if (row.id > 0 && row.name.length > 0) {
            names.push(row.name.trim().toLowerCase());
        }
    }
    return names;
}";
    let options = TSEDOptions::default();
    let reported =
        |code: &str| find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap().len();
    assert_eq!(
        reported(&format!(
            "function first(rows: Row[]) {body}\n\nfunction second(rows: Row[]) {body}\n"
        )),
        1
    );
    assert_eq!(
        reported(&format!(
            "function first(rows: Row[]) {body}\n\n// similarity-ignore\nfunction second(rows: Row[]) {body}\n"
        )),
        0
    );
    // The method is ignored through its class.
    assert_eq!(
        reported(&format!(
            "function first(rows: Row[]) {body}\n\n// similarity-ignore\nclass Legacy {{\n    second(rows: Row[]) {body}\n}}\n"
        )),
        0
    );
}

#[test]
fn functions_know_the_form_their_source_takes() {
    use crate::function_extractor::SourceForm;
    let code = r"
function declared(a: number) { return a; }
const arrow = (a: number) => a;
const expression = function (a: number) { return a; };
const named = function recur(a: number): number { return a > 0 ? recur(a - 1) : a; };
export default function (a: number) { return a; }
class Box {
    method(a: number) { return a; }
    field = (a: number) => a;
    get size() { return 1; }
}
const table = {
    member(a: number) { return a; },
    valued: function (a: number) { return a; },
    arrowed: (a: number) => a,
};
";
    let functions = extract_functions("test.ts", code).unwrap();
    let forms: Vec<(&str, SourceForm, &str)> = functions
        .iter()
        .map(|function| {
            (
                function.name.as_str(),
                function.form,
                &code[function.span.start as usize..function.span.end as usize],
            )
        })
        .collect();
    assert_eq!(
        forms,
        [
            ("declared", SourceForm::Statement, "function declared(a: number) { return a; }"),
            ("arrow", SourceForm::Expression, "(a: number) => a"),
            ("expression", SourceForm::Expression, "function (a: number) { return a; }"),
            (
                "named",
                SourceForm::Statement,
                "function recur(a: number): number { return a > 0 ? recur(a - 1) : a; }"
            ),
            ("default", SourceForm::Expression, "function (a: number) { return a; }"),
            ("method", SourceForm::ClassMember, "method(a: number) { return a; }"),
            ("field", SourceForm::Expression, "(a: number) => a"),
            ("size", SourceForm::ClassMember, "get size() { return 1; }"),
            ("table.member", SourceForm::ObjectMember, "member(a: number) { return a; }"),
            (
                "table.valued",
                SourceForm::ObjectMember,
                "valued: function (a: number) { return a; }"
            ),
            ("table.arrowed", SourceForm::Expression, "(a: number) => a"),
        ]
    );
    // Each form parses on its own once wrapped.
    for function in &functions {
        let standalone = crate::function_fragment::standalone_source(function, code).unwrap();
        assert!(
            crate::parser::parse_and_convert_to_tree("test.ts", &standalone.text).is_ok(),
            "{}",
            standalone.text
        );
        let offset = standalone.offset as usize;
        assert_eq!(
            &standalone.text[offset..offset + function.span.size() as usize],
            &code[function.span.start as usize..function.span.end as usize]
        );
    }
    // An arrow field is reported at its field, the arrow alone is compared.
    let field = &functions[6];
    assert_eq!((field.start_line, field.end_line), (9, 9));
}

#[test]
fn bodiless_declarations_are_not_extracted() {
    let code = r"
export function visit(node: Node | undefined, test: (node: Node) => boolean): Node;
export function visit(node: Node | undefined, test?: (node: Node) => boolean): Node | undefined;
export function visit(node: Node | undefined, test?: (node: Node) => boolean): Node | undefined {
    return node;
}
declare function readBuffer(path: string): string;
export abstract class Store {
    abstract find(id: string): Promise<string>;
    remove(id: string): Promise<string> {
        return Promise.resolve(id);
    }
}
";
    let functions = extract_functions("test.ts", code).unwrap();
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["visit", "remove"]);
}

#[test]
fn function_expressions_namespaces_and_arrow_fields_are_extracted() {
    let code = r"
export const parse = function (line: string): string {
    return line.trim();
};
export namespace Legacy {
    export function normalize(raw: string): string {
        return raw.trim();
    }
    export namespace Inner {
        export function deep(raw: string): string {
            return raw.trim();
        }
    }
}
export class Relay {
    handle = (event: string): void => {
        console.log(event);
    };
    static tick = (): number => 1;
}
";
    let functions = extract_functions("test.ts", code).unwrap();
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["parse", "normalize", "deep", "handle", "tick"]);
    assert_eq!(functions[0].function_type, FunctionType::Function);
    let handle = functions.iter().find(|f| f.name == "handle").unwrap();
    assert_eq!(handle.function_type, FunctionType::Method);
    assert_eq!(handle.class_name.as_deref(), Some("Relay"));
    assert!(!handle.is_arrow_expression);
    let tick = functions.iter().find(|f| f.name == "tick").unwrap();
    assert!(tick.is_static);
    assert!(tick.is_arrow_expression);
}

#[test]
fn arrow_field_and_method_twins_are_reported() {
    // XF-P45 shape.
    let code = r"
export class ClickRelay {
    handle = (event: string): void => {
        if (!event) {
            return;
        }
        this.sink(event.trim());
    };
}
export class TapRelay {
    handle(event: string): void {
        if (!event) {
            return;
        }
        this.sink(event.trim());
    }
}
";
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1, "arrow field vs method must report one pair");
    assert!((pairs[0].similarity - 1.0).abs() < 1e-9, "got {}", pairs[0].similarity);
}

#[test]
fn named_function_expressions_keep_their_self_binding() {
    // `const factorial = function recur(n) { … recur(n - 1) … }`: the
    // fragment used to be rebuilt as `function factorial`, leaving
    // `recur` a free identifier that differed from the twin's `go`.
    let code = r"
export const factorial = function recur(n: number): number {
    if (n <= 1) {
        return 1;
    }
    return n * recur(n - 1);
};
export const fact = function go(n: number): number {
    if (n <= 1) {
        return 1;
    }
    return n * go(n - 1);
};
";
    let functions = extract_functions("test.ts", code).unwrap();
    assert_eq!(functions[0].name, "factorial");
    assert_eq!(functions[0].display_name, "recur");
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1, "recursive twins must pair");
    assert!((pairs[0].similarity - 1.0).abs() < 1e-9, "got {}", pairs[0].similarity);
}

#[test]
fn default_exports_take_part() {
    let code = r"
export default class Relay {
    handle(event: string): void {
        if (!event) {
            return;
        }
        this.sink(event.trim());
    }
}
export class TapRelay {
    handle(event: string): void {
        if (!event) {
            return;
        }
        this.sink(event.trim());
    }
}
";
    let functions = extract_functions("test.ts", code).unwrap();
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["handle", "handle"]);
    assert_eq!(functions[0].class_name.as_deref(), Some("Relay"));
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1, "default-exported class methods must pair");

    let expressions = extract_functions(
        "test.ts",
        "export default function (line: string): string {\n    return line.trim();\n}\n",
    )
    .unwrap();
    assert_eq!(expressions[0].name, "default");
    assert_eq!(
        build_normalized_fragment(
            &expressions[0],
            "export default function (line: string): string {\n    return line.trim();\n}\n"
        )
        .text,
        "function __f_default(line: string) {\n    return line.trim();\n}"
    );
    let arrows =
        extract_functions("test.ts", "export default (line: string) => line.trim();\n").unwrap();
    assert_eq!(arrows[0].name, "default");
    assert!(arrows[0].is_arrow_expression);
}

#[test]
fn non_identifier_member_keys_are_named_by_source() {
    let code = r#"
export class Mapper {
    "alpha"(value: number): number { return value + 1; }
    [Symbol.iterator]() { return this; }
    0() { return 0; }
    #hidden() { return 1; }
    ["dyn" + "amic"] = (): void => {};
}
"#;
    let functions = extract_functions("test.ts", code).unwrap();
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "[Symbol.iterator]", "0", "#hidden", "[\"dyn\" + \"amic\"]"]);
    let display: Vec<&str> = functions.iter().map(|f| f.display_name.as_str()).collect();
    assert_eq!(
        display,
        vec!["\"alpha\"", "[Symbol.iterator]", "0", "#hidden", "[\"dyn\" + \"amic\"]"]
    );
}

#[test]
fn overload_signatures_never_pair() {
    // XF-N41 shape: identical (type-less) parameter lists across
    // overload signatures used to score 1.0 against each other.
    let code = r"
export function visitNode<TIn extends Node | undefined, TOut extends Node>(
    node: TIn,
    visitor: Visitor,
    test: (node: Node) => node is TOut,
): TOut | (TIn & undefined);
export function visitNode<TIn extends Node | undefined>(
    node: TIn,
    visitor: Visitor,
    test?: (node: Node) => boolean,
): Node | (TIn & undefined);
export function visitNode(node: Node | undefined, visitor: Visitor, test?: (node: Node) => boolean): Node | undefined {
    if (node === undefined) {
        return node;
    }
    return visitor(node);
}
";
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "overload signatures must not pair, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );
}

#[test]
fn named_function_expressions_recurse_through_either_binding() {
    // `const factorial = function recur(n) { … factorial(n - 1) … }`
    // recurses through the VARIABLE binding: rebuilding the fragment
    // under `recur` left `factorial` a free identifier that differed
    // from the twin's `fact`, so the pair scored 0.75.
    let code = r"
export const factorial = function recur(n: number): number {
    if (n <= 1) {
        return 1;
    }
    const rest = factorial(n - 1);
    return n * rest;
};
export const fact = function go(n: number): number {
    if (n <= 1) {
        return 1;
    }
    const rest = fact(n - 1);
    return n * rest;
};
export function product(n: number): number {
    if (n <= 1) {
        return 1;
    }
    const rest = product(n - 1);
    return n * rest;
}
";
    let functions = extract_functions("test.ts", code).unwrap();
    assert_eq!(functions[0].self_alias.as_deref(), Some("factorial"));
    assert_eq!(functions[2].self_alias, None);
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 3, "all three recursive twins must pair");
    assert!(pairs.iter().all(|p| (p.similarity - 1.0).abs() < 1e-9));

    // A local that shadows the alias is the local, not the function.
    let shadowed = r"
export const walk = function step(n: number): number {
    const walk = (k: number) => k * 2;
    if (n <= 1) {
        return 1;
    }
    return walk(n - 1);
};
export const visit = function hop(n: number): number {
    const visit = (k: number) => k * 2;
    if (n <= 1) {
        return 1;
    }
    return visit(n - 1);
};
";
    let pairs = find_similar_functions_in_file("test.ts", shadowed, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1);
}

#[test]
fn transparent_wrappers_do_not_hide_function_initializers() {
    let code = r"
export const parse = (function (line: string): string[] {
    return line.trim().split(',');
});
export const handle = ((line: string): string[] => line.trim().split(',')) satisfies Splitter;
export const cast = ((line: string) => line.trim().split(',')) as Splitter;
export const asserted = <Splitter>((line: string) => line.trim().split(','));
export const forced = ((line: string) => line.trim().split(','))!;
export default ((line: string) => line.trim().split(','));
";
    let functions = extract_functions("test.ts", code).unwrap();
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["parse", "handle", "cast", "asserted", "forced", "default"]);
}

#[test]
fn object_literal_members_are_extracted() {
    let code = r#"
export const handlers = {
    onSave(doc: Doc): void {
        if (!doc.dirty) return;
        store.write(doc.id, doc.body);
    },
    onClose: (doc: Doc): void => {
        store.release(doc.id);
    },
    get size(): number {
        return store.count();
    },
    "on-reset": function (doc: Doc): void {
        doc.body = "";
    },
    [Kind.Block]: (data: Data) => visit(data.statements),
    nested: { flush(): void { store.flush(); } },
    plain: 42,
    shorthand,
};
export default {
    transform(code: string): string {
        return code.trim();
    },
};
function factory() {
    return { inner(): void { work(); } };
}
"#;
    let functions = extract_functions("test.ts", code).unwrap();
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "handlers.onSave",
            "handlers.onClose",
            "handlers.size",
            "handlers.on-reset",
            "handlers[Kind.Block]",
            "handlers.nested.flush",
            "default.transform",
            "factory",
        ],
        "objects bound to a name are scanned; a returned object stays part of its builder"
    );
    assert_eq!(functions[0].function_type, FunctionType::Method);
    assert_eq!(functions[1].function_type, FunctionType::Arrow);
    assert_eq!(functions[2].method_kind, MethodKind::Getter);

    // An object method and its standalone twin compare as equal.
    let twins = r"
export const handlers = {
    onSave(doc: Doc): void {
        if (!doc.dirty) return;
        doc.validate();
        store.write(doc.id, doc.body);
    },
};
export function persist(document: Doc): void {
    if (!document.dirty) return;
    document.validate();
    store.write(document.id, document.body);
}
";
    let options = TSEDOptions::default();
    let pairs = find_similar_functions_in_file("test.ts", twins, 0.8, &options).unwrap();
    assert_eq!(pairs.len(), 1);
    assert!((pairs[0].similarity - 1.0).abs() < 1e-9, "got {}", pairs[0].similarity);
}

#[test]
fn object_members_recurse_through_their_own_name() {
    // `visit: function walk(node) {… walk(node.left) …}` recurses
    // through the function expression's own name, which the property
    // key does not bind. Identifier keys (function-shaped fragment),
    // reserved-word keys and computed keys (class-wrapped fragment) all
    // resolve it.
    let options = TSEDOptions::default();
    for (left_key, right_key) in
        [("visit", "visit"), ("delete", "delete"), ("[Kind.Sum]", "[Kind.Sum]")]
    {
        let code = format!(
            r"
export const visitorA = {{
    {left_key}: function walk(node: TreeNode): number {{
        if (node.leaf) {{
            return node.value;
        }}
        const left = walk(node.left);
        const right = walk(node.right);
        return left + right;
    }},
}};
export const visitorB = {{
    {right_key}: function traverse(node: TreeNode): number {{
        if (node.leaf) {{
            return node.value;
        }}
        const left = traverse(node.left);
        const right = traverse(node.right);
        return left + right;
    }},
}};
"
        );
        let functions = extract_functions("test.ts", &code).unwrap();
        assert_eq!(functions[0].self_alias.as_deref(), Some("walk"));
        let pairs = find_similar_functions_in_file("test.ts", &code, 0.8, &options).unwrap();
        assert_eq!(pairs.len(), 1, "{left_key}: recursive object members must pair");
        assert!(
            (pairs[0].similarity - 1.0).abs() < 1e-9,
            "{left_key}: got {}",
            pairs[0].similarity
        );
    }

    // The own name resolves to the function only where it is the own
    // name: a twin calling an outer `walk` helper is not recursion, so
    // the two bodies are not the same tree.
    let outer = r"
export const visitorA = {
    visit: function walk(node: TreeNode): number {
        if (node.leaf) {
            return node.value;
        }
        const left = walk(node.left);
        const right = walk(node.right);
        return left + right;
    },
};
export const visitorB = {
    visit: function traverse(node: TreeNode): number {
        if (node.leaf) {
            return node.value;
        }
        const left = walk(node.left);
        const right = walk(node.right);
        return left + right;
    },
};
";
    let functions = extract_functions("test.ts", outer).unwrap();
    let similarity =
        compare_functions(&functions[0], &functions[1], outer, outer, &options).unwrap();
    assert!(similarity < 1.0 - 1e-9, "recursion vs an outer helper, got {similarity}");

    // Nor is a call spelled like the KEY: the key binds nothing in the
    // body, so `visit(…)` reaches an outer `visit` and must not resolve
    // to the function along with the own name.
    let template = r"
export const visitorA = {
    visit: function walk(node: TreeNode): number {
        if (node.leaf) {
            return node.value;
        }
        const left = walk(node.left);
        const right = LEFT_RIGHT(node.right);
        return left + right;
    },
};
export const visitorB = {
    visit: function traverse(node: TreeNode): number {
        if (node.leaf) {
            return node.value;
        }
        const left = RIGHT_LEFT(node.left);
        const right = traverse(node.right);
        return left + right;
    },
};
";
    for (left_call, right_call, case) in [
        // Recursion on the left, an outer `visit` on the right.
        ("walk", "visit", "own name vs key"),
        // One body calls itself and the outer `visit`, the other
        // calls itself twice.
        ("visit", "traverse", "key and own name vs own name twice"),
    ] {
        let code = template.replace("LEFT_RIGHT", left_call).replace("RIGHT_LEFT", right_call);
        let functions = extract_functions("test.ts", &code).unwrap();
        let similarity =
            compare_functions(&functions[0], &functions[1], &code, &code, &options).unwrap();
        assert!(similarity < 1.0 - 1e-9, "{case}: got {similarity}");
    }
}

#[test]
fn member_keys_bind_nothing_inside_the_body() {
    let options = TSEDOptions::default();

    // Methods delegating to a helper named like themselves call
    // different helpers: a facade over two sinks is not one function
    // twice.
    let facade = r"
export const logger = {
    debug(message: string, ...args: unknown[]): void {
        if (!settings.enabled) {
            return;
        }
        const line = format(message, args);
        debug(`${settings.prefix} ${line}`);
    },
    info(message: string, ...args: unknown[]): void {
        if (!settings.enabled) {
            return;
        }
        const line = format(message, args);
        info(`${settings.prefix} ${line}`);
    },
};
";
    let pairs = find_similar_functions_in_file("test.ts", facade, 0.8, &options).unwrap();
    assert!(
        pairs.is_empty(),
        "different sinks must not pair, got {:?}",
        pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
    );

    // Every member spelling delegating to the outer `visit` is the same
    // code: shorthand, named function expression, arrow value, class
    // method. A recursive `function visit` is different code.
    let delegating = r"
export const shorthand = {
    visit(node: TreeNode): number {
        if (node.leaf) {
            return node.value;
        }
        const left = visit(node.left);
        return left + node.value;
    },
};
export const named = {
    visit: function walk(node: TreeNode): number {
        if (node.leaf) {
            return node.value;
        }
        const left = visit(node.left);
        return left + node.value;
    },
};
export const arrow = {
    visit: (node: TreeNode): number => {
        if (node.leaf) {
            return node.value;
        }
        const left = visit(node.left);
        return left + node.value;
    },
};
export class Visitor {
    visit(node: TreeNode): number {
        if (node.leaf) {
            return node.value;
        }
        const left = visit(node.left);
        return left + node.value;
    }
}
export function visit(node: TreeNode): number {
    if (node.leaf) {
        return node.value;
    }
    const left = visit(node.left);
    return left + node.value;
}
";
    let functions = extract_functions("test.ts", delegating).unwrap();
    assert_eq!(functions.len(), 5);
    let (members, recursive) = functions.split_at(4);
    for (i, left) in members.iter().enumerate() {
        for right in &members[i + 1..] {
            let similarity =
                compare_functions(left, right, delegating, delegating, &options).unwrap();
            assert!(
                (similarity - 1.0).abs() < 1e-9,
                "{} vs {}: got {similarity}",
                left.name,
                right.name
            );
        }
        let similarity =
            compare_functions(left, &recursive[0], delegating, delegating, &options).unwrap();
        assert!(similarity < 1.0 - 1e-9, "{} vs recursion: got {similarity}", left.name);
    }
}

#[test]
fn synthetic_fragment_names_never_capture_source_references() {
    let options = TSEDOptions::default();
    let similarity_of_first_and_last = |code: &str| {
        let functions = extract_functions("test.ts", code).unwrap();
        let last = functions.last().unwrap();
        compare_functions(&functions[0], last, code, code, &options).unwrap()
    };
    let body = |callee: &str| {
        format!(
            "(n: number): number {{
        if (n <= 0) {{
            return 0;
        }}
        const rest = {callee}(n - 1);
        return rest + 1;
    }}"
        )
    };
    let recursive = format!("export function countdown{}\n", body("countdown"));

    // A call to an outer function spelled like the synthetic name the
    // fragment is declared under (the method placeholder, the name an
    // arrow-valued member gets) is not recursion.
    let method = format!("export const a = {{\n    countdown{},\n}};\n{recursive}", body("__m__"));
    let similarity = similarity_of_first_and_last(&method);
    assert!(similarity < 1.0 - 1e-9, "method calling __m__: got {similarity}");

    let arrow = format!(
        "export const a = {{\n    countdown: (n: number): number => {{
        if (n <= 0) {{
            return 0;
        }}
        const rest = __sim__(n - 1);
        return rest + 1;
    }},\n}};\n{recursive}"
    );
    let similarity = similarity_of_first_and_last(&arrow);
    assert!(similarity < 1.0 - 1e-9, "arrow member calling __sim__: got {similarity}");

    // Class-wrapped: a member calling an outer `__C__` is not the twin
    // that recurses through its own name.
    let wrapped = format!(
        "export const a = {{\n    [Kind.Sum]: function walk{},\n}};
export const b = {{\n    [Kind.Sum]: function step{},\n}};\n",
        body("walk"),
        body("__C__")
    );
    let similarity = similarity_of_first_and_last(&wrapped);
    assert!(similarity < 1.0 - 1e-9, "member calling __C__: got {similarity}");

    // When the source spells the placeholder, the fragment takes
    // another name, and the own name still resolves to it.
    let spelled = r#"
export const a = {
    visit: function walk(n: number): string {
        if (n <= 0) {
            return "__m__";
        }
        const rest = walk(n - 1);
        return rest + "!";
    },
};
export const b = {
    visit: function step(n: number): string {
        if (n <= 0) {
            return "__m__";
        }
        const rest = step(n - 1);
        return rest + "!";
    },
};
"#;
    let functions = extract_functions("test.ts", spelled).unwrap();
    let fragment = build_normalized_fragment(&functions[0], spelled);
    assert_eq!(fragment.binding, "__m__1");
    assert!(fragment.text.starts_with("function __m__1("), "{}", fragment.text);
    let similarity = similarity_of_first_and_last(spelled);
    assert!((similarity - 1.0).abs() < 1e-9, "own-name recursion: got {similarity}");
}

#[test]
fn async_and_generator_contracts_cap_identical_bodies() {
    let options = TSEDOptions::default();
    let compare = |code: &str| {
        let functions = extract_functions("test.ts", code).unwrap();
        compare_functions(&functions[0], &functions[1], code, code, &options).unwrap()
    };
    // The sync and async API siblings: same body, different contract.
    let async_vs_sync = compare(
        r#"
export async function getSymbol(declaration: Declaration): Promise<Symbol> {
    const api = getNodeAPI(declaration);
    if (!api) throw new Error("Source file not found for declaration");
    return api.getSymbol(declaration);
}
export function getSymbolSync(declaration: Declaration): Symbol {
    const api = getNodeAPI(declaration);
    if (!api) throw new Error("Source file not found for declaration");
    return api.getSymbol(declaration);
}
"#,
    );
    assert!(async_vs_sync < 0.8, "got {async_vs_sync}");

    // Static methods and accessors keep the class wrapper, whose
    // method label never carried `async` at all.
    let static_methods = compare(
        r#"
export class Loader {
    static async load(path: string): Promise<string> {
        const text = readFileSync(path, "utf8");
        if (text.length === 0) throw new Error("empty");
        return text.trim();
    }
    static loadSync(path: string): string {
        const text = readFileSync(path, "utf8");
        if (text.length === 0) throw new Error("empty");
        return text.trim();
    }
}
"#,
    );
    assert!(static_methods < 0.8, "got {static_methods}");

    // A sync function chaining the same promise is the async twin.
    let then_chain = compare(
        r#"
export function loadOrder(id: string): Promise<unknown> {
    return fetch("/api/orders/" + id).then((response) => {
        if (!response.ok) throw new Error("HTTP " + response.status);
        return response.json();
    });
}
export async function readOrder(id: string): Promise<unknown> {
    const response = await fetch("/api/orders/" + id);
    if (!response.ok) throw new Error("HTTP " + response.status);
    return response.json();
}
"#,
    );
    assert!(then_chain >= 0.8, "got {then_chain}");

    // A sync function that declares a promise, or returns one it
    // builds, has the async twin's contract.
    let declared = compare(
        r#"
export function loadUser(id: string): Promise<User> {
    const url = "/api/users/" + id;
    log("load", url);
    return fetch(url).json();
}
export async function loadUserAsync(id: string): Promise<User> {
    const url = "/api/users/" + id;
    log("load", url);
    return fetch(url).json();
}
"#,
    );
    assert!(declared >= 0.8, "got {declared}");
    let built = compare(
        r#"
export function settle(result: Result) {
    const value = normalize(result);
    record(value);
    return Promise.resolve(value);
}
export async function settleAsync(result: Result) {
    const value = normalize(result);
    record(value);
    return Promise.resolve(value);
}
"#,
    );
    assert!(built >= 0.8, "got {built}");

    // An await inside a nested callback is the callback's own.
    let nested = compare(
        r#"
export function startAll(jobs: Job[]): Promise<void>[] {
    const started = jobs.filter((job) => job.ready);
    return started.map(async (job) => {
        await job.run();
    });
}
export async function startAllAsync(jobs: Job[]): Promise<Promise<void>[]> {
    const started = jobs.filter((job) => job.ready);
    return started.map(async (job) => {
        await job.run();
    });
}
"#,
    );
    assert!(nested < 0.8, "got {nested}");
}
