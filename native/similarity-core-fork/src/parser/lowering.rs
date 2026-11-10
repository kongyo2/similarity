//! Canonical statement-level lowerings: the rewrites that turn one
//! statement into the statements it is equivalent to.

use oxc_ast::ast::{
    BinaryOperator, BindingPattern, ConditionalExpression, Expression, FormalParameters,
    PropertyKey, Statement, SwitchStatement, UnaryOperator, VariableDeclaration,
};
use std::rc::Rc;

use super::expressions::{
    expression_to_tree_node, negated_expression_to_tree_node, test_expression_to_tree_node,
};
use super::patterns::binding_pattern_to_tree_node;
use super::peephole::normalize_statement_nodes;
use super::predicates::{
    classify_nullish_test, is_side_effect_free, pure_expressions_equal,
    statement_always_terminates, NullishTest,
};
use super::statements::{
    append_statement_to_list, flatten_statement_into, statement_to_block_node,
    statement_to_tree_nodes,
};
use super::{leaf, make_node, strip_parentheses, ConversionContext};
use crate::tree::TreeNode;

pub(super) enum CallbackBody<'a, 'b> {
    Expression(&'b Expression<'a>),
    Block(&'b oxc_allocator::Vec<'a, Statement<'a>>),
}

pub(super) struct ThenCallParts<'a, 'b> {
    object: &'b Expression<'a>,
    param: Option<&'b BindingPattern<'a>>,
    pub(super) body: CallbackBody<'a, 'b>,
}

pub(super) enum ThenPosition {
    Return,
    Statement,
}

/// Whether a callback's parameter list is at most one plain parameter (and
/// at least `min_params`): no rest parameter and no default value. A
/// default applies when the callback is passed `undefined`, which the
/// declaration a lowering replaces the parameter with would not.
fn is_plain_callback_signature(params: &FormalParameters, min_params: usize) -> bool {
    params.rest.is_none()
        && (min_params..=1).contains(&params.items.len())
        && params.items.iter().all(|param| param.initializer.is_none())
}

/// Match `E.then(callback)` where the callback is an inline arrow or
/// function expression with at most one parameter. Two-argument `then`
/// calls (`onFulfilled, onRejected`) and named-function callbacks stay
/// untouched — only the directly-inlineable single-callback form is
/// equivalent to an `await` sequence.
pub(super) fn match_then_call<'a, 'b>(expr: &'b Expression<'a>) -> Option<ThenCallParts<'a, 'b>> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    if call.optional || call.arguments.len() != 1 {
        return None;
    }
    let Expression::StaticMemberExpression(member) = strip_parentheses(&call.callee) else {
        return None;
    };
    if member.optional || member.property.name.as_str() != "then" {
        return None;
    }
    let callback = call.arguments[0].as_expression().map(strip_parentheses)?;
    let (param, body) = match callback {
        Expression::ArrowFunctionExpression(arrow) => {
            if !is_plain_callback_signature(&arrow.params, 0) {
                return None;
            }
            let body = if arrow.expression {
                let Some(Statement::ExpressionStatement(expr_stmt)) = arrow.body.statements.first()
                else {
                    return None;
                };
                CallbackBody::Expression(&expr_stmt.expression)
            } else {
                CallbackBody::Block(&arrow.body.statements)
            };
            (arrow.params.items.first().map(|p| &p.pattern), body)
        }
        Expression::FunctionExpression(func) => {
            if func.generator || !is_plain_callback_signature(&func.params, 0) {
                return None;
            }
            let body = func.body.as_ref()?;
            (func.params.items.first().map(|p| &p.pattern), CallbackBody::Block(&body.statements))
        }
        _ => return None,
    };
    Some(ThenCallParts { object: &member.object, param, body })
}

/// Whether any statement in the list is a `return` belonging to the
/// list's own function scope. Recurses through blocks, branches, loops,
/// switch cases, try clauses and labels, but not into nested functions,
/// arrows or classes — their `return`s exit the nested scope, not this
/// one.
pub(super) fn statements_contain_own_return<'a>(
    statements: &oxc_allocator::Vec<'a, Statement<'a>>,
) -> bool {
    statements.iter().any(statement_contains_own_return)
}

