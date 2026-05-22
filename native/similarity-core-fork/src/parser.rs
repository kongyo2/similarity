use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, BlockStatement, Class, ClassElement, Declaration, ExportDefaultDeclarationKind,
    Expression, FormalParameter, Function, FunctionBody, Program, PropertyKey, Statement,
    VariableDeclaration, VariableDeclarator,
};
use oxc_parser::Parser;
use oxc_span::SourceType;
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
        if let Some(child) = statement_to_tree_node(stmt, id_counter) {
            root.add_child(child);
        }
    }

    Rc::new(root)
}

fn make_node(label: &str, kind: &str, id_counter: &mut usize) -> TreeNode {
    let node = TreeNode::new(label.to_string(), kind.to_string(), *id_counter);
    *id_counter += 1;
    node
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
            // Wrap in ExpressionStatement so the role of an expression
            // appearing as a statement (e.g. `result += 1;`) is preserved
            // when comparing against bare expressions in other contexts.
            // The wrapper is a single extra node so it doesn't significantly
            // inflate tree sizes, but it does noticeably improve
            // discrimination on short loop-body diffs.
            let mut node = make_node("ExpressionStatement", "ExpressionStatement", id_counter);
            if let Some(child) = expression_to_tree_node(&expr_stmt.expression, id_counter) {
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
            if let Some(cons_node) = statement_to_tree_node(&if_stmt.consequent, id_counter) {
                node.add_child(cons_node);
            }
            if let Some(alt) = &if_stmt.alternate {
                if let Some(alt_node) = statement_to_tree_node(alt, id_counter) {
                    node.add_child(alt_node);
                }
            }
            Some(Rc::new(node))
        }
        Statement::ReturnStatement(ret_stmt) => {
            let mut node = make_node("ReturnStatement", "ReturnStatement", id_counter);
            if let Some(arg) = &ret_stmt.argument {
                if let Some(arg_node) = expression_to_tree_node(arg, id_counter) {
                    node.add_child(arg_node);
                }
            }
            Some(Rc::new(node))
        }
        Statement::ForStatement(for_stmt) => {
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
                if let Some(update_node) = expression_to_tree_node(update, id_counter) {
                    node.add_child(update_node);
                }
            }
            if let Some(body_node) = statement_to_tree_node(&for_stmt.body, id_counter) {
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
            if let Some(body_node) = statement_to_tree_node(&for_of.body, id_counter) {
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
            if let Some(body_node) = statement_to_tree_node(&for_in.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::WhileStatement(while_stmt) => {
            let mut node = make_node("WhileStatement", "WhileStatement", id_counter);
            if let Some(test_node) = expression_to_tree_node(&while_stmt.test, id_counter) {
                node.add_child(test_node);
            }
            if let Some(body_node) = statement_to_tree_node(&while_stmt.body, id_counter) {
                node.add_child(body_node);
            }
            Some(Rc::new(node))
        }
        Statement::DoWhileStatement(do_stmt) => {
            let mut node = make_node("DoWhileStatement", "DoWhileStatement", id_counter);
            if let Some(body_node) = statement_to_tree_node(&do_stmt.body, id_counter) {
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
                    if let Some(child) = statement_to_tree_node(cons_stmt, id_counter) {
                        case_node.add_child(child);
                    }
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
    let label = func.id.as_ref().map_or(default_label, |id| id.name.as_str()).to_string();
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
        if let Some(stmt_node) = statement_to_tree_node(stmt, id_counter) {
            node.add_child(stmt_node);
        }
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
        if let Some(stmt_node) = statement_to_tree_node(stmt, id_counter) {
            node.add_child(stmt_node);
        }
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
            let key_label = match &method.key {
                PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
                PropertyKey::PrivateIdentifier(ident) => format!("#{}", ident.name.as_str()),
                _ => "Method".to_string(),
            };
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
            let key_label = match &prop.key {
                PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
                PropertyKey::PrivateIdentifier(ident) => format!("#{}", ident.name.as_str()),
                _ => "Property".to_string(),
            };
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
                if let Some(child) = statement_to_tree_node(stmt, id_counter) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        ClassElement::AccessorProperty(_) => {
            Some(leaf("AccessorProperty", "AccessorProperty", id_counter))
        }
        _ => None,
    }
}
