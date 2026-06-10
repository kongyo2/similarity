use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AssignmentOperator, BindingPattern, BlockStatement, Class, ClassElement,
    ConditionalExpression, Declaration, ExportDefaultDeclarationKind, Expression, FormalParameter,
    Function, FunctionBody, Program, PropertyKey, Statement, SwitchStatement, UpdateOperator,
    VariableDeclaration, VariableDeclarator,
};
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::cell::Cell;
use std::rc::Rc;

use crate::tree::TreeNode;

// The label set is intentionally rich: every distinct syntactic role gets a
// distinct label so APTED's structural comparison can tell apart constructs
// that share a parent kind (e.g. `+=` vs `=`, `for-of` vs `while`,
// `obj.prop` vs `obj[expr]`). Coarser labels were the dominant source of
// false-positive matches between semantically different functions — the
// original parser fell back to a generic "Statement" / "Expression" node
// for many constructs which let unrelated code shapes collapse into the
// same tree.

/// Parse TypeScript code and convert to `TreeNode` structure
///
/// # Errors
///
/// Returns an error if parsing fails due to syntax errors
pub fn parse_and_convert_to_tree(
    filename: &str,
    source_text: &str,
) -> Result<Rc<TreeNode>, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(filename).unwrap_or(SourceType::tsx());
    let ret = Parser::new(&allocator, source_text, source_type).parse();

    if !ret.errors.is_empty() {
        // Create a more readable error message
        let error_messages: Vec<String> =
            ret.errors.iter().map(|e| e.message.to_string()).collect();
        return Err(format!("Parse errors: {}", error_messages.join(", ")));
    }

    let mut id_counter = 0;
    Ok(ast_to_tree_node(&ret.program, &mut id_counter))
}

pub fn ast_to_tree_node(program: &Program, id_counter: &mut usize) -> Rc<TreeNode> {
    let mut root = TreeNode::new("Program".to_string(), "Program".to_string(), *id_counter);
    *id_counter += 1;

    for stmt in &program.body {
        append_statement_to_list(&mut root, stmt, id_counter);
    }

    Rc::new(root)
}

// ---------------------------------------------------------------------------
// Refactor-equivalence canonicalization
// ---------------------------------------------------------------------------
//
// The function comparator wants two snippets that a developer would call
// "the same code written differently" to parse into the same tree. The
// transforms below rewrite common style alternatives onto one canonical
// shape at AST→TreeNode conversion time:
//
//   * `` `a ${x}` `` → `"a " + x` (template literal as concatenation)
//   * `x += y` → `x = x + y` (compound assignment), `i++` → `i = i + 1`
//     in statement/for-update position
//   * `return c ? a : b` → `if (c) { return a } else { return b }`, and
//     the assignment/declaration ternary forms accordingly
//   * `xs.forEach((x) => {…})` → `for (const x of xs) {…}`
//   * `return p.then((v) => {…})` → `const v = await p; …` (single-level)
//   * `for (; cond; )` → `while (cond)`
//   * `switch` with jump-terminated cases → `if`/`else if` chain
//   * `Object.assign({}, a, {k: v})` → `{ ...a, k: v }`
//   * single-statement bodies of `if`/loops → block-wrapped bodies
//
// They are gated behind a flag (instead of always-on) because the literal
// tree shape still matters to the other consumers of this parser — the
// overlap detector reports source windows and must mirror what the user
// wrote, and `compare_values`-style comparators tuned their own costs
// against the literal shapes.
//
// The flag is thread-local rather than threaded through every conversion
// helper: conversion is a single synchronous call tree rooted at
// `parse_and_convert_to_tree`, and ~20 helper signatures would otherwise
// change for a value that never varies within one parse.

thread_local! {
    static CANONICALIZE: Cell<bool> = const { Cell::new(false) };
}

fn canonicalize_enabled() -> bool {
    CANONICALIZE.with(Cell::get)
}

/// Restores the previous canonicalization flag on drop so nested or
/// panicking parses can't leak the enabled state into later parses on the
/// same thread.
struct CanonicalizeGuard {
    previous: bool,
}

impl CanonicalizeGuard {
    fn enable() -> Self {
        let previous = CANONICALIZE.with(|flag| flag.replace(true));
        CanonicalizeGuard { previous }
    }
}

impl Drop for CanonicalizeGuard {
    fn drop(&mut self) {
        let previous = self.previous;
        CANONICALIZE.with(|flag| flag.set(previous));
    }
}

/// Variant of [`parse_and_convert_to_tree`] that applies the
/// refactor-equivalence canonicalization described above. Used by the
/// function comparator so style-only rewrites (template literals,
/// `.then()` vs `await`, `forEach` vs `for-of`, …) compare as equal trees.
///
/// # Errors
///
/// Returns an error if parsing fails due to syntax errors
pub fn parse_and_convert_to_tree_canonical(
    filename: &str,
    source_text: &str,
) -> Result<Rc<TreeNode>, String> {
    let _guard = CanonicalizeGuard::enable();
    parse_and_convert_to_tree(filename, source_text)
}

fn strip_parentheses<'a, 'b>(expr: &'b Expression<'a>) -> &'b Expression<'a> {
    let mut current = expr;
    while let Expression::ParenthesizedExpression(paren) = current {
        current = &paren.expression;
    }
    current
}

/// Convert one statement into the nodes it contributes to a statement
/// list. Almost always a single node, but canonical lowering of
/// `return p.then(cb)` / `p.then(cb);` and of declaration-initializing
/// ternaries (`const x = c ? a : b`) expands one statement into several.
fn statement_to_tree_nodes(stmt: &Statement, id_counter: &mut usize) -> Vec<Rc<TreeNode>> {
    if canonicalize_enabled() {
        match stmt {
            Statement::ReturnStatement(ret_stmt) => {
                if let Some(arg) = &ret_stmt.argument {
                    let mut effective = strip_parentheses(arg);
                    if let Expression::AwaitExpression(await_expr) = effective {
                        effective = strip_parentheses(&await_expr.argument);
                    }
                    if let Some(parts) = match_then_call(effective) {
                        return lower_then_call(&parts, ThenPosition::Return, id_counter);
                    }
                }
            }
            Statement::ExpressionStatement(expr_stmt) => {
                let mut effective = strip_parentheses(&expr_stmt.expression);
                if let Expression::AwaitExpression(await_expr) = effective {
                    effective = strip_parentheses(&await_expr.argument);
                }
                if let Some(parts) = match_then_call(effective) {
                    return lower_then_call(&parts, ThenPosition::Statement, id_counter);
                }
            }
            Statement::VariableDeclaration(var_decl) => {
                if let Some(nodes) = lower_declaration_ternary(var_decl, id_counter) {
                    return nodes;
                }
            }
            _ => {}
        }
    }
    statement_to_tree_node(stmt, id_counter).into_iter().collect()
}

fn append_statement_to_list(parent: &mut TreeNode, stmt: &Statement, id_counter: &mut usize) {
    for node in statement_to_tree_nodes(stmt, id_counter) {
        parent.add_child(node);
    }
}

/// Canonical-mode body conversion for `if`/loop bodies: wraps a
/// single-statement body in a synthetic block so `if (x) return y;` and
/// `if (x) { return y; }` produce the same tree. Outside canonical mode
/// this is exactly `statement_to_tree_node`.
fn statement_to_block_node(stmt: &Statement, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
    if !canonicalize_enabled() || matches!(stmt, Statement::BlockStatement(_)) {
        return statement_to_tree_node(stmt, id_counter);
    }
    let mut block = make_node("BlockStatement", "BlockStatement", id_counter);
    append_statement_to_list(&mut block, stmt, id_counter);
    Some(Rc::new(block))
}

enum CallbackBody<'a, 'b> {
    Expression(&'b Expression<'a>),
    Block(&'b oxc_allocator::Vec<'a, Statement<'a>>),
}

struct ThenCallParts<'a, 'b> {
    object: &'b Expression<'a>,
    param: Option<&'b BindingPattern<'a>>,
    body: CallbackBody<'a, 'b>,
}

enum ThenPosition {
    Return,
    Statement,
}

/// Match `E.then(callback)` where the callback is an inline arrow or
/// function expression with at most one parameter. Two-argument `then`
/// calls (`onFulfilled, onRejected`) and named-function callbacks stay
/// untouched — only the directly-inlineable single-callback form is
/// equivalent to an `await` sequence.
fn match_then_call<'a, 'b>(expr: &'b Expression<'a>) -> Option<ThenCallParts<'a, 'b>> {
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
            if arrow.params.rest.is_some() || arrow.params.items.len() > 1 {
                return None;
            }
            let body = if arrow.expression {
                let Some(Statement::ExpressionStatement(expr_stmt)) =
                    arrow.body.statements.first()
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
            if func.generator || func.params.rest.is_some() || func.params.items.len() > 1 {
                return None;
            }
            let body = func.body.as_ref()?;
            (
                func.params.items.first().map(|p| &p.pattern),
                CallbackBody::Block(&body.statements),
            )
        }
        _ => return None,
    };
    Some(ThenCallParts { object: &member.object, param, body })
}

