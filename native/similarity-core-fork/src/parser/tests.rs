//! Canonicalization tests: which spellings convert to the same tree.

use std::rc::Rc;

use super::{parse_and_convert_to_tree, parse_and_convert_to_tree_canonical};
use crate::apted::{compute_edit_distance, APTEDOptions};
use crate::tree::TreeNode;

fn canonical_tree(source: &str) -> Rc<TreeNode> {
    parse_and_convert_to_tree_canonical("test.ts", source).expect("source must parse")
}

fn structural_options() -> APTEDOptions {
    APTEDOptions { rename_cost: 0.3, compare_values: false, ..Default::default() }
}

fn assert_canonically_equal(source1: &str, source2: &str) {
    let tree1 = canonical_tree(source1);
    let tree2 = canonical_tree(source2);
    let distance = compute_edit_distance(&tree1, &tree2, &structural_options());
    assert!(
        distance == 0.0,
        "expected canonical trees to be identical (distance {distance}):\n--- left ---\n{source1}\n--- right ---\n{source2}"
    );
}

fn assert_canonically_distinct(source1: &str, source2: &str) {
    let tree1 = canonical_tree(source1);
    let tree2 = canonical_tree(source2);
    let distance = compute_edit_distance(&tree1, &tree2, &structural_options());
    assert!(
        distance > 0.0,
        "expected canonical trees to differ but they were identical:\n--- left ---\n{source1}\n--- right ---\n{source2}"
    );
}

#[test]
fn template_literal_equals_string_concatenation() {
    assert_canonically_equal(
        r#"function f(name: string) { return `Hello ${name}!`; }"#,
        r#"function f(name: string) { return "Hello " + name + "!"; }"#,
    );
}

#[test]
fn expression_only_template_equals_plain_string() {
    assert_canonically_equal(
        r#"function f() { const greeting = `plain`; return greeting; }"#,
        r#"function f() { const greeting = "plain"; return greeting; }"#,
    );
}

#[test]
fn adjacent_interpolations_stay_distinct_from_numeric_addition() {
    // `` `${a}${b}` `` stringifies both values ("1" + "2" → "12");
    // `a + b` adds them (3). The trees must not collapse.
    assert_canonically_distinct(
        "function f(a: number, b: number) { const t = `${a}${b}`; return t; }",
        "function f(a: number, b: number) { const t = a + b; return t; }",
    );
}

#[test]
fn adjacent_interpolations_equal_empty_string_headed_concatenation() {
    assert_canonically_equal(
        r#"function f(a: number, b: number) { const t = `${a}${b}`; return t; }"#,
        r#"function f(a: number, b: number) { const t = "" + a + b; return t; }"#,
    );
}

#[test]
fn lone_interpolation_stays_distinct_from_bare_value() {
    assert_canonically_distinct(
        "function f(v: number) { const out = `${v}`; return out; }",
        "function f(v: number) { const out = v; return out; }",
    );
}

#[test]
fn lone_interpolation_equals_empty_string_concatenation() {
    assert_canonically_equal(
        r#"function f(v: number) { const out = `${v}`; return out; }"#,
        r#"function f(v: number) { const out = "" + v; return out; }"#,
    );
}

#[test]
fn leading_interpolation_followed_by_text_still_equals_concatenation() {
    // With a string in the second slot the chain is already guaranteed
    // to concatenate, so no `""` head appears on either side.
    assert_canonically_equal(
        r#"function f(name: string) { return `${name} is ready`; }"#,
        r#"function f(name: string) { return name + " is ready"; }"#,
    );
}

#[test]
fn template_literals_with_different_text_stay_distinct() {
    assert_canonically_distinct(
        r#"function f(name: string) { return `Hello ${name}`; }"#,
        r#"function f(name: string) { return `Goodbye ${name}`; }"#,
    );
}

#[test]
fn expanded_assignment_contracts_onto_compound_form() {
    assert_canonically_equal(
        "function f(x: number, y: number) { x = x + y; return x; }",
        "function f(x: number, y: number) { x += y; return x; }",
    );
}

#[test]
fn member_target_assignment_contracts_too() {
    assert_canonically_equal(
        "class C { total = 0; add(n: number) { this.total = this.total + n; } }",
        "class C { total = 0; add(n: number) { this.total += n; } }",
    );
}

#[test]
fn prepend_fold_does_not_contract() {
    // `x = y + x` reverses operand order (string prepend) — it must NOT
    // collapse onto `x += y`.
    assert_canonically_distinct(
        "function f(x: string, y: string) { x = y + x; return x; }",
        "function f(x: string, y: string) { x += y; return x; }",
    );
}

#[test]
fn logical_assignment_does_not_contract() {
    assert_canonically_distinct(
        "function f(x: boolean, y: boolean) { x = x && y; return x; }",
        "function f(x: boolean, y: boolean) { x &&= y; return x; }",
    );
}

#[test]
fn increment_statement_equals_compound_and_expanded_forms() {
    assert_canonically_equal(
        "function f(i: number) { i++; return i; }",
        "function f(i: number) { i += 1; return i; }",
    );
    assert_canonically_equal(
        "function f(i: number) { i++; return i; }",
        "function f(i: number) { i = i + 1; return i; }",
    );
}