pub(super) fn statement_contains_own_return(stmt: &Statement) -> bool {
    match stmt {
        Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(block) => statements_contain_own_return(&block.body),
        Statement::IfStatement(if_stmt) => {
            statement_contains_own_return(&if_stmt.consequent)
                || if_stmt.alternate.as_ref().is_some_and(statement_contains_own_return)
        }
        Statement::ForStatement(for_stmt) => statement_contains_own_return(&for_stmt.body),
        Statement::ForInStatement(for_in) => statement_contains_own_return(&for_in.body),
        Statement::ForOfStatement(for_of) => statement_contains_own_return(&for_of.body),
        Statement::WhileStatement(while_stmt) => statement_contains_own_return(&while_stmt.body),
        Statement::DoWhileStatement(do_stmt) => statement_contains_own_return(&do_stmt.body),
        Statement::SwitchStatement(switch_stmt) => {
            switch_stmt.cases.iter().any(|case| statements_contain_own_return(&case.consequent))
        }
        Statement::TryStatement(try_stmt) => {
            statements_contain_own_return(&try_stmt.block.body)
                || try_stmt
                    .handler
                    .as_ref()
                    .is_some_and(|handler| statements_contain_own_return(&handler.body.body))
                || try_stmt
                    .finalizer
                    .as_ref()
                    .is_some_and(|finalizer| statements_contain_own_return(&finalizer.body))
        }
        Statement::LabeledStatement(labeled) => statement_contains_own_return(&labeled.body),
        Statement::WithStatement(with_stmt) => statement_contains_own_return(&with_stmt.body),
        _ => false,
    }
}

/// Lower `return E.then((v) => body)` (or the bare-statement form) into
/// the equivalent `await` sequence:
///
/// ```text
/// const v = await E;   // or `await E;` when the callback takes no value
/// …body…               // expression bodies become `return expr` / `expr;`
/// ```
pub(super) fn lower_then_call(
    parts: &ThenCallParts,
    position: ThenPosition,
    cx: &ConversionContext,
) -> Vec<Rc<TreeNode>> {
    let mut nodes = Vec::new();

    let await_node = {
        let mut node = make_node("AwaitExpression", "AwaitExpression");
        if let Some(object_node) = expression_to_tree_node(parts.object, cx) {
            node.add_child(object_node);
        }
        Rc::new(node)
    };

    if let Some(pattern) = parts.param {
        let mut declarator = make_node("VariableDeclarator", "VariableDeclarator");
        if let Some(binding) = binding_pattern_to_tree_node(pattern, cx) {
            declarator.add_child(binding);
        }
        declarator.add_child(await_node);
        let mut decl = make_node("ConstDeclaration", "VariableDeclaration");
        decl.add_child(Rc::new(declarator));
        nodes.push(Rc::new(decl));
    } else {
        let mut stmt = make_node("ExpressionStatement", "ExpressionStatement");
        stmt.add_child(await_node);
        nodes.push(Rc::new(stmt));
    }

    match &parts.body {
        CallbackBody::Expression(expr) => {
            let label = match position {
                ThenPosition::Return => "ReturnStatement",
                ThenPosition::Statement => "ExpressionStatement",
            };
            let mut stmt = make_node(label, label);
            if let Some(child) = expression_to_tree_node(expr, cx) {
                stmt.add_child(child);
            }
            nodes.push(Rc::new(stmt));
        }
        CallbackBody::Block(statements) => {
            for stmt in statements.iter() {
                nodes.extend(statement_to_tree_nodes(stmt, cx));
            }
        }
    }

    nodes
}

/// Split a ternary's pieces after negation normalization: a negated test
/// (`!c ? a : b`, `x !== y ? a : b`) swaps its branches so the emitted
/// `if` shows the positive condition, matching what [`canonical_if_nodes`]
/// does for hand-written `if (!c) { … } else { … }`.
pub(super) struct NormalizedTernary<'a, 'b> {
    /// Converts the (positive-form) test into a tree.
    test_was_negated_binary: bool,
    test: &'b Expression<'a>,
    consequent: &'b Expression<'a>,
    alternate: &'b Expression<'a>,
}