/// Lower `return E.then((v) => body)` (or the bare-statement form) into
/// the equivalent `await` sequence:
///
/// ```text
/// const v = await E;   // or `await E;` when the callback takes no value
/// …body…               // expression bodies become `return expr` / `expr;`
/// ```
fn lower_then_call(
    parts: &ThenCallParts,
    position: ThenPosition,
    id_counter: &mut usize,
) -> Vec<Rc<TreeNode>> {
    let mut nodes = Vec::new();

    let await_node = {
        let mut node = make_node("AwaitExpression", "AwaitExpression", id_counter);
        if let Some(object_node) = expression_to_tree_node(parts.object, id_counter) {
            node.add_child(object_node);
        }
        Rc::new(node)
    };

    if let Some(pattern) = parts.param {
        let mut declarator = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
        if let Some(binding) = binding_pattern_to_tree_node(pattern, id_counter) {
            declarator.add_child(binding);
        }
        declarator.add_child(await_node);
        let mut decl = make_node("ConstDeclaration", "VariableDeclaration", id_counter);
        decl.add_child(Rc::new(declarator));
        nodes.push(Rc::new(decl));
    } else {
        let mut stmt = make_node("ExpressionStatement", "ExpressionStatement", id_counter);
        stmt.add_child(await_node);
        nodes.push(Rc::new(stmt));
    }

    match &parts.body {
        CallbackBody::Expression(expr) => {
            let label = match position {
                ThenPosition::Return => "ReturnStatement",
                ThenPosition::Statement => "ExpressionStatement",
            };
            let mut stmt = make_node(label, label, id_counter);
            if let Some(child) = expression_to_tree_node(expr, id_counter) {
                stmt.add_child(child);
            }
            nodes.push(Rc::new(stmt));
        }
        CallbackBody::Block(statements) => {
            for stmt in statements.iter() {
                nodes.extend(statement_to_tree_nodes(stmt, id_counter));
            }
        }
    }

    nodes
}

/// Lower a ternary into an `if`/`else` chain. `make_leaf_statement`
/// builds the statement that consumes each branch value (a `return` or an
/// assignment); nested ternaries in the alternate position become
/// `else if` arms, mirroring how the explicit chain is written by hand.
fn lower_ternary_chain(
    cond: &ConditionalExpression,
    make_leaf_statement: &mut dyn FnMut(&Expression, &mut usize) -> Rc<TreeNode>,
    id_counter: &mut usize,
) -> Rc<TreeNode> {
    let mut if_node = make_node("IfStatement", "IfStatement", id_counter);
    if let Some(test_node) = expression_to_tree_node(&cond.test, id_counter) {
        if_node.add_child(test_node);
    }

    let mut consequent_block = make_node("BlockStatement", "BlockStatement", id_counter);
    consequent_block
        .add_child(make_leaf_statement(strip_parentheses(&cond.consequent), id_counter));
    if_node.add_child(Rc::new(consequent_block));

    let alternate = strip_parentheses(&cond.alternate);
    if let Expression::ConditionalExpression(nested) = alternate {
        if_node.add_child(lower_ternary_chain(nested, make_leaf_statement, id_counter));
    } else {
        let mut alternate_block = make_node("BlockStatement", "BlockStatement", id_counter);
        alternate_block.add_child(make_leaf_statement(alternate, id_counter));
        if_node.add_child(Rc::new(alternate_block));
    }
    Rc::new(if_node)
}

fn return_leaf_statement(expr: &Expression, id_counter: &mut usize) -> Rc<TreeNode> {
    let mut node = make_node("ReturnStatement", "ReturnStatement", id_counter);
    if let Some(child) = expression_to_tree_node(expr, id_counter) {
        node.add_child(child);
    }
    Rc::new(node)
}

fn assignment_leaf_statement(
    target_node: Option<Rc<TreeNode>>,
    expr: &Expression,
    id_counter: &mut usize,
) -> Rc<TreeNode> {
    let mut assign = make_node("Assign", "AssignmentExpression", id_counter);
    if let Some(target) = target_node {
        assign.add_child(target);
    }
    if let Some(value) = expression_to_tree_node(expr, id_counter) {
        assign.add_child(value);
    }
    let mut stmt = make_node("ExpressionStatement", "ExpressionStatement", id_counter);
    stmt.add_child(Rc::new(assign));
    Rc::new(stmt)
}

/// Lower `const x = c ? a : b;` into `let x; if (c) { x = a } else { x = b }`
/// so it lines up with the hand-written `let`-plus-branches form. Only
/// single-declarator declarations binding a plain identifier qualify.
fn lower_declaration_ternary(
    var_decl: &VariableDeclaration,
    id_counter: &mut usize,
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

    // The lowered binding is mutated by the branches, so it canonicalizes
    // to `let` regardless of the source keyword.
    let mut decl = make_node("LetDeclaration", "VariableDeclaration", id_counter);
    let mut declarator_node = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
    declarator_node.add_child(leaf(ident.name.as_str(), "BindingIdentifier", id_counter));
    decl.add_child(Rc::new(declarator_node));

    let name = ident.name.as_str();
    let if_node = lower_ternary_chain(
        cond,
        &mut |expr, ids| {
            let target = Some(leaf(name, "Identifier", ids));
            assignment_leaf_statement(target, expr, ids)
        },
        id_counter,
    );

    Some(vec![Rc::new(decl), if_node])
}

/// Lower `xs.forEach((x) => { … })` in statement position into
/// `for (const x of xs) { … }`. Only the single-simple-parameter inline
/// callback qualifies — index-taking callbacks observe positions and are
/// not equivalent to a plain `for-of`.
fn lower_foreach_statement(expr: &Expression, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
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
            if arrow.params.rest.is_some() || arrow.params.items.len() != 1 {
                return None;
            }
            let BindingPattern::BindingIdentifier(ident) = &arrow.params.items[0].pattern else {
                return None;
            };
            let body = if arrow.expression {
                let Some(Statement::ExpressionStatement(expr_stmt)) =
                    arrow.body.statements.first()
                else {
                    return None;
                };
                CallbackBody::Expression(&expr_stmt.expression)
            } else {
                CallbackBody::Block(&arrow.body.statements)
            };
            (ident.name.as_str(), body)
        }
        Expression::FunctionExpression(func) if !func.r#async && !func.generator => {
            if func.params.rest.is_some() || func.params.items.len() != 1 {
                return None;
            }
            let BindingPattern::BindingIdentifier(ident) = &func.params.items[0].pattern else {
                return None;
            };
            let body = func.body.as_ref()?;
            (ident.name.as_str(), CallbackBody::Block(&body.statements))
        }
        _ => return None,
    };

    let mut for_node = make_node("ForOfStatement", "ForOfStatement", id_counter);
    let mut decl = make_node("ConstDeclaration", "VariableDeclaration", id_counter);
    let mut declarator = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
    declarator.add_child(leaf(param_name, "BindingIdentifier", id_counter));
    decl.add_child(Rc::new(declarator));
    for_node.add_child(Rc::new(decl));
    if let Some(iterable) = expression_to_tree_node(&member.object, id_counter) {
        for_node.add_child(iterable);
    }
    let mut block = make_node("BlockStatement", "BlockStatement", id_counter);
    match body {
        CallbackBody::Expression(expr) => {
            let mut stmt = make_node("ExpressionStatement", "ExpressionStatement", id_counter);
            if let Some(child) = expression_to_tree_node(expr, id_counter) {
                stmt.add_child(child);
            }
            block.add_child(Rc::new(stmt));
        }
        CallbackBody::Block(statements) => {
            for stmt in statements.iter() {
                append_statement_to_list(&mut block, stmt, id_counter);
            }
        }
    }
    for_node.add_child(Rc::new(block));
    Some(Rc::new(for_node))
}

fn is_switch_jump_statement(stmt: &Statement) -> bool {
    matches!(
        stmt,
        Statement::BreakStatement(_)
            | Statement::ReturnStatement(_)
            | Statement::ThrowStatement(_)
            | Statement::ContinueStatement(_)
    )
}