#[test]
fn return_ternary_equals_if_else_return() {
    assert_canonically_equal(
        r#"function f(n: number) { return n >= 0 ? "pos" : "neg"; }"#,
        r#"function f(n: number) { if (n >= 0) { return "pos"; } else { return "neg"; } }"#,
    );
}

#[test]
fn declaration_ternary_equals_let_if_else() {
    assert_canonically_equal(
        r#"function f(w: number) { const label = w > 20 ? "freight" : "parcel"; return label; }"#,
        r#"function f(w: number) { let label; if (w > 20) { label = "freight"; } else { label = "parcel"; } return label; }"#,
    );
}

#[test]
fn foreach_equals_for_of() {
    assert_canonically_equal(
        "function f(items: string[]) { items.forEach((item) => { console.log(item); }); }",
        "function f(items: string[]) { for (const item of items) { console.log(item); } }",
    );
}

#[test]
fn foreach_with_index_param_stays_a_call() {
    assert_canonically_distinct(
        "function f(items: string[]) { items.forEach((item, index) => { console.log(item, index); }); }",
        "function f(items: string[]) { for (const item of items) { console.log(item, item); } }",
    );
}

#[test]
fn then_chain_equals_await_sequence() {
    assert_canonically_equal(
        r#"
async function f(url: string) {
  return fetch(url).then((res) => {
    if (!res.ok) {
      throw new Error("bad");
    }
    return res.json();
  });
}
"#,
        r#"
async function f(url: string) {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error("bad");
  }
  return res.json();
}
"#,
    );
}

#[test]
fn statement_then_without_return_still_lowers_to_await() {
    assert_canonically_equal(
        r#"
async function f(p: Promise<number>) {
  p.then((v) => {
    console.log(v);
  });
  console.log("after");
}
"#,
        r#"
async function f(p: Promise<number>) {
  const v = await p;
  console.log(v);
  console.log("after");
}
"#,
    );
}

#[test]
fn statement_then_with_returning_callback_stays_a_call() {
    // The callback's `return` only exits the callback — `"after"` is
    // always logged. In the await form the `return` exits `f` and
    // skips it. Inlining would erase that difference.
    assert_canonically_distinct(
        r#"
async function f(p: Promise<number>, flag: boolean) {
  p.then((v) => {
    if (flag) return;
    console.log(v);
  });
  console.log("after");
}
"#,
        r#"
async function f(p: Promise<number>, flag: boolean) {
  const v = await p;
  if (flag) return;
  console.log(v);
  console.log("after");
}
"#,
    );
}

#[test]
fn two_argument_then_stays_a_call() {
    // `then(onFulfilled, onRejected)` also handles rejection — it is not
    // equivalent to a bare await sequence.
    assert_canonically_distinct(
        "async function f(p: Promise<number>) { return p.then((v) => v + 1, (e) => 0); }",
        "async function f(p: Promise<number>) { const v = await p; return v + 1; }",
    );
}

#[test]
fn return_await_equals_plain_return() {
    assert_canonically_equal(
        "async function f(p: Promise<number>) { return await p; }",
        "async function f(p: Promise<number>) { return p; }",
    );
}

#[test]
fn for_without_init_and_update_equals_while() {
    assert_canonically_equal(
        "function f(q: { pop(): string | undefined }) { let item = q.pop(); for (; item !== undefined; ) { item = q.pop(); } }",
        "function f(q: { pop(): string | undefined }) { let item = q.pop(); while (item !== undefined) { item = q.pop(); } }",
    );
}

#[test]
fn jump_terminated_switch_equals_if_chain() {
    assert_canonically_equal(
        r#"
function f(code: number) {
  switch (code) {
    case 200:
      return "ok";
    case 404:
      return "missing";
    default:
      return "unknown";
  }
}
"#,
        r#"
function f(code: number) {
  if (code === 200) {
    return "ok";
  } else if (code === 404) {
    return "missing";
  } else {
    return "unknown";
  }
}
"#,
    );
}

#[test]
fn fallthrough_switch_stays_a_switch() {
    let source = r#"
function f(code: number) {
  let label = "";
  switch (code) {
    case 200:
      label = "ok";
    case 201:
      label += "!";
      break;
    default:
      label = "unknown";
  }
  return label;
}
"#;
    // The first case falls through; lowering it to an if-chain would
    // change behaviour, so the canonical tree keeps a SwitchStatement.
    let tree = canonical_tree(source);
    fn contains_label(node: &TreeNode, label: &str) -> bool {
        node.label == label || node.children.iter().any(|c| contains_label(c, label))
    }
    assert!(contains_label(&tree, "SwitchStatement"));
}

#[test]
fn object_assign_with_empty_target_equals_spread() {
    assert_canonically_equal(
        "function f(a: object, b: object) { return Object.assign({}, a, b, { done: true }); }",
        "function f(a: object, b: object) { return { ...a, ...b, done: true }; }",
    );
}

#[test]
fn object_assign_with_mutating_target_stays_a_call() {
    assert_canonically_distinct(
        "function f(a: object, b: object) { return Object.assign(a, b); }",
        "function f(a: object, b: object) { return { ...a, ...b }; }",
    );
}

