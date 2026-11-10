//! Statement-list peepholes (tree level).
//!
//! These run over already-converted statement lists, where earlier
//! lowerings (`forEach` → `for-of`, `i++` → `i += 1`, ternaries → `if`/`else`)
//! have normalized their inputs — so one pattern here covers every source
//! spelling of the idiom.

use std::rc::Rc;

use super::{leaf, make_node, ConversionContext};
use crate::tree::TreeNode;

pub(super) fn tree_mentions_identifier(node: &TreeNode, label: &str) -> bool {
    if (node.kind == "Identifier" || node.kind == "BindingIdentifier") && node.label == label {
        return true;
    }
    node.children.iter().any(|child| tree_mentions_identifier(child, label))
}

/// The single binding introduced by a `<Kind>Declaration [VariableDeclarator
/// [BindingIdentifier …]]` node, together with its initializer (if any).
pub(super) fn single_declarator(node: &TreeNode) -> Option<(&Rc<TreeNode>, Option<&Rc<TreeNode>>)> {
    if node.kind != "VariableDeclaration" || node.children.len() != 1 {
        return None;
    }
    let declarator = &node.children[0];
    if declarator.kind != "VariableDeclarator" || declarator.children.is_empty() {
        return None;
    }
    let binding = &declarator.children[0];
    if binding.kind != "BindingIdentifier" {
        return None;
    }
    Some((binding, declarator.children.get(1)))
}

/// Rewrite `for (let i = 0; i < xs.length; i += 1) { const x = xs[i]; … }`
/// (with `i` unused elsewhere) into `for (const x of xs) { … }`. All the
/// index-stepping spellings (`i++`, `++i`, `i += 1`, `i = i + 1`) already
/// canonicalized onto the compound form before this runs.
pub(super) fn rewrite_index_loop(node: &Rc<TreeNode>) -> Option<Rc<TreeNode>> {
    if node.label != "ForStatement" || node.children.len() != 4 {
        return None;
    }
    let (init, test, update, body) =
        (&node.children[0], &node.children[1], &node.children[2], &node.children[3]);

    let (index_binding, index_init) = single_declarator(init)?;
    if index_init?.label != "0" {
        return None;
    }
    let index_label = index_binding.label.clone();

    if test.label != "LessThan" || test.children.len() != 2 {
        return None;
    }
    if test.children[0].label != index_label || test.children[0].kind != "Identifier" {
        return None;
    }
    let length_access = &test.children[1];
    if length_access.label != "." || length_access.children.len() != 2 {
        return None;
    }
    if length_access.children[1].label != "length" {
        return None;
    }
    let iterated = &length_access.children[0];
    if tree_mentions_identifier(iterated, &index_label) {
        return None;
    }

    if update.kind != "AssignmentExpression"
        || update.label != "Addition"
        || update.children.len() != 2
        || update.children[0].label != index_label
        || update.children[1].label != "1"
    {
        return None;
    }

    if body.kind != "BlockStatement" || body.children.is_empty() {
        return None;
    }
    let first = &body.children[0];
    let (element_binding, element_init) = single_declarator(first)?;
    let element_init = element_init?;
    if element_init.label != "[]"
        || element_init.children.len() != 2
        || !element_init.children[0].structurally_equals(iterated)
        || element_init.children[1].label != index_label
        || element_init.children[1].kind != "Identifier"
    {
        return None;
    }
    if body.children[1..].iter().any(|stmt| tree_mentions_identifier(stmt, &index_label)) {
        return None;
    }

    let mut for_node = make_node("ForOfStatement", "ForOfStatement");
    let mut decl = make_node("ConstDeclaration", "VariableDeclaration");
    let mut declarator = make_node("VariableDeclarator", "VariableDeclarator");
    declarator.add_child(Rc::clone(element_binding));
    decl.add_child(Rc::new(declarator));
    for_node.add_child(Rc::new(decl));
    for_node.add_child(Rc::clone(iterated));
    let mut block = make_node("BlockStatement", "BlockStatement");
    for stmt in &body.children[1..] {
        block.add_child(Rc::clone(stmt));
    }
    for_node.add_child(Rc::new(block));
    Some(Rc::new(for_node))
}

