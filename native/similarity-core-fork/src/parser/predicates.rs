//! Predicates over the AST shared by the value-preserving
//! canonicalizations.

use oxc_ast::ast::{
    BinaryOperator, CallExpression, Expression, LogicalOperator, Statement, UnaryOperator,
};

use super::{strip_parentheses, ConversionContext};

/// Side-effect-free expressions the canonicalizations may duplicate or
/// merge: literals, identifiers, `this`, and non-optional static member
/// chains of those. Property getters can technically observe reads — that
/// caveat is accepted by design, mirroring the common lint rewrites these
/// canonicalizations model.
pub(super) fn is_side_effect_free(expr: &Expression) -> bool {
    match strip_parentheses(expr) {
        Expression::Identifier(_)
        | Expression::ThisExpression(_)
        | Expression::StringLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_) => true,
        Expression::StaticMemberExpression(member) => {
            !member.optional && is_side_effect_free(&member.object)
        }
        _ => false,
    }
}

/// Structural equality of two side-effect-free expressions: identifiers
/// by name, literals by value, static member chains memberwise. Used to
/// recognize `x ? x : y`, `arr[arr.length - 1]`, and the null-test pairs.
pub(super) fn pure_expressions_equal(a: &Expression, b: &Expression) -> bool {
    use Expression as E;
    match (strip_parentheses(a), strip_parentheses(b)) {
        (E::Identifier(x), E::Identifier(y)) => x.name == y.name,
        (E::ThisExpression(_), E::ThisExpression(_)) => true,
        (E::NullLiteral(_), E::NullLiteral(_)) => true,
        (E::StringLiteral(x), E::StringLiteral(y)) => x.value == y.value,
        (E::NumericLiteral(x), E::NumericLiteral(y)) => x.value == y.value,
        (E::BooleanLiteral(x), E::BooleanLiteral(y)) => x.value == y.value,
        (E::StaticMemberExpression(x), E::StaticMemberExpression(y)) => {
            !x.optional
                && !y.optional
                && x.property.name == y.property.name
                && pure_expressions_equal(&x.object, &y.object)
        }
        _ => false,
    }
}

/// `undefined` (as the global) or `void <pure>` — the two spellings of
/// the undefined value.
pub(super) fn is_undefined_expr(expr: &Expression, cx: &ConversionContext) -> bool {
    match strip_parentheses(expr) {
        Expression::Identifier(ident) => {
            ident.name == "undefined" && !cx.is_locally_bound(ident.span)
        }
        Expression::UnaryExpression(unary) => {
            unary.operator == UnaryOperator::Void && is_side_effect_free(&unary.argument)
        }
        _ => false,
    }
}

pub(super) fn is_null_expr(expr: &Expression) -> bool {
    matches!(strip_parentheses(expr), Expression::NullLiteral(_))
}

/// Literals eligible for the Yoda-order flip on symmetric equality
/// operators (`5 === x` ⇔ `x === 5`).
pub(super) fn is_literal_operand(expr: &Expression, cx: &ConversionContext) -> bool {
    matches!(
        strip_parentheses(expr),
        Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::BigIntLiteral(_)
    ) || is_undefined_expr(expr, cx)
}

/// `subject === null` / `subject === undefined` (or the `!==` forms when
/// `expect_equal` is false), in either operand order, with a
/// side-effect-free subject. Returns the subject and whether the literal
/// side was `null` (vs `undefined`).
pub(super) fn strict_null_comparison<'a, 'b>(
    expr: &'b Expression<'a>,
    expect_equal: bool,
    cx: &ConversionContext,
) -> Option<(&'b Expression<'a>, bool)> {
    let Expression::BinaryExpression(bin) = strip_parentheses(expr) else {
        return None;
    };
    let wanted = if expect_equal {
        BinaryOperator::StrictEquality
    } else {
        BinaryOperator::StrictInequality
    };
    if bin.operator != wanted {
        return None;
    }
    let (subject, literal) = if is_null_expr(&bin.right) || is_undefined_expr(&bin.right, cx) {
        (&bin.left, &bin.right)
    } else if is_null_expr(&bin.left) || is_undefined_expr(&bin.left, cx) {
        (&bin.right, &bin.left)
    } else {
        return None;
    };
    if !is_side_effect_free(subject) {
        return None;
    }
    Some((subject, is_null_expr(literal)))
}