#[test]
fn braceless_bodies_equal_braced_bodies() {
    assert_canonically_equal(
        r#"function f(tag: string) { if (!tag) return ""; return tag.trim(); }"#,
        r#"function f(tag: string) { if (!tag) { return ""; } return tag.trim(); }"#,
    );
}

#[test]
fn async_marker_survives_canonicalization() {
    assert_canonically_distinct(
        "async function f() { const next = 42; return next; }",
        "function f() { const next = 42; return next; }",
    );
}

#[test]
fn canonical_flag_does_not_leak_into_plain_parses() {
    let canonical = canonical_tree("function f() { return `a ${1}`; }");
    let plain = parse_and_convert_to_tree("test.ts", "function f() { return `a ${1}`; }")
        .expect("source must parse");
    // The canonical tree rewrites the template into a concatenation;
    // the plain parse right after must still see a TemplateLiteral.
    fn contains_kind(node: &TreeNode, kind: &str) -> bool {
        node.kind == kind || node.children.iter().any(|c| contains_kind(c, kind))
    }
    assert!(!contains_kind(&canonical, "TemplateLiteral"));
    assert!(contains_kind(&plain, "TemplateLiteral"));
}

// -- alpha-renaming ---------------------------------------------------

#[test]
fn consistent_local_renames_produce_identical_trees() {
    assert_canonically_equal(
        r"function calculateCartTotal(prices: number[]): number {
  if (prices.length === 0) return 0;
  let total = 0;
  for (const price of prices) {
    total += price;
  }
  return total;
}",
        r"function sumInvoiceAmounts(amounts: number[]): number {
  if (amounts.length === 0) return 0;
  let sum = 0;
  for (const amount of amounts) {
    sum += amount;
  }
  return sum;
}",
    );
}

#[test]
fn free_identifiers_keep_their_names() {
    assert_canonically_distinct(
        "function f(user: string) { notifyByEmail(user); logDelivery(user); return user; }",
        "function g(user: string) { notifyBySms(user); logDelivery(user); return user; }",
    );
}

#[test]
fn recursion_renames_consistently() {
    assert_canonically_equal(
        "function fib(n: number): number { if (n < 2) return n; return fib(n - 1) + fib(n - 2); }",
        "function fibonacci(k: number): number { if (k < 2) return k; return fibonacci(k - 1) + fibonacci(k - 2); }",
    );
}

#[test]
fn shadowing_stays_distinct_from_reuse() {
    // Inner `value` shadows the parameter in the left function; the
    // right one keeps using the parameter. Different data flow.
    assert_canonically_distinct(
        "function f(value: number) { const inner = () => { const value = 1; return value; }; return inner() + value; }",
        "function f(value: number) { const inner = () => { return value; }; return inner() + value; }",
    );
}

// -- guard style / negation -------------------------------------------

#[test]
fn else_return_equals_guard_return() {
    assert_canonically_equal(
        r#"function f(ok: boolean) { if (ok) { return "yes"; } else { return "no"; } }"#,
        r#"function f(ok: boolean) { if (ok) { return "yes"; } return "no"; }"#,
    );
}

#[test]
fn ternary_return_equals_guard_return() {
    assert_canonically_equal(
        r#"function f(ok: boolean) { return ok ? "yes" : "no"; }"#,
        r#"function f(ok: boolean) { if (ok) { return "yes"; } return "no"; }"#,
    );
}

#[test]
fn negated_if_swaps_branches() {
    assert_canonically_equal(
        "function f(ready: boolean) { if (!ready) { prepare(); } else { launch(); } }",
        "function f(ready: boolean) { if (ready) { launch(); } else { prepare(); } }",
    );
}

#[test]
fn inequality_guard_equals_flipped_equality_guard() {
    assert_canonically_equal(
        r#"function f(mode: string) { if (mode !== "on") { return 0; } return 1; }"#,
        r#"function f(mode: string) { if (mode === "on") { return 1; } return 0; }"#,
    );
}

#[test]
fn terminating_else_hoists_to_guard() {
    // NOTE: `!==` is used rather than `> 0` because only exact
    // complements (equality operators, `!`) participate in negation
    // normalization — `!(x > 0)` is not `x <= 0` when NaN is around.
    assert_canonically_equal(
        "function f(input: string) { if (input.length !== 0) { process(input); } else { return; } finish(input); }",
        "function f(input: string) { if (input.length === 0) { return; } process(input); finish(input); }",
    );
}

#[test]
fn de_morgan_forms_converge() {
    assert_canonically_equal(
        "function f(a: boolean, b: boolean) { if (!(a && b)) { return 1; } return 2; }",
        "function f(a: boolean, b: boolean) { if (!a || !b) { return 1; } return 2; }",
    );
}

#[test]
fn wrong_de_morgan_distribution_stays_distinct() {
    assert_canonically_distinct(
        "function f(a: boolean, b: boolean) { if (!(a && b)) { return 1; } return 2; }",
        "function f(a: boolean, b: boolean) { if (!a && !b) { return 1; } return 2; }",
    );
}

#[test]
fn negated_equality_flips_operator() {
    assert_canonically_equal(
        "function f(n: number) { if (!(n === 0)) { return n; } return 1; }",
        "function f(n: number) { if (n !== 0) { return n; } return 1; }",
    );
}