/// Rewrite the accumulate-into-array idiom onto its method form:
///
/// ```text
/// const out = [];                       const out = xs.map((x) => E);
/// for (const x of xs) {          ⇒
///   out.push(E);
/// }
/// ```
///
/// and the guarded push of the element itself onto `.filter`. Runs after
/// forEach→for-of lowering, so both loop spellings participate.
pub(super) fn rewrite_push_loop(
    declaration: &Rc<TreeNode>,
    loop_node: &Rc<TreeNode>,
) -> Option<Rc<TreeNode>> {
    let (accumulator_binding, accumulator_init) = single_declarator(declaration)?;
    let accumulator_init = accumulator_init?;
    if accumulator_init.kind != "ArrayExpression" || !accumulator_init.children.is_empty() {
        return None;
    }
    let accumulator = accumulator_binding.label.clone();

    if loop_node.label != "ForOfStatement" || loop_node.children.len() != 3 {
        return None;
    }
    let (left, iterated, body) =
        (&loop_node.children[0], &loop_node.children[1], &loop_node.children[2]);
    let (element_binding, element_init) = single_declarator(left)?;
    if element_init.is_some() {
        return None;
    }
    if tree_mentions_identifier(iterated, &accumulator) {
        return None;
    }
    if body.kind != "BlockStatement" || body.children.len() != 1 {
        return None;
    }

    let push_argument_of = |stmt: &Rc<TreeNode>| -> Option<Rc<TreeNode>> {
        if stmt.kind != "ExpressionStatement" || stmt.children.len() != 1 {
            return None;
        }
        let call = &stmt.children[0];
        if call.label != "CallExpression" || call.children.len() != 2 {
            return None;
        }
        let callee = &call.children[0];
        if callee.label != "."
            || callee.children.len() != 2
            || callee.children[0].label != accumulator
            || callee.children[0].kind != "Identifier"
            || callee.children[1].label != "push"
        {
            return None;
        }
        Some(Rc::clone(&call.children[1]))
    };

    let statement = &body.children[0];
    let (method, callback_body_expr) = if let Some(argument) = push_argument_of(statement) {
        if tree_mentions_identifier(&argument, &accumulator) {
            return None;
        }
        ("map", argument)
    } else if statement.kind == "IfStatement" && statement.children.len() == 2 {
        // if (COND) { out.push(x) }  — pushing the element itself.
        let condition = &statement.children[0];
        let then_block = &statement.children[1];
        if then_block.kind != "BlockStatement" || then_block.children.len() != 1 {
            return None;
        }
        let argument = push_argument_of(&then_block.children[0])?;
        if argument.kind != "Identifier" || argument.label != element_binding.label {
            return None;
        }
        if tree_mentions_identifier(condition, &accumulator) {
            return None;
        }
        ("filter", Rc::clone(condition))
    } else {
        return None;
    };

    // const out = xs.<method>((x) => { return E; });
    let mut arrow = make_node("ArrowFunction", "ArrowFunctionExpression");
    let mut parameter = make_node("Parameter", "Parameter");
    parameter.add_child(Rc::clone(element_binding));
    arrow.add_child(Rc::new(parameter));
    let mut arrow_block = make_node("BlockStatement", "BlockStatement");
    let mut return_node = make_node("ReturnStatement", "ReturnStatement");
    return_node.add_child(callback_body_expr);
    arrow_block.add_child(Rc::new(return_node));
    arrow.add_child(Rc::new(arrow_block));

    let mut member = make_node(".", "StaticMemberExpression");
    member.add_child(Rc::clone(iterated));
    member.add_child(leaf(method, "Identifier"));

    let mut call = make_node("CallExpression", "CallExpression");
    call.add_child(Rc::new(member));
    call.add_child(Rc::new(arrow));

    let mut declarator = make_node("VariableDeclarator", "VariableDeclarator");
    declarator.add_child(Rc::clone(accumulator_binding));
    declarator.add_child(Rc::new(call));
    let mut decl = make_node(&declaration.label, "VariableDeclaration");
    decl.add_child(Rc::new(declarator));
    Some(Rc::new(decl))
}

