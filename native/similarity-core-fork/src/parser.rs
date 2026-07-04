use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AssignmentOperator, BinaryOperator, BindingPattern, BlockStatement, Class, ClassElement,
    ConditionalExpression, Declaration, ExportDefaultDeclarationKind, Expression, FormalParameter,
    Function, FunctionBody, LogicalOperator, Program, PropertyKey, Statement, SwitchStatement,
    UnaryOperator, UpdateOperator, VariableDeclaration, VariableDeclarator,
};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

    // Alpha-renaming only applies in canonical mode: literal-shape
    // consumers (overlap windows, class/type comparators) must keep the
    // user's identifiers.
    let _rename_guard = if canonicalize_enabled() {
        Some(RenameGuard::install(build_rename_map(&ret.program)))
    } else {
        None
    };

    let mut id_counter = 0;
    let tree = ast_to_tree_node(&ret.program, &mut id_counter);
    if canonicalize_enabled() {
        // Structural rewrites (push-loop → map, temp-return elimination,
        // index-loop → for-of) can drop symbols entirely, which would
        // leave gaps in the declaration-ordered `§N` numbering and make
        // otherwise-identical trees differ by ordinal. Renumber by first
        // occurrence in the FINAL tree so the numbering only depends on
        // the shape being compared. The rebuild also assigns fresh
        // contiguous node ids.
        return Ok(relabel_canonical_ordinals(&tree));
    }
    Ok(tree)
}

/// Rebuild the tree, renumbering every `§N` token (in identifier-carrying
/// labels) by first DFS occurrence and assigning fresh contiguous ids.
fn relabel_canonical_ordinals(root: &Rc<TreeNode>) -> Rc<TreeNode> {
    fn remap_label(label: &str, mapping: &mut HashMap<String, String>, next: &mut usize) -> String {
        if !label.contains('§') {
            return label.to_string();
        }
        let mut result = String::with_capacity(label.len());
        let mut chars = label.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '§' {
                result.push(ch);
                continue;
            }
            let mut token = String::from("§");
            while let Some(&digit) = chars.peek() {
                if digit.is_ascii_digit() {
                    token.push(digit);
                    chars.next();
                } else {
                    break;
                }
            }
            if token.len() == 1 {
                result.push('§');
                continue;
            }
            let renamed = mapping.entry(token).or_insert_with(|| {
                let fresh = format!("§{next}");
                *next += 1;
                fresh
            });
            result.push_str(renamed);
        }
        result
    }

    fn rebuild(
        node: &TreeNode,
        mapping: &mut HashMap<String, String>,
        next: &mut usize,
        ids: &mut usize,
    ) -> TreeNode {
        let relabel = matches!(
            node.value.as_str(),
            "Identifier" | "BindingIdentifier" | "FunctionDeclaration" | "ClassDeclaration"
        );
        let label = if relabel {
            remap_label(&node.label, mapping, next)
        } else {
            node.label.clone()
        };
        let mut fresh = TreeNode::new(label, node.value.clone(), *ids);
        *ids += 1;
        fresh.source_span = node.source_span;
        for child in &node.children {
            fresh.add_child(Rc::new(rebuild(child, mapping, next, ids)));
        }
        fresh
    }

    let mut mapping = HashMap::new();
    let mut next = 0usize;
    let mut ids = 0usize;
    Rc::new(rebuild(root, &mut mapping, &mut next, &mut ids))
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

thread_local! {
    /// Span → canonical name for every symbol bound inside the fragment
    /// being converted. Populated per canonical parse; empty otherwise.
    static RENAMES: RefCell<HashMap<(u32, u32), Rc<str>>> = RefCell::new(HashMap::new());
}

/// Installs a rename map for the duration of one conversion and restores
/// the previous map on drop, mirroring [`CanonicalizeGuard`].
struct RenameGuard {
    previous: HashMap<(u32, u32), Rc<str>>,
}

impl RenameGuard {
    fn install(map: HashMap<(u32, u32), Rc<str>>) -> Self {
        let previous = RENAMES.with(|cell| cell.replace(map));
        RenameGuard { previous }
    }
}

impl Drop for RenameGuard {
    fn drop(&mut self) {
        let previous = std::mem::take(&mut self.previous);
        RENAMES.with(|cell| cell.replace(previous));
    }
}

/// Alpha-renaming map: every symbol *declared inside* the parsed fragment
/// (parameters, local `let`/`const`/`var` bindings, inner function/class
/// names, catch bindings, the fragment's own top-level name) is assigned a
/// positional canonical name `§0`, `§1`, … in source-declaration order,
/// and every resolved reference to it is renamed consistently.
///
/// This makes consistently-renamed duplicates (type-2 clones) compare as
/// exactly equal trees instead of paying a rename cost per occurrence —
/// the dominant reason rename-heavy duplicates used to fall below
/// threshold. Free identifiers (imports, globals, outer-scope captures,
/// property names) are *not* symbols of the fragment and keep their real
/// names, so calling `sendEmail` vs `sendSms` still registers as a
/// semantic difference.
///
/// `§` is not a valid identifier character in TypeScript, so canonical
/// names can never collide with real code.
fn build_rename_map(program: &Program) -> HashMap<(u32, u32), Rc<str>> {
    let semantic = SemanticBuilder::new().build(program).semantic;
    let scoping = semantic.scoping();
    let nodes = semantic.nodes();

    let mut symbols: Vec<_> = scoping.symbol_ids().collect();
    symbols.sort_by_key(|&symbol_id| {
        let span = scoping.symbol_span(symbol_id);
        (span.start, span.end)
    });

    let mut map = HashMap::new();
    for (ordinal, symbol_id) in symbols.into_iter().enumerate() {
        let canonical: Rc<str> = Rc::from(format!("§{ordinal}"));
        let declaration = scoping.symbol_span(symbol_id);
        map.insert((declaration.start, declaration.end), Rc::clone(&canonical));
        for &reference_id in scoping.get_resolved_reference_ids(symbol_id) {
            let reference = scoping.get_reference(reference_id);
            let span = nodes.get_node(reference.node_id()).kind().span();
            map.insert((span.start, span.end), Rc::clone(&canonical));
        }
    }
    map
}