#[test]
fn yoda_comparison_equals_natural_order() {
    assert_canonically_equal(
        "function f(n: number) { if (0 === n) { return 1; } return 2; }",
        "function f(n: number) { if (n === 0) { return 1; } return 2; }",
    );
}

// -- logic sugar -------------------------------------------------------

#[test]
fn self_selecting_ternary_equals_logical_or() {
    assert_canonically_equal(
        "function f(label: string, fallback: string) { const chosen = label ? label : fallback; return chosen; }",
        "function f(label: string, fallback: string) { const chosen = label || fallback; return chosen; }",
    );
}

#[test]
fn nullish_ternary_equals_coalesce() {
    assert_canonically_equal(
        "function f(input: string | null, fallback: string) { const value = input == null ? fallback : input; return value; }",
        "function f(input: string | null, fallback: string) { const value = input ?? fallback; return value; }",
    );
    assert_canonically_equal(
        "function f(input: string | null, fallback: string) { const value = input != null ? input : fallback; return value; }",
        "function f(input: string | null, fallback: string) { const value = input ?? fallback; return value; }",
    );
}

#[test]
fn strict_null_pair_equals_loose_null_check() {
    assert_canonically_equal(
        "function f(x: unknown) { if (x === null || x === undefined) { return 0; } return 1; }",
        "function f(x: unknown) { if (x == null) { return 0; } return 1; }",
    );
    assert_canonically_equal(
        "function f(x: unknown) { if (x !== null && x !== undefined) { return 1; } return 0; }",
        "function f(x: unknown) { if (x != null) { return 1; } return 0; }",
    );
}

#[test]
fn loose_null_stays_distinct_from_strict_null_only() {
    assert_canonically_distinct(
        "function f(x: unknown) { if (x == null) { return 0; } return 1; }",
        "function f(x: unknown) { if (x === null) { return 0; } return 1; }",
    );
}

#[test]
fn coalesce_stays_distinct_from_logical_or() {
    assert_canonically_distinct(
        "function f(count: number | null) { const value = count ?? 10; return value; }",
        "function f(count: number | null) { const value = count || 10; return value; }",
    );
}

#[test]
fn void_zero_equals_undefined() {
    assert_canonically_equal(
        "function f(x: unknown) { if (x === void 0) { return 1; } return 0; }",
        "function f(x: unknown) { if (x === undefined) { return 1; } return 0; }",
    );
}

#[test]
fn boolean_call_equals_double_negation() {
    assert_canonically_equal(
        "function f(x: unknown) { const flag = Boolean(x); return flag; }",
        "function f(x: unknown) { const flag = !!x; return flag; }",
    );
}

#[test]
fn test_position_double_negation_is_transparent() {
    assert_canonically_equal(
        "function f(x: unknown) { if (!!x) { return 1; } return 0; }",
        "function f(x: unknown) { if (x) { return 1; } return 0; }",
    );
}

// -- destructuring ------------------------------------------------------

#[test]
fn object_destructuring_equals_member_declarations() {
    assert_canonically_equal(
        "function f(config: { host: string; port: number }) { const { host, port } = config; return host + port; }",
        "function f(config: { host: string; port: number }) { const host = config.host; const port = config.port; return host + port; }",
    );
}

#[test]
fn renaming_destructuring_equals_member_declaration() {
    assert_canonically_equal(
        "function f(response: { data: string }) { const { data: payload } = response; return payload; }",
        "function f(response: { data: string }) { const payload = response.data; return payload; }",
    );
}

#[test]
fn destructuring_with_defaults_keeps_its_shape() {
    assert_canonically_distinct(
        "function f(config: { host?: string }) { const { host = \"localhost\" } = config; return host; }",
        "function f(config: { host?: string }) { const host = config.host; return host; }",
    );
}

// -- loop forms ----------------------------------------------------------

#[test]
fn index_loop_equals_for_of() {
    assert_canonically_equal(
        r"function f(items: string[]) {
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    handle(item);
  }
}",
        r"function f(items: string[]) {
  for (const item of items) {
    handle(item);
  }
}",
    );
}

#[test]
fn index_loop_using_index_elsewhere_keeps_its_shape() {
    assert_canonically_distinct(
        r"function f(items: string[]) {
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    handle(item, i);
  }
}",
        r"function f(items: string[]) {
  for (const item of items) {
    handle(item);
  }
}",
    );
}

#[test]
fn push_loop_equals_map_call() {
    assert_canonically_equal(
        r"function f(values: number[]) {
  const doubled = [];
  for (const value of values) {
    doubled.push(value * 2);
  }
  return doubled;
}",
        r"function f(values: number[]) {
  return values.map((value) => value * 2);
}",
    );
}

#[test]
fn guarded_push_loop_equals_filter_call() {
    assert_canonically_equal(
        r"function f(values: number[]) {
  const positive = [];
  for (const value of values) {
    if (value > 0) {
      positive.push(value);
    }
  }
  return positive;
}",
        r"function f(values: number[]) {
  return values.filter((value) => value > 0);
}",
    );
}

#[test]
fn map_stays_distinct_from_filter() {
    assert_canonically_distinct(
        "function f(xs: number[]) { const out = xs.map((x) => x > 0); return out; }",
        "function f(xs: number[]) { const out = xs.filter((x) => x > 0); return out; }",
    );
}

// -- micro idioms ----------------------------------------------------------