/// Lower a `switch` whose cases all end in a jump (no fallthrough, default
/// last) into the equivalent `if (disc === t1) {…} else if (…) {…} else {…}`
/// chain. Returns `None` — leaving the literal switch shape — whenever the
/// rewrite wouldn't be behavior-preserving.
fn lower_switch_to_if_chain(
    switch_stmt: &SwitchStatement,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
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

    let case_body_block = |case: &oxc_ast::ast::SwitchCase, ids: &mut usize| -> Rc<TreeNode> {
        let mut block = make_node("BlockStatement", "BlockStatement", ids);
        let mut statements: Vec<&Statement> = case.consequent.iter().collect();
        if let Some(Statement::BreakStatement(break_stmt)) = statements.last() {
            // The trailing unlabeled `break` is the switch's own exit and
            // has no analogue in the if-chain. Labeled breaks target an
            // outer statement and must stay.
            if break_stmt.label.is_none() {
                statements.pop();
            }
        }
        for stmt in statements {
            append_statement_to_list(&mut block, stmt, ids);
        }
        Rc::new(block)
    };

    let mut else_node: Option<Rc<TreeNode>> = None;
    for case in cases.iter().rev() {
        match &case.test {
            None => {
                else_node = Some(case_body_block(case, id_counter));
            }
            Some(test) => {
                let mut if_node = make_node("IfStatement", "IfStatement", id_counter);
                let mut equality = make_node("StrictEquality", "BinaryExpression", id_counter);
                if let Some(disc) =
                    expression_to_tree_node(&switch_stmt.discriminant, id_counter)
                {
                    equality.add_child(disc);
                }
                if let Some(test_node) = expression_to_tree_node(test, id_counter) {
                    equality.add_child(test_node);
                }
                if_node.add_child(Rc::new(equality));
                if_node.add_child(case_body_block(case, id_counter));
                if let Some(alternate) = else_node.take() {
                    if_node.add_child(alternate);
                }
                else_node = Some(Rc::new(if_node));
            }
        }
    }
    else_node
}

/// Canonicalization for expressions whose value is discarded (statement
/// position and `for` update slots): `i++` / `--i` become the compound
/// form `i += 1` / `i -= 1`, so all three in-place-update spellings
/// (`i++`, `i += 1`, and `i = i + 1` via the assignment contraction)
/// produce the same tree.
fn statement_value_expression_to_tree_node(
    expr: &Expression,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    if canonicalize_enabled() {
        if let Expression::UpdateExpression(update) = strip_parentheses(expr) {
            let operator_label = match update.operator {
                UpdateOperator::Increment => "Addition",
                UpdateOperator::Decrement => "Subtraction",
            };
            let mut assign = make_node(operator_label, "AssignmentExpression", id_counter);
            if let Some(target) =
                simple_assignment_target_to_tree_node(&update.argument, id_counter)
            {
                assign.add_child(target);
            }
            assign.add_child(leaf("1", "NumericLiteral", id_counter));
            return Some(Rc::new(assign));
        }
    }
    expression_to_tree_node(expr, id_counter)
}