/// Resolve the label for an identifier occurrence: the canonical `§N`
/// name when the span belongs to a fragment-local symbol, the original
/// name otherwise (free identifiers, property names, non-canonical mode).
fn identifier_label(span: Span, original: &str) -> String {
    if !canonicalize_enabled() {
        return original.to_string();
    }
    RENAMES.with(|cell| {
        cell.borrow()
            .get(&(span.start, span.end))
            .map_or_else(|| original.to_string(), |name| name.to_string())
    })
}

/// Whether `name` (as it appears in source) resolves to a fragment-local
/// binding at `span`. Used to guard builtin-global rewrites (`Boolean`,
/// `Object.assign`) against local shadowing.
fn is_locally_bound(span: Span) -> bool {
    RENAMES.with(|cell| cell.borrow().contains_key(&(span.start, span.end)))
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
/// Convert one statement into the nodes it contributes to a statement
/// list, stamping each top-level node with the statement's source span so
/// downstream consumers (the overlap fingerprints in particular) can
/// report real line ranges.
fn statement_to_tree_nodes(stmt: &Statement, id_counter: &mut usize) -> Vec<Rc<TreeNode>> {
    let span = stmt.span();
    let mut nodes = statement_to_tree_nodes_unstamped(stmt, id_counter);
    for node in &mut nodes {
        if node.source_span == (0, 0) {
            if let Some(unique) = Rc::get_mut(node) {
                unique.set_source_span(span.start, span.end);
            }
        }
    }
    nodes
}

fn statement_to_tree_nodes_unstamped(stmt: &Statement, id_counter: &mut usize) -> Vec<Rc<TreeNode>> {
    if canonicalize_enabled() {
        match stmt {
            Statement::ReturnStatement(ret_stmt) => {
                return canonical_return_nodes(ret_stmt.argument.as_ref(), id_counter);
            }
            Statement::IfStatement(if_stmt) => {
                return canonical_if_nodes(if_stmt, id_counter);
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
                        return lower_then_call(&parts, ThenPosition::Statement, id_counter);
                    }
                }
            }
            Statement::VariableDeclaration(var_decl) => {
                if let Some(nodes) = lower_declaration_ternary(var_decl, id_counter) {
                    return nodes;
                }
                if let Some(nodes) = lower_destructuring_declaration(var_decl, id_counter) {
                    return nodes;
                }
            }
            Statement::SwitchStatement(switch_stmt) => {
                if let Some(nodes) = lower_switch_statement(switch_stmt, id_counter) {
                    return nodes;
                }
            }
            _ => {}
        }
    }
    statement_to_tree_node(stmt, id_counter).into_iter().collect()
}

/// Canonical conversion of `return <argument>`: strips `return await X`
/// down to `return X`, inlines single-callback `.then()` chains, and
/// lowers ternary returns into the `if`/`else` chain shape. Shared by the
/// statement converter and the expression-bodied-arrow wrapper so
/// `(x) => expr` and `function (x) { return expr; }` produce identical
/// body trees.
fn canonical_return_nodes(
    argument: Option<&Expression>,
    id_counter: &mut usize,
) -> Vec<Rc<TreeNode>> {
    let Some(argument) = argument else {
        return vec![leaf("ReturnStatement", "ReturnStatement", id_counter)];
    };
    // `return await X` and `return X` resolve to the same value from the
    // caller's perspective; strip the await so the two spellings align.
    let mut effective = strip_parentheses(argument);
    if let Expression::AwaitExpression(await_expr) = effective {
        effective = strip_parentheses(&await_expr.argument);
    }
    if let Some(parts) = match_then_call(effective) {
        return lower_then_call(&parts, ThenPosition::Return, id_counter);
    }
    if let Expression::ConditionalExpression(cond) = effective {
        return lower_return_ternary(cond, id_counter);
    }
    let mut node = make_node("ReturnStatement", "ReturnStatement", id_counter);
    if let Some(arg_node) = expression_to_tree_node(effective, id_counter) {
        node.add_child(arg_node);
    }
    vec![Rc::new(node)]
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
    let mut statements = statement_to_tree_nodes(stmt, id_counter);
    normalize_statement_nodes(&mut statements, id_counter);
    for child in statements {
        block.add_child(child);
    }
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

/// Whether any statement in the list is a `return` belonging to the
/// list's own function scope. Recurses through blocks, branches, loops,
/// switch cases, try clauses and labels, but not into nested functions,
/// arrows or classes — their `return`s exit the nested scope, not this
/// one.
fn statements_contain_own_return<'a>(
    statements: &oxc_allocator::Vec<'a, Statement<'a>>,
) -> bool {
    statements.iter().any(statement_contains_own_return)
}