impl NormalizedTernary<'_, '_> {
    fn render_test(&self, cx: &ConversionContext) -> Option<Rc<TreeNode>> {
        if self.test_was_negated_binary {
            // The swap flipped `!==`/`!=` to its positive complement.
            negated_expression_to_tree_node(self.test, cx)
        } else {
            test_expression_to_tree_node(self.test, cx)
        }
    }
}

pub(super) fn normalize_ternary<'a, 'b>(
    cond: &'b ConditionalExpression<'a>,
) -> NormalizedTernary<'a, 'b> {
    let test = strip_parentheses(&cond.test);
    let consequent = strip_parentheses(&cond.consequent);
    let alternate = strip_parentheses(&cond.alternate);
    match test {
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
            NormalizedTernary {
                test_was_negated_binary: false,
                test: strip_parentheses(&unary.argument),
                consequent: alternate,
                alternate: consequent,
            }
        }
        Expression::BinaryExpression(bin)
            if matches!(
                bin.operator,
                BinaryOperator::StrictInequality | BinaryOperator::Inequality
            ) =>
        {
            NormalizedTernary {
                test_was_negated_binary: true,
                test,
                consequent: alternate,
                alternate: consequent,
            }
        }
        _ => NormalizedTernary { test_was_negated_binary: false, test, consequent, alternate },
    }
}

/// Lower a ternary into an `if`/`else` chain. `make_leaf_statement`
/// builds the statement that consumes each branch value (an assignment);
/// nested ternaries in the alternate position become `else if` arms,
/// mirroring how the explicit chain is written by hand.
pub(super) fn lower_ternary_chain(
    cond: &ConditionalExpression,
    make_leaf_statement: &dyn Fn(&Expression) -> Rc<TreeNode>,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    let normalized = normalize_ternary(cond);
    let mut if_node = make_node("IfStatement", "IfStatement");
    if let Some(test_node) = normalized.render_test(cx) {
        if_node.add_child(test_node);
    }

    let mut consequent_block = make_node("BlockStatement", "BlockStatement");
    consequent_block.add_child(make_leaf_statement(normalized.consequent));
    if_node.add_child(Rc::new(consequent_block));

    let alternate = normalized.alternate;
    if let Expression::ConditionalExpression(nested) = alternate {
        if_node.add_child(lower_ternary_chain(nested, make_leaf_statement, cx));
    } else {
        let mut alternate_block = make_node("BlockStatement", "BlockStatement");
        alternate_block.add_child(make_leaf_statement(alternate));
        if_node.add_child(Rc::new(alternate_block));
    }
    Rc::new(if_node)
}

/// Canonical conversion of `return <argument>`: strips `return await X`
/// down to `return X`, inlines single-callback `.then()` chains, and
/// lowers ternary returns into the `if`/`else` chain shape. Shared by the
/// statement converter and the expression-bodied-arrow wrapper so
/// `(x) => expr` and `function (x) { return expr; }` produce identical
/// body trees.
pub(super) fn canonical_return_nodes(
    argument: Option<&Expression>,
    cx: &ConversionContext,
) -> Vec<Rc<TreeNode>> {
    let Some(argument) = argument else {
        return vec![leaf("ReturnStatement", "ReturnStatement")];
    };
    // `return await X` and `return X` resolve to the same value from the
    // caller's perspective; strip the await so the two spellings align.
    let mut effective = strip_parentheses(argument);
    if let Expression::AwaitExpression(await_expr) = effective {
        effective = strip_parentheses(&await_expr.argument);
    }
    if let Some(parts) = match_then_call(effective) {
        return lower_then_call(&parts, ThenPosition::Return, cx);
    }
    if let Expression::ConditionalExpression(cond) = effective {
        return lower_return_ternary(cond, cx);
    }
    let mut node = make_node("ReturnStatement", "ReturnStatement");
    if let Some(arg_node) = expression_to_tree_node(effective, cx) {
        node.add_child(arg_node);
    }
    vec![Rc::new(node)]
}