/// TreeNode-level "control cannot continue" check, mirroring
/// [`statement_always_terminates`] for already-converted nodes.
pub(super) fn node_always_terminates(node: &TreeNode) -> bool {
    match node.kind {
        "ReturnStatement" | "ThrowStatement" | "BreakStatement" | "ContinueStatement" => true,
        "BlockStatement" => node.children.last().is_some_and(|last| node_always_terminates(last)),
        "IfStatement" => {
            node.children.len() == 3
                && node_always_terminates(&node.children[1])
                && node_always_terminates(&node.children[2])
        }
        _ => false,
    }
}

/// Flip an else-less negative guard so both complementary spellings of a
/// terminating branch pair converge:
///
/// ```text
/// if (a !== b) { return X; }        if (a === b) { return Y; }
/// return Y;                    ⇔    return X;
/// ```
///
/// Sound only when both the guard body and the tail always exit — then
/// "which branch is the guard" is pure style. Only exact complements flip
/// (equality operators and `!`); ordering comparisons stay put because of
/// NaN.
pub(super) fn rewrite_negative_guard(nodes: &mut Vec<Rc<TreeNode>>) {
    for index in 0..nodes.len() {
        let is_last = index + 1 == nodes.len();
        if is_last {
            break;
        }
        let guard = &nodes[index];
        if guard.kind != "IfStatement" || guard.children.len() != 2 {
            continue;
        }
        let test = &guard.children[0];
        let flipped_label = match test.label.as_str() {
            "StrictInequality" if test.kind == "BinaryExpression" => Some("StrictEquality"),
            "Inequality" if test.kind == "BinaryExpression" => Some("Equality"),
            "LogicalNot" if test.kind == "UnaryExpression" => None,
            _ => continue,
        };
        if !node_always_terminates(&guard.children[1]) {
            continue;
        }
        let mut tail: Vec<Rc<TreeNode>> = nodes[index + 1..].to_vec();
        if !tail.last().is_some_and(|last| node_always_terminates(last)) {
            continue;
        }
        // The tail may itself start with a negative guard — normalize it
        // before it becomes the flipped consequent.
        rewrite_negative_guard(&mut tail);

        let positive_test: Rc<TreeNode> = if let Some(label) = flipped_label {
            let mut flipped = make_node(label, "BinaryExpression");
            for child in &test.children {
                flipped.add_child(Rc::clone(child));
            }
            Rc::new(flipped)
        } else {
            // `!x` — the positive form is the bare operand.
            match test.children.first() {
                Some(inner) => Rc::clone(inner),
                None => continue,
            }
        };

        let mut tail_block = make_node("BlockStatement", "BlockStatement");
        for stmt in tail {
            tail_block.add_child(stmt);
        }
        let mut flipped_guard = make_node("IfStatement", "IfStatement");
        flipped_guard.add_child(positive_test);
        flipped_guard.add_child(Rc::new(tail_block));

        let old_consequent = Rc::clone(&guard.children[1]);
        nodes.truncate(index);
        nodes.push(Rc::new(flipped_guard));
        nodes.extend(old_consequent.children.iter().cloned());
        break;
    }
}

/// TreeNode-level purity: labels/kinds that are safe to move across
/// adjacent statements (identifiers, literals, `this`, and static member
/// chains of those). Mirrors [`is_side_effect_free`] for converted nodes.
pub(super) fn node_is_pure_chain(node: &TreeNode) -> bool {
    match node.kind {
        "Identifier" | "BindingIdentifier" | "ThisExpression" | "StringLiteral"
        | "NumericLiteral" | "BooleanLiteral" | "NullLiteral" | "BigIntLiteral" => true,
        "StaticMemberExpression" if node.label == "." => {
            node.children.first().is_some_and(|object| node_is_pure_chain(object))
        }
        _ => false,
    }
}

