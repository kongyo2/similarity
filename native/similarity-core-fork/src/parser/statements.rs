//! Statements and declarations.

use oxc_ast::ast::{
    AssignmentOperator, BlockStatement, Class, Declaration, ExportDefaultDeclarationKind,
    Expression, ForStatementInit, ForStatementLeft, Function, FunctionBody, Statement,
    VariableDeclaration, VariableDeclarator,
};
use oxc_span::GetSpan;
use std::rc::Rc;

use super::classes::class_element_to_tree_node;
use super::expressions::{
    assignment_target_to_tree_node, element_access_node, expression_to_tree_node,
    private_field_node, statement_value_expression_to_tree_node, static_member_node,
    test_expression_to_tree_node,
};
use super::lowering::{
    assignment_leaf_statement, canonical_if_nodes, canonical_return_nodes,
    lower_declaration_ternary, lower_destructuring_declaration, lower_foreach_statement,
    lower_switch_statement, lower_ternary_chain, lower_then_call, match_then_call,
    statements_contain_own_return, CallbackBody, ThenPosition,
};
use super::patterns::{binding_pattern_to_tree_node, formal_parameters_to_tree_nodes};
use super::peephole::normalize_statement_nodes;
use super::{leaf, make_node, stamp, strip_parentheses, ConversionContext};
use crate::tree::TreeNode;

/// Convert one statement into the nodes it contributes to a statement
/// list. Almost always a single node, but canonical lowering of
/// `return p.then(cb)` / `p.then(cb);` and of declaration-initializing
/// ternaries (`const x = c ? a : b`) expands one statement into several.
/// Convert one statement into the nodes it contributes to a statement
/// list, stamping each top-level node with the statement's source span so
/// downstream consumers (the overlap fingerprints in particular) can
/// report real line ranges.
pub(super) fn statement_to_tree_nodes(
    stmt: &Statement,
    cx: &ConversionContext,
) -> Vec<Rc<TreeNode>> {
    let span = stmt.span();
    statement_to_tree_nodes_unstamped(stmt, cx).into_iter().map(|node| stamp(node, span)).collect()
}

pub(super) fn statement_to_tree_nodes_unstamped(
    stmt: &Statement,
    cx: &ConversionContext,
) -> Vec<Rc<TreeNode>> {
    if cx.canonical {
        match stmt {
            Statement::ReturnStatement(ret_stmt) => {
                return canonical_return_nodes(ret_stmt.argument.as_ref(), cx);
            }
            Statement::IfStatement(if_stmt) => {
                return canonical_if_nodes(if_stmt, cx);
            }
            Statement::ExpressionStatement(expr_stmt) => {
                let mut effective = strip_parentheses(&expr_stmt.expression);
                if let Expression::AwaitExpression(await_expr) = effective {
                    effective = strip_parentheses(&await_expr.argument);
                }
                if let Some(parts) = match_then_call(effective) {
                    // In statement position a `return` inside the callback
                    // only exits the callback; inlined it would read as an
                    // early exit of the enclosing function — a genuinely
                    // different control flow. (In return position callback
                    // returns map onto function returns exactly, so no
                    // guard is needed there.)
                    let callback_returns = match &parts.body {
                        CallbackBody::Block(statements) => {
                            statements_contain_own_return(statements)
                        }
                        CallbackBody::Expression(_) => false,
                    };
                    if !callback_returns {
                        return lower_then_call(&parts, ThenPosition::Statement, cx);
                    }
                }
            }
            Statement::VariableDeclaration(var_decl) => {
                if let Some(nodes) = lower_declaration_ternary(var_decl, cx) {
                    return nodes;
                }
                if let Some(nodes) = lower_destructuring_declaration(var_decl, cx) {
                    return nodes;
                }
            }
            Statement::SwitchStatement(switch_stmt) => {
                if let Some(nodes) = lower_switch_statement(switch_stmt, cx) {
                    return nodes;
                }
            }
            _ => {}
        }
    }
    statement_to_tree_node(stmt, cx).into_iter().collect()
}

pub(super) fn append_statement_to_list(
    parent: &mut TreeNode,
    stmt: &Statement,
    cx: &ConversionContext,
) {
    for node in statement_to_tree_nodes(stmt, cx) {
        parent.add_child(node);
    }
}