/// A boolean expression that tests "subject is (not) null-or-undefined":
/// the loose forms `x == null` / `x != null` (also spelled against
/// `undefined` — loose equality makes them interchangeable) and the
/// strict pairs `x === null || x === undefined` / `x !== null && x !==
/// undefined`.
pub(super) enum NullishTest<'a, 'b> {
    IsNullish(&'b Expression<'a>),
    NotNullish(&'b Expression<'a>),
}

pub(super) fn classify_nullish_test<'a, 'b>(
    expr: &'b Expression<'a>,
    cx: &ConversionContext,
) -> Option<NullishTest<'a, 'b>> {
    let expr = strip_parentheses(expr);
    if let Expression::BinaryExpression(bin) = expr {
        let equal = match bin.operator {
            BinaryOperator::Equality => true,
            BinaryOperator::Inequality => false,
            _ => return None,
        };
        let subject = if is_null_expr(&bin.right) || is_undefined_expr(&bin.right, cx) {
            &bin.left
        } else if is_null_expr(&bin.left) || is_undefined_expr(&bin.left, cx) {
            &bin.right
        } else {
            return None;
        };
        if !is_side_effect_free(subject) {
            return None;
        }
        return Some(if equal {
            NullishTest::IsNullish(subject)
        } else {
            NullishTest::NotNullish(subject)
        });
    }
    if let Expression::LogicalExpression(log) = expr {
        match log.operator {
            LogicalOperator::Or => {
                let (left_subject, left_is_null) = strict_null_comparison(&log.left, true, cx)?;
                let (right_subject, right_is_null) = strict_null_comparison(&log.right, true, cx)?;
                if left_is_null != right_is_null
                    && pure_expressions_equal(left_subject, right_subject)
                {
                    return Some(NullishTest::IsNullish(left_subject));
                }
            }
            LogicalOperator::And => {
                let (left_subject, left_is_null) = strict_null_comparison(&log.left, false, cx)?;
                let (right_subject, right_is_null) = strict_null_comparison(&log.right, false, cx)?;
                if left_is_null != right_is_null
                    && pure_expressions_equal(left_subject, right_subject)
                {
                    return Some(NullishTest::NotNullish(left_subject));
                }
            }
            LogicalOperator::Coalesce => {}
        }
    }
    None
}

/// The argument of a call to the `Boolean` global with a single plain
/// argument, looking through parentheses.
pub(super) fn match_boolean_call<'a, 'b>(
    expr: &'b Expression<'a>,
    cx: &ConversionContext,
) -> Option<&'b Expression<'a>> {
    let Expression::CallExpression(call) = strip_parentheses(expr) else {
        return None;
    };
    boolean_call_argument(call, cx)
}

/// [`match_boolean_call`] for a call expression.
pub(super) fn boolean_call_argument<'a, 'b>(
    call: &'b CallExpression<'a>,
    cx: &ConversionContext,
) -> Option<&'b Expression<'a>> {
    if call.optional || call.arguments.len() != 1 {
        return None;
    }
    let Expression::Identifier(ident) = strip_parentheses(&call.callee) else {
        return None;
    };
    if ident.name.as_str() != "Boolean" || cx.is_locally_bound(ident.span) {
        return None;
    }
    call.arguments[0].as_expression()
}

/// Whether control flow can never continue past `stmt` (it returns,
/// throws, breaks, continues, or is a block/if that always does). Used to
/// decide when `if (c) { …jump } else { B }` can flatten to
/// `if (c) { …jump } B`.
pub(super) fn statement_always_terminates(stmt: &Statement) -> bool {
    match stmt {
        Statement::ReturnStatement(_)
        | Statement::ThrowStatement(_)
        | Statement::BreakStatement(_)
        | Statement::ContinueStatement(_) => true,
        Statement::BlockStatement(block) => {
            block.body.last().is_some_and(statement_always_terminates)
        }
        Statement::IfStatement(if_stmt) => {
            statement_always_terminates(&if_stmt.consequent)
                && if_stmt.alternate.as_ref().is_some_and(|alt| statement_always_terminates(alt))
        }
        _ => false,
    }
}