pub(super) fn count_identifier_uses(node: &TreeNode, label: &str) -> usize {
    let own = usize::from(node.kind == "Identifier" && node.label == label);
    own + node.children.iter().map(|child| count_identifier_uses(child, label)).sum::<usize>()
}

/// Clone `node` with the single `Identifier` leaf labeled `label`
/// replaced by `replacement`. `replaced` flips when the substitution
/// happens so only the first occurrence is rewritten.
pub(super) fn substitute_identifier(
    node: &Rc<TreeNode>,
    label: &str,
    replacement: &Rc<TreeNode>,
    replaced: &mut bool,
) -> Rc<TreeNode> {
    if !*replaced && node.kind == "Identifier" && node.label == label {
        *replaced = true;
        return Rc::clone(replacement);
    }
    if node.children.is_empty() {
        return Rc::clone(node);
    }
    let mut fresh = make_node(&node.label, node.kind);
    fresh.source_span = node.source_span;
    for child in &node.children {
        fresh.add_child(substitute_identifier(child, label, replacement, replaced));
    }
    Rc::new(fresh)
}

/// Inline single-use pure-chain constants:
///
/// ```text
/// const id = payload.member_id;      return { id: payload.member_id };
/// return { id };                ⇒
/// ```
///
/// Only fires for canonical `§`-renamed locals whose initializer is a
/// pure chain (identifier / literal / `this` / static member access) and
/// that are referenced exactly once afterwards — then moving the read to
/// the use site cannot change behavior (modulo getters, an accepted
/// caveat shared with the destructuring lowering that feeds this pass).
pub(super) fn inline_single_use_constants(nodes: &mut Vec<Rc<TreeNode>>) {
    let mut index = 0;
    while index < nodes.len() {
        let Some((binding, Some(init))) = single_declarator(&nodes[index]) else {
            index += 1;
            continue;
        };
        if nodes[index].label != "ConstDeclaration"
            || !binding.label.starts_with('§')
            || !node_is_pure_chain(init)
        {
            index += 1;
            continue;
        }
        let label = binding.label.clone();
        let uses: usize =
            nodes[index + 1..].iter().map(|stmt| count_identifier_uses(stmt, &label)).sum();
        if uses != 1 {
            index += 1;
            continue;
        }
        let Some(use_offset) =
            nodes[index + 1..].iter().position(|stmt| count_identifier_uses(stmt, &label) > 0)
        else {
            index += 1;
            continue;
        };
        // Moving the read to the use site is only order-safe when nothing
        // in between can mutate what the chain reads: every intervening
        // statement must itself be a pure-chain const declaration (the
        // destructuring-lowering shape this pass exists for).
        let gap_is_pure = nodes[index + 1..index + 1 + use_offset].iter().all(|stmt| {
            matches!(single_declarator(stmt), Some((_, Some(init)))
                if stmt.label == "ConstDeclaration" && node_is_pure_chain(init))
        });
        if !gap_is_pure {
            index += 1;
            continue;
        }
        let init = Rc::clone(init);
        let mut replaced = false;
        let slot = &mut nodes[index + 1 + use_offset];
        *slot = substitute_identifier(slot, &label, &init, &mut replaced);
        nodes.remove(index);
        // Re-check the same index — the next statement may now qualify.
    }
}

/// Whether an initializer is a pure computation (no calls, construction,
/// awaits, or mutation anywhere) — such initializers commute with each
/// other, so declaration order between them is style, not behavior.
pub(super) fn node_is_reorderable_init(node: &TreeNode) -> bool {
    match node.kind {
        "CallExpression"
        | "NewExpression"
        | "AwaitExpression"
        | "YieldExpression"
        | "AssignmentExpression"
        | "UpdateExpression"
        | "TaggedTemplateExpression"
        | "ImportExpression" => false,
        _ => node.children.iter().all(|child| node_is_reorderable_init(child)),
    }
}

