//! Formal parameters and binding patterns.

use oxc_ast::ast::{BindingPattern, FormalParameters};
use std::rc::Rc;

use super::expressions::{expression_to_tree_node, member_label, property_key_to_tree_node};
use super::{leaf, make_node, stamp, ConversionContext};
use crate::tree::TreeNode;

/// The nodes of a parameter list: one `Parameter` per parameter, the rest
/// parameter included.
///
/// Each binding sits under a `Parameter` node so its role is preserved
/// even when the binding itself is a destructuring pattern. A default
/// value is part of the parameter (`limit = 10` is an `AssignmentPattern`,
/// like a default inside a destructuring pattern), and so is being the
/// rest parameter. Both used to be dropped, which made `f(limit = 10)`,
/// `f(limit = compute())` and `f(limit)` — or `f(...items)` and `f()` —
/// the same signature.
pub(super) fn formal_parameters_to_tree_nodes(
    params: &FormalParameters,
    cx: &ConversionContext,
) -> Vec<Rc<TreeNode>> {
    let mut nodes = Vec::with_capacity(params.items.len() + 1);
    for param in &params.items {
        let mut node = make_node("Parameter", "Parameter");
        let binding = binding_pattern_to_tree_node(&param.pattern, cx);
        match &param.initializer {
            Some(initializer) => {
                let mut default = make_node("AssignmentPattern", "AssignmentPattern");
                if let Some(binding) = binding {
                    default.add_child(binding);
                }
                if let Some(value) = expression_to_tree_node(initializer, cx) {
                    default.add_child(value);
                }
                node.add_child(Rc::new(default));
            }
            None => {
                if let Some(binding) = binding {
                    node.add_child(binding);
                }
            }
        }
        nodes.push(stamp(Rc::new(node), param.span));
    }
    if let Some(rest) = &params.rest {
        let mut rest_node = make_node("RestElement", "RestElement");
        if let Some(binding) = binding_pattern_to_tree_node(&rest.rest.argument, cx) {
            rest_node.add_child(binding);
        }
        let mut node = make_node("Parameter", "Parameter");
        node.add_child(Rc::new(rest_node));
        nodes.push(stamp(Rc::new(node), rest.span));
    }
    nodes
}

pub(super) fn binding_pattern_to_tree_node(
    pattern: &BindingPattern,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    match pattern {
        BindingPattern::BindingIdentifier(ident) => {
            Some(leaf(&cx.identifier_label(ident.span, &ident.name), "BindingIdentifier"))
        }
        BindingPattern::ObjectPattern(obj) => {
            let mut node = make_node("ObjectPattern", "ObjectPattern");
            for prop in &obj.properties {
                let label = member_label("BindingProperty", &prop.key, prop.computed);
                let mut prop_node = make_node(label, "BindingProperty");
                if let Some(key_node) = property_key_to_tree_node(&prop.key, cx) {
                    prop_node.add_child(key_node);
                }
                if let Some(val_node) = binding_pattern_to_tree_node(&prop.value, cx) {
                    prop_node.add_child(val_node);
                }
                node.add_child(Rc::new(prop_node));
            }
            if let Some(rest) = &obj.rest {
                let mut rest_node = make_node("RestElement", "RestElement");
                if let Some(arg_node) = binding_pattern_to_tree_node(&rest.argument, cx) {
                    rest_node.add_child(arg_node);
                }
                node.add_child(Rc::new(rest_node));
            }
            Some(Rc::new(node))
        }
        BindingPattern::ArrayPattern(arr) => {
            let mut node = make_node("ArrayPattern", "ArrayPattern");
            for elem in &arr.elements {
                if let Some(elem_pat) = elem {
                    if let Some(child) = binding_pattern_to_tree_node(elem_pat, cx) {
                        node.add_child(child);
                    }
                } else {
                    node.add_child(leaf("Elision", "Elision"));
                }
            }
            if let Some(rest) = &arr.rest {
                let mut rest_node = make_node("RestElement", "RestElement");
                if let Some(arg_node) = binding_pattern_to_tree_node(&rest.argument, cx) {
                    rest_node.add_child(arg_node);
                }
                node.add_child(Rc::new(rest_node));
            }
            Some(Rc::new(node))
        }
        BindingPattern::AssignmentPattern(assign) => {
            let mut node = make_node("AssignmentPattern", "AssignmentPattern");
            if let Some(left_node) = binding_pattern_to_tree_node(&assign.left, cx) {
                node.add_child(left_node);
            }
            if let Some(right_node) = expression_to_tree_node(&assign.right, cx) {
                node.add_child(right_node);
            }
            Some(Rc::new(node))
        }
    }
}