/// Lower `return c ? a : b` into the flat guard form
/// `if (c) { return a; } return b;` — the same shape hand-written
/// early-return code and the `if`/`else` spelling (after jump
/// flattening) normalize to. Branch values recurse through
/// [`canonical_return_nodes`], so nested ternaries flatten into
/// successive guards.
pub(super) fn lower_return_ternary(
    cond: &ConditionalExpression,
    cx: &ConversionContext,
) -> Vec<Rc<TreeNode>> {
    let normalized = normalize_ternary(cond);
    let mut if_node = make_node("IfStatement", "IfStatement");
    if let Some(test_node) = normalized.render_test(cx) {
        if_node.add_child(test_node);
    }
    let mut consequent_block = make_node("BlockStatement", "BlockStatement");
    for node in canonical_return_nodes(Some(normalized.consequent), cx) {
        consequent_block.add_child(node);
    }
    if_node.add_child(Rc::new(consequent_block));

    let mut nodes = vec![Rc::new(if_node)];
    nodes.extend(canonical_return_nodes(Some(normalized.alternate), cx));
    nodes
}

pub(super) fn assignment_leaf_statement(
    target_node: Option<Rc<TreeNode>>,
    expr: &Expression,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    let mut assign = make_node("Assign", "AssignmentExpression");
    if let Some(target) = target_node {
        assign.add_child(target);
    }
    if let Some(value) = expression_to_tree_node(expr, cx) {
        assign.add_child(value);
    }
    let mut stmt = make_node("ExpressionStatement", "ExpressionStatement");
    stmt.add_child(Rc::new(assign));
    Rc::new(stmt)
}

/// Lower `const x = c ? a : b;` into `let x; if (c) { x = a } else { x = b }`
/// so it lines up with the hand-written `let`-plus-branches form. Only
/// single-declarator declarations binding a plain identifier qualify.
pub(super) fn lower_declaration_ternary(
    var_decl: &VariableDeclaration,
    cx: &ConversionContext,
) -> Option<Vec<Rc<TreeNode>>> {
    if var_decl.declarations.len() != 1 {
        return None;
    }
    let declarator = &var_decl.declarations[0];
    let BindingPattern::BindingIdentifier(ident) = &declarator.id else {
        return None;
    };
    let init = declarator.init.as_ref()?;
    let Expression::ConditionalExpression(cond) = strip_parentheses(init) else {
        return None;
    };
    // Ternaries that contract to `||` / `&&` / `??` sugar are handled at
    // the expression level; lowering them to `if`/`else` here would hide
    // that equivalence.
    {
        let test = strip_parentheses(&cond.test);
        let consequent = strip_parentheses(&cond.consequent);
        let alternate = strip_parentheses(&cond.alternate);
        match classify_nullish_test(test, cx) {
            Some(NullishTest::IsNullish(subject)) if pure_expressions_equal(subject, alternate) => {
                return None;
            }
            Some(NullishTest::NotNullish(subject))
                if pure_expressions_equal(subject, consequent) =>
            {
                return None;
            }
            _ => {}
        }
        if is_side_effect_free(test)
            && (pure_expressions_equal(test, consequent) || pure_expressions_equal(test, alternate))
        {
            return None;
        }
    }

    // The lowered binding is mutated by the branches, so it canonicalizes
    // to `let` regardless of the source keyword.
    let name = cx.identifier_label(ident.span, &ident.name);
    let mut decl = make_node("LetDeclaration", "VariableDeclaration");
    let mut declarator_node = make_node("VariableDeclarator", "VariableDeclarator");
    declarator_node.add_child(leaf(&name, "BindingIdentifier"));
    decl.add_child(Rc::new(declarator_node));

    let if_node = lower_ternary_chain(
        cond,
        &|expr| assignment_leaf_statement(Some(leaf(&name, "Identifier")), expr, cx),
        cx,
    );

    Some(vec![Rc::new(decl), if_node])
}