#[test]
fn temp_return_equals_direct_return() {
    assert_canonically_equal(
        "function f(a: number, b: number) { const result = a * b + 1; return result; }",
        "function f(a: number, b: number) { return a * b + 1; }",
    );
}

#[test]
fn last_index_access_equals_at_minus_one() {
    assert_canonically_equal(
        "function f(items: string[]) { const last = items[items.length - 1]; return last; }",
        "function f(items: string[]) { const last = items.at(-1); return last; }",
    );
}

#[test]
fn at_minus_one_stays_distinct_from_at_zero() {
    assert_canonically_distinct(
        "function f(items: string[]) { const edge = items.at(-1); return edge; }",
        "function f(items: string[]) { const edge = items.at(0); return edge; }",
    );
}

#[test]
fn arrow_expression_body_equals_block_body_in_callbacks() {
    assert_canonically_equal(
        "function f(xs: number[]) { const out = xs.map((x) => x * 2); return out; }",
        "function f(xs: number[]) { const out = xs.map((x) => { return x * 2; }); return out; }",
    );
}

#[test]
fn else_block_wrapping_an_if_equals_else_if() {
    assert_canonically_equal(
        "function f(n: number) { if (n > 10) { big(); } else { if (n > 5) { medium(); } } }",
        "function f(n: number) { if (n > 10) { big(); } else if (n > 5) { medium(); } }",
    );
}

#[test]
fn optional_chain_stays_distinct_from_plain_access() {
    assert_canonically_distinct(
        "function f(user: { name?: string } | null) { const name = user?.name; return name; }",
        "function f(user: { name: string }) { const name = user.name; return name; }",
    );
}

#[test]
fn combo_of_transforms_converges() {
    assert_canonically_equal(
        r#"function describeOrders(orders: { id: string; total: number }[]) {
  const labels = [];
  for (const order of orders) {
    labels.push(`Order ${order.id}: ${order.total}`);
  }
  if (labels.length === 0) {
    return "none";
  } else {
    const joined = labels.join(", ");
    return joined;
  }
}"#,
        r#"function summarizeInvoices(invoices: { id: string; total: number }[]) {
  const lines = invoices.map((invoice) => "Order " + invoice.id + ": " + invoice.total);
  if (0 === lines.length) return "none";
  return lines.join(", ");
}"#,
    );
}

// -- constructor parameter properties -----------------------------------

#[test]
fn parameter_properties_equal_explicit_field_assignments() {
    assert_canonically_equal(
        "class A { constructor(private readonly repo: Repo, public clock: Clock) { this.startedAt = clock.now(); } }",
        "class A { constructor(repo: Repo, clock: Clock) { this.repo = repo; this.clock = clock; this.startedAt = clock.now(); } }",
    );
}

#[test]
fn parameter_property_assignments_follow_the_super_call() {
    assert_canonically_equal(
        "class A extends B { constructor(private readonly repo: Repo) { super(); } }",
        "class A extends B { constructor(repo: Repo) { super(); this.repo = repo; } }",
    );
    // Statements may precede `super()` (TS 4.6+); the assignments
    // still land right after it, never ahead of it.
    assert_canonically_equal(
        "class A extends B { constructor(private readonly repo: Repo) { logStart(); super(); } }",
        "class A extends B { constructor(repo: Repo) { logStart(); super(); this.repo = repo; } }",
    );
    assert_canonically_distinct(
        "class A extends B { constructor(private readonly repo: Repo) { logStart(); super(); } }",
        "class A extends B { constructor(repo: Repo) { this.repo = repo; logStart(); super(); } }",
    );
}

#[test]
fn plain_constructor_parameters_are_not_desugared() {
    assert_canonically_distinct(
        "class A { constructor(repo: Repo) { this.other = repo; } }",
        "class A { constructor(repo: Repo) { this.repo = repo; this.other = repo; } }",
    );
}

// -- destructuring assignment --------------------------------------------

#[test]
fn a_destructuring_assignment_says_what_goes_where() {
    // The pattern on the left used to be one leaf, whatever it held: any
    // two destructuring assignments were the same tree.
    assert_canonically_distinct(
        "function f(s: T) { let { a, b, c } = s; [a, b, c] = [b, c, a]; return [a, b, c]; }",
        "function f(s: T) { let { a, b, c } = s; [c, b, a] = [b, c, a]; return [a, b, c]; }",
    );
    assert_canonically_distinct(
        "function f(e: E, p: P) { let x = p.x; let y = p.y; ({ clientX: x, clientY: y } = e); return [x, y]; }",
        "function f(e: E, p: P) { let x = p.x; let y = p.y; ({ offsetX: x, offsetY: y } = e); return [x, y]; }",
    );
    // The variables assigned are the function's own: renamed, a twin.
    assert_canonically_equal(
        "function swap(pair: P) { let lo = pair.lo; let hi = pair.hi; [lo, hi] = [hi, lo]; return [lo, hi]; }",
        "function flip(input: P) { let min = input.lo; let max = input.hi; [min, max] = [max, min]; return [min, max]; }",
    );
    // `{ a }` reads the property `a` into the variable `a`; so does
    // `{ a: b }` into `b`.
    assert_canonically_equal(
        "function f(o: O) { let a = 0; ({ a } = o); return a; }",
        "function g(o: O) { let b = 0; ({ a: b } = o); return b; }",
    );
    assert_canonically_distinct(
        "function f(o: O) { let a = 0; ({ a } = o); return a; }",
        "function g(o: O) { let b = 0; ({ c: b } = o); return b; }",
    );
}