/// Structural equality for the simple assignment-target shapes that show
/// up on both sides of `x = x op E` / `this.x = this.x op E`. Used to
/// decide whether an expanded assignment can contract onto the compound
/// form; anything more exotic (computed members, chained paths) returns
/// `false` and keeps the literal shape.
fn assignment_target_matches_expression(
    target: &oxc_ast::ast::AssignmentTarget,
    expr: &Expression,
) -> bool {
    use oxc_ast::ast::AssignmentTarget;
    match (target, strip_parentheses(expr)) {
        (AssignmentTarget::AssignmentTargetIdentifier(ident), Expression::Identifier(expr_ident)) => {
            ident.name == expr_ident.name
        }
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
fn lower_object_assign_to_spread(
    call_expr: &oxc_ast::ast::CallExpression,
    id_counter: &mut usize,
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
    if object_ident.name.as_str() != "Object" {
        return None;
    }
    let first = call_expr.arguments[0].as_expression().map(strip_parentheses)?;
    let Expression::ObjectExpression(target) = first else {
        return None;
    };
    if !target.properties.is_empty() {
        return None;
    }

    let mut node = make_node("ObjectExpression", "ObjectExpression", id_counter);
    for argument in call_expr.arguments.iter().skip(1) {
        match argument.as_expression().map(strip_parentheses) {
            Some(Expression::ObjectExpression(obj)) => {
                // Inline literal arguments — `Object.assign({}, {k: v})`
                // contributes its properties directly, just like the
                // spread form `{ k: v }` would.
                for prop in &obj.properties {
                    if let Some(child) = object_property_to_tree_node(prop, id_counter) {
                        node.add_child(child);
                    }
                }
            }
            Some(expr) => {
                let mut spread = make_node("SpreadProperty", "SpreadProperty", id_counter);
                if let Some(child) = expression_to_tree_node(expr, id_counter) {
                    spread.add_child(child);
                }
                node.add_child(Rc::new(spread));
            }
            None => return None,
        }
    }
    Some(Rc::new(node))
}

fn make_node(label: &str, kind: &str, id_counter: &mut usize) -> TreeNode {
    let node = TreeNode::new(label.to_string(), kind.to_string(), *id_counter);
    *id_counter += 1;
    node
}

/// Stringify a class member's `PropertyKey` into a label that's unique per
/// distinct key. Identifier and private-identifier keys already produce a
/// stable string form, but the original handler collapsed every other key
/// (string literals like `"alpha"`, numeric literals, computed
/// expressions) onto a single `"Method"` / `"Property"` placeholder so two
/// runtime-distinct members ended up sharing a tree label. The richer
/// labels below let APTED see those as a single-rename difference instead
/// of a perfect match.
fn property_key_label(key: &PropertyKey) -> String {
    match key {
        PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
        PropertyKey::PrivateIdentifier(ident) => format!("#{}", ident.name.as_str()),
        PropertyKey::StringLiteral(lit) => format!("\"{}\"", lit.value.as_str()),
        PropertyKey::NumericLiteral(lit) => lit.value.to_string(),
        PropertyKey::BigIntLiteral(lit) => lit.value.as_str().to_string(),
        PropertyKey::TemplateLiteral(_) => "TemplateKey".to_string(),
        // Computed keys fall through here. Use a generic label rather than
        // recursing into the expression — the inner expression nodes are
        // already part of the tree elsewhere, and labeling each computed
        // key by its raw shape would interact badly with the wrapper-level
        // normalization that uses `display_name`.
        _ => "ComputedKey".to_string(),
    }
}

fn leaf(label: &str, kind: &str, id_counter: &mut usize) -> Rc<TreeNode> {
    Rc::new(make_node(label, kind, id_counter))
}

fn statement_to_tree_node(stmt: &Statement, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            function_declaration_to_tree_node(func, id_counter, "Function")
        }
        Statement::ClassDeclaration(class) => {
            class_declaration_to_tree_node(class, id_counter, "Class")
        }
        Statement::VariableDeclaration(var_decl) => {
            variable_declaration_to_tree_node(var_decl, id_counter)
        }
        Statement::ExpressionStatement(expr_stmt) => {
            if canonicalize_enabled() {
                let effective = strip_parentheses(&expr_stmt.expression);
                if let Some(node) = lower_foreach_statement(effective, id_counter) {
                    return Some(node);
                }
                // `x = c ? a : b;` lowers to the explicit `if`/`else`
                // assignment form so both spellings compare as equal.
                if let Expression::AssignmentExpression(assign) = effective {
                    if assign.operator == AssignmentOperator::Assign {
                        if let Expression::ConditionalExpression(cond) =
                            strip_parentheses(&assign.right)
                        {
                            let target = &assign.left;
                            return Some(lower_ternary_chain(
                                cond,
                                &mut |expr, ids| {
                                    let target_node =
                                        assignment_target_to_tree_node(target, ids);
                                    assignment_leaf_statement(target_node, expr, ids)
                                },
                                id_counter,
                            ));
                        }
                    }
                }
            }
            // Wrap in ExpressionStatement so the role of an expression
            // appearing as a statement (e.g. `result += 1;`) is preserved
            // when comparing against bare expressions in other contexts.
            // The wrapper is a single extra node so it doesn't significantly
            // inflate tree sizes, but it does noticeably improve
            // discrimination on short loop-body diffs.
            let mut node = make_node("ExpressionStatement", "ExpressionStatement", id_counter);
            if let Some(child) =
                statement_value_expression_to_tree_node(&expr_stmt.expression, id_counter)
            {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        Statement::BlockStatement(block) => block_statement_to_tree_node(block, id_counter),
        Statement::ExportNamedDeclaration(export) => {
            if let Some(decl) = &export.declaration {
                declaration_to_tree_node(decl, id_counter)
            } else {
                Some(leaf("ExportNamedDeclaration", "ExportNamedDeclaration", id_counter))
            }
        }
        Statement::ExportDefaultDeclaration(export) => {
            export_default_declaration_to_tree_node(&export.declaration, id_counter)
        }
        Statement::IfStatement(if_stmt) => {
            let mut node = make_node("IfStatement", "IfStatement", id_counter);

            if let Some(test_node) = expression_to_tree_node(&if_stmt.test, id_counter) {
                node.add_child(test_node);
            }
            if let Some(cons_node) = statement_to_block_node(&if_stmt.consequent, id_counter) {
                node.add_child(cons_node);
            }
            if let Some(alt) = &if_stmt.alternate {
                // `else if` chains stay bare so a lowered switch and a
                // hand-written chain produce the same nesting.
                let alt_node = if matches!(alt, Statement::IfStatement(_)) {
                    statement_to_tree_node(alt, id_counter)
                } else {
                    statement_to_block_node(alt, id_counter)
                };
                if let Some(alt_node) = alt_node {
                    node.add_child(alt_node);
                }
            }
            Some(Rc::new(node))
        }
        Statement::ReturnStatement(ret_stmt) => {
            if canonicalize_enabled() {
                if let Some(arg) = &ret_stmt.argument {
                    // `return await X` and `return X` resolve to the same
                    // value from the caller's perspective; strip the await
                    // so the two spellings align.
                    let mut effective = strip_parentheses(arg);
                    if let Expression::AwaitExpression(await_expr) = effective {
                        effective = strip_parentheses(&await_expr.argument);
                    }
                    if let Expression::ConditionalExpression(cond) = effective {
                        return Some(lower_ternary_chain(
                            cond,
                            &mut return_leaf_statement,
                            id_counter,
                        ));
                    }
                    let mut node = make_node("ReturnStatement", "ReturnStatement", id_counter);
                    if let Some(arg_node) = expression_to_tree_node(effective, id_counter) {
                        node.add_child(arg_node);
                    }
                    return Some(Rc::new(node));
                }
            }
            let mut node = make_node("ReturnStatement", "ReturnStatement", id_counter);
            if let Some(arg) = &ret_stmt.argument {
                if let Some(arg_node) = expression_to_tree_node(arg, id_counter) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        Statement::ForStatement(for_stmt) => {
            // `for (; cond; )` is a `while (cond)` spelled differently, and
            // `for (;;)` is `while (true)` — canonicalize so the loop-form
            // choice doesn't register as a structural difference.
            if canonicalize_enabled() && for_stmt.init.is_none() && for_stmt.update.is_none() {
                let mut node = make_node("WhileStatement", "WhileStatement", id_counter);
                if let Some(test) = &for_stmt.test {
                    if let Some(test_node) = expression_to_tree_node(test, id_counter) {
                        node.add_child(test_node);
                    }
                } else {
                    node.add_child(leaf("true", "BooleanLiteral", id_counter));
                }
                if let Some(body_node) = statement_to_block_node(&for_stmt.body, id_counter) {
                    node.add_child(body_node);
                }
                return Some(Rc::new(node));
            }
            let mut node = make_node("ForStatement", "ForStatement", id_counter);
            if let Some(init) = &for_stmt.init {
                if let Some(init_node) = for_statement_init_to_tree_node(init, id_counter) {
                    node.add_child(init_node);
                }
            }
            if let Some(test) = &for_stmt.test {
                if let Some(test_node) = expression_to_tree_node(test, id_counter) {
                    node.add_child(test_node);
                }
            }
            if let Some(update) = &for_stmt.update {
                if let Some(update_node) =
                    statement_value_expression_to_tree_node(update, id_counter)
                {
                    node.add_child(update_node);
                }
            }
            if let Some(body_node) = statement_to_block_node(&for_stmt.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::ForOfStatement(for_of) => {
            // Encode `await` directly into the label so `for await` and
            // `for` of the same shape don't collapse together.
            let label = if for_of.r#await { "ForAwaitOfStatement" } else { "ForOfStatement" };
            let mut node = make_node(label, "ForOfStatement", id_counter);
            if let Some(left_node) = for_statement_left_to_tree_node(&for_of.left, id_counter) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&for_of.right, id_counter) {
                node.add_child(right_node);
            }
            if let Some(body_node) = statement_to_block_node(&for_of.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::ForInStatement(for_in) => {
            let mut node = make_node("ForInStatement", "ForInStatement", id_counter);
            if let Some(left_node) = for_statement_left_to_tree_node(&for_in.left, id_counter) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&for_in.right, id_counter) {
                node.add_child(right_node);
            }
            if let Some(body_node) = statement_to_block_node(&for_in.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::WhileStatement(while_stmt) => {
            let mut node = make_node("WhileStatement", "WhileStatement", id_counter);
            if let Some(test_node) = expression_to_tree_node(&while_stmt.test, id_counter) {
                node.add_child(test_node);
            }
            if let Some(body_node) = statement_to_block_node(&while_stmt.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::DoWhileStatement(do_stmt) => {
            let mut node = make_node("DoWhileStatement", "DoWhileStatement", id_counter);
            if let Some(body_node) = statement_to_block_node(&do_stmt.body, id_counter) {
                node.add_child(body_node);
            }
            if let Some(test_node) = expression_to_tree_node(&do_stmt.test, id_counter) {
                node.add_child(test_node);
            }
            Some(Rc::new(node))
        }
        Statement::ThrowStatement(throw_stmt) => {
            let mut node = make_node("ThrowStatement", "ThrowStatement", id_counter);
            if let Some(arg_node) = expression_to_tree_node(&throw_stmt.argument, id_counter) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Statement::TryStatement(try_stmt) => {
            let mut node = make_node("TryStatement", "TryStatement", id_counter);
            if let Some(block_node) = block_statement_to_tree_node(&try_stmt.block, id_counter) {
                node.add_child(block_node);
            }
            if let Some(handler) = &try_stmt.handler {
                let mut catch_node = make_node("CatchClause", "CatchClause", id_counter);
                if let Some(param) = &handler.param {
                    if let Some(param_node) =
                        binding_pattern_to_tree_node(&param.pattern, id_counter)
                    {
                        catch_node.add_child(param_node);
                    }
                }
                if let Some(body_node) =
                    block_statement_to_tree_node(&handler.body, id_counter)
                {
                    catch_node.add_child(body_node);
                }
                node.add_child(Rc::new(catch_node));
            }
            if let Some(finalizer) = &try_stmt.finalizer {
                let mut fin_node = make_node("FinallyClause", "FinallyClause", id_counter);
                if let Some(body_node) = block_statement_to_tree_node(finalizer, id_counter) {
                    fin_node.add_child(body_node);
                }
                node.add_child(Rc::new(fin_node));
            }
            Some(Rc::new(node))
        }
        Statement::SwitchStatement(switch_stmt) => {
            if canonicalize_enabled() {
                if let Some(node) = lower_switch_to_if_chain(switch_stmt, id_counter) {
                    return Some(node);
                }
            }
            let mut node = make_node("SwitchStatement", "SwitchStatement", id_counter);
            if let Some(disc_node) =
                expression_to_tree_node(&switch_stmt.discriminant, id_counter)
            {
                node.add_child(disc_node);
            }
            for case in &switch_stmt.cases {
                let case_label = if case.test.is_some() { "SwitchCase" } else { "DefaultCase" };
                let mut case_node = make_node(case_label, "SwitchCase", id_counter);
                if let Some(test) = &case.test {
                    if let Some(test_node) = expression_to_tree_node(test, id_counter) {
                        case_node.add_child(test_node);
                    }
                }
                for cons_stmt in &case.consequent {
                    append_statement_to_list(&mut case_node, cons_stmt, id_counter);
                }
                node.add_child(Rc::new(case_node));
            }
            Some(Rc::new(node))
        }
        Statement::BreakStatement(break_stmt) => {
            let label = break_stmt
                .label
                .as_ref()
                .map(|l| format!("break:{}", l.name.as_str()))
                .unwrap_or_else(|| "BreakStatement".to_string());
            Some(leaf(&label, "BreakStatement", id_counter))
        }
        Statement::ContinueStatement(cont_stmt) => {
            let label = cont_stmt
                .label
                .as_ref()
                .map(|l| format!("continue:{}", l.name.as_str()))
                .unwrap_or_else(|| "ContinueStatement".to_string());
            Some(leaf(&label, "ContinueStatement", id_counter))
        }
        Statement::LabeledStatement(labeled) => {
            let mut node = make_node(
                &format!("label:{}", labeled.label.name.as_str()),
                "LabeledStatement",
                id_counter,
            );
            if let Some(body_node) = statement_to_tree_node(&labeled.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::EmptyStatement(_) => Some(leaf("EmptyStatement", "EmptyStatement", id_counter)),
        Statement::DebuggerStatement(_) => {
            Some(leaf("DebuggerStatement", "DebuggerStatement", id_counter))
        }
        Statement::WithStatement(with_stmt) => {
            let mut node = make_node("WithStatement", "WithStatement", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&with_stmt.object, id_counter) {
                node.add_child(obj_node);
            }
            if let Some(body_node) = statement_to_tree_node(&with_stmt.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        _ => {
            // Generic fallback for anything not specifically handled above
            // (predominantly TS-specific declarations we don't need to
            // structurally distinguish for the purposes of refactoring
            // detection).
            Some(leaf("Statement", "Statement", id_counter))
        }
    }
}

fn for_statement_init_to_tree_node(
    init: &oxc_ast::ast::ForStatementInit,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::ForStatementInit;
    match init {
        ForStatementInit::VariableDeclaration(var_decl) => {
            variable_declaration_to_tree_node(var_decl, id_counter)
        }
        _ => init.as_expression().and_then(|expr| expression_to_tree_node(expr, id_counter)),
    }
}

fn for_statement_left_to_tree_node(
    left: &oxc_ast::ast::ForStatementLeft,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::ForStatementLeft;
    match left {
        ForStatementLeft::VariableDeclaration(var_decl) => {
            variable_declaration_to_tree_node(var_decl, id_counter)
        }
        // `ForStatementLeft` inherits every `AssignmentTarget` variant, so we
        // route them through the same shape-preserving handlers used for
        // assignments elsewhere. Otherwise `for (x of xs)` and
        // `for (obj.prop of xs)` would collapse onto the same generic leaf
        // and the parser-level distinction we get for assignments would be
        // lost the moment they appeared in a loop head.
        ForStatementLeft::AssignmentTargetIdentifier(ident) => {
            Some(leaf(ident.name.as_str(), "Identifier", id_counter))
        }
        ForStatementLeft::StaticMemberExpression(mem) => {
            let mut node = make_node(".", "StaticMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(mem.property.name.as_str(), "Identifier", id_counter));
            Some(Rc::new(node))
        }
        ForStatementLeft::ComputedMemberExpression(mem) => {
            let mut node = make_node("[]", "ComputedMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            if let Some(prop_node) = expression_to_tree_node(&mem.expression, id_counter) {
                node.add_child(prop_node);
            }
            Some(Rc::new(node))
        }
        ForStatementLeft::PrivateFieldExpression(mem) => {
            let mut node = make_node(".#", "PrivateFieldExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(
                &format!("#{}", mem.field.name.as_str()),
                "PrivateIdentifier",
                id_counter,
            ));
            Some(Rc::new(node))
        }
        ForStatementLeft::ArrayAssignmentTarget(_) => {
            Some(leaf("ArrayAssignmentTarget", "ArrayAssignmentTarget", id_counter))
        }
        ForStatementLeft::ObjectAssignmentTarget(_) => {
            Some(leaf("ObjectAssignmentTarget", "ObjectAssignmentTarget", id_counter))
        }
        _ => Some(leaf("ForStatementLeft", "ForStatementLeft", id_counter)),
    }
}

fn declaration_to_tree_node(decl: &Declaration, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
    match decl {
        Declaration::FunctionDeclaration(func) => {
            function_declaration_to_tree_node(func, id_counter, "Function")
        }
        Declaration::ClassDeclaration(class) => {
            class_declaration_to_tree_node(class, id_counter, "Class")
        }
        Declaration::VariableDeclaration(var_decl) => {
            variable_declaration_to_tree_node(var_decl, id_counter)
        }
        _ => Some(leaf("Declaration", "Declaration", id_counter)),
    }
}

fn export_default_declaration_to_tree_node(
    decl: &ExportDefaultDeclarationKind,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    match decl {
        ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
            function_declaration_to_tree_node(func, id_counter, "DefaultFunction")
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            class_declaration_to_tree_node(class, id_counter, "DefaultClass")
        }
        _ => {
            // Try to treat it as an expression default export
            if let Some(expr) = decl.as_expression() {
                if let Some(node) = expression_to_tree_node(expr, id_counter) {
                    let mut wrapper = make_node(
                        "ExportDefaultDeclaration",
                        "ExportDefaultDeclaration",
                        id_counter,
                    );
                    wrapper.add_child(node);
                    return Some(Rc::new(wrapper));
                }
            }
            Some(leaf("ExportDefaultDeclaration", "ExportDefaultDeclaration", id_counter))
        }
    }
}

fn function_declaration_to_tree_node(
    func: &Function,
    id_counter: &mut usize,
    default_label: &str,
) -> Option<Rc<TreeNode>> {
    let mut label = func.id.as_ref().map_or(default_label, |id| id.name.as_str()).to_string();
    // Canonical mode encodes async / generator into the declaration label
    // (the way arrow and function expressions already do) so an `async`
    // function and its sync twin register as a labelled difference instead
    // of relying on their names happening to differ.
    if canonicalize_enabled() {
        if func.generator {
            label = format!("*{label}");
        }
        if func.r#async {
            label = format!("async {label}");
        }
    }
    let mut node = TreeNode::new(label, "FunctionDeclaration".to_string(), *id_counter);
    *id_counter += 1;

    for param in &func.params.items {
        if let Some(param_node) = formal_parameter_to_tree_node(param, id_counter) {
            node.add_child(param_node);
        }
    }

    if let Some(body) = &func.body {
        if let Some(body_node) = function_body_to_tree_node(body, id_counter) {
            node.add_child(body_node);
        }
    }

    Some(Rc::new(node))
}

fn class_declaration_to_tree_node(
    class: &Class,
    id_counter: &mut usize,
    default_label: &str,
) -> Option<Rc<TreeNode>> {
    let label = class.id.as_ref().map_or(default_label, |id| id.name.as_str()).to_string();
    let mut node = TreeNode::new(label, "ClassDeclaration".to_string(), *id_counter);
    *id_counter += 1;

    for element in &class.body.body {
        if let Some(elem_node) = class_element_to_tree_node(element, id_counter) {
            node.add_child(elem_node);
        }
    }

    Some(Rc::new(node))
}

fn variable_declaration_to_tree_node(
    var_decl: &VariableDeclaration,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    // Encode `var`/`let`/`const` directly into the label so a `const`
    // declaration and a `let` declaration of the same shape don't collapse.
    let label = format!("{:?}Declaration", var_decl.kind);
    let mut node = TreeNode::new(label, "VariableDeclaration".to_string(), *id_counter);
    *id_counter += 1;

    for decl in &var_decl.declarations {
        if let Some(decl_node) = variable_declarator_to_tree_node(decl, id_counter) {
            node.add_child(decl_node);
        }
    }

    Some(Rc::new(node))
}

fn expression_to_tree_node(expr: &Expression, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
    match expr {
        Expression::Identifier(ident) => {
            Some(leaf(ident.name.as_str(), "Identifier", id_counter))
        }
        Expression::StringLiteral(str_lit) => {
            let label = format!("\"{}\"", str_lit.value.as_str());
            Some(leaf(&label, "StringLiteral", id_counter))
        }
        Expression::NumericLiteral(num_lit) => {
            let label = num_lit.value.to_string();
            Some(leaf(&label, "NumericLiteral", id_counter))
        }
        Expression::BooleanLiteral(bool_lit) => {
            let label = bool_lit.value.to_string();
            Some(leaf(&label, "BooleanLiteral", id_counter))
        }
        Expression::NullLiteral(_) => Some(leaf("null", "NullLiteral", id_counter)),
        Expression::BigIntLiteral(big) => {
            Some(leaf(big.value.as_str(), "BigIntLiteral", id_counter))
        }
        Expression::RegExpLiteral(re) => {
            // Encode both the pattern text and the flags into the label so
            // distinct regexes don't collide. `/foo/i` (case-insensitive
            // match) and `/foo/g` (global match) have meaningfully
            // different runtime behaviour and shouldn't be treated as
            // identical leaves.
            let label = format!("/{}/{}", re.regex.pattern.text.as_str(), re.regex.flags);
            Some(leaf(&label, "RegExpLiteral", id_counter))
        }
        Expression::TemplateLiteral(tpl) => {
            // Canonical mode rewrites an (untagged) template literal into
            // the string-concatenation chain it is equivalent to:
            // `` `a ${x} b` `` and `"a " + x + " b"` are the same string
            // computation, and the choice between them is pure style. The
            // operand order interleaves quasis and expressions exactly as
            // they appear; empty quasis (e.g. before a leading `${`)
            // contribute nothing to the concatenation and are dropped.
            if canonicalize_enabled() {
                let mut operands: Vec<Rc<TreeNode>> = Vec::new();
                for (index, quasi) in tpl.quasis.iter().enumerate() {
                    let cooked: &str = quasi.value.cooked.as_deref().unwrap_or("");
                    if !cooked.is_empty() {
                        operands.push(leaf(
                            &format!("\"{cooked}\""),
                            "StringLiteral",
                            id_counter,
                        ));
                    }
                    if let Some(inner_expr) = tpl.expressions.get(index) {
                        if let Some(child) = expression_to_tree_node(inner_expr, id_counter) {
                            operands.push(child);
                        }
                    }
                }
                return match operands.len() {
                    0 => Some(leaf("\"\"", "StringLiteral", id_counter)),
                    1 => operands.pop(),
                    _ => {
                        let mut iter = operands.into_iter();
                        let mut chain = iter.next().expect("len checked above");
                        for operand in iter {
                            let mut add = make_node("Addition", "BinaryExpression", id_counter);
                            add.add_child(chain);
                            add.add_child(operand);
                            chain = Rc::new(add);
                        }
                        Some(chain)
                    }
                };
            }
            let mut node = make_node("TemplateLiteral", "TemplateLiteral", id_counter);
            for q in &tpl.quasis {
                let cooked: &str = q.value.cooked.as_deref().unwrap_or("");
                let label = format!("`{}`", cooked);
                node.add_child(leaf(&label, "TemplateElement", id_counter));
            }
            for inner_expr in &tpl.expressions {
                if let Some(child) = expression_to_tree_node(inner_expr, id_counter) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::TaggedTemplateExpression(tag) => {
            let mut node =
                make_node("TaggedTemplateExpression", "TaggedTemplateExpression", id_counter);
            if let Some(callee) = expression_to_tree_node(&tag.tag, id_counter) {
                node.add_child(callee);
            }
            // Inline the template literal as a child
            let mut tpl_node = make_node("TemplateLiteral", "TemplateLiteral", id_counter);
            for q in &tag.quasi.quasis {
                let cooked: &str = q.value.cooked.as_deref().unwrap_or("");
                let label = format!("`{}`", cooked);
                tpl_node.add_child(leaf(&label, "TemplateElement", id_counter));
            }
            for inner_expr in &tag.quasi.expressions {
                if let Some(child) = expression_to_tree_node(inner_expr, id_counter) {
                    tpl_node.add_child(child);
                }
            }
            node.add_child(Rc::new(tpl_node));
            Some(Rc::new(node))
        }
        Expression::BinaryExpression(bin_expr) => {
            let mut node = TreeNode::new(
                format!("{:?}", bin_expr.operator),
                "BinaryExpression".to_string(),
                *id_counter,
            );
            *id_counter += 1;

            if let Some(left_node) = expression_to_tree_node(&bin_expr.left, id_counter) {
                node.add_child(left_node);
            }

            if let Some(right_node) = expression_to_tree_node(&bin_expr.right, id_counter) {
                node.add_child(right_node);
            }

            Some(Rc::new(node))
        }
        Expression::LogicalExpression(log_expr) => {
            let mut node = TreeNode::new(
                format!("{:?}", log_expr.operator),
                "LogicalExpression".to_string(),
                *id_counter,
            );
            *id_counter += 1;
            if let Some(left_node) = expression_to_tree_node(&log_expr.left, id_counter) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&log_expr.right, id_counter) {
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
            if canonicalize_enabled() && assign.operator == AssignmentOperator::Assign {
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
                        let mut node = TreeNode::new(
                            operator_label,
                            "AssignmentExpression".to_string(),
                            *id_counter,
                        );
                        *id_counter += 1;
                        if let Some(left_node) =
                            assignment_target_to_tree_node(&assign.left, id_counter)
                        {
                            node.add_child(left_node);
                        }
                        if let Some(right_node) =
                            expression_to_tree_node(&binary.right, id_counter)
                        {
                            node.add_child(right_node);
                        }
                        return Some(Rc::new(node));
                    }
                }
            }
            // Operator (=, +=, -=, etc.) directly distinguishes assignment
            // shapes that would otherwise look identical.
            let mut node = TreeNode::new(
                format!("{:?}", assign.operator),
                "AssignmentExpression".to_string(),
                *id_counter,
            );
            *id_counter += 1;
            if let Some(left_node) = assignment_target_to_tree_node(&assign.left, id_counter) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&assign.right, id_counter) {
                node.add_child(right_node);
            }
            Some(Rc::new(node))
        }
        Expression::UnaryExpression(unary) => {
            let mut node = TreeNode::new(
                format!("{:?}", unary.operator),
                "UnaryExpression".to_string(),
                *id_counter,
            );
            *id_counter += 1;
            if let Some(arg_node) = expression_to_tree_node(&unary.argument, id_counter) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Expression::UpdateExpression(update) => {
            let prefix_marker = if update.prefix { "prefix" } else { "postfix" };
            let label = format!("{:?}-{}", update.operator, prefix_marker);
            let mut node = TreeNode::new(label, "UpdateExpression".to_string(), *id_counter);
            *id_counter += 1;
            if let Some(arg_node) = simple_assignment_target_to_tree_node(&update.argument, id_counter) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Expression::ConditionalExpression(cond) => {
            let mut node = make_node("ConditionalExpression", "ConditionalExpression", id_counter);
            if let Some(test_node) = expression_to_tree_node(&cond.test, id_counter) {
                node.add_child(test_node);
            }
            if let Some(cons_node) = expression_to_tree_node(&cond.consequent, id_counter) {
                node.add_child(cons_node);
            }
            if let Some(alt_node) = expression_to_tree_node(&cond.alternate, id_counter) {
                node.add_child(alt_node);
            }
            Some(Rc::new(node))
        }
        Expression::SequenceExpression(seq) => {
            let mut node = make_node("SequenceExpression", "SequenceExpression", id_counter);
            for e in &seq.expressions {
                if let Some(child) = expression_to_tree_node(e, id_counter) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ParenthesizedExpression(paren) => {
            // Parentheses are noise — descend straight through them.
            expression_to_tree_node(&paren.expression, id_counter)
        }
        Expression::StaticMemberExpression(mem) => {
            let label = if mem.optional { "?." } else { "." };
            let mut node = TreeNode::new(
                label.to_string(),
                "StaticMemberExpression".to_string(),
                *id_counter,
            );
            *id_counter += 1;
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(mem.property.name.as_str(), "Identifier", id_counter));
            Some(Rc::new(node))
        }
        Expression::ComputedMemberExpression(mem) => {
            let label = if mem.optional { "?.[]" } else { "[]" };
            let mut node = TreeNode::new(
                label.to_string(),
                "ComputedMemberExpression".to_string(),
                *id_counter,
            );
            *id_counter += 1;
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            if let Some(prop_node) = expression_to_tree_node(&mem.expression, id_counter) {
                node.add_child(prop_node);
            }
            Some(Rc::new(node))
        }
        Expression::PrivateFieldExpression(mem) => {
            let label = if mem.optional { "?.#" } else { ".#" };
            let mut node = TreeNode::new(
                label.to_string(),
                "PrivateFieldExpression".to_string(),
                *id_counter,
            );
            *id_counter += 1;
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(
                &format!("#{}", mem.field.name.as_str()),
                "PrivateIdentifier",
                id_counter,
            ));
            Some(Rc::new(node))
        }
        Expression::CallExpression(call_expr) => {
            if canonicalize_enabled() {
                if let Some(node) = lower_object_assign_to_spread(call_expr, id_counter) {
                    return Some(node);
                }
            }
            let label = if call_expr.optional { "?.()" } else { "CallExpression" };
            let mut node =
                TreeNode::new(label.to_string(), "CallExpression".to_string(), *id_counter);
            *id_counter += 1;

            if let Some(callee_node) = expression_to_tree_node(&call_expr.callee, id_counter) {
                node.add_child(callee_node);
            }

            for arg in &call_expr.arguments {
                if let Some(arg_node) = argument_to_tree_node(arg, id_counter) {
                    node.add_child(arg_node);
                }
            }

            Some(Rc::new(node))
        }
        Expression::NewExpression(new_expr) => {
            let mut node = make_node("NewExpression", "NewExpression", id_counter);
            if let Some(callee_node) = expression_to_tree_node(&new_expr.callee, id_counter) {
                node.add_child(callee_node);
            }
            for arg in &new_expr.arguments {
                if let Some(arg_node) = argument_to_tree_node(arg, id_counter) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        Expression::AwaitExpression(await_expr) => {
            let mut node = make_node("AwaitExpression", "AwaitExpression", id_counter);
            if let Some(arg_node) = expression_to_tree_node(&await_expr.argument, id_counter) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Expression::YieldExpression(yield_expr) => {
            let label = if yield_expr.delegate { "YieldDelegateExpression" } else { "YieldExpression" };
            let mut node = make_node(label, "YieldExpression", id_counter);
            if let Some(arg) = &yield_expr.argument {
                if let Some(arg_node) = expression_to_tree_node(arg, id_counter) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ObjectExpression(obj) => {
            let mut node = make_node("ObjectExpression", "ObjectExpression", id_counter);
            for prop in &obj.properties {
                if let Some(child) = object_property_to_tree_node(prop, id_counter) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ArrayExpression(arr) => {
            let mut node = make_node("ArrayExpression", "ArrayExpression", id_counter);
            for elem in &arr.elements {
                if let Some(child) = array_element_to_tree_node(elem, id_counter) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ThisExpression(_) => Some(leaf("ThisExpression", "ThisExpression", id_counter)),
        Expression::Super(_) => Some(leaf("Super", "Super", id_counter)),
        Expression::MetaProperty(meta) => {
            let label = format!(
                "{}.{}",
                meta.meta.name.as_str(),
                meta.property.name.as_str()
            );
            Some(leaf(&label, "MetaProperty", id_counter))
        }
        Expression::ImportExpression(import_expr) => {
            let mut node = make_node("ImportExpression", "ImportExpression", id_counter);
            if let Some(child) = expression_to_tree_node(&import_expr.source, id_counter) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        Expression::ChainExpression(chain) => {
            // Optional chaining wrapper — descend through.
            let mut node = make_node("ChainExpression", "ChainExpression", id_counter);
            if let Some(child) = chain_element_to_tree_node(&chain.expression, id_counter) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        Expression::ArrowFunctionExpression(arrow) => {
            // Encode async/generator into the label so async vs sync arrow
            // functions don't collapse into the same node.
            let label = if arrow.r#async { "AsyncArrowFunction" } else { "ArrowFunction" };
            let mut node =
                make_node(label, "ArrowFunctionExpression", id_counter);

            for param in &arrow.params.items {
                if let Some(param_node) = formal_parameter_to_tree_node(param, id_counter) {
                    node.add_child(param_node);
                }
            }

            if arrow.expression {
                if let Some(Statement::ExpressionStatement(expr_stmt)) =
                    arrow.body.statements.first()
                {
                    if let Some(expr_node) =
                        expression_to_tree_node(&expr_stmt.expression, id_counter)
                    {
                        node.add_child(expr_node);
                    }
                }
            } else {
                if let Some(body_node) = function_body_to_tree_node(&arrow.body, id_counter) {
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
            let mut node = make_node(label, "FunctionExpression", id_counter);
            for param in &func.params.items {
                if let Some(p) = formal_parameter_to_tree_node(param, id_counter) {
                    node.add_child(p);
                }
            }
            if let Some(body) = &func.body {
                if let Some(b) = function_body_to_tree_node(body, id_counter) {
                    node.add_child(b);
                }
            }
            Some(Rc::new(node))
        }
        Expression::ClassExpression(class) => {
            class_declaration_to_tree_node(class, id_counter, "ClassExpression")
        }
        Expression::TSAsExpression(ts) => expression_to_tree_node(&ts.expression, id_counter),
        Expression::TSSatisfiesExpression(ts) => {
            expression_to_tree_node(&ts.expression, id_counter)
        }
        Expression::TSNonNullExpression(ts) => expression_to_tree_node(&ts.expression, id_counter),
        Expression::TSTypeAssertion(ts) => expression_to_tree_node(&ts.expression, id_counter),
        Expression::TSInstantiationExpression(ts) => {
            expression_to_tree_node(&ts.expression, id_counter)
        }
        _ => Some(leaf("Expression", "Expression", id_counter)),
    }
}

fn argument_to_tree_node(
    arg: &oxc_ast::ast::Argument,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::Argument;
    match arg {
        Argument::SpreadElement(spread) => {
            let mut node = make_node("SpreadElement", "SpreadElement", id_counter);
            if let Some(child) = expression_to_tree_node(&spread.argument, id_counter) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        _ => arg
            .as_expression()
            .and_then(|expr| expression_to_tree_node(expr, id_counter)),
    }
}

fn array_element_to_tree_node(
    elem: &oxc_ast::ast::ArrayExpressionElement,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::ArrayExpressionElement;
    match elem {
        ArrayExpressionElement::SpreadElement(spread) => {
            let mut node = make_node("SpreadElement", "SpreadElement", id_counter);
            if let Some(child) = expression_to_tree_node(&spread.argument, id_counter) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        ArrayExpressionElement::Elision(_) => Some(leaf("Elision", "Elision", id_counter)),
        _ => elem
            .as_expression()
            .and_then(|expr| expression_to_tree_node(expr, id_counter)),
    }
}

fn object_property_to_tree_node(
    prop: &oxc_ast::ast::ObjectPropertyKind,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::ObjectPropertyKind;
    match prop {
        ObjectPropertyKind::ObjectProperty(p) => {
            let mut node = make_node("ObjectProperty", "ObjectProperty", id_counter);
            if let Some(key_node) = property_key_to_tree_node(&p.key, id_counter) {
                node.add_child(key_node);
            }
            // Only include the value if it differs from a shorthand id mention
            // (oxc still gives us a value node even in shorthand form, which
            // we keep — TSED size-normalisation handles the small inflation).
            if let Some(val_node) = expression_to_tree_node(&p.value, id_counter) {
                node.add_child(val_node);
            }
            Some(Rc::new(node))
        }
        ObjectPropertyKind::SpreadProperty(spread) => {
            let mut node = make_node("SpreadProperty", "SpreadProperty", id_counter);
            if let Some(child) = expression_to_tree_node(&spread.argument, id_counter) {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
    }
}

fn property_key_to_tree_node(
    key: &PropertyKey,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    match key {
        PropertyKey::StaticIdentifier(ident) => Some(leaf(
            ident.name.as_str(),
            "Identifier",
            id_counter,
        )),
        PropertyKey::PrivateIdentifier(ident) => Some(leaf(
            &format!("#{}", ident.name.as_str()),
            "PrivateIdentifier",
            id_counter,
        )),
        _ => key
            .as_expression()
            .and_then(|expr| expression_to_tree_node(expr, id_counter)),
    }
}

fn assignment_target_to_tree_node(
    target: &oxc_ast::ast::AssignmentTarget,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::AssignmentTarget;
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(ident) => Some(leaf(
            ident.name.as_str(),
            "Identifier",
            id_counter,
        )),
        AssignmentTarget::ArrayAssignmentTarget(_) => Some(leaf(
            "ArrayAssignmentTarget",
            "ArrayAssignmentTarget",
            id_counter,
        )),
        AssignmentTarget::ObjectAssignmentTarget(_) => Some(leaf(
            "ObjectAssignmentTarget",
            "ObjectAssignmentTarget",
            id_counter,
        )),
        AssignmentTarget::StaticMemberExpression(mem) => {
            let mut node = make_node(".", "StaticMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(mem.property.name.as_str(), "Identifier", id_counter));
            Some(Rc::new(node))
        }
        AssignmentTarget::ComputedMemberExpression(mem) => {
            let mut node = make_node("[]", "ComputedMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            if let Some(prop_node) = expression_to_tree_node(&mem.expression, id_counter) {
                node.add_child(prop_node);
            }
            Some(Rc::new(node))
        }
        AssignmentTarget::PrivateFieldExpression(mem) => {
            let mut node = make_node(".#", "PrivateFieldExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(
                &format!("#{}", mem.field.name.as_str()),
                "PrivateIdentifier",
                id_counter,
            ));
            Some(Rc::new(node))
        }
        _ => Some(leaf("AssignmentTarget", "AssignmentTarget", id_counter)),
    }
}

fn simple_assignment_target_to_tree_node(
    target: &oxc_ast::ast::SimpleAssignmentTarget,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::SimpleAssignmentTarget;
    match target {
        SimpleAssignmentTarget::AssignmentTargetIdentifier(ident) => Some(leaf(
            ident.name.as_str(),
            "Identifier",
            id_counter,
        )),
        SimpleAssignmentTarget::StaticMemberExpression(mem) => {
            let mut node = make_node(".", "StaticMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(mem.property.name.as_str(), "Identifier", id_counter));
            Some(Rc::new(node))
        }
        SimpleAssignmentTarget::ComputedMemberExpression(mem) => {
            let mut node = make_node("[]", "ComputedMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            if let Some(prop_node) = expression_to_tree_node(&mem.expression, id_counter) {
                node.add_child(prop_node);
            }
            Some(Rc::new(node))
        }
        SimpleAssignmentTarget::PrivateFieldExpression(mem) => {
            let mut node = make_node(".#", "PrivateFieldExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(
                &format!("#{}", mem.field.name.as_str()),
                "PrivateIdentifier",
                id_counter,
            ));
            Some(Rc::new(node))
        }
        _ => Some(leaf("SimpleAssignmentTarget", "SimpleAssignmentTarget", id_counter)),
    }
}

fn chain_element_to_tree_node(
    elem: &oxc_ast::ast::ChainElement,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    use oxc_ast::ast::ChainElement;
    match elem {
        ChainElement::CallExpression(call) => {
            let mut node =
                TreeNode::new("?.()".to_string(), "CallExpression".to_string(), *id_counter);
            *id_counter += 1;
            if let Some(callee_node) = expression_to_tree_node(&call.callee, id_counter) {
                node.add_child(callee_node);
            }
            for arg in &call.arguments {
                if let Some(arg_node) = argument_to_tree_node(arg, id_counter) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        ChainElement::StaticMemberExpression(mem) => {
            let mut node = make_node("?.", "StaticMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(mem.property.name.as_str(), "Identifier", id_counter));
            Some(Rc::new(node))
        }
        ChainElement::ComputedMemberExpression(mem) => {
            let mut node = make_node("?.[]", "ComputedMemberExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            if let Some(prop_node) = expression_to_tree_node(&mem.expression, id_counter) {
                node.add_child(prop_node);
            }
            Some(Rc::new(node))
        }
        ChainElement::PrivateFieldExpression(mem) => {
            let mut node = make_node("?.#", "PrivateFieldExpression", id_counter);
            if let Some(obj_node) = expression_to_tree_node(&mem.object, id_counter) {
                node.add_child(obj_node);
            }
            node.add_child(leaf(
                &format!("#{}", mem.field.name.as_str()),
                "PrivateIdentifier",
                id_counter,
            ));
            Some(Rc::new(node))
        }
        _ => Some(leaf("ChainElement", "ChainElement", id_counter)),
    }
}

fn formal_parameter_to_tree_node(
    param: &FormalParameter,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    // Wrap the binding pattern under a `Parameter` node so the role is
    // preserved even when the binding itself is a destructuring pattern.
    let mut node = make_node("Parameter", "Parameter", id_counter);
    if let Some(child) = binding_pattern_to_tree_node(&param.pattern, id_counter) {
        node.add_child(child);
    }
    Some(Rc::new(node))
}

fn binding_pattern_to_tree_node(
    pattern: &BindingPattern,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    match pattern {
        BindingPattern::BindingIdentifier(ident) => Some(leaf(
            ident.name.as_str(),
            "BindingIdentifier",
            id_counter,
        )),
        BindingPattern::ObjectPattern(obj) => {
            let mut node = make_node("ObjectPattern", "ObjectPattern", id_counter);
            for prop in &obj.properties {
                let mut prop_node = make_node("BindingProperty", "BindingProperty", id_counter);
                if let Some(key_node) = property_key_to_tree_node(&prop.key, id_counter) {
                    prop_node.add_child(key_node);
                }
                if let Some(val_node) = binding_pattern_to_tree_node(&prop.value, id_counter) {
                    prop_node.add_child(val_node);
                }
                node.add_child(Rc::new(prop_node));
            }
            if let Some(rest) = &obj.rest {
                let mut rest_node = make_node("RestElement", "RestElement", id_counter);
                if let Some(arg_node) = binding_pattern_to_tree_node(&rest.argument, id_counter) {
                    rest_node.add_child(arg_node);
                }
                node.add_child(Rc::new(rest_node));
            }
            Some(Rc::new(node))
        }
        BindingPattern::ArrayPattern(arr) => {
            let mut node = make_node("ArrayPattern", "ArrayPattern", id_counter);
            for elem in &arr.elements {
                if let Some(elem_pat) = elem {
                    if let Some(child) = binding_pattern_to_tree_node(elem_pat, id_counter) {
                        node.add_child(child);
                    }
                } else {
                    node.add_child(leaf("Elision", "Elision", id_counter));
                }
            }
            if let Some(rest) = &arr.rest {
                let mut rest_node = make_node("RestElement", "RestElement", id_counter);
                if let Some(arg_node) = binding_pattern_to_tree_node(&rest.argument, id_counter) {
                    rest_node.add_child(arg_node);
                }
                node.add_child(Rc::new(rest_node));
            }
            Some(Rc::new(node))
        }
        BindingPattern::AssignmentPattern(assign) => {
            let mut node = make_node("AssignmentPattern", "AssignmentPattern", id_counter);
            if let Some(left_node) = binding_pattern_to_tree_node(&assign.left, id_counter) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&assign.right, id_counter) {
                node.add_child(right_node);
            }
            Some(Rc::new(node))
        }
    }
}

fn function_body_to_tree_node(body: &FunctionBody, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
    let mut node =
        TreeNode::new("BlockStatement".to_string(), "BlockStatement".to_string(), *id_counter);
    *id_counter += 1;

    for stmt in &body.statements {
        append_statement_to_list(&mut node, stmt, id_counter);
    }

    Some(Rc::new(node))
}

fn block_statement_to_tree_node(
    block: &BlockStatement,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    let mut node =
        TreeNode::new("BlockStatement".to_string(), "BlockStatement".to_string(), *id_counter);
    *id_counter += 1;

    for stmt in &block.body {
        append_statement_to_list(&mut node, stmt, id_counter);
    }

    Some(Rc::new(node))
}

fn variable_declarator_to_tree_node(
    decl: &VariableDeclarator,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    let mut node = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
    if let Some(pat_node) = binding_pattern_to_tree_node(&decl.id, id_counter) {
        node.add_child(pat_node);
    }
    if let Some(init) = &decl.init {
        if let Some(init_node) = expression_to_tree_node(init, id_counter) {
            node.add_child(init_node);
        }
    }
    Some(Rc::new(node))
}

fn class_element_to_tree_node(
    element: &ClassElement,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    match element {
        ClassElement::MethodDefinition(method) => {
            // Encode kind (constructor/get/set/method) + static into the
            // label so disparate method shapes don't collapse.
            let kind_marker = match method.kind {
                oxc_ast::ast::MethodDefinitionKind::Constructor => "Constructor",
                oxc_ast::ast::MethodDefinitionKind::Method => "Method",
                oxc_ast::ast::MethodDefinitionKind::Get => "Getter",
                oxc_ast::ast::MethodDefinitionKind::Set => "Setter",
            };
            let key_label = property_key_label(&method.key);
            let static_marker = if method.r#static { "static_" } else { "" };
            let label = format!("{}{}:{}", static_marker, kind_marker, key_label);
            let mut node = TreeNode::new(label, "MethodDefinition".to_string(), *id_counter);
            *id_counter += 1;

            // Walk the function value structurally — parameters and body
            // are part of the method's identity.
            for param in &method.value.params.items {
                if let Some(p) = formal_parameter_to_tree_node(param, id_counter) {
                    node.add_child(p);
                }
            }
            if let Some(body) = &method.value.body {
                if let Some(body_node) = function_body_to_tree_node(body, id_counter) {
                    node.add_child(body_node);
                }
            }

            Some(Rc::new(node))
        }
        ClassElement::PropertyDefinition(prop) => {
            let key_label = property_key_label(&prop.key);
            let static_marker = if prop.r#static { "static_" } else { "" };
            let label = format!("{}{}", static_marker, key_label);
            let mut node = TreeNode::new(label, "PropertyDefinition".to_string(), *id_counter);
            *id_counter += 1;

            if let Some(value) = &prop.value {
                if let Some(child) = expression_to_tree_node(value, id_counter) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        ClassElement::StaticBlock(block) => {
            let mut node = make_node("StaticBlock", "StaticBlock", id_counter);
            for stmt in &block.body {
                append_statement_to_list(&mut node, stmt, id_counter);
            }
            Some(Rc::new(node))
        }
        ClassElement::AccessorProperty(_) => {
            Some(leaf("AccessorProperty", "AccessorProperty", id_counter))
        }
        _ => None,
    }
}

#[cfg(test)]
mod canonicalization_tests {
    use super::*;
    use crate::apted::APTEDOptions;
    use crate::apted::compute_edit_distance;

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
            node.value == kind || node.children.iter().any(|c| contains_kind(c, kind))
        }
        assert!(!contains_kind(&canonical, "TemplateLiteral"));
        assert!(contains_kind(&plain, "TemplateLiteral"));
    }
}