/// Lower `xs.forEach((x) => { … })` in statement position into
/// `for (const x of xs) { … }`. Only the single-simple-parameter inline
/// callback qualifies — index-taking callbacks observe positions and are
/// not equivalent to a plain `for-of`.
pub(super) fn lower_foreach_statement(
    expr: &Expression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    if call.optional || call.arguments.len() != 1 {
        return None;
    }
    let Expression::StaticMemberExpression(member) = strip_parentheses(&call.callee) else {
        return None;
    };
    if member.optional || member.property.name.as_str() != "forEach" {
        return None;
    }
    let callback = call.arguments[0].as_expression().map(strip_parentheses)?;
    let (param_name, body) = match callback {
        Expression::ArrowFunctionExpression(arrow) if !arrow.r#async => {
            if !is_plain_callback_signature(&arrow.params, 1) {
                return None;
            }
            let BindingPattern::BindingIdentifier(ident) = &arrow.params.items[0].pattern else {
                return None;
            };
            let body = if arrow.expression {
                let Some(Statement::ExpressionStatement(expr_stmt)) = arrow.body.statements.first()
                else {
                    return None;
                };
                CallbackBody::Expression(&expr_stmt.expression)
            } else {
                CallbackBody::Block(&arrow.body.statements)
            };
            (cx.identifier_label(ident.span, &ident.name), body)
        }
        Expression::FunctionExpression(func) if !func.r#async && !func.generator => {
            if !is_plain_callback_signature(&func.params, 1) {
                return None;
            }
            let BindingPattern::BindingIdentifier(ident) = &func.params.items[0].pattern else {
                return None;
            };
            let body = func.body.as_ref()?;
            (cx.identifier_label(ident.span, &ident.name), CallbackBody::Block(&body.statements))
        }
        _ => return None,
    };

    let mut for_node = make_node("ForOfStatement", "ForOfStatement");
    let mut decl = make_node("ConstDeclaration", "VariableDeclaration");
    let mut declarator = make_node("VariableDeclarator", "VariableDeclarator");
    declarator.add_child(leaf(&param_name, "BindingIdentifier"));
    decl.add_child(Rc::new(declarator));
    for_node.add_child(Rc::new(decl));
    if let Some(iterable) = expression_to_tree_node(&member.object, cx) {
        for_node.add_child(iterable);
    }
    let mut block = make_node("BlockStatement", "BlockStatement");
    match body {
        CallbackBody::Expression(expr) => {
            let mut stmt = make_node("ExpressionStatement", "ExpressionStatement");
            if let Some(child) = expression_to_tree_node(expr, cx) {
                stmt.add_child(child);
            }
            block.add_child(Rc::new(stmt));
        }
        CallbackBody::Block(statements) => {
            for stmt in statements.iter() {
                append_statement_to_list(&mut block, stmt, cx);
            }
        }
    }
    for_node.add_child(Rc::new(block));
    Some(Rc::new(for_node))
}

pub(super) fn is_switch_jump_statement(stmt: &Statement) -> bool {
    matches!(
        stmt,
        Statement::BreakStatement(_)
            | Statement::ReturnStatement(_)
            | Statement::ThrowStatement(_)
            | Statement::ContinueStatement(_)
    )
}

/// Case body minus the trailing unlabeled `break` (the switch's own exit,
/// which has no analogue in an if-chain; labeled breaks target an outer
/// statement and must stay).
pub(super) fn switch_case_statements<'a, 'b>(
    case: &'b oxc_ast::ast::SwitchCase<'a>,
) -> Vec<&'b Statement<'a>> {
    let mut statements: Vec<&Statement> = case.consequent.iter().collect();
    if let Some(Statement::BreakStatement(break_stmt)) = statements.last() {
        if break_stmt.label.is_none() {
            statements.pop();
        }
    }
    statements
}

