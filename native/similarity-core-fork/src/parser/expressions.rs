//! Expressions, including the canonical expression-level rewrites.

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget, BinaryOperator,
    CallExpression, ChainElement, ComputedMemberExpression, Expression, LogicalOperator,
    ObjectPropertyKind, PrivateFieldExpression, PropertyKey, SimpleAssignmentTarget, Statement,
    StaticMemberExpression, UnaryOperator, UpdateOperator,
};
use oxc_span::GetSpan;
use std::rc::Rc;

use super::jsx::{jsx_element_to_tree_node, jsx_fragment_to_tree_node};
use super::lowering::canonical_return_nodes;
use super::patterns::formal_parameters_to_tree_nodes;
use super::predicates::{
    boolean_call_argument, classify_nullish_test, is_literal_operand, is_side_effect_free,
    match_boolean_call, pure_expressions_equal, NullishTest,
};
use super::statements::{class_declaration_to_tree_node, function_body_to_tree_node};
use super::{leaf, make_node, stamp, strip_parentheses, ConversionContext};
use crate::tree::TreeNode;

/// Canonicalization for expressions whose value is discarded (statement
/// position and `for` update slots): `i++` / `--i` become the compound
/// form `i += 1` / `i -= 1`, so all three in-place-update spellings
/// (`i++`, `i += 1`, and `i = i + 1` via the assignment contraction)
/// produce the same tree.
pub(super) fn statement_value_expression_to_tree_node(
    expr: &Expression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    if cx.canonical {
        if let Expression::UpdateExpression(update) = strip_parentheses(expr) {
            let operator_label = match update.operator {
                UpdateOperator::Increment => "Addition",
                UpdateOperator::Decrement => "Subtraction",
            };
            let mut assign = make_node(operator_label, "AssignmentExpression");
            if let Some(target) = simple_assignment_target_to_tree_node(&update.argument, cx) {
                assign.add_child(target);
            }
            assign.add_child(leaf("1", "NumericLiteral"));
            return Some(Rc::new(assign));
        }
    }
    expression_to_tree_node(expr, cx)
}

/// Structural equality for the simple assignment-target shapes that show
/// up on both sides of `x = x op E` / `this.x = this.x op E`. Used to
/// decide whether an expanded assignment can contract onto the compound
/// form; anything more exotic (computed members, chained paths) returns
/// `false` and keeps the literal shape.
pub(super) fn assignment_target_matches_expression(
    target: &AssignmentTarget,
    expr: &Expression,
) -> bool {
    match (target, strip_parentheses(expr)) {
        (
            AssignmentTarget::AssignmentTargetIdentifier(ident),
            Expression::Identifier(expr_ident),
        ) => ident.name == expr_ident.name,
        (
            AssignmentTarget::StaticMemberExpression(member),
            Expression::StaticMemberExpression(expr_member),
        ) => {
            if member.property.name != expr_member.property.name {
                return false;
            }
            match (strip_parentheses(&member.object), strip_parentheses(&expr_member.object)) {
                (Expression::ThisExpression(_), Expression::ThisExpression(_)) => true,
                (Expression::Identifier(a), Expression::Identifier(b)) => a.name == b.name,
                _ => false,
            }
        }
        _ => false,
    }
}

/// Lower `Object.assign({}, a, {k: v})` into the object-spread literal it
/// is equivalent to: `{ ...a, k: v }`. Only the empty-fresh-target form
/// qualifies — with a non-empty first argument `Object.assign` mutates its
/// target, which the spread form never does.
pub(super) fn lower_object_assign_to_spread(
    call_expr: &CallExpression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    if call_expr.optional || call_expr.arguments.len() < 2 {
        return None;
    }
    let Expression::StaticMemberExpression(member) = strip_parentheses(&call_expr.callee) else {
        return None;
    };
    if member.optional || member.property.name.as_str() != "assign" {
        return None;
    }
    let Expression::Identifier(object_ident) = strip_parentheses(&member.object) else {
        return None;
    };
    if object_ident.name.as_str() != "Object" || cx.is_locally_bound(object_ident.span) {
        return None;
    }
    let first = call_expr.arguments[0].as_expression().map(strip_parentheses)?;
    let Expression::ObjectExpression(target) = first else {
        return None;
    };
    if !target.properties.is_empty() {
        return None;
    }

    let mut node = make_node("ObjectExpression", "ObjectExpression");
    for argument in call_expr.arguments.iter().skip(1) {
        match argument.as_expression().map(strip_parentheses) {
            Some(Expression::ObjectExpression(obj)) => {
                // Inline literal arguments — `Object.assign({}, {k: v})`
                // contributes its properties directly, just like the
                // spread form `{ k: v }` would.
                for prop in &obj.properties {
                    if let Some(child) = object_property_to_tree_node(prop, cx) {
                        node.add_child(child);
                    }
                }
            }
            Some(expr) => {
                let mut spread = make_node("SpreadProperty", "SpreadProperty");
                if let Some(child) = expression_to_tree_node(expr, cx) {
                    spread.add_child(child);
                }
                node.add_child(Rc::new(spread));
            }
            None => return None,
        }
    }
    Some(Rc::new(node))
}