/// Order-canonicalize runs of consecutive, mutually independent, pure
/// `const` declarations by the structure of their initializers.
/// `const a = o.x; const b = o.y;` and `const b = o.y; const a = o.x;`
/// are the same code — sorting removes the incidental order (which the
/// destructuring lowering and hand-written spellings frequently disagree
/// on) so it stops costing tree distance. Binding names are excluded
/// from the sort key: they carry pre-relabel ordinals that differ by
/// construction.
pub(super) fn sort_reorderable_declarations(nodes: &mut [Rc<TreeNode>]) {
    fn init_sort_key(node: &TreeNode) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let (_, Some(init)) = single_declarator(node)? else {
            return None;
        };
        if node.label != "ConstDeclaration" || !node_is_reorderable_init(init) {
            return None;
        }
        fn walk(node: &TreeNode, hasher: &mut impl Hasher) {
            node.label.hash(hasher);
            node.kind.hash(hasher);
            node.children.len().hash(hasher);
            for child in &node.children {
                walk(child, hasher);
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        walk(init, &mut hasher);
        Some(hasher.finish())
    }

    let mut index = 0;
    while index < nodes.len() {
        // Grow a run of sortable const declarations.
        let mut end = index;
        while end < nodes.len() && init_sort_key(&nodes[end]).is_some() {
            end += 1;
        }
        if end - index >= 2 {
            // Independence: no init may mention a binding declared inside
            // the run (that would be a real data dependency).
            let bindings: Vec<String> = nodes[index..end]
                .iter()
                .filter_map(|node| single_declarator(node).map(|(b, _)| b.label.clone()))
                .collect();
            let independent = nodes[index..end].iter().all(|node| {
                single_declarator(node).and_then(|(_, init)| init).is_some_and(|init| {
                    bindings.iter().all(|binding| !tree_mentions_identifier(init, binding))
                })
            });
            if independent {
                nodes[index..end].sort_by_key(|node| init_sort_key(node).unwrap_or(u64::MAX));
            }
        }
        index = end.max(index + 1);
    }
}

/// `const t = E; return t;` at the end of a block ⇔ `return E;`.
pub(super) fn rewrite_temp_return(nodes: &mut Vec<Rc<TreeNode>>) {
    let count = nodes.len();
    if count < 2 {
        return;
    }
    let Some((binding, Some(init))) = single_declarator(&nodes[count - 2]) else {
        return;
    };
    let return_node = &nodes[count - 1];
    if return_node.label != "ReturnStatement"
        || return_node.children.len() != 1
        || return_node.children[0].kind != "Identifier"
        || return_node.children[0].label != binding.label
    {
        return;
    }
    let mut replacement = make_node("ReturnStatement", "ReturnStatement");
    replacement.add_child(Rc::clone(init));
    nodes.truncate(count - 2);
    nodes.push(Rc::new(replacement));
}

/// Run the statement-list peepholes over a freshly built block body.
pub(super) fn normalize_statement_nodes(nodes: &mut Vec<Rc<TreeNode>>, cx: &ConversionContext) {
    if !cx.canonical {
        return;
    }
    for slot in nodes.iter_mut() {
        if let Some(rewritten) = rewrite_index_loop(slot) {
            *slot = rewritten;
        }
    }
    inline_single_use_constants(nodes);
    let mut index = 0;
    while index + 1 < nodes.len() {
        if let Some(replacement) = rewrite_push_loop(&nodes[index], &nodes[index + 1]) {
            nodes[index] = replacement;
            nodes.remove(index + 1);
        } else {
            index += 1;
        }
    }
    sort_reorderable_declarations(nodes);
    rewrite_temp_return(nodes);
    rewrite_negative_guard(nodes);
}