/// Lower a `switch` whose cases all end in a jump (no fallthrough, default
/// last) into the equivalent `if (disc === t1) {…}` chain — flat guards
/// for cases whose bodies exit the function (`return`/`throw`/`continue`),
/// mirroring what [`canonical_if_nodes`] does to hand-written chains, and
/// a nested `if`/`else` tail for the remainder. Returns `None` — leaving
/// the literal switch shape — whenever the rewrite wouldn't be
/// behavior-preserving.
pub(super) fn lower_switch_statement(
    switch_stmt: &SwitchStatement,
    cx: &ConversionContext,
) -> Option<Vec<Rc<TreeNode>>> {
    let cases = &switch_stmt.cases;
    if cases.is_empty() || cases.iter().all(|case| case.test.is_none()) {
        return None;
    }
    for (index, case) in cases.iter().enumerate() {
        let is_last = index == cases.len() - 1;
        // A non-last default would reorder evaluation; bail out.
        if case.test.is_none() && !is_last {
            return None;
        }
        // Empty consequents group several labels onto one body
        // (fallthrough), and non-jump endings fall through too.
        if !is_last {
            let last_stmt = case.consequent.last()?;
            if !is_switch_jump_statement(last_stmt) {
                return None;
            }
        }
    }

    let case_body_block =
        |case: &oxc_ast::ast::SwitchCase, cx: &ConversionContext| -> Rc<TreeNode> {
            let mut block = make_node("BlockStatement", "BlockStatement");
            let mut statements = Vec::new();
            for stmt in switch_case_statements(case) {
                statements.extend(statement_to_tree_nodes(stmt, cx));
            }
            normalize_statement_nodes(&mut statements, cx);
            for child in statements {
                block.add_child(child);
            }
            Rc::new(block)
        };

    let guard_node = |case: &oxc_ast::ast::SwitchCase,
                      test: &Expression,
                      cx: &ConversionContext|
     -> Rc<TreeNode> {
        let mut if_node = make_node("IfStatement", "IfStatement");
        let mut equality = make_node("StrictEquality", "BinaryExpression");
        if let Some(disc) = expression_to_tree_node(&switch_stmt.discriminant, cx) {
            equality.add_child(disc);
        }
        if let Some(test_node) = expression_to_tree_node(test, cx) {
            equality.add_child(test_node);
        }
        if_node.add_child(Rc::new(equality));
        if_node.add_child(case_body_block(case, cx));
        Rc::new(if_node)
    };

    // Whether a case body (after break-stripping) still always exits —
    // those cases become flat guards; break-terminated ones must keep the
    // else-chain so control flow past the switch stays represented.
    let case_flattens = |case: &oxc_ast::ast::SwitchCase| -> bool {
        switch_case_statements(case).last().is_some_and(|stmt| statement_always_terminates(stmt))
    };

    let mut nodes: Vec<Rc<TreeNode>> = Vec::new();
    let mut index = 0;
    while index < cases.len() {
        let case = &cases[index];
        match &case.test {
            None => {
                // Validated to be last: the default body continues after
                // the guards, exactly like a hand-written chain's tail.
                let statements = switch_case_statements(case);
                let mut converted = Vec::new();
                for stmt in statements {
                    converted.extend(statement_to_tree_nodes(stmt, cx));
                }
                normalize_statement_nodes(&mut converted, cx);
                nodes.extend(converted);
                index += 1;
            }
            Some(test) if case_flattens(case) => {
                nodes.push(guard_node(case, test, cx));
                index += 1;
            }
            Some(_) => {
                // A break-terminated case: this and every remaining case
                // form one nested if/else chain (matching the shape the
                // equivalent hand-written chain keeps, since its branches
                // don't exit).
                let mut else_node: Option<Rc<TreeNode>> = None;
                for case in cases[index..].iter().rev() {
                    match &case.test {
                        None => {
                            else_node = Some(case_body_block(case, cx));
                        }
                        Some(test) => {
                            let if_node = guard_node(case, test, cx);
                            // `guard_node` returns an if without an else;
                            // graft the accumulated alternate on.
                            let mut if_inner = (*if_node).clone();
                            if let Some(alternate) = else_node.take() {
                                if_inner.add_child(alternate);
                            }
                            else_node = Some(Rc::new(if_inner));
                        }
                    }
                }
                if let Some(chain) = else_node {
                    nodes.push(chain);
                }
                index = cases.len();
            }
        }
    }
    Some(nodes)
}