/// Build the canonical tree for `subject == null` (`equal`) or
/// `subject != null` — the target shape both the loose spelling and the
/// strict two-comparison spelling collapse onto.
pub(super) fn nullish_comparison_node(
    subject: &Expression,
    equal: bool,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    let label = if equal { "Equality" } else { "Inequality" };
    let mut node = make_node(label, "BinaryExpression");
    node.add_child(expression_to_tree_node(subject, cx)?);
    node.add_child(leaf("null", "NullLiteral"));
    Some(Rc::new(node))
}

/// Convert the *negation* of `expr` to a tree. Equality-family operators
/// flip to their exact complements (`!(a === b)` ⇔ `a !== b`); `&&`/`||`
/// distribute via De Morgan (exact under JS truthiness); everything else
/// keeps a literal `!` wrapper. Ordering comparisons (`<`, `>=`, …) are
/// deliberately NOT flipped — `!(a < b)` and `a >= b` disagree on NaN.
pub(super) fn negated_expression_to_tree_node(
    expr: &Expression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    let expr = strip_parentheses(expr);
    if let Expression::BinaryExpression(bin) = expr {
        let flipped = match bin.operator {
            BinaryOperator::StrictEquality => Some("StrictInequality"),
            BinaryOperator::StrictInequality => Some("StrictEquality"),
            BinaryOperator::Equality => Some("Inequality"),
            BinaryOperator::Inequality => Some("Equality"),
            _ => None,
        };
        if let Some(label) = flipped {
            let mut node = make_node(label, "BinaryExpression");
            let (first, second) = equality_operand_order(&bin.left, &bin.right, cx);
            if let Some(child) = expression_to_tree_node(first, cx) {
                node.add_child(child);
            }
            if let Some(child) = expression_to_tree_node(second, cx) {
                node.add_child(child);
            }
            return Some(Rc::new(node));
        }
    }
    if let Expression::LogicalExpression(log) = expr {
        let distributed = match log.operator {
            LogicalOperator::And => Some("Or"),
            LogicalOperator::Or => Some("And"),
            LogicalOperator::Coalesce => None,
        };
        if let Some(label) = distributed {
            let mut node = make_node(label, "LogicalExpression");
            if let Some(child) = negated_expression_to_tree_node(&log.left, cx) {
                node.add_child(child);
            }
            if let Some(child) = negated_expression_to_tree_node(&log.right, cx) {
                node.add_child(child);
            }
            return Some(Rc::new(node));
        }
    }
    let mut node = make_node("LogicalNot", "UnaryExpression");
    if let Some(child) = expression_to_tree_node(expr, cx) {
        node.add_child(child);
    }
    Some(Rc::new(node))
}

/// Yoda normalization for the symmetric equality operators: when exactly
/// one operand is a literal, the literal goes second, so `5 === x` and
/// `x === 5` produce the same tree. Literal-literal and expr-expr pairs
/// keep their source order.
pub(super) fn equality_operand_order<'a, 'b>(
    left: &'b Expression<'a>,
    right: &'b Expression<'a>,
    cx: &ConversionContext,
) -> (&'b Expression<'a>, &'b Expression<'a>) {
    if is_literal_operand(left, cx) && !is_literal_operand(right, cx) {
        (right, left)
    } else {
        (left, right)
    }
}

/// Boolean-context simplification: strip `!!` pairs and `Boolean(...)`
/// wrappers from an expression consumed only for its truthiness (an
/// `if`/`while`/`for`/ternary test). Value positions keep them — there
/// `!!x` and `x` are different values.
pub(super) fn test_expression_to_tree_node(
    expr: &Expression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    if !cx.canonical {
        return expression_to_tree_node(expr, cx);
    }
    let mut current = strip_parentheses(expr);
    loop {
        if let Expression::UnaryExpression(outer) = current {
            if outer.operator == UnaryOperator::LogicalNot {
                if let Expression::UnaryExpression(inner) = strip_parentheses(&outer.argument) {
                    if inner.operator == UnaryOperator::LogicalNot {
                        current = strip_parentheses(&inner.argument);
                        continue;
                    }
                }
            }
        }
        if let Some(inner) = match_boolean_call(current, cx) {
            current = strip_parentheses(inner);
            continue;
        }
        break;
    }
    expression_to_tree_node(current, cx)
}

pub(super) fn expression_to_tree_node(
    expr: &Expression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    convert_expression(expr, cx).map(|node| stamp(node, expr.span()))
}