/// Canonical-mode body conversion for `if`/loop bodies: wraps a
/// single-statement body in a synthetic block so `if (x) return y;` and
/// `if (x) { return y; }` produce the same tree. Outside canonical mode
/// this is exactly `statement_to_tree_node`.
pub(super) fn statement_to_block_node(
    stmt: &Statement,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    if !cx.canonical || matches!(stmt, Statement::BlockStatement(_)) {
        return statement_to_tree_node(stmt, cx);
    }
    Some(block_of(statement_to_tree_nodes(stmt, cx), cx))
}

/// One node for a statement that is the body of a label or a `with`: the
/// statement itself, or — when a canonical lowering turned it into
/// several — a block holding them.
fn statement_to_single_node(stmt: &Statement, cx: &ConversionContext) -> Option<Rc<TreeNode>> {
    let mut nodes = statement_to_tree_nodes(stmt, cx);
    if nodes.len() == 1 {
        nodes.pop()
    } else {
        Some(block_of(nodes, cx))
    }
}

/// Append every statement of `stmt` (flattening a block) to `nodes` via
/// the canonical statement conversion.
pub(super) fn flatten_statement_into(
    nodes: &mut Vec<Rc<TreeNode>>,
    stmt: &Statement,
    cx: &ConversionContext,
) {
    if let Statement::BlockStatement(block) = stmt {
        for inner in &block.body {
            nodes.extend(statement_to_tree_nodes(inner, cx));
        }
    } else {
        nodes.extend(statement_to_tree_nodes(stmt, cx));
    }
}

