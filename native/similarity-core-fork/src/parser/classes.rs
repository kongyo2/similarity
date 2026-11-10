//! Class bodies: methods, fields, static blocks, constructors.

use oxc_ast::ast::{BindingPattern, ClassElement, Function, FunctionBody, PropertyKey};
use oxc_span::GetSpan;
use std::rc::Rc;

use super::expressions::expression_to_tree_node;
use super::patterns::formal_parameters_to_tree_nodes;
use super::statements::{
    append_statement_to_list, block_of, directive_nodes, function_body_to_tree_node,
    statement_to_tree_nodes,
};
use super::{leaf, make_node, stamp, ConversionContext};
use crate::tree::TreeNode;

/// Stringify a class member's `PropertyKey` into a label that's unique per
/// distinct key. Identifier and private-identifier keys already produce a
/// stable string form, but the original handler collapsed every other key
/// (string literals like `"alpha"`, numeric literals, computed
/// expressions) onto a single `"Method"` / `"Property"` placeholder so two
/// runtime-distinct members ended up sharing a tree label. The richer
/// labels below let APTED see those as a single-rename difference instead
/// of a perfect match.
pub(super) fn property_key_label(key: &PropertyKey) -> String {
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

/// Canonical constructor body. TypeScript parameter properties
/// (`constructor(private readonly repo: Repo) {}`) are sugar for an
/// explicit field assignment, so the body is converted with the
/// `this.repo = repo;` statements the compiler emits — right after the
/// top-level `super(…)` call when there is one, at the top otherwise. The
/// shorthand and the explicit `constructor(repo: Repo) { this.repo =
/// repo; }` spelling then produce identical trees, in the function
/// comparator and in the class comparator's method fingerprints alike.
pub(super) fn constructor_body_to_tree_node(
    function: &Function,
    body: &FunctionBody,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    let mut statements = directive_nodes(body);
    for stmt in &body.statements {
        statements.extend(statement_to_tree_nodes(stmt, cx));
    }

    let mut synthesized = Vec::new();
    for param in &function.params.items {
        let is_property = param.accessibility.is_some() || param.readonly || param.r#override;
        if !is_property {
            continue;
        }
        let BindingPattern::BindingIdentifier(ident) = &param.pattern else {
            continue;
        };
        // `this.<name> = <name>;` — the same shape `statement_to_tree_node`
        // builds for the hand-written assignment.
        let mut member = make_node(".", "StaticMemberExpression");
        member.add_child(leaf("ThisExpression", "ThisExpression"));
        member.add_child(leaf(ident.name.as_str(), "Identifier"));
        let mut assign = make_node("Assign", "AssignmentExpression");
        assign.add_child(Rc::new(member));
        assign.add_child(leaf(&cx.identifier_label(ident.span, &ident.name), "Identifier"));
        let mut stmt = make_node("ExpressionStatement", "ExpressionStatement");
        stmt.add_child(Rc::new(assign));
        synthesized.push(Rc::new(stmt));
    }
    // The compiler emits the assignments right after the constructor's
    // top-level `super(…)` call, which (since TS 4.6) may be preceded by
    // statements that do not touch `this`; a constructor without one gets
    // them at the top.
    let insert_at =
        statements.iter().position(|stmt| node_is_super_call(stmt)).map_or(0, |index| index + 1);
    statements.splice(insert_at..insert_at, synthesized);
    Some(block_of(statements, cx))
}

/// `super(…);` in statement position (already converted).
pub(super) fn node_is_super_call(node: &TreeNode) -> bool {
    node.kind == "ExpressionStatement"
        && node.children.first().is_some_and(|expr| {
            expr.kind == "CallExpression"
                && expr.children.first().is_some_and(|callee| callee.kind == "Super")
        })
}

pub(super) fn class_element_to_tree_node(
    element: &ClassElement,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    convert_class_element(element, cx).map(|node| stamp(node, element.span()))
}

fn convert_class_element(element: &ClassElement, cx: &ConversionContext) -> Option<Rc<TreeNode>> {
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
            let mut node = TreeNode::new(label, "MethodDefinition");

            // Walk the function value structurally — parameters and body
            // are part of the method's identity.
            for parameter in formal_parameters_to_tree_nodes(&method.value.params, cx) {
                node.add_child(parameter);
            }
            if let Some(body) = &method.value.body {
                let body_node = if cx.canonical
                    && method.kind == oxc_ast::ast::MethodDefinitionKind::Constructor
                {
                    constructor_body_to_tree_node(&method.value, body, cx)
                } else {
                    function_body_to_tree_node(body, cx)
                };
                if let Some(body_node) = body_node {
                    node.add_child(body_node);
                }
            }

            Some(Rc::new(node))
        }
        ClassElement::PropertyDefinition(prop) => {
            let key_label = property_key_label(&prop.key);
            let static_marker = if prop.r#static { "static_" } else { "" };
            let label = format!("{}{}", static_marker, key_label);
            let mut node = TreeNode::new(label, "PropertyDefinition");

            if let Some(value) = &prop.value {
                if let Some(child) = expression_to_tree_node(value, cx) {
                    node.add_child(child);
                }
            }
            Some(Rc::new(node))
        }
        ClassElement::StaticBlock(block) => {
            let mut node = make_node("StaticBlock", "StaticBlock");
            for stmt in &block.body {
                append_statement_to_list(&mut node, stmt, cx);
            }
            Some(Rc::new(node))
        }
        ClassElement::AccessorProperty(_) => Some(leaf("AccessorProperty", "AccessorProperty")),
        _ => None,
    }
}