fn convert_expression(expr: &Expression, cx: &ConversionContext) -> Option<Rc<TreeNode>> {
    match expr {
        Expression::Identifier(ident) => {
            Some(leaf(&cx.identifier_label(ident.span, &ident.name), "Identifier"))
        }
        Expression::StringLiteral(str_lit) => {
            let label = format!("\"{}\"", str_lit.value.as_str());
            Some(leaf(&label, "StringLiteral"))
        }
        Expression::NumericLiteral(num_lit) => {
            let label = num_lit.value.to_string();
            Some(leaf(&label, "NumericLiteral"))
        }
        Expression::BooleanLiteral(bool_lit) => {
            let label = bool_lit.value.to_string();
            Some(leaf(&label, "BooleanLiteral"))
        }
        Expression::NullLiteral(_) => Some(leaf("null", "NullLiteral")),
        Expression::BigIntLiteral(big) => Some(leaf(big.value.as_str(), "BigIntLiteral")),
        Expression::RegExpLiteral(re) => {
            // Encode both the pattern text and the flags into the label so
            // distinct regexes don't collide. `/foo/i` (case-insensitive
            // match) and `/foo/g` (global match) have meaningfully
            // different runtime behaviour and shouldn't be treated as
            // identical leaves.
            let label = format!("/{}/{}", re.regex.pattern.text.as_str(), re.regex.flags);
            Some(leaf(&label, "RegExpLiteral"))
        }
        Expression::TemplateLiteral(tpl) => {
            // Canonical mode rewrites an (untagged) template literal into
            // the string-concatenation chain it is equivalent to:
            // `` `a ${x} b` `` and `"a " + x + " b"` are the same string
            // computation, and the choice between them is pure style. The
            // operand order interleaves quasis and expressions exactly as
            // they appear; empty quasis (e.g. before a leading `${`)
            // contribute nothing to the concatenation and are dropped.
            if cx.canonical {
                // `+` only guarantees string concatenation once a string
                // operand has entered the left-associated chain. A template
                // whose first two operands are both interpolations
                // (`` `${a}${b}…` ``) — or a lone interpolation
                // (`` `${a}` ``) — would otherwise produce the same tree as
                // numeric `a + b` / bare `a`, which compute different
                // values. Seed those chains with the explicit `""` head the
                // equivalent hand-written concatenation carries.
                //
                // Hint-sensitive coercion (`Symbol.toPrimitive` / `valueOf`
                // observing the "string" vs "default" hint) can still tell
                // `${x}` apart from `"" + x` at runtime. That caveat applies
                // equally to every template⇔concatenation pairing this
                // lowering produces (`` `Hello ${name}` `` vs
                // `"Hello " + name` included) and is accepted by design: a
                // syntactic analyzer cannot see types, and the pairing
                // mirrors the `prefer-template` rewrite, which treats the
                // two spellings as interchangeable for ordinary values. The
                // line drawn is: collapse pairs that agree on every
                // primitive, keep pairs apart that diverge on primitives
                // (numeric `a + b` vs `` `${a}${b}` ``).
                let first_quasi_empty = tpl
                    .quasis
                    .first()
                    .is_none_or(|q| q.value.cooked.as_deref().unwrap_or("").is_empty());
                let second_quasi_empty = tpl
                    .quasis
                    .get(1)
                    .is_none_or(|q| q.value.cooked.as_deref().unwrap_or("").is_empty());
                let needs_empty_head =
                    first_quasi_empty && !tpl.expressions.is_empty() && second_quasi_empty;

                let mut operands: Vec<Rc<TreeNode>> = Vec::new();
                if needs_empty_head {
                    operands.push(leaf("\"\"", "StringLiteral"));
                }
                for (index, quasi) in tpl.quasis.iter().enumerate() {
                    let cooked: &str = quasi.value.cooked.as_deref().unwrap_or("");
                    if !cooked.is_empty() {
                        operands.push(leaf(&format!("\"{cooked}\""), "StringLiteral"));
                    }
                    if let Some(inner_expr) = tpl.expressions.get(index) {
                        if let Some(child) = expression_to_tree_node(inner_expr, cx) {
                            operands.push(child);
                        }
                    }
                }
                return match operands.len() {
                    0 => Some(leaf("\"\"", "StringLiteral")),
                    1 => operands.pop(),
                    _ => {
                        let mut iter = operands.into_iter();
                        let mut chain = iter.next().expect("len checked above");
                        for operand in iter {
                            let mut add = make_node("Addition", "BinaryExpression");
                            add.add_child(chain);
                            add.add_child(operand);
                            chain = Rc::new(add);
                        }
                        Some(chain)
                    }
                };
            }
            let mut node = make_node("TemplateLiteral", "TemplateLiteral");
            for q in &tpl.quasis {
                let cooked: &str = q.value.cooked.as_deref().unwrap_or("");
                let label = format!("`{}`", cooked);
                node.add_child(leaf(&label, "TemplateElement"));
            }
            for inner_expr in &tpl.expressions {
                if let Some(child) = expression_to_tree_node(inner_expr, cx) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::TaggedTemplateExpression(tag) => {
            let mut node = make_node("TaggedTemplateExpression", "TaggedTemplateExpression");
            if let Some(callee) = expression_to_tree_node(&tag.tag, cx) {
                node.add_child(callee);
            }
            // Inline the template literal as a child
            let mut tpl_node = make_node("TemplateLiteral", "TemplateLiteral");
            for q in &tag.quasi.quasis {
                let cooked: &str = q.value.cooked.as_deref().unwrap_or("");
                let label = format!("`{}`", cooked);
                tpl_node.add_child(leaf(&label, "TemplateElement"));
            }
            for inner_expr in &tag.quasi.expressions {
                if let Some(child) = expression_to_tree_node(inner_expr, cx) {
                    tpl_node.add_child(child);
                }
            }
            node.add_child(Rc::new(tpl_node));
            Some(Rc::new(node))
        }
        Expression::BinaryExpression(bin_expr) => {
            let mut node = TreeNode::new(format!("{:?}", bin_expr.operator), "BinaryExpression");

            // Symmetric equality operators put a lone literal operand
            // second, so Yoda-style `5 === x` matches `x === 5`.
            let (first, second) = if cx.canonical
                && matches!(
                    bin_expr.operator,
                    BinaryOperator::StrictEquality
                        | BinaryOperator::StrictInequality
                        | BinaryOperator::Equality
                        | BinaryOperator::Inequality
                ) {
                equality_operand_order(&bin_expr.left, &bin_expr.right, cx)
            } else {
                (&bin_expr.left, &bin_expr.right)
            };

            if let Some(left_node) = expression_to_tree_node(first, cx) {
                node.add_child(left_node);
            }

            if let Some(right_node) = expression_to_tree_node(second, cx) {
                node.add_child(right_node);
            }

            Some(Rc::new(node))
        }
        Expression::LogicalExpression(log_expr) => {
            // The strict null/undefined pair contracts onto the loose
            // comparison it is equivalent to: `x === null || x ===
            // undefined` ⇔ `x == null` (and the `!==`/`&&`/`!=` duals).
            if cx.canonical {
                match classify_nullish_test(expr, cx) {
                    Some(NullishTest::IsNullish(subject)) => {
                        return nullish_comparison_node(subject, true, cx);
                    }
                    Some(NullishTest::NotNullish(subject)) => {
                        return nullish_comparison_node(subject, false, cx);
                    }
                    None => {}
                }
            }
            let mut node = TreeNode::new(format!("{:?}", log_expr.operator), "LogicalExpression");
            if let Some(left_node) = expression_to_tree_node(&log_expr.left, cx) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&log_expr.right, cx) {
                node.add_child(right_node);
            }
            Some(Rc::new(node))
        }
        Expression::AssignmentExpression(assign) => {
            // Canonical mode contracts `x = x + y` onto the compound form
            // `x += y` (same label and children as a literal `+=`), so the
            // two spellings of an in-place update compare as equal. The
            // contraction only fires when the binary's FIRST operand is the
            // assignment target itself — `x = y + x` is a different fold
            // (prepend vs append for strings) and keeps its literal shape.
            // Logical operators short-circuit and are not in the
            // contractible set, so `x = x && y` stays as-is too.
            if cx.canonical && assign.operator == AssignmentOperator::Assign {
                if let Expression::BinaryExpression(binary) = strip_parentheses(&assign.right) {
                    let operator_label = format!("{:?}", binary.operator);
                    let is_contractible = matches!(
                        operator_label.as_str(),
                        "Addition"
                            | "Subtraction"
                            | "Multiplication"
                            | "Division"
                            | "Remainder"
                            | "Exponential"
                            | "ShiftLeft"
                            | "ShiftRight"
                            | "ShiftRightZeroFill"
                            | "BitwiseOR"
                            | "BitwiseXOR"
                            | "BitwiseAnd"
                    );
                    if is_contractible
                        && assignment_target_matches_expression(&assign.left, &binary.left)
                    {
                        let mut node = TreeNode::new(operator_label, "AssignmentExpression");
                        if let Some(left_node) = assignment_target_to_tree_node(&assign.left, cx) {
                            node.add_child(left_node);
                        }
                        if let Some(right_node) = expression_to_tree_node(&binary.right, cx) {
                            node.add_child(right_node);
                        }
                        return Some(Rc::new(node));
                    }
                }
            }
            // Operator (=, +=, -=, etc.) directly distinguishes assignment
            // shapes that would otherwise look identical.
            let mut node = TreeNode::new(format!("{:?}", assign.operator), "AssignmentExpression");
            if let Some(left_node) = assignment_target_to_tree_node(&assign.left, cx) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&assign.right, cx) {
                node.add_child(right_node);
            }
            Some(Rc::new(node))
        }
        Expression::UnaryExpression(unary) => {
            if cx.canonical {
                // `!<expr>` flips equality operators to their exact
                // complements and distributes over `&&`/`||` (De Morgan),
                // so `!(a === b)` ⇔ `a !== b` and `!(a && b)` ⇔
                // `!a || !b` produce one shape.
                if unary.operator == UnaryOperator::LogicalNot {
                    return negated_expression_to_tree_node(&unary.argument, cx);
                }
                // `void 0` (or `void x` for pure x) IS the undefined
                // value; collapse onto the `undefined` identifier so both
                // spellings compare as equal.
                if unary.operator == UnaryOperator::Void && is_side_effect_free(&unary.argument) {
                    return Some(leaf("undefined", "Identifier"));
                }
            }
            let mut node = TreeNode::new(format!("{:?}", unary.operator), "UnaryExpression");
            if let Some(arg_node) = expression_to_tree_node(&unary.argument, cx) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Expression::UpdateExpression(update) => {
            let prefix_marker = if update.prefix { "prefix" } else { "postfix" };
            let label = format!("{:?}-{}", update.operator, prefix_marker);
            let mut node = TreeNode::new(label, "UpdateExpression");
            if let Some(arg_node) = simple_assignment_target_to_tree_node(&update.argument, cx) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Expression::ConditionalExpression(cond) => {
            if cx.canonical {
                let test = strip_parentheses(&cond.test);
                let consequent = strip_parentheses(&cond.consequent);
                let alternate = strip_parentheses(&cond.alternate);
                // Nullish-defaulting ternaries contract onto `??`:
                //   `x == null ? d : x` ⇔ `x != null ? x : d` ⇔ `x ?? d`.
                match classify_nullish_test(test, cx) {
                    Some(NullishTest::IsNullish(subject))
                        if pure_expressions_equal(subject, alternate) =>
                    {
                        let mut node = make_node("Coalesce", "LogicalExpression");
                        if let Some(child) = expression_to_tree_node(alternate, cx) {
                            node.add_child(child);
                        }
                        if let Some(child) = expression_to_tree_node(consequent, cx) {
                            node.add_child(child);
                        }
                        return Some(Rc::new(node));
                    }
                    Some(NullishTest::NotNullish(subject))
                        if pure_expressions_equal(subject, consequent) =>
                    {
                        let mut node = make_node("Coalesce", "LogicalExpression");
                        if let Some(child) = expression_to_tree_node(consequent, cx) {
                            node.add_child(child);
                        }
                        if let Some(child) = expression_to_tree_node(alternate, cx) {
                            node.add_child(child);
                        }
                        return Some(Rc::new(node));
                    }
                    _ => {}
                }
                // `x ? x : y` ⇔ `x || y` and `x ? y : x` ⇔ `x && y` for
                // side-effect-free x (evaluating x twice is only safe
                // when x is pure).
                if is_side_effect_free(test) {
                    let logical = if pure_expressions_equal(test, consequent) {
                        Some(("Or", alternate))
                    } else if pure_expressions_equal(test, alternate) {
                        Some(("And", consequent))
                    } else {
                        None
                    };
                    if let Some((label, other)) = logical {
                        let mut node = make_node(label, "LogicalExpression");
                        if let Some(child) = expression_to_tree_node(test, cx) {
                            node.add_child(child);
                        }
                        if let Some(child) = expression_to_tree_node(other, cx) {
                            node.add_child(child);
                        }
                        return Some(Rc::new(node));
                    }
                }
            }
            let mut node = make_node("ConditionalExpression", "ConditionalExpression");
            if let Some(test_node) = test_expression_to_tree_node(&cond.test, cx) {
                node.add_child(test_node);
            }
            if let Some(cons_node) = expression_to_tree_node(&cond.consequent, cx) {
                node.add_child(cons_node);
            }
            if let Some(alt_node) = expression_to_tree_node(&cond.alternate, cx) {
                node.add_child(alt_node);
            }
            Some(Rc::new(node))
        }
        Expression::SequenceExpression(seq) => {
            let mut node = make_node("SequenceExpression", "SequenceExpression");
            for e in &seq.expressions {
                if let Some(child) = expression_to_tree_node(e, cx) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ParenthesizedExpression(paren) => {
            // Parentheses are noise — descend straight through them.
            expression_to_tree_node(&paren.expression, cx)
        }
        Expression::StaticMemberExpression(mem) => Some(static_member_node(mem, cx)),
        Expression::ComputedMemberExpression(mem) => Some(computed_member_node(mem, cx)),
        Expression::PrivateFieldExpression(mem) => Some(private_field_node(mem, cx)),
        Expression::CallExpression(call_expr) => Some(call_expression_node(call_expr, cx)),
        Expression::PrivateInExpression(private_in) => {
            // `#brand in value`
            let mut node = make_node("PrivateInExpression", "PrivateInExpression");
            node.add_child(leaf(
                &format!("#{}", private_in.left.name.as_str()),
                "PrivateIdentifier",
            ));
            if let Some(right_node) = expression_to_tree_node(&private_in.right, cx) {
                node.add_child(right_node);
            }
            Some(Rc::new(node))
        }
        Expression::JSXElement(element) => Some(jsx_element_to_tree_node(element, cx)),
        Expression::JSXFragment(fragment) => Some(jsx_fragment_to_tree_node(fragment, cx)),
        Expression::NewExpression(new_expr) => {
            let mut node = make_node("NewExpression", "NewExpression");
            if let Some(callee_node) = expression_to_tree_node(&new_expr.callee, cx) {
                node.add_child(callee_node);
            }
            for arg in &new_expr.arguments {
                if let Some(arg_node) = argument_to_tree_node(arg, cx) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        Expression::AwaitExpression(await_expr) => {
            let mut node = make_node("AwaitExpression", "AwaitExpression");
            if let Some(arg_node) = expression_to_tree_node(&await_expr.argument, cx) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Expression::YieldExpression(yield_expr) => {
            let label =
                if yield_expr.delegate { "YieldDelegateExpression" } else { "YieldExpression" };
            let mut node = make_node(label, "YieldExpression");
            if let Some(arg) = &yield_expr.argument {
                if let Some(arg_node) = expression_to_tree_node(arg, cx) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ObjectExpression(obj) => {
            let mut node = make_node("ObjectExpression", "ObjectExpression");
            for prop in &obj.properties {
                if let Some(child) = object_property_to_tree_node(prop, cx) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ArrayExpression(arr) => {
            let mut node = make_node("ArrayExpression", "ArrayExpression");
            for elem in &arr.elements {
                if let Some(child) = array_element_to_tree_node(elem, cx) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ThisExpression(_) => Some(leaf("ThisExpression", "ThisExpression")),
        Expression::Super(_) => Some(leaf("Super", "Super")),
        Expression::MetaProperty(meta) => {
            let label = format!("{}.{}", meta.meta.name.as_str(), meta.property.name.as_str());
            Some(leaf(&label, "MetaProperty"))
        }
        Expression::ImportExpression(import_expr) => {
            let mut node = make_node("ImportExpression", "ImportExpression");
            if let Some(child) = expression_to_tree_node(&import_expr.source, cx) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        Expression::ChainExpression(chain) => {
            // Optional chaining wrapper — descend through.
            let mut node = make_node("ChainExpression", "ChainExpression");
            if let Some(child) = chain_element_to_tree_node(&chain.expression, cx) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        Expression::ArrowFunctionExpression(arrow) => {
            // Encode async/generator into the label so async vs sync arrow
            // functions don't collapse into the same node.
            let label = if arrow.r#async { "AsyncArrowFunction" } else { "ArrowFunction" };
            let mut node = make_node(label, "ArrowFunctionExpression");

            for parameter in formal_parameters_to_tree_nodes(&arrow.params, cx) {
                node.add_child(parameter);
            }

            if arrow.expression {
                if let Some(Statement::ExpressionStatement(expr_stmt)) =
                    arrow.body.statements.first()
                {
                    if cx.canonical {
                        // `(x) => expr` is the same function as
                        // `(x) => { return expr; }` — wrap the expression
                        // body in the explicit block/return shape so both
                        // spellings (and nested callbacks in particular)
                        // produce identical trees.
                        let mut block = make_node("BlockStatement", "BlockStatement");
                        for child in canonical_return_nodes(Some(&expr_stmt.expression), cx) {
                            block.add_child(child);
                        }
                        node.add_child(Rc::new(block));
                    } else if let Some(expr_node) =
                        expression_to_tree_node(&expr_stmt.expression, cx)
                    {
                        node.add_child(expr_node);
                    }
                }
            } else {
                if let Some(body_node) = function_body_to_tree_node(&arrow.body, cx) {
                    node.add_child(body_node);
                }
            }

            Some(Rc::new(node))
        }
        Expression::FunctionExpression(func) => {
            let label = if func.r#async {
                if func.generator {
                    "AsyncGeneratorFunctionExpression"
                } else {
                    "AsyncFunctionExpression"
                }
            } else if func.generator {
                "GeneratorFunctionExpression"
            } else {
                "FunctionExpression"
            };
            let mut node = make_node(label, "FunctionExpression");
            for parameter in formal_parameters_to_tree_nodes(&func.params, cx) {
                node.add_child(parameter);
            }
            if let Some(body) = &func.body {
                if let Some(b) = function_body_to_tree_node(body, cx) {
                    node.add_child(b);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ClassExpression(class) => {
            class_declaration_to_tree_node(class, cx, "ClassExpression")
        }
        Expression::TSAsExpression(ts) => expression_to_tree_node(&ts.expression, cx),
        Expression::TSSatisfiesExpression(ts) => expression_to_tree_node(&ts.expression, cx),
        Expression::TSNonNullExpression(ts) => expression_to_tree_node(&ts.expression, cx),
        Expression::TSTypeAssertion(ts) => expression_to_tree_node(&ts.expression, cx),
        Expression::TSInstantiationExpression(ts) => expression_to_tree_node(&ts.expression, cx),
        _ => Some(leaf("Expression", "Expression")),
    }
}

/// `object.property` / `object?.property`.
pub(super) fn static_member_node(
    mem: &StaticMemberExpression,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    let mut node = make_node(if mem.optional { "?." } else { "." }, "StaticMemberExpression");
    if let Some(obj_node) = expression_to_tree_node(&mem.object, cx) {
        node.add_child(obj_node);
    }
    node.add_child(leaf(mem.property.name.as_str(), "Identifier"));
    Rc::new(node)
}

/// `object[expression]` / `object?.[expression]`, read.
pub(super) fn computed_member_node(
    mem: &ComputedMemberExpression,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    if let Some(at_call) = last_element_as_at_call(mem, cx) {
        return at_call;
    }
    element_access_node(mem, cx)
}

/// `object[expression]` as written. This is also the target of an
/// assignment or update, which is never the `.at(-1)` read
/// [`computed_member_node`] canonicalizes: `xs[xs.length - 1] = v`
/// assigns, `xs.at(-1) = v` does not parse.
pub(super) fn element_access_node(
    mem: &ComputedMemberExpression,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    let mut node = make_node(if mem.optional { "?.[]" } else { "[]" }, "ComputedMemberExpression");
    if let Some(obj_node) = expression_to_tree_node(&mem.object, cx) {
        node.add_child(obj_node);
    }
    if let Some(prop_node) = expression_to_tree_node(&mem.expression, cx) {
        node.add_child(prop_node);
    }
    Rc::new(node)
}

/// `object.#field` / `object?.#field`.
pub(super) fn private_field_node(
    mem: &PrivateFieldExpression,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    let mut node = make_node(if mem.optional { "?.#" } else { ".#" }, "PrivateFieldExpression");
    if let Some(obj_node) = expression_to_tree_node(&mem.object, cx) {
        node.add_child(obj_node);
    }
    node.add_child(leaf(&format!("#{}", mem.field.name.as_str()), "PrivateIdentifier"));
    Rc::new(node)
}

/// Canonical mode: `arr[arr.length - 1]` ⇔ `arr.at(-1)` for a pure `arr` —
/// identical values including the out-of-bounds/`undefined` case. The
/// canonical form is the `.at(-1)` call.
fn last_element_as_at_call(
    mem: &ComputedMemberExpression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    if !cx.canonical || mem.optional || !is_side_effect_free(&mem.object) {
        return None;
    }
    let Expression::BinaryExpression(bin) = strip_parentheses(&mem.expression) else {
        return None;
    };
    let (Expression::StaticMemberExpression(length_member), Expression::NumericLiteral(one)) =
        (strip_parentheses(&bin.left), strip_parentheses(&bin.right))
    else {
        return None;
    };
    let is_last_index = bin.operator == BinaryOperator::Subtraction
        && (one.value - 1.0).abs() < f64::EPSILON
        && !length_member.optional
        && length_member.property.name.as_str() == "length"
        && pure_expressions_equal(&mem.object, &length_member.object);
    if !is_last_index {
        return None;
    }

    let mut member = make_node(".", "StaticMemberExpression");
    if let Some(object) = expression_to_tree_node(&mem.object, cx) {
        member.add_child(object);
    }
    member.add_child(leaf("at", "Identifier"));
    let mut minus_one = make_node("UnaryNegation", "UnaryExpression");
    minus_one.add_child(leaf("1", "NumericLiteral"));
    let mut call = make_node("CallExpression", "CallExpression");
    call.add_child(Rc::new(member));
    call.add_child(Rc::new(minus_one));
    Some(Rc::new(call))
}

/// `callee(arguments)` / `callee?.(arguments)`.
pub(super) fn call_expression_node(call: &CallExpression, cx: &ConversionContext) -> Rc<TreeNode> {
    if cx.canonical {
        if let Some(node) = lower_object_assign_to_spread(call, cx) {
            return node;
        }
        // `Boolean(x)` ⇔ `!!x` — same value for every input. Canonical
        // form is the double negation.
        if let Some(argument) = boolean_call_argument(call, cx) {
            let mut inner = make_node("LogicalNot", "UnaryExpression");
            if let Some(child) = expression_to_tree_node(argument, cx) {
                inner.add_child(child);
            }
            let mut outer = make_node("LogicalNot", "UnaryExpression");
            outer.add_child(Rc::new(inner));
            return Rc::new(outer);
        }
    }
    let mut node =
        make_node(if call.optional { "?.()" } else { "CallExpression" }, "CallExpression");
    if let Some(callee_node) = expression_to_tree_node(&call.callee, cx) {
        node.add_child(callee_node);
    }
    for arg in &call.arguments {
        if let Some(arg_node) = argument_to_tree_node(arg, cx) {
            node.add_child(arg_node);
        }
    }
    Rc::new(node)
}

pub(super) fn argument_to_tree_node(
    arg: &Argument,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match arg {
        Argument::SpreadElement(spread) => {
            let mut node = make_node("SpreadElement", "SpreadElement");
            if let Some(child) = expression_to_tree_node(&spread.argument, cx) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        _ => arg.as_expression().and_then(|expr| expression_to_tree_node(expr, cx)),
    }
}

pub(super) fn array_element_to_tree_node(
    elem: &ArrayExpressionElement,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match elem {
        ArrayExpressionElement::SpreadElement(spread) => {
            let mut node = make_node("SpreadElement", "SpreadElement");
            if let Some(child) = expression_to_tree_node(&spread.argument, cx) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        ArrayExpressionElement::Elision(_) => Some(leaf("Elision", "Elision")),
        _ => elem.as_expression().and_then(|expr| expression_to_tree_node(expr, cx)),
    }
}

pub(super) fn object_property_to_tree_node(
    prop: &ObjectPropertyKind,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match prop {
        ObjectPropertyKind::ObjectProperty(p) => {
            let label = member_label("ObjectProperty", &p.key, p.computed);
            let mut node = make_node(label, "ObjectProperty");
            if let Some(key_node) = property_key_to_tree_node(&p.key, cx) {
                node.add_child(key_node);
            }
            // A shorthand property (`{ id }`) has its value node too: the
            // same tree as `{ id: id }`.
            if let Some(val_node) = expression_to_tree_node(&p.value, cx) {
                node.add_child(val_node);
            }
            Some(stamp(Rc::new(node), p.span))
        }
        ObjectPropertyKind::SpreadProperty(spread) => {
            let mut node = make_node("SpreadProperty", "SpreadProperty");
            if let Some(child) = expression_to_tree_node(&spread.argument, cx) {
                node.add_child(child);
            }
            Some(stamp(Rc::new(node), spread.span))
        }
    }
}

/// The label of an object-literal or destructuring member: `static_label`,
/// or `ComputedProperty` when the key is an expression evaluated at run
/// time. `{ [id]: value }` reads the variable `id` for its key and
/// `{ id: value }` does not, yet both keys convert to an identifier named
/// `id`; the label keeps the two members apart. A literal in brackets
/// (`{ ["id"]: value }`) is the static key it spells.
pub(super) fn member_label(
    static_label: &'static str,
    key: &PropertyKey,
    computed: bool,
) -> &'static str {
    let is_literal_key = matches!(
        key,
        PropertyKey::StringLiteral(_)
            | PropertyKey::NumericLiteral(_)
            | PropertyKey::BigIntLiteral(_)
    );
    if computed && !is_literal_key {
        "ComputedProperty"
    } else {
        static_label
    }
}

pub(super) fn property_key_to_tree_node(
    key: &PropertyKey,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match key {
        PropertyKey::StaticIdentifier(ident) => Some(leaf(ident.name.as_str(), "Identifier")),
        PropertyKey::PrivateIdentifier(ident) => {
            Some(leaf(&format!("#{}", ident.name.as_str()), "PrivateIdentifier"))
        }
        _ => key.as_expression().and_then(|expr| expression_to_tree_node(expr, cx)),
    }
}

pub(super) fn assignment_target_to_tree_node(
    target: &AssignmentTarget,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    Some(match target {
        AssignmentTarget::AssignmentTargetIdentifier(ident) => {
            leaf(&cx.identifier_label(ident.span, &ident.name), "Identifier")
        }
        AssignmentTarget::ArrayAssignmentTarget(_) => {
            leaf("ArrayAssignmentTarget", "ArrayAssignmentTarget")
        }
        AssignmentTarget::ObjectAssignmentTarget(_) => {
            leaf("ObjectAssignmentTarget", "ObjectAssignmentTarget")
        }
        AssignmentTarget::StaticMemberExpression(mem) => static_member_node(mem, cx),
        AssignmentTarget::ComputedMemberExpression(mem) => element_access_node(mem, cx),
        AssignmentTarget::PrivateFieldExpression(mem) => private_field_node(mem, cx),
        _ => leaf("AssignmentTarget", "AssignmentTarget"),
    })
}

pub(super) fn simple_assignment_target_to_tree_node(
    target: &SimpleAssignmentTarget,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    Some(match target {
        SimpleAssignmentTarget::AssignmentTargetIdentifier(ident) => {
            leaf(&cx.identifier_label(ident.span, &ident.name), "Identifier")
        }
        SimpleAssignmentTarget::StaticMemberExpression(mem) => static_member_node(mem, cx),
        SimpleAssignmentTarget::ComputedMemberExpression(mem) => element_access_node(mem, cx),
        SimpleAssignmentTarget::PrivateFieldExpression(mem) => private_field_node(mem, cx),
        _ => leaf("SimpleAssignmentTarget", "SimpleAssignmentTarget"),
    })
}

/// The expression an optional chain (`a?.b.c()`) wraps. Each link carries
/// its own optionality: in `a?.b.c` only the first access is optional, and
/// in `a?.run()` the call is not. (Every link of a chain used to be
/// labelled optional, so `a?.b.c` and `a?.b?.c`, or `a?.run()` and
/// `a?.run?.()`, converted to the same tree.)
pub(super) fn chain_element_to_tree_node(
    elem: &ChainElement,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match elem {
        ChainElement::CallExpression(call) => Some(call_expression_node(call, cx)),
        ChainElement::StaticMemberExpression(mem) => Some(static_member_node(mem, cx)),
        ChainElement::ComputedMemberExpression(mem) => Some(computed_member_node(mem, cx)),
        ChainElement::PrivateFieldExpression(mem) => Some(private_field_node(mem, cx)),
        // `a?.b!`: the assertion is erased, like every other one.
        ChainElement::TSNonNullExpression(ts) => expression_to_tree_node(&ts.expression, cx),
    }
}