/// Canonical conversion for `if` statements. Two normalizations compose
/// here so every guard-style spelling of the same branch logic lands on
/// one shape:
///
///   * negation swap — `if (!c) { A } else { B }` renders as
///     `if (c) { B } else { A }`, and `if (a !== b) …` flips to the `===`
///     form with swapped branches (exact complements; ordering operators
///     are left alone because of NaN);
///   * jump flattening — when one branch always exits (`return`/`throw`/
///     `break`/`continue`), the other branch hoists out of the `else`:
///     `if (c) { return x; } else { rest… }` ⇔
///     `if (c) { return x; } rest…` ⇔ (via the ternary lowering)
///     `return c ? x : …`.
pub(super) fn canonical_if_nodes(
    if_stmt: &oxc_ast::ast::IfStatement,
    cx: &ConversionContext,
) -> Vec<Rc<TreeNode>> {
    let mut consequent: &Statement = &if_stmt.consequent;
    let mut alternate: Option<&Statement> = if_stmt.alternate.as_ref();
    let mut test: &Expression = strip_parentheses(&if_stmt.test);

    /// How the source spelled the (now positive-form) test, so the
    /// renderers know whether "positive" means "as-is", "the unwrapped
    /// operand", or "the flipped complement". Deriving this later by
    /// re-matching the test's shape is WRONG: `!(a > b)` unwraps to a
    /// binary expression and would masquerade as the flipped-equality
    /// case.
    enum NegationStyle {
        Plain,
        /// `if (!x) …` — `test` already holds the unwrapped operand.
        Unwrapped,
        /// `if (a !== b) …` — `test` still holds the source binary; the
        /// positive form is its flipped complement.
        FlippedBinary,
    }

    let mut negation = NegationStyle::Plain;

    // Negation swap (only meaningful when there is an alternate to swap
    // with). One unwrap is enough: `!!c` in test position is already
    // simplified by `test_expression_to_tree_node`.
    if let Some(alt) = alternate {
        let negated_form = match test {
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
                Some((strip_parentheses(&unary.argument), NegationStyle::Unwrapped))
            }
            Expression::BinaryExpression(bin)
                if matches!(
                    bin.operator,
                    BinaryOperator::StrictInequality | BinaryOperator::Inequality
                ) =>
            {
                Some((test, NegationStyle::FlippedBinary))
            }
            _ => None,
        };
        if let Some((positive, style)) = negated_form {
            test = positive;
            negation = style;
            let previous_consequent = consequent;
            consequent = alt;
            alternate = Some(previous_consequent);
        }
    }

    // Rendering helpers: `positive` is the test as the swapped `if`
    // should show it; `negated` is its exact complement.
    let render_positive = |cx: &ConversionContext| -> Option<Rc<TreeNode>> {
        match negation {
            NegationStyle::FlippedBinary => negated_expression_to_tree_node(test, cx),
            NegationStyle::Plain | NegationStyle::Unwrapped => {
                test_expression_to_tree_node(test, cx)
            }
        }
    };
    let render_negated = |cx: &ConversionContext| -> Option<Rc<TreeNode>> {
        match negation {
            NegationStyle::FlippedBinary => test_expression_to_tree_node(test, cx),
            NegationStyle::Plain | NegationStyle::Unwrapped => {
                negated_expression_to_tree_node(test, cx)
            }
        }
    };

    if let Some(alt) = alternate {
        if statement_always_terminates(consequent) {
            let mut if_node = make_node("IfStatement", "IfStatement");
            if let Some(test_node) = render_positive(cx) {
                if_node.add_child(test_node);
            }
            if let Some(cons_node) = statement_to_block_node(consequent, cx) {
                if_node.add_child(cons_node);
            }
            let mut nodes = vec![Rc::new(if_node)];
            flatten_statement_into(&mut nodes, alt, cx);
            return nodes;
        }
        if statement_always_terminates(alt) {
            let mut if_node = make_node("IfStatement", "IfStatement");
            if let Some(test_node) = render_negated(cx) {
                if_node.add_child(test_node);
            }
            if let Some(alt_node) = statement_to_block_node(alt, cx) {
                if_node.add_child(alt_node);
            }
            let mut nodes = vec![Rc::new(if_node)];
            flatten_statement_into(&mut nodes, consequent, cx);
            return nodes;
        }
    }

    // No flattening possible: plain if/else, with `else { if … }` blocks
    // unwrapped so they align with bare `else if` chains.
    let mut node = make_node("IfStatement", "IfStatement");
    if let Some(test_node) = render_positive(cx) {
        node.add_child(test_node);
    }
    if let Some(cons_node) = statement_to_block_node(consequent, cx) {
        node.add_child(cons_node);
    }
    if let Some(alt) = alternate {
        let effective_alt: &Statement = match alt {
            Statement::BlockStatement(block)
                if block.body.len() == 1
                    && matches!(block.body.first(), Some(Statement::IfStatement(_))) =>
            {
                block.body.first().map_or(alt, |inner| inner)
            }
            _ => alt,
        };
        if let Statement::IfStatement(inner_if) = effective_alt {
            let mut alt_nodes = canonical_if_nodes(inner_if, cx);
            if alt_nodes.len() == 1 {
                if let Some(single) = alt_nodes.pop() {
                    node.add_child(single);
                }
            } else {
                // The nested chain flattened into several statements —
                // hold them in a block, mirroring how the explicit
                // `else { … }` spelling converts.
                let mut block = make_node("BlockStatement", "BlockStatement");
                for alt_node in alt_nodes {
                    block.add_child(alt_node);
                }
                node.add_child(Rc::new(block));
            }
        } else if let Some(alt_node) = statement_to_block_node(effective_alt, cx) {
            node.add_child(alt_node);
        }
    }
    vec![Rc::new(node)]
}