#[test]
fn destructuring_assignments_keep_their_holes_defaults_and_rest() {
    let assign = |pattern: &str| {
        format!(
            "function f(xs: number[]) {{ let a = 0; let b = 0; {pattern} = xs; return a + b; }}"
        )
    };
    let object = |pattern: &str| {
        format!("function f(o: O) {{ let a = 0; let b = 0; ({pattern} = o); return a + b; }}")
    };
    for (left, right) in [
        (assign("[a, b]"), assign("[, a, b]")),
        (assign("[a, b]"), assign("[a = 1, b]")),
        (assign("[a = 1, b]"), assign("[a = 2, b]")),
        (assign("[a, b]"), assign("[a, ...b]")),
        (object("{ a, b }"), object("{ a, b = 1 }")),
        (object("{ a, b }"), object("{ a, ...b }")),
        (object("{ a, b }"), object("{ a, [b]: b }")),
        // Nested patterns and member targets are targets like any other.
        (object("{ a, p: [b] }"), object("{ a, p: { b } }")),
        (assign("[this.x, this.y]"), assign("[this.y, this.x]")),
    ] {
        assert_canonically_distinct(&left, &right);
    }
    // A default is spelled the way a declaration's pattern spells it.
    assert_canonically_equal(&assign("[a = 1, b]"), &assign("[a = 1, b]"));
}

#[test]
fn a_loop_head_that_destructures_says_what_goes_where() {
    assert_canonically_distinct(
        "function f(entries: [string, number][]) { let key = ''; let value = 0; for ([key, value] of entries) { use(key, value); } }",
        "function f(entries: [string, number][]) { let key = ''; let value = 0; for ([value, key] of entries) { use(key, value); } }",
    );
    assert_canonically_distinct(
        "function f(rows: Row[]) { let id = ''; let name = ''; for ({ id, name } of rows) { use(id, name); } }",
        "function f(rows: Row[]) { let id = ''; let name = ''; for ({ id, title: name } of rows) { use(id, name); } }",
    );
}

// -- parameters ----------------------------------------------------------

#[test]
fn default_parameter_values_are_part_of_the_signature() {
    // The default used to be dropped: all three were one signature.
    assert_canonically_distinct(
        "function page(items: Item[], size = 10) { return items.slice(0, size); }",
        "function page(items: Item[], size: number) { return items.slice(0, size); }",
    );
    assert_canonically_distinct(
        "function page(items: Item[], size = 10) { return items.slice(0, size); }",
        "function page(items: Item[], size = pageSize()) { return items.slice(0, size); }",
    );
    assert_canonically_equal(
        "function page(items: Item[], size = 10) { return items.slice(0, size); }",
        "function paginate(rows: Item[], limit = 10) { return rows.slice(0, limit); }",
    );
    // Arrows and methods take their parameters the same way.
    assert_canonically_distinct(
        "const page = (items: Item[], size = 10) => items.slice(0, size);",
        "const page = (items: Item[], size = 20) => items.slice(0, size);",
    );
    assert_canonically_distinct(
        "class Pager { page(items: Item[], size = 10) { return items.slice(0, size); } }",
        "class Pager { page(items: Item[], size: number) { return items.slice(0, size); } }",
    );
}

#[test]
fn a_default_value_costs_what_it_holds() {
    // The value sits next to the binding. Wrapped around it, a literal
    // default cost 2.9: the binding renamed into the wrapper, and the
    // wrapper's two children inserted.
    let distance = |left: &str, right: &str| {
        compute_edit_distance(&canonical_tree(left), &canonical_tree(right), &structural_options())
    };
    let plain = "function page(items: Item[], size: number) { return items.slice(0, size); }";
    let with_literal = "function page(items: Item[], size = 10) { return items.slice(0, size); }";
    let with_call =
        "function page(items: Item[], size = pageSize()) { return items.slice(0, size); }";
    assert!(
        (distance(plain, with_literal) - 1.0).abs() < 1e-9,
        "{}",
        distance(plain, with_literal)
    );
    // A call is the call and its callee.
    assert!((distance(plain, with_call) - 2.0).abs() < 1e-9, "{}", distance(plain, with_call));
}

#[test]
fn the_rest_parameter_is_a_parameter() {
    // The rest parameter used to be dropped, so `log(...parts)` had the
    // signature of `log()` and `parts` was a free identifier.
    assert_canonically_distinct(
        "function log(...parts: string[]) { return parts.join(\" \"); }",
        "function log() { return parts.join(\" \"); }",
    );
    assert_canonically_distinct(
        "function log(...parts: string[]) { return parts.join(\" \"); }",
        "function log(parts: string[]) { return parts.join(\" \"); }",
    );
    assert_canonically_equal(
        "function log(level: Level, ...parts: string[]) { return level + parts.join(\" \"); }",
        "function write(severity: Level, ...pieces: string[]) { return severity + pieces.join(\" \"); }",
    );
}