fn statement_contains_own_return(stmt: &Statement) -> bool {
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
        Statement::SwitchStatement(switch_stmt) => switch_stmt
            .cases
            .iter()
            .any(|case| statements_contain_own_return(&case.consequent)),
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

/// Split a ternary's pieces after negation normalization: a negated test
/// (`!c ? a : b`, `x !== y ? a : b`) swaps its branches so the emitted
/// `if` shows the positive condition, matching what [`canonical_if_nodes`]
/// does for hand-written `if (!c) { … } else { … }`.
struct NormalizedTernary<'a, 'b> {
    /// Converts the (positive-form) test into a tree.
    test_was_negated_binary: bool,
    test: &'b Expression<'a>,
    consequent: &'b Expression<'a>,
    alternate: &'b Expression<'a>,
}

impl NormalizedTernary<'_, '_> {
    fn render_test(&self, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
        if self.test_was_negated_binary {
            // The swap flipped `!==`/`!=` to its positive complement.
            negated_expression_to_tree_node(self.test, id_counter)
        } else {
            test_expression_to_tree_node(self.test, id_counter)
        }
    }
}

fn normalize_ternary<'a, 'b>(cond: &'b ConditionalExpression<'a>) -> NormalizedTernary<'a, 'b> {
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
fn lower_ternary_chain(
    cond: &ConditionalExpression,
    make_leaf_statement: &mut dyn FnMut(&Expression, &mut usize) -> Rc<TreeNode>,
    id_counter: &mut usize,
) -> Rc<TreeNode> {
    let normalized = normalize_ternary(cond);
    let mut if_node = make_node("IfStatement", "IfStatement", id_counter);
    if let Some(test_node) = normalized.render_test(id_counter) {
        if_node.add_child(test_node);
    }

    let mut consequent_block = make_node("BlockStatement", "BlockStatement", id_counter);
    consequent_block.add_child(make_leaf_statement(normalized.consequent, id_counter));
    if_node.add_child(Rc::new(consequent_block));

    let alternate = normalized.alternate;
    if let Expression::ConditionalExpression(nested) = alternate {
        if_node.add_child(lower_ternary_chain(nested, make_leaf_statement, id_counter));
    } else {
        let mut alternate_block = make_node("BlockStatement", "BlockStatement", id_counter);
        alternate_block.add_child(make_leaf_statement(alternate, id_counter));
        if_node.add_child(Rc::new(alternate_block));
    }
    Rc::new(if_node)
}

/// Lower `return c ? a : b` into the flat guard form
/// `if (c) { return a; } return b;` — the same shape hand-written
/// early-return code and the `if`/`else` spelling (after jump
/// flattening) normalize to. Branch values recurse through
/// [`canonical_return_nodes`], so nested ternaries flatten into
/// successive guards.
fn lower_return_ternary(
    cond: &ConditionalExpression,
    id_counter: &mut usize,
) -> Vec<Rc<TreeNode>> {
    let normalized = normalize_ternary(cond);
    let mut if_node = make_node("IfStatement", "IfStatement", id_counter);
    if let Some(test_node) = normalized.render_test(id_counter) {
        if_node.add_child(test_node);
    }
    let mut consequent_block = make_node("BlockStatement", "BlockStatement", id_counter);
    for node in canonical_return_nodes(Some(normalized.consequent), id_counter) {
        consequent_block.add_child(node);
    }
    if_node.add_child(Rc::new(consequent_block));

    let mut nodes = vec![Rc::new(if_node)];
    nodes.extend(canonical_return_nodes(Some(normalized.alternate), id_counter));
    nodes
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
    // Ternaries that contract to `||` / `&&` / `??` sugar are handled at
    // the expression level; lowering them to `if`/`else` here would hide
    // that equivalence.
    {
        let test = strip_parentheses(&cond.test);
        let consequent = strip_parentheses(&cond.consequent);
        let alternate = strip_parentheses(&cond.alternate);
        match classify_nullish_test(test) {
            Some(NullishTest::IsNullish(subject))
                if pure_expressions_equal(subject, alternate) =>
            {
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
            && (pure_expressions_equal(test, consequent)
                || pure_expressions_equal(test, alternate))
        {
            return None;
        }
    }

    // The lowered binding is mutated by the branches, so it canonicalizes
    // to `let` regardless of the source keyword.
    let name = identifier_label(ident.span, &ident.name);
    let mut decl = make_node("LetDeclaration", "VariableDeclaration", id_counter);
    let mut declarator_node = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
    declarator_node.add_child(leaf(&name, "BindingIdentifier", id_counter));
    decl.add_child(Rc::new(declarator_node));

    let if_node = lower_ternary_chain(
        cond,
        &mut |expr, ids| {
            let target = Some(leaf(&name, "Identifier", ids));
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
            (identifier_label(ident.span, &ident.name), body)
        }
        Expression::FunctionExpression(func) if !func.r#async && !func.generator => {
            if func.params.rest.is_some() || func.params.items.len() != 1 {
                return None;
            }
            let BindingPattern::BindingIdentifier(ident) = &func.params.items[0].pattern else {
                return None;
            };
            let body = func.body.as_ref()?;
            (identifier_label(ident.span, &ident.name), CallbackBody::Block(&body.statements))
        }
        _ => return None,
    };

    let mut for_node = make_node("ForOfStatement", "ForOfStatement", id_counter);
    let mut decl = make_node("ConstDeclaration", "VariableDeclaration", id_counter);
    let mut declarator = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
    declarator.add_child(leaf(&param_name, "BindingIdentifier", id_counter));
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

/// Case body minus the trailing unlabeled `break` (the switch's own exit,
/// which has no analogue in an if-chain; labeled breaks target an outer
/// statement and must stay).
fn switch_case_statements<'a, 'b>(case: &'b oxc_ast::ast::SwitchCase<'a>) -> Vec<&'b Statement<'a>> {
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
fn lower_switch_statement(
    switch_stmt: &SwitchStatement,
    id_counter: &mut usize,
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

    let case_body_block = |case: &oxc_ast::ast::SwitchCase, ids: &mut usize| -> Rc<TreeNode> {
        let mut block = make_node("BlockStatement", "BlockStatement", ids);
        let mut statements = Vec::new();
        for stmt in switch_case_statements(case) {
            statements.extend(statement_to_tree_nodes(stmt, ids));
        }
        normalize_statement_nodes(&mut statements, ids);
        for child in statements {
            block.add_child(child);
        }
        Rc::new(block)
    };

    let guard_node = |case: &oxc_ast::ast::SwitchCase,
                      test: &Expression,
                      ids: &mut usize|
     -> Rc<TreeNode> {
        let mut if_node = make_node("IfStatement", "IfStatement", ids);
        let mut equality = make_node("StrictEquality", "BinaryExpression", ids);
        if let Some(disc) = expression_to_tree_node(&switch_stmt.discriminant, ids) {
            equality.add_child(disc);
        }
        if let Some(test_node) = expression_to_tree_node(test, ids) {
            equality.add_child(test_node);
        }
        if_node.add_child(Rc::new(equality));
        if_node.add_child(case_body_block(case, ids));
        Rc::new(if_node)
    };

    // Whether a case body (after break-stripping) still always exits —
    // those cases become flat guards; break-terminated ones must keep the
    // else-chain so control flow past the switch stays represented.
    let case_flattens = |case: &oxc_ast::ast::SwitchCase| -> bool {
        switch_case_statements(case)
            .last()
            .is_some_and(|stmt| statement_always_terminates(stmt))
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
                    converted.extend(statement_to_tree_nodes(stmt, id_counter));
                }
                normalize_statement_nodes(&mut converted, id_counter);
                nodes.extend(converted);
                index += 1;
            }
            Some(test) if case_flattens(case) => {
                nodes.push(guard_node(case, test, id_counter));
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
                            else_node = Some(case_body_block(case, id_counter));
                        }
                        Some(test) => {
                            let if_node = guard_node(case, test, id_counter);
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
    if object_ident.name.as_str() != "Object" || is_locally_bound(object_ident.span) {
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

// ---------------------------------------------------------------------------
// Shared predicates for the value-preserving canonicalizations below
// ---------------------------------------------------------------------------

/// Side-effect-free expressions the canonicalizations may duplicate or
/// merge: literals, identifiers, `this`, and non-optional static member
/// chains of those. Property getters can technically observe reads — that
/// caveat is accepted by design, mirroring the common lint rewrites these
/// canonicalizations model.
fn is_side_effect_free(expr: &Expression) -> bool {
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
fn pure_expressions_equal(a: &Expression, b: &Expression) -> bool {
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
fn is_undefined_expr(expr: &Expression) -> bool {
    match strip_parentheses(expr) {
        Expression::Identifier(ident) => {
            ident.name == "undefined" && !is_locally_bound(ident.span)
        }
        Expression::UnaryExpression(unary) => {
            unary.operator == UnaryOperator::Void && is_side_effect_free(&unary.argument)
        }
        _ => false,
    }
}

fn is_null_expr(expr: &Expression) -> bool {
    matches!(strip_parentheses(expr), Expression::NullLiteral(_))
}

/// Literals eligible for the Yoda-order flip on symmetric equality
/// operators (`5 === x` ⇔ `x === 5`).
fn is_literal_operand(expr: &Expression) -> bool {
    matches!(
        strip_parentheses(expr),
        Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::BigIntLiteral(_)
    ) || is_undefined_expr(expr)
}

/// `subject === null` / `subject === undefined` (or the `!==` forms when
/// `expect_equal` is false), in either operand order, with a
/// side-effect-free subject. Returns the subject and whether the literal
/// side was `null` (vs `undefined`).
fn strict_null_comparison<'a, 'b>(
    expr: &'b Expression<'a>,
    expect_equal: bool,
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
    let (subject, literal) = if is_null_expr(&bin.right) || is_undefined_expr(&bin.right) {
        (&bin.left, &bin.right)
    } else if is_null_expr(&bin.left) || is_undefined_expr(&bin.left) {
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
enum NullishTest<'a, 'b> {
    IsNullish(&'b Expression<'a>),
    NotNullish(&'b Expression<'a>),
}

fn classify_nullish_test<'a, 'b>(expr: &'b Expression<'a>) -> Option<NullishTest<'a, 'b>> {
    let expr = strip_parentheses(expr);
    if let Expression::BinaryExpression(bin) = expr {
        let equal = match bin.operator {
            BinaryOperator::Equality => true,
            BinaryOperator::Inequality => false,
            _ => return None,
        };
        let subject = if is_null_expr(&bin.right) || is_undefined_expr(&bin.right) {
            &bin.left
        } else if is_null_expr(&bin.left) || is_undefined_expr(&bin.left) {
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
                let (left_subject, left_is_null) = strict_null_comparison(&log.left, true)?;
                let (right_subject, right_is_null) = strict_null_comparison(&log.right, true)?;
                if left_is_null != right_is_null
                    && pure_expressions_equal(left_subject, right_subject)
                {
                    return Some(NullishTest::IsNullish(left_subject));
                }
            }
            LogicalOperator::And => {
                let (left_subject, left_is_null) = strict_null_comparison(&log.left, false)?;
                let (right_subject, right_is_null) = strict_null_comparison(&log.right, false)?;
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

/// Build the canonical tree for `subject == null` (`equal`) or
/// `subject != null` — the target shape both the loose spelling and the
/// strict two-comparison spelling collapse onto.
fn nullish_comparison_node(
    subject: &Expression,
    equal: bool,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    let label = if equal { "Equality" } else { "Inequality" };
    let mut node = make_node(label, "BinaryExpression", id_counter);
    node.add_child(expression_to_tree_node(subject, id_counter)?);
    node.add_child(leaf("null", "NullLiteral", id_counter));
    Some(Rc::new(node))
}

/// Convert the *negation* of `expr` to a tree. Equality-family operators
/// flip to their exact complements (`!(a === b)` ⇔ `a !== b`); `&&`/`||`
/// distribute via De Morgan (exact under JS truthiness); everything else
/// keeps a literal `!` wrapper. Ordering comparisons (`<`, `>=`, …) are
/// deliberately NOT flipped — `!(a < b)` and `a >= b` disagree on NaN.
fn negated_expression_to_tree_node(
    expr: &Expression,
    id_counter: &mut usize,
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
            let mut node = make_node(label, "BinaryExpression", id_counter);
            let (first, second) = equality_operand_order(&bin.left, &bin.right);
            if let Some(child) = expression_to_tree_node(first, id_counter) {
                node.add_child(child);
            }
            if let Some(child) = expression_to_tree_node(second, id_counter) {
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
            let mut node = make_node(label, "LogicalExpression", id_counter);
            if let Some(child) = negated_expression_to_tree_node(&log.left, id_counter) {
                node.add_child(child);
            }
            if let Some(child) = negated_expression_to_tree_node(&log.right, id_counter) {
                node.add_child(child);
            }
            return Some(Rc::new(node));
        }
    }
    let mut node = make_node("LogicalNot", "UnaryExpression", id_counter);
    if let Some(child) = expression_to_tree_node(expr, id_counter) {
        node.add_child(child);
    }
    Some(Rc::new(node))
}

/// Yoda normalization for the symmetric equality operators: when exactly
/// one operand is a literal, the literal goes second, so `5 === x` and
/// `x === 5` produce the same tree. Literal-literal and expr-expr pairs
/// keep their source order.
fn equality_operand_order<'a, 'b>(
    left: &'b Expression<'a>,
    right: &'b Expression<'a>,
) -> (&'b Expression<'a>, &'b Expression<'a>) {
    if is_literal_operand(left) && !is_literal_operand(right) {
        (right, left)
    } else {
        (left, right)
    }
}

/// Boolean-context simplification: strip `!!` pairs and `Boolean(...)`
/// wrappers from an expression consumed only for its truthiness (an
/// `if`/`while`/`for`/ternary test). Value positions keep them — there
/// `!!x` and `x` are different values.
fn test_expression_to_tree_node(
    expr: &Expression,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    if !canonicalize_enabled() {
        return expression_to_tree_node(expr, id_counter);
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
        if let Some(inner) = match_boolean_call(current) {
            current = strip_parentheses(inner);
            continue;
        }
        break;
    }
    expression_to_tree_node(current, id_counter)
}

/// Match a call to the `Boolean` global with a single plain argument.
fn match_boolean_call<'a, 'b>(expr: &'b Expression<'a>) -> Option<&'b Expression<'a>> {
    let Expression::CallExpression(call) = strip_parentheses(expr) else {
        return None;
    };
    if call.optional || call.arguments.len() != 1 {
        return None;
    }
    let Expression::Identifier(ident) = strip_parentheses(&call.callee) else {
        return None;
    };
    if ident.name.as_str() != "Boolean" || is_locally_bound(ident.span) {
        return None;
    }
    call.arguments[0].as_expression()
}

/// Whether control flow can never continue past `stmt` (it returns,
/// throws, breaks, continues, or is a block/if that always does). Used to
/// decide when `if (c) { …jump } else { B }` can flatten to
/// `if (c) { …jump } B`.
fn statement_always_terminates(stmt: &Statement) -> bool {
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

/// Append every statement of `stmt` (flattening a block) to `nodes` via
/// the canonical statement conversion.
fn flatten_statement_into(nodes: &mut Vec<Rc<TreeNode>>, stmt: &Statement, id_counter: &mut usize) {
    if let Statement::BlockStatement(block) = stmt {
        for inner in &block.body {
            nodes.extend(statement_to_tree_nodes(inner, id_counter));
        }
    } else {
        nodes.extend(statement_to_tree_nodes(stmt, id_counter));
    }
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
fn canonical_if_nodes(
    if_stmt: &oxc_ast::ast::IfStatement,
    id_counter: &mut usize,
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
            Expression::UnaryExpression(unary)
                if unary.operator == UnaryOperator::LogicalNot =>
            {
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
    let render_positive = |ids: &mut usize| -> Option<Rc<TreeNode>> {
        match negation {
            NegationStyle::FlippedBinary => negated_expression_to_tree_node(test, ids),
            NegationStyle::Plain | NegationStyle::Unwrapped => {
                test_expression_to_tree_node(test, ids)
            }
        }
    };
    let render_negated = |ids: &mut usize| -> Option<Rc<TreeNode>> {
        match negation {
            NegationStyle::FlippedBinary => test_expression_to_tree_node(test, ids),
            NegationStyle::Plain | NegationStyle::Unwrapped => {
                negated_expression_to_tree_node(test, ids)
            }
        }
    };

    if let Some(alt) = alternate {
        if statement_always_terminates(consequent) {
            let mut if_node = make_node("IfStatement", "IfStatement", id_counter);
            if let Some(test_node) = render_positive(id_counter) {
                if_node.add_child(test_node);
            }
            if let Some(cons_node) = statement_to_block_node(consequent, id_counter) {
                if_node.add_child(cons_node);
            }
            let mut nodes = vec![Rc::new(if_node)];
            flatten_statement_into(&mut nodes, alt, id_counter);
            return nodes;
        }
        if statement_always_terminates(alt) {
            let mut if_node = make_node("IfStatement", "IfStatement", id_counter);
            if let Some(test_node) = render_negated(id_counter) {
                if_node.add_child(test_node);
            }
            if let Some(alt_node) = statement_to_block_node(alt, id_counter) {
                if_node.add_child(alt_node);
            }
            let mut nodes = vec![Rc::new(if_node)];
            flatten_statement_into(&mut nodes, consequent, id_counter);
            return nodes;
        }
    }

    // No flattening possible: plain if/else, with `else { if … }` blocks
    // unwrapped so they align with bare `else if` chains.
    let mut node = make_node("IfStatement", "IfStatement", id_counter);
    if let Some(test_node) = render_positive(id_counter) {
        node.add_child(test_node);
    }
    if let Some(cons_node) = statement_to_block_node(consequent, id_counter) {
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
            let mut alt_nodes = canonical_if_nodes(inner_if, id_counter);
            if alt_nodes.len() == 1 {
                if let Some(single) = alt_nodes.pop() {
                    node.add_child(single);
                }
            } else {
                // The nested chain flattened into several statements —
                // hold them in a block, mirroring how the explicit
                // `else { … }` spelling converts.
                let mut block = make_node("BlockStatement", "BlockStatement", id_counter);
                for alt_node in alt_nodes {
                    block.add_child(alt_node);
                }
                node.add_child(Rc::new(block));
            }
        } else if let Some(alt_node) = statement_to_block_node(effective_alt, id_counter) {
            node.add_child(alt_node);
        }
    }
    vec![Rc::new(node)]
}

/// Lower `const { a, b: renamed } = source;` (side-effect-free source,
/// plain identifier bindings, no defaults/rest/nesting) into the
/// per-property member-access declarations it is equivalent to:
/// `const a = source.a; const renamed = source.b;`.
fn lower_destructuring_declaration(
    var_decl: &VariableDeclaration,
    id_counter: &mut usize,
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
        lowered.push((key, identifier_label(binding.span, &binding.name)));
    }

    let kind_label = format!("{:?}Declaration", var_decl.kind);
    let mut nodes = Vec::with_capacity(lowered.len());
    for (key, binding_label) in lowered {
        let mut decl = make_node(&kind_label, "VariableDeclaration", id_counter);
        let mut declarator_node =
            make_node("VariableDeclarator", "VariableDeclarator", id_counter);
        declarator_node.add_child(leaf(&binding_label, "BindingIdentifier", id_counter));
        let access: Rc<TreeNode> = match key {
            KeyAccess::Static(name) => {
                let mut member = make_node(".", "StaticMemberExpression", id_counter);
                if let Some(object) = expression_to_tree_node(init, id_counter) {
                    member.add_child(object);
                }
                member.add_child(leaf(&name, "Identifier", id_counter));
                Rc::new(member)
            }
            KeyAccess::Quoted(value) => {
                let mut member = make_node("[]", "ComputedMemberExpression", id_counter);
                if let Some(object) = expression_to_tree_node(init, id_counter) {
                    member.add_child(object);
                }
                member.add_child(leaf(&format!("\"{value}\""), "StringLiteral", id_counter));
                Rc::new(member)
            }
        };
        declarator_node.add_child(access);
        decl.add_child(Rc::new(declarator_node));
        nodes.push(Rc::new(decl));
    }
    Some(nodes)
}

// ---------------------------------------------------------------------------
// Statement-list peepholes (TreeNode level)
// ---------------------------------------------------------------------------
//
// These run over already-converted statement lists, where earlier
// lowerings (forEach → for-of, `i++` → `i += 1`, ternaries → if/else)
// have normalized their inputs — so one pattern here covers every source
// spelling of the idiom.

fn trees_equal(a: &TreeNode, b: &TreeNode) -> bool {
    a.label == b.label
        && a.value == b.value
        && a.children.len() == b.children.len()
        && a.children.iter().zip(b.children.iter()).all(|(x, y)| trees_equal(x, y))
}

fn tree_mentions_identifier(node: &TreeNode, label: &str) -> bool {
    if (node.value == "Identifier" || node.value == "BindingIdentifier") && node.label == label {
        return true;
    }
    node.children.iter().any(|child| tree_mentions_identifier(child, label))
}

/// The single binding introduced by a `<Kind>Declaration [VariableDeclarator
/// [BindingIdentifier …]]` node, together with its initializer (if any).
fn single_declarator(node: &TreeNode) -> Option<(&Rc<TreeNode>, Option<&Rc<TreeNode>>)> {
    if node.value != "VariableDeclaration" || node.children.len() != 1 {
        return None;
    }
    let declarator = &node.children[0];
    if declarator.value != "VariableDeclarator" || declarator.children.is_empty() {
        return None;
    }
    let binding = &declarator.children[0];
    if binding.value != "BindingIdentifier" {
        return None;
    }
    Some((binding, declarator.children.get(1)))
}

/// Rewrite `for (let i = 0; i < xs.length; i += 1) { const x = xs[i]; … }`
/// (with `i` unused elsewhere) into `for (const x of xs) { … }`. All the
/// index-stepping spellings (`i++`, `++i`, `i += 1`, `i = i + 1`) already
/// canonicalized onto the compound form before this runs.
fn rewrite_index_loop(node: &Rc<TreeNode>, id_counter: &mut usize) -> Option<Rc<TreeNode>> {
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
    if test.children[0].label != index_label || test.children[0].value != "Identifier" {
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

    if update.value != "AssignmentExpression"
        || update.label != "Addition"
        || update.children.len() != 2
        || update.children[0].label != index_label
        || update.children[1].label != "1"
    {
        return None;
    }

    if body.value != "BlockStatement" || body.children.is_empty() {
        return None;
    }
    let first = &body.children[0];
    let (element_binding, element_init) = single_declarator(first)?;
    let element_init = element_init?;
    if element_init.label != "[]"
        || element_init.children.len() != 2
        || !trees_equal(&element_init.children[0], iterated)
        || element_init.children[1].label != index_label
        || element_init.children[1].value != "Identifier"
    {
        return None;
    }
    if body.children[1..].iter().any(|stmt| tree_mentions_identifier(stmt, &index_label)) {
        return None;
    }

    let mut for_node = make_node("ForOfStatement", "ForOfStatement", id_counter);
    let mut decl = make_node("ConstDeclaration", "VariableDeclaration", id_counter);
    let mut declarator = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
    declarator.add_child(Rc::clone(element_binding));
    decl.add_child(Rc::new(declarator));
    for_node.add_child(Rc::new(decl));
    for_node.add_child(Rc::clone(iterated));
    let mut block = make_node("BlockStatement", "BlockStatement", id_counter);
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
fn rewrite_push_loop(
    declaration: &Rc<TreeNode>,
    loop_node: &Rc<TreeNode>,
    id_counter: &mut usize,
) -> Option<Rc<TreeNode>> {
    let (accumulator_binding, accumulator_init) = single_declarator(declaration)?;
    let accumulator_init = accumulator_init?;
    if accumulator_init.value != "ArrayExpression" || !accumulator_init.children.is_empty() {
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
    if body.value != "BlockStatement" || body.children.len() != 1 {
        return None;
    }

    let push_argument_of = |stmt: &Rc<TreeNode>| -> Option<Rc<TreeNode>> {
        if stmt.value != "ExpressionStatement" || stmt.children.len() != 1 {
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
            || callee.children[0].value != "Identifier"
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
    } else if statement.value == "IfStatement" && statement.children.len() == 2 {
        // if (COND) { out.push(x) }  — pushing the element itself.
        let condition = &statement.children[0];
        let then_block = &statement.children[1];
        if then_block.value != "BlockStatement" || then_block.children.len() != 1 {
            return None;
        }
        let argument = push_argument_of(&then_block.children[0])?;
        if argument.value != "Identifier" || argument.label != element_binding.label {
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
    let mut arrow = make_node("ArrowFunction", "ArrowFunctionExpression", id_counter);
    let mut parameter = make_node("Parameter", "Parameter", id_counter);
    parameter.add_child(Rc::clone(element_binding));
    arrow.add_child(Rc::new(parameter));
    let mut arrow_block = make_node("BlockStatement", "BlockStatement", id_counter);
    let mut return_node = make_node("ReturnStatement", "ReturnStatement", id_counter);
    return_node.add_child(callback_body_expr);
    arrow_block.add_child(Rc::new(return_node));
    arrow.add_child(Rc::new(arrow_block));

    let mut member = make_node(".", "StaticMemberExpression", id_counter);
    member.add_child(Rc::clone(iterated));
    member.add_child(leaf(method, "Identifier", id_counter));

    let mut call = make_node("CallExpression", "CallExpression", id_counter);
    call.add_child(Rc::new(member));
    call.add_child(Rc::new(arrow));

    let mut declarator = make_node("VariableDeclarator", "VariableDeclarator", id_counter);
    declarator.add_child(Rc::clone(accumulator_binding));
    declarator.add_child(Rc::new(call));
    let mut decl = make_node(&declaration.label, "VariableDeclaration", id_counter);
    decl.add_child(Rc::new(declarator));
    Some(Rc::new(decl))
}

/// TreeNode-level "control cannot continue" check, mirroring
/// [`statement_always_terminates`] for already-converted nodes.
fn node_always_terminates(node: &TreeNode) -> bool {
    match node.value.as_str() {
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
fn rewrite_negative_guard(nodes: &mut Vec<Rc<TreeNode>>, id_counter: &mut usize) {
    for index in 0..nodes.len() {
        let is_last = index + 1 == nodes.len();
        if is_last {
            break;
        }
        let guard = &nodes[index];
        if guard.value != "IfStatement" || guard.children.len() != 2 {
            continue;
        }
        let test = &guard.children[0];
        let flipped_label = match test.label.as_str() {
            "StrictInequality" if test.value == "BinaryExpression" => Some("StrictEquality"),
            "Inequality" if test.value == "BinaryExpression" => Some("Equality"),
            "LogicalNot" if test.value == "UnaryExpression" => None,
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
        rewrite_negative_guard(&mut tail, id_counter);

        let positive_test: Rc<TreeNode> = if let Some(label) = flipped_label {
            let mut flipped = make_node(label, "BinaryExpression", id_counter);
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

        let mut tail_block = make_node("BlockStatement", "BlockStatement", id_counter);
        for stmt in tail {
            tail_block.add_child(stmt);
        }
        let mut flipped_guard = make_node("IfStatement", "IfStatement", id_counter);
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
fn node_is_pure_chain(node: &TreeNode) -> bool {
    match node.value.as_str() {
        "Identifier" | "BindingIdentifier" | "ThisExpression" | "StringLiteral"
        | "NumericLiteral" | "BooleanLiteral" | "NullLiteral" | "BigIntLiteral" => true,
        "StaticMemberExpression" if node.label == "." => {
            node.children.first().is_some_and(|object| node_is_pure_chain(object))
        }
        _ => false,
    }
}

fn count_identifier_uses(node: &TreeNode, label: &str) -> usize {
    let own = usize::from(node.value == "Identifier" && node.label == label);
    own + node.children.iter().map(|child| count_identifier_uses(child, label)).sum::<usize>()
}

/// Clone `node` with the single `Identifier` leaf labeled `label`
/// replaced by `replacement`. `replaced` flips when the substitution
/// happens so only the first occurrence is rewritten.
fn substitute_identifier(
    node: &Rc<TreeNode>,
    label: &str,
    replacement: &Rc<TreeNode>,
    replaced: &mut bool,
    id_counter: &mut usize,
) -> Rc<TreeNode> {
    if !*replaced && node.value == "Identifier" && node.label == label {
        *replaced = true;
        return Rc::clone(replacement);
    }
    if node.children.is_empty() {
        return Rc::clone(node);
    }
    let mut fresh = make_node(&node.label, &node.value, id_counter);
    fresh.source_span = node.source_span;
    for child in &node.children {
        fresh.add_child(substitute_identifier(child, label, replacement, replaced, id_counter));
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
fn inline_single_use_constants(nodes: &mut Vec<Rc<TreeNode>>, id_counter: &mut usize) {
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
        let Some(use_offset) = nodes[index + 1..]
            .iter()
            .position(|stmt| count_identifier_uses(stmt, &label) > 0)
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
        *slot = substitute_identifier(slot, &label, &init, &mut replaced, id_counter);
        nodes.remove(index);
        // Re-check the same index — the next statement may now qualify.
    }
}

/// Whether an initializer is a pure computation (no calls, construction,
/// awaits, or mutation anywhere) — such initializers commute with each
/// other, so declaration order between them is style, not behavior.
fn node_is_reorderable_init(node: &TreeNode) -> bool {
    match node.value.as_str() {
        "CallExpression" | "NewExpression" | "AwaitExpression" | "YieldExpression"
        | "AssignmentExpression" | "UpdateExpression" | "TaggedTemplateExpression"
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
fn sort_reorderable_declarations(nodes: &mut [Rc<TreeNode>]) {
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
            node.value.hash(hasher);
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
fn rewrite_temp_return(nodes: &mut Vec<Rc<TreeNode>>, id_counter: &mut usize) {
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
        || return_node.children[0].value != "Identifier"
        || return_node.children[0].label != binding.label
    {
        return;
    }
    let mut replacement = make_node("ReturnStatement", "ReturnStatement", id_counter);
    replacement.add_child(Rc::clone(init));
    nodes.truncate(count - 2);
    nodes.push(Rc::new(replacement));
}

/// Run the statement-list peepholes over a freshly built block body.
fn normalize_statement_nodes(nodes: &mut Vec<Rc<TreeNode>>, id_counter: &mut usize) {
    if !canonicalize_enabled() {
        return;
    }
    for slot in nodes.iter_mut() {
        if let Some(rewritten) = rewrite_index_loop(slot, id_counter) {
            *slot = rewritten;
        }
    }
    inline_single_use_constants(nodes, id_counter);
    let mut index = 0;
    while index + 1 < nodes.len() {
        if let Some(replacement) =
            rewrite_push_loop(&nodes[index], &nodes[index + 1], id_counter)
        {
            nodes[index] = replacement;
            nodes.remove(index + 1);
        } else {
            index += 1;
        }
    }
    sort_reorderable_declarations(nodes);
    rewrite_temp_return(nodes, id_counter);
    rewrite_negative_guard(nodes, id_counter);
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
                    if let Some(test_node) = test_expression_to_tree_node(test, id_counter) {
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
                if let Some(test_node) = test_expression_to_tree_node(test, id_counter) {
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
            if let Some(test_node) = test_expression_to_tree_node(&while_stmt.test, id_counter) {
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
            if let Some(test_node) = test_expression_to_tree_node(&do_stmt.test, id_counter) {
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
            Some(leaf(&identifier_label(ident.span, &ident.name), "Identifier", id_counter))
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
    let mut label = func
        .id
        .as_ref()
        .map_or_else(|| default_label.to_string(), |id| identifier_label(id.span, &id.name));
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
    let label = class
        .id
        .as_ref()
        .map_or_else(|| default_label.to_string(), |id| identifier_label(id.span, &id.name));
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
            Some(leaf(&identifier_label(ident.span, &ident.name), "Identifier", id_counter))
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
                    operands.push(leaf("\"\"", "StringLiteral", id_counter));
                }
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

            // Symmetric equality operators put a lone literal operand
            // second, so Yoda-style `5 === x` matches `x === 5`.
            let (first, second) = if canonicalize_enabled()
                && matches!(
                    bin_expr.operator,
                    BinaryOperator::StrictEquality
                        | BinaryOperator::StrictInequality
                        | BinaryOperator::Equality
                        | BinaryOperator::Inequality
                ) {
                equality_operand_order(&bin_expr.left, &bin_expr.right)
            } else {
                (&bin_expr.left, &bin_expr.right)
            };

            if let Some(left_node) = expression_to_tree_node(first, id_counter) {
                node.add_child(left_node);
            }

            if let Some(right_node) = expression_to_tree_node(second, id_counter) {
                node.add_child(right_node);
            }

            Some(Rc::new(node))
        }
        Expression::LogicalExpression(log_expr) => {
            // The strict null/undefined pair contracts onto the loose
            // comparison it is equivalent to: `x === null || x ===
            // undefined` ⇔ `x == null` (and the `!==`/`&&`/`!=` duals).
            if canonicalize_enabled() {
                match classify_nullish_test(expr) {
                    Some(NullishTest::IsNullish(subject)) => {
                        return nullish_comparison_node(subject, true, id_counter);
                    }
                    Some(NullishTest::NotNullish(subject)) => {
                        return nullish_comparison_node(subject, false, id_counter);
                    }
                    None => {}
                }
            }
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
            if canonicalize_enabled() {
                // `!<expr>` flips equality operators to their exact
                // complements and distributes over `&&`/`||` (De Morgan),
                // so `!(a === b)` ⇔ `a !== b` and `!(a && b)` ⇔
                // `!a || !b` produce one shape.
                if unary.operator == UnaryOperator::LogicalNot {
                    return negated_expression_to_tree_node(&unary.argument, id_counter);
                }
                // `void 0` (or `void x` for pure x) IS the undefined
                // value; collapse onto the `undefined` identifier so both
                // spellings compare as equal.
                if unary.operator == UnaryOperator::Void
                    && is_side_effect_free(&unary.argument)
                {
                    return Some(leaf("undefined", "Identifier", id_counter));
                }
            }
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
            if canonicalize_enabled() {
                let test = strip_parentheses(&cond.test);
                let consequent = strip_parentheses(&cond.consequent);
                let alternate = strip_parentheses(&cond.alternate);
                // Nullish-defaulting ternaries contract onto `??`:
                //   `x == null ? d : x` ⇔ `x != null ? x : d` ⇔ `x ?? d`.
                match classify_nullish_test(test) {
                    Some(NullishTest::IsNullish(subject))
                        if pure_expressions_equal(subject, alternate) =>
                    {
                        let mut node = make_node("Coalesce", "LogicalExpression", id_counter);
                        if let Some(child) = expression_to_tree_node(alternate, id_counter) {
                            node.add_child(child);
                        }
                        if let Some(child) = expression_to_tree_node(consequent, id_counter) {
                            node.add_child(child);
                        }
                        return Some(Rc::new(node));
                    }
                    Some(NullishTest::NotNullish(subject))
                        if pure_expressions_equal(subject, consequent) =>
                    {
                        let mut node = make_node("Coalesce", "LogicalExpression", id_counter);
                        if let Some(child) = expression_to_tree_node(consequent, id_counter) {
                            node.add_child(child);
                        }
                        if let Some(child) = expression_to_tree_node(alternate, id_counter) {
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
                        let mut node = make_node(label, "LogicalExpression", id_counter);
                        if let Some(child) = expression_to_tree_node(test, id_counter) {
                            node.add_child(child);
                        }
                        if let Some(child) = expression_to_tree_node(other, id_counter) {
                            node.add_child(child);
                        }
                        return Some(Rc::new(node));
                    }
                }
            }
            let mut node = make_node("ConditionalExpression", "ConditionalExpression", id_counter);
            if let Some(test_node) = test_expression_to_tree_node(&cond.test, id_counter) {
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
            // `arr[arr.length - 1]` ⇔ `arr.at(-1)` for a pure `arr` —
            // identical values including the out-of-bounds/`undefined`
            // case. Canonical form is the `.at(-1)` call.
            if canonicalize_enabled() && !mem.optional && is_side_effect_free(&mem.object) {
                if let Expression::BinaryExpression(bin) = strip_parentheses(&mem.expression) {
                    if bin.operator == BinaryOperator::Subtraction {
                        if let (
                            Expression::StaticMemberExpression(length_member),
                            Expression::NumericLiteral(one),
                        ) = (strip_parentheses(&bin.left), strip_parentheses(&bin.right))
                        {
                            if (one.value - 1.0).abs() < f64::EPSILON
                                && !length_member.optional
                                && length_member.property.name.as_str() == "length"
                                && pure_expressions_equal(&mem.object, &length_member.object)
                            {
                                let mut member =
                                    make_node(".", "StaticMemberExpression", id_counter);
                                if let Some(object) =
                                    expression_to_tree_node(&mem.object, id_counter)
                                {
                                    member.add_child(object);
                                }
                                member.add_child(leaf("at", "Identifier", id_counter));
                                let mut minus_one =
                                    make_node("UnaryNegation", "UnaryExpression", id_counter);
                                minus_one.add_child(leaf("1", "NumericLiteral", id_counter));
                                let mut call =
                                    make_node("CallExpression", "CallExpression", id_counter);
                                call.add_child(Rc::new(member));
                                call.add_child(Rc::new(minus_one));
                                return Some(Rc::new(call));
                            }
                        }
                    }
                }
            }
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
                // `Boolean(x)` ⇔ `!!x` — same value for every input.
                // Canonical form is the double negation.
                if let Some(argument) = match_boolean_call(expr) {
                    let mut inner = make_node("LogicalNot", "UnaryExpression", id_counter);
                    if let Some(child) = expression_to_tree_node(argument, id_counter) {
                        inner.add_child(child);
                    }
                    let mut outer = make_node("LogicalNot", "UnaryExpression", id_counter);
                    outer.add_child(Rc::new(inner));
                    return Some(Rc::new(outer));
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
                    if canonicalize_enabled() {
                        // `(x) => expr` is the same function as
                        // `(x) => { return expr; }` — wrap the expression
                        // body in the explicit block/return shape so both
                        // spellings (and nested callbacks in particular)
                        // produce identical trees.
                        let mut block =
                            make_node("BlockStatement", "BlockStatement", id_counter);
                        for child in
                            canonical_return_nodes(Some(&expr_stmt.expression), id_counter)
                        {
                            block.add_child(child);
                        }
                        node.add_child(Rc::new(block));
                    } else if let Some(expr_node) =
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
            &identifier_label(ident.span, &ident.name),
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
            &identifier_label(ident.span, &ident.name),
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
            &identifier_label(ident.span, &ident.name),
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

    let mut statements = Vec::with_capacity(body.statements.len());
    for stmt in &body.statements {
        statements.extend(statement_to_tree_nodes(stmt, id_counter));
    }
    normalize_statement_nodes(&mut statements, id_counter);
    for child in statements {
        node.add_child(child);
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

    let mut statements = Vec::with_capacity(block.body.len());
    for stmt in &block.body {
        statements.extend(statement_to_tree_nodes(stmt, id_counter));
    }
    normalize_statement_nodes(&mut statements, id_counter);
    for child in statements {
        node.add_child(child);
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
            node.value == kind || node.children.iter().any(|c| contains_kind(c, kind))
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
}