/// Lower `const { a, b: renamed } = source;` (side-effect-free source,
/// plain identifier bindings, no defaults/rest/nesting) into the
/// per-property member-access declarations it is equivalent to:
/// `const a = source.a; const renamed = source.b;`.
pub(super) fn lower_destructuring_declaration(
    var_decl: &VariableDeclaration,
    cx: &ConversionContext,
) -> Option<Vec<Rc<TreeNode>>> {
    if var_decl.declarations.len() != 1 {
        return None;
    }
    let declarator = &var_decl.declarations[0];
    let BindingPattern::ObjectPattern(pattern) = &declarator.id else {
        return None;
    };
    if pattern.rest.is_some() || pattern.properties.is_empty() {
        return None;
    }
    let init = declarator.init.as_ref()?;
    if !is_side_effect_free(init) {
        return None;
    }

    enum KeyAccess {
        Static(String),
        Quoted(String),
    }

    let mut lowered = Vec::new();
    for property in &pattern.properties {
        if property.computed {
            return None;
        }
        let key = match &property.key {
            PropertyKey::StaticIdentifier(ident) => KeyAccess::Static(ident.name.to_string()),
            PropertyKey::StringLiteral(lit) => KeyAccess::Quoted(lit.value.to_string()),
            _ => return None,
        };
        let BindingPattern::BindingIdentifier(binding) = &property.value else {
            return None;
        };
        lowered.push((key, cx.identifier_label(binding.span, &binding.name)));
    }

    let kind_label = format!("{:?}Declaration", var_decl.kind);
    let mut nodes = Vec::with_capacity(lowered.len());
    for (key, binding_label) in lowered {
        let mut decl = make_node(&kind_label, "VariableDeclaration");
        let mut declarator_node = make_node("VariableDeclarator", "VariableDeclarator");
        declarator_node.add_child(leaf(&binding_label, "BindingIdentifier"));
        let access: Rc<TreeNode> = match key {
            KeyAccess::Static(name) => {
                let mut member = make_node(".", "StaticMemberExpression");
                if let Some(object) = expression_to_tree_node(init, cx) {
                    member.add_child(object);
                }
                member.add_child(leaf(&name, "Identifier"));
                Rc::new(member)
            }
            KeyAccess::Quoted(value) => {
                let mut member = make_node("[]", "ComputedMemberExpression");
                if let Some(object) = expression_to_tree_node(init, cx) {
                    member.add_child(object);
                }
                member.add_child(leaf(&format!("\"{value}\""), "StringLiteral"));
                Rc::new(member)
            }
        };
        declarator_node.add_child(access);
        decl.add_child(Rc::new(declarator_node));
        nodes.push(Rc::new(decl));
    }
    Some(nodes)
}