#[test]
fn callbacks_with_defaults_or_rest_parameters_are_not_lowered() {
    // `for (const row of rows)` binds every element as it is; the callback
    // parameter `row = fallback` replaces `undefined` elements.
    assert_canonically_distinct(
        "function f(rows: Row[]) { rows.forEach((row = fallback) => { use(row); }); }",
        "function f(rows: Row[]) { for (const row of rows) { use(row); } }",
    );
    assert_canonically_distinct(
        "function f(rows: Row[]) { rows.forEach((...args) => { use(args); }); }",
        "function f(rows: Row[]) { for (const args of rows) { use(args); } }",
    );
    assert_canonically_distinct(
        "async function f(p: Promise<Row>) { return p.then((row = fallback) => { return use(row); }); }",
        "async function f(p: Promise<Row>) { const row = await p; return use(row); }",
    );
    // The plain forms still lower.
    assert_canonically_equal(
        "function f(rows: Row[]) { rows.forEach((row) => { use(row); }); }",
        "function f(rows: Row[]) { for (const row of rows) { use(row); } }",
    );
}

// -- member access --------------------------------------------------------

#[test]
fn each_link_of_an_optional_chain_has_its_own_optionality() {
    // Every link used to be labelled optional: these pairs were one tree.
    assert_canonically_distinct(
        "function f(a: A) { return a?.b.c; }",
        "function f(a: A) { return a?.b?.c; }",
    );
    assert_canonically_distinct(
        "function f(a: A) { return a?.run(); }",
        "function f(a: A) { return a?.run?.(); }",
    );
    assert_canonically_distinct(
        "function f(a: A) { return a?.items[0]; }",
        "function f(a: A) { return a?.items?.[0]; }",
    );
    assert_canonically_equal(
        "function f(a: A) { return a?.b.c(); }",
        "function g(value: A) { return value?.b.c(); }",
    );
    // A non-null assertion inside a chain is erased like any other.
    assert_canonically_equal(
        "function f(a: A) { return a?.b!.c; }",
        "function f(a: A) { return a?.b.c; }",
    );
}

#[test]
fn computed_keys_stay_distinct_from_static_keys() {
    // `{ [id]: value }` reads the variable `id` for its key; `{ id: value }`
    // names the key `id`. Both keys used to convert alike.
    assert_canonically_distinct(
        "function f(id: string, value: number) { return { [id]: value }; }",
        "function f(id: string, value: number) { return { id: value }; }",
    );
    assert_canonically_distinct(
        "function f(source: Source, key: string) { const { [key]: found } = source; return found; }",
        "function f(source: Source, key: string) { const { key: found } = source; return found; }",
    );
    // A literal in brackets is the static key it spells.
    assert_canonically_equal(
        "function f(value: number) { return { [\"total\"]: value }; }",
        "function f(value: number) { return { \"total\": value }; }",
    );
}

#[test]
fn private_brand_checks_are_converted() {
    assert_canonically_distinct(
        "class A { #brand = 1; static is(value: object) { return #brand in value; } }",
        "class A { #brand = 1; static is(value: object) { return #other in value; } }",
    );
    assert_canonically_distinct(
        "class A { #brand = 1; static is(value: object, other: object) { return #brand in value; } }",
        "class A { #brand = 1; static is(value: object, other: object) { return #brand in other; } }",
    );
}

// -- function bodies -----------------------------------------------------

#[test]
fn directives_are_part_of_the_body() {
    // oxc keeps the directive prologue apart from the statements; it used
    // to be dropped, so a server action equalled its plain twin.
    assert_canonically_distinct(
        "async function save(form: FormData) { \"use server\"; await store(form); }",
        "async function save(form: FormData) { await store(form); }",
    );
    assert_canonically_distinct(
        "function f() { \"use strict\"; return this; }",
        "function f() { \"use asm\"; return this; }",
    );
    assert_canonically_equal(
        "class A { constructor(private repo: Repo) { \"use strict\"; this.ready = true; } }",
        "class A { constructor(private repo: Repo) { 'use strict'; this.ready = true; } }",
    );
    assert_canonically_distinct(
        "class A { constructor(private repo: Repo) { \"use strict\"; this.ready = true; } }",
        "class A { constructor(private repo: Repo) { this.ready = true; } }",
    );
}

#[test]
fn a_labeled_statement_keeps_every_statement_its_body_lowers_to() {
    // `return c ? a : b` lowers to two statements; under a label they stay
    // together, in a block.
    assert_canonically_equal(
        "function f(c: boolean, a: number, b: number) { pick: return c ? a : b; }",
        "function f(c: boolean, a: number, b: number) { pick: { if (c) { return a; } return b; } }",
    );
    assert_canonically_distinct(
        "function f(c: boolean, a: number, b: number) { pick: return c ? a : b; }",
        "function f(c: boolean, a: number, b: number) { pick: { if (c) { return a; } } }",
    );
}

#[test]
fn redeclared_variables_keep_one_name() {
    // `var` may declare one variable twice; the second declaration used to
    // keep its source name while every other occurrence was renamed.
    assert_canonically_equal(
        "function f(n: number) { var total = 0; if (n > 0) { var total = n; } return total; }",
        "function g(n: number) { var sum = 0; if (n > 0) { var sum = n; } return sum; }",
    );
}