pub(super) fn statement_to_tree_node(
    stmt: &Statement,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            function_declaration_to_tree_node(func, cx, "Function")
        }
        Statement::ClassDeclaration(class) => class_declaration_to_tree_node(class, cx, "Class"),
        Statement::VariableDeclaration(var_decl) => variable_declaration_to_tree_node(var_decl, cx),
        Statement::ExpressionStatement(expr_stmt) => {
            if cx.canonical {
                let effective = strip_parentheses(&expr_stmt.expression);
                if let Some(node) = lower_foreach_statement(effective, cx) {
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
                                &|expr| {
                                    let target_node = assignment_target_to_tree_node(target, cx);
                                    assignment_leaf_statement(target_node, expr, cx)
                                },
                                cx,
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
            let mut node = make_node("ExpressionStatement", "ExpressionStatement");
            if let Some(child) = statement_value_expression_to_tree_node(&expr_stmt.expression, cx)
            {
                node.add_child(child);
            }
            Some(Rc::new(node))
        }
        Statement::BlockStatement(block) => block_statement_to_tree_node(block, cx),
        Statement::ExportNamedDeclaration(export) => {
            if let Some(decl) = &export.declaration {
                declaration_to_tree_node(decl, cx)
            } else {
                Some(leaf("ExportNamedDeclaration", "ExportNamedDeclaration"))
            }
        }
        Statement::ExportDefaultDeclaration(export) => {
            export_default_declaration_to_tree_node(&export.declaration, cx)
        }
        Statement::IfStatement(if_stmt) => {
            let mut node = make_node("IfStatement", "IfStatement");

            if let Some(test_node) = expression_to_tree_node(&if_stmt.test, cx) {
                node.add_child(test_node);
            }
            if let Some(cons_node) = statement_to_block_node(&if_stmt.consequent, cx) {
                node.add_child(cons_node);
            }
            if let Some(alt) = &if_stmt.alternate {
                // `else if` chains stay bare so a lowered switch and a
                // hand-written chain produce the same nesting.
                let alt_node = if matches!(alt, Statement::IfStatement(_)) {
                    statement_to_tree_node(alt, cx)
                } else {
                    statement_to_block_node(alt, cx)
                };
                if let Some(alt_node) = alt_node {
                    node.add_child(alt_node);
                }
            }
            Some(Rc::new(node))
        }
        // Literal conversion: in canonical mode every `return` goes through
        // `canonical_return_nodes` (see `statement_to_tree_nodes`).
        Statement::ReturnStatement(ret_stmt) => {
            let mut node = make_node("ReturnStatement", "ReturnStatement");
            if let Some(arg) = &ret_stmt.argument {
                if let Some(arg_node) = expression_to_tree_node(arg, cx) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        Statement::ForStatement(for_stmt) => {
            // `for (; cond; )` is a `while (cond)` spelled differently, and
            // `for (;;)` is `while (true)` — canonicalize so the loop-form
            // choice doesn't register as a structural difference.
            if cx.canonical && for_stmt.init.is_none() && for_stmt.update.is_none() {
                let mut node = make_node("WhileStatement", "WhileStatement");
                if let Some(test) = &for_stmt.test {
                    if let Some(test_node) = test_expression_to_tree_node(test, cx) {
                        node.add_child(test_node);
                    }
                } else {
                    node.add_child(leaf("true", "BooleanLiteral"));
                }
                if let Some(body_node) = statement_to_block_node(&for_stmt.body, cx) {
                    node.add_child(body_node);
                }
                return Some(Rc::new(node));
            }
            let mut node = make_node("ForStatement", "ForStatement");
            if let Some(init) = &for_stmt.init {
                if let Some(init_node) = for_statement_init_to_tree_node(init, cx) {
                    node.add_child(init_node);
                }
            }
            if let Some(test) = &for_stmt.test {
                if let Some(test_node) = test_expression_to_tree_node(test, cx) {
                    node.add_child(test_node);
                }
            }
            if let Some(update) = &for_stmt.update {
                if let Some(update_node) = statement_value_expression_to_tree_node(update, cx) {
                    node.add_child(update_node);
                }
            }
            if let Some(body_node) = statement_to_block_node(&for_stmt.body, cx) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::ForOfStatement(for_of) => {
            // Encode `await` directly into the label so `for await` and
            // `for` of the same shape don't collapse together.
            let label = if for_of.r#await { "ForAwaitOfStatement" } else { "ForOfStatement" };
            let mut node = make_node(label, "ForOfStatement");
            if let Some(left_node) = for_statement_left_to_tree_node(&for_of.left, cx) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&for_of.right, cx) {
                node.add_child(right_node);
            }
            if let Some(body_node) = statement_to_block_node(&for_of.body, cx) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::ForInStatement(for_in) => {
            let mut node = make_node("ForInStatement", "ForInStatement");
            if let Some(left_node) = for_statement_left_to_tree_node(&for_in.left, cx) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&for_in.right, cx) {
                node.add_child(right_node);
            }
            if let Some(body_node) = statement_to_block_node(&for_in.body, cx) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::WhileStatement(while_stmt) => {
            let mut node = make_node("WhileStatement", "WhileStatement");
            if let Some(test_node) = test_expression_to_tree_node(&while_stmt.test, cx) {
                node.add_child(test_node);
            }
            if let Some(body_node) = statement_to_block_node(&while_stmt.body, cx) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::DoWhileStatement(do_stmt) => {
            let mut node = make_node("DoWhileStatement", "DoWhileStatement");
            if let Some(body_node) = statement_to_block_node(&do_stmt.body, cx) {
                node.add_child(body_node);
            }
            if let Some(test_node) = test_expression_to_tree_node(&do_stmt.test, cx) {
                node.add_child(test_node);
            }
            Some(Rc::new(node))
        }
        Statement::ThrowStatement(throw_stmt) => {
            let mut node = make_node("ThrowStatement", "ThrowStatement");
            if let Some(arg_node) = expression_to_tree_node(&throw_stmt.argument, cx) {
                node.add_child(arg_node);
            }
            Some(Rc::new(node))
        }
        Statement::TryStatement(try_stmt) => {
            let mut node = make_node("TryStatement", "TryStatement");
            if let Some(block_node) = block_statement_to_tree_node(&try_stmt.block, cx) {
                node.add_child(block_node);
            }
            if let Some(handler) = &try_stmt.handler {
                let mut catch_node = make_node("CatchClause", "CatchClause");
                if let Some(param) = &handler.param {
                    if let Some(param_node) = binding_pattern_to_tree_node(&param.pattern, cx) {
                        catch_node.add_child(param_node);
                    }
                }
                if let Some(body_node) = block_statement_to_tree_node(&handler.body, cx) {
                    catch_node.add_child(body_node);
                }
                node.add_child(stamp(Rc::new(catch_node), handler.span));
            }
            if let Some(finalizer) = &try_stmt.finalizer {
                let mut fin_node = make_node("FinallyClause", "FinallyClause");
                if let Some(body_node) = block_statement_to_tree_node(finalizer, cx) {
                    fin_node.add_child(body_node);
                }
                node.add_child(Rc::new(fin_node));
            }
            Some(Rc::new(node))
        }
        Statement::SwitchStatement(switch_stmt) => {
            let mut node = make_node("SwitchStatement", "SwitchStatement");
            if let Some(disc_node) = expression_to_tree_node(&switch_stmt.discriminant, cx) {
                node.add_child(disc_node);
            }
            for case in &switch_stmt.cases {
                let case_label = if case.test.is_some() { "SwitchCase" } else { "DefaultCase" };
                let mut case_node = make_node(case_label, "SwitchCase");
                if let Some(test) = &case.test {
                    if let Some(test_node) = expression_to_tree_node(test, cx) {
                        case_node.add_child(test_node);
                    }
                }
                for cons_stmt in &case.consequent {
                    append_statement_to_list(&mut case_node, cons_stmt, cx);
                }
                node.add_child(stamp(Rc::new(case_node), case.span));
            }
            Some(Rc::new(node))
        }
        Statement::BreakStatement(break_stmt) => {
            let label = break_stmt
                .label
                .as_ref()
                .map(|l| format!("break:{}", l.name.as_str()))
                .unwrap_or_else(|| "BreakStatement".to_string());
            Some(leaf(&label, "BreakStatement"))
        }
        Statement::ContinueStatement(cont_stmt) => {
            let label = cont_stmt
                .label
                .as_ref()
                .map(|l| format!("continue:{}", l.name.as_str()))
                .unwrap_or_else(|| "ContinueStatement".to_string());
            Some(leaf(&label, "ContinueStatement"))
        }
        Statement::LabeledStatement(labeled) => {
            let mut node =
                make_node(&format!("label:{}", labeled.label.name.as_str()), "LabeledStatement");
            if let Some(body_node) = statement_to_single_node(&labeled.body, cx) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::EmptyStatement(_) => Some(leaf("EmptyStatement", "EmptyStatement")),
        Statement::DebuggerStatement(_) => Some(leaf("DebuggerStatement", "DebuggerStatement")),
        Statement::WithStatement(with_stmt) => {
            let mut node = make_node("WithStatement", "WithStatement");
            if let Some(obj_node) = expression_to_tree_node(&with_stmt.object, cx) {
                node.add_child(obj_node);
            }
            if let Some(body_node) = statement_to_single_node(&with_stmt.body, cx) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        _ => {
            // Generic fallback for anything not specifically handled above
            // (predominantly TS-specific declarations we don't need to
            // structurally distinguish for the purposes of refactoring
            // detection).
            Some(leaf("Statement", "Statement"))
        }
    }
}

pub(super) fn for_statement_init_to_tree_node(
    init: &ForStatementInit,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match init {
        ForStatementInit::VariableDeclaration(var_decl) => {
            variable_declaration_to_tree_node(var_decl, cx)
        }
        _ => init.as_expression().and_then(|expr| expression_to_tree_node(expr, cx)),
    }
}

pub(super) fn for_statement_left_to_tree_node(
    left: &ForStatementLeft,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    // `ForStatementLeft` inherits every `AssignmentTarget` variant; they
    // convert the way assignment targets do, so `for (x of xs)` and
    // `for (obj.prop of xs)` stay distinct loop heads.
    Some(match left {
        ForStatementLeft::VariableDeclaration(var_decl) => {
            return variable_declaration_to_tree_node(var_decl, cx);
        }
        ForStatementLeft::AssignmentTargetIdentifier(ident) => {
            leaf(&cx.identifier_label(ident.span, &ident.name), "Identifier")
        }
        ForStatementLeft::StaticMemberExpression(mem) => static_member_node(mem, cx),
        ForStatementLeft::ComputedMemberExpression(mem) => element_access_node(mem, cx),
        ForStatementLeft::PrivateFieldExpression(mem) => private_field_node(mem, cx),
        ForStatementLeft::ArrayAssignmentTarget(_) => {
            leaf("ArrayAssignmentTarget", "ArrayAssignmentTarget")
        }
        ForStatementLeft::ObjectAssignmentTarget(_) => {
            leaf("ObjectAssignmentTarget", "ObjectAssignmentTarget")
        }
        _ => leaf("ForStatementLeft", "ForStatementLeft"),
    })
}

pub(super) fn declaration_to_tree_node(
    decl: &Declaration,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match decl {
        Declaration::FunctionDeclaration(func) => {
            function_declaration_to_tree_node(func, cx, "Function")
        }
        Declaration::ClassDeclaration(class) => class_declaration_to_tree_node(class, cx, "Class"),
        Declaration::VariableDeclaration(var_decl) => {
            variable_declaration_to_tree_node(var_decl, cx)
        }
        _ => Some(leaf("Declaration", "Declaration")),
    }
}

pub(super) fn export_default_declaration_to_tree_node(
    decl: &ExportDefaultDeclarationKind,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match decl {
        ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
            function_declaration_to_tree_node(func, cx, "DefaultFunction")
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            class_declaration_to_tree_node(class, cx, "DefaultClass")
        }
        _ => {
            // Try to treat it as an expression default export
            if let Some(expr) = decl.as_expression() {
                if let Some(node) = expression_to_tree_node(expr, cx) {
                    let mut wrapper =
                        make_node("ExportDefaultDeclaration", "ExportDefaultDeclaration");
                    wrapper.add_child(node);
                    return Some(Rc::new(wrapper));
                }
            }
            Some(leaf("ExportDefaultDeclaration", "ExportDefaultDeclaration"))
        }
    }
}

pub(super) fn function_declaration_to_tree_node(
    func: &Function,
    cx: &ConversionContext,
    default_label: &str,
) -> Option<Rc<TreeNode>> {
    let mut label = func
        .id
        .as_ref()
        .map_or_else(|| default_label.to_string(), |id| cx.identifier_label(id.span, &id.name));
    // Canonical mode encodes async / generator into the declaration label
    // (the way arrow and function expressions already do) so an `async`
    // function and its sync twin register as a labelled difference instead
    // of relying on their names happening to differ.
    if cx.canonical {
        if func.generator {
            label = format!("*{label}");
        }
        if func.r#async {
            label = format!("async {label}");
        }
    }
    let mut node = TreeNode::new(label, "FunctionDeclaration");

    for parameter in formal_parameters_to_tree_nodes(&func.params, cx) {
        node.add_child(parameter);
    }

    if let Some(body) = &func.body {
        if let Some(body_node) = function_body_to_tree_node(body, cx) {
            node.add_child(body_node);
        }
    }

    Some(Rc::new(node))
}

pub(super) fn class_declaration_to_tree_node(
    class: &Class,
    cx: &ConversionContext,
    default_label: &str,
) -> Option<Rc<TreeNode>> {
    let label = class
        .id
        .as_ref()
        .map_or_else(|| default_label.to_string(), |id| cx.identifier_label(id.span, &id.name));
    let mut node = TreeNode::new(label, "ClassDeclaration");

    for element in &class.body.body {
        if let Some(elem_node) = class_element_to_tree_node(element, cx) {
            node.add_child(elem_node);
        }
    }

    Some(Rc::new(node))
}

pub(super) fn variable_declaration_to_tree_node(
    var_decl: &VariableDeclaration,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    // Encode `var`/`let`/`const` directly into the label so a `const`
    // declaration and a `let` declaration of the same shape don't collapse.
    let label = format!("{:?}Declaration", var_decl.kind);
    let mut node = TreeNode::new(label, "VariableDeclaration");

    for decl in &var_decl.declarations {
        if let Some(decl_node) = variable_declarator_to_tree_node(decl, cx) {
            node.add_child(decl_node);
        }
    }

    Some(Rc::new(node))
}

pub(super) fn function_body_to_tree_node(
    body: &FunctionBody,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    let mut statements = directive_nodes(body);
    for stmt in &body.statements {
        statements.extend(statement_to_tree_nodes(stmt, cx));
    }
    Some(block_of(statements, cx))
}

/// The directive prologue of a function body (`"use strict"`,
/// `"use server"`): one leaf per directive. A directive changes how the
/// body runs, and oxc keeps it apart from the body's statements.
pub(super) fn directive_nodes(body: &FunctionBody) -> Vec<Rc<TreeNode>> {
    body.directives
        .iter()
        .map(|directive| leaf(&format!("\"{}\"", directive.directive.as_str()), "Directive"))
        .collect()
}

/// A block holding `statements`, after the canonical statement-list
/// peepholes.
pub(super) fn block_of(mut statements: Vec<Rc<TreeNode>>, cx: &ConversionContext) -> Rc<TreeNode> {
    normalize_statement_nodes(&mut statements, cx);
    let mut block = make_node("BlockStatement", "BlockStatement");
    for statement in statements {
        block.add_child(statement);
    }
    Rc::new(block)
}

pub(super) fn block_statement_to_tree_node(
    block: &BlockStatement,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    let mut statements = Vec::with_capacity(block.body.len());
    for stmt in &block.body {
        statements.extend(statement_to_tree_nodes(stmt, cx));
    }
    Some(block_of(statements, cx))
}

pub(super) fn variable_declarator_to_tree_node(
    decl: &VariableDeclarator,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    let mut node = make_node("VariableDeclarator", "VariableDeclarator");
    if let Some(pat_node) = binding_pattern_to_tree_node(&decl.id, cx) {
        node.add_child(pat_node);
    }
    if let Some(init) = &decl.init {
        if let Some(init_node) = expression_to_tree_node(init, cx) {
            node.add_child(init_node);
        }
    }
    Some(stamp(Rc::new(node), decl.span))
}