// -- JSX -------------------------------------------------------------------

fn canonical_tsx(source: &str) -> Rc<TreeNode> {
    parse_and_convert_to_tree_canonical("test.tsx", source).expect("source must parse")
}

fn tsx_distance(source1: &str, source2: &str) -> f64 {
    compute_edit_distance(&canonical_tsx(source1), &canonical_tsx(source2), &structural_options())
}

#[test]
fn jsx_is_converted_structurally() {
    // Every JSX expression used to be one opaque leaf: two components were
    // the same tree whatever they rendered.
    let card = "function Card({ title }: Props) { return <section className=\"card\"><h2>{title}</h2><Body compact /></section>; }";
    let list = "function List({ items }: Props) { return <ul>{items.map((item) => <li key={item.id}>{item.name}</li>)}</ul>; }";
    assert!(tsx_distance(card, list) > 5.0, "{}", tsx_distance(card, list));

    // A renamed parameter is the same component; another property is not.
    let by_props = "function Card(props: Props) { return <section className=\"card\"><h2>{props.title}</h2><Body compact /></section>; }";
    let renamed = "function Panel(p: Props) { return <section className=\"card\"><h2>{p.title}</h2><Body compact /></section>; }";
    assert!(tsx_distance(by_props, renamed) == 0.0, "{}", tsx_distance(by_props, renamed));
    let other_property = "function Panel(p: Props) { return <section className=\"card\"><h2>{p.heading}</h2><Body compact /></section>; }";
    assert!(tsx_distance(by_props, other_property) > 0.0);
}

#[test]
fn jsx_elements_differ_by_tag_attribute_and_text() {
    let base = "function View() { return <div className=\"a\">Hello</div>; }";
    for other in [
        // Another intrinsic tag.
        "function View() { return <span className=\"a\">Hello</span>; }",
        // Another attribute, and another value of the same attribute.
        "function View() { return <div id=\"a\">Hello</div>; }",
        "function View() { return <div className=\"b\">Hello</div>; }",
        // Another text.
        "function View() { return <div className=\"a\">Goodbye</div>; }",
        // A spread instead of the attribute.
        "function View() { return <div {...props}>Hello</div>; }",
        // A fragment around it.
        "function View() { return <><div className=\"a\">Hello</div></>; }",
    ] {
        assert!(tsx_distance(base, other) > 0.0, "{other}");
    }
}

#[test]
fn jsx_text_follows_the_compiler_whitespace_rule() {
    // Indentation and line breaks around JSX children are not text.
    assert!(
        tsx_distance(
            "function View() { return <p>Hello world</p>; }",
            "function View() {\n  return (\n    <p>\n      Hello world\n    </p>\n  );\n}",
        ) == 0.0
    );
    // An expression container with a string is the text it holds; an empty
    // container (a comment) is nothing.
    assert!(
        tsx_distance(
            "function View() { return <p>{/* note */}Hello</p>; }",
            "function View() { return <p>Hello</p>; }",
        ) == 0.0
    );
}

#[test]
fn jsx_components_are_references_and_intrinsic_tags_are_not() {
    // `<Row />` refers to the binding `Row`: a local component renames
    // with its declaration, and a tag name never does.
    assert!(
        tsx_distance(
            "function Table() { const Row = pick(); return <Row dense />; }",
            "function Table() { const Line = pick(); return <Line dense />; }",
        ) == 0.0
    );
    assert!(
        tsx_distance(
            "function Table() { const tr = pick(); return <tr />; }",
            "function Table() { const td = pick(); return <td />; }",
        ) > 0.0
    );
    // Member components keep their path.
    assert!(
        tsx_distance(
            "function Menu() { return <Menu.Item />; }",
            "function Menu() { return <Menu.Group />; }",
        ) > 0.0
    );
}

// -- source positions -------------------------------------------------------

#[test]
fn converted_nodes_know_the_source_they_come_from() {
    let source = "function total(rows: Row[], rate = 2) {\n  let sum = 0;\n  for (const row of rows) {\n    sum += row.price * rate;\n  }\n  return { sum, rate };\n}\n";
    let tree = parse_and_convert_to_tree("test.ts", source).unwrap();

    fn collect<'t>(node: &'t TreeNode, out: &mut Vec<&'t TreeNode>) {
        out.push(node);
        for child in &node.children {
            collect(child, out);
        }
    }
    let mut nodes = Vec::new();
    collect(&tree, &mut nodes);
    let text_of = |kind: &str| -> Vec<&str> {
        nodes
            .iter()
            .filter(|node| node.kind == kind && node.has_source_span())
            .map(|node| &source[node.source_span.0 as usize..node.source_span.1 as usize])
            .collect()
    };
    // Statements, expressions, parameters and object members each carry
    // the span of their own text, not that of the statement around them.
    assert_eq!(text_of("Parameter"), ["rows: Row[]", "rate = 2"]);
    assert_eq!(text_of("BinaryExpression"), ["row.price * rate"]);
    assert_eq!(text_of("ReturnStatement"), ["return { sum, rate };"]);
    assert_eq!(text_of("ObjectProperty"), ["sum", "rate"]);
    assert_eq!(text_of("ForOfStatement").len(), 1);
    assert!(text_of("ForOfStatement")[0].starts_with("for (const row of rows) {"));
}
