//! JSX elements and fragments.
//!
//! An element converts to a node whose children are its name, its
//! attributes and its content, in that order:
//!
//! ```text
//! <Card title="Hi" {...rest}>{user.name}<b>!</b></Card>
//!
//! JSXElement
//!   Identifier Card                  (a component: a reference like any other)
//!   JSXAttribute  [Identifier title, StringLiteral "Hi"]
//!   JSXSpreadAttribute [Identifier rest]
//!   . [Identifier user, Identifier name]
//!   JSXElement [JSXIdentifier b, StringLiteral "!"]
//! ```
//!
//! Component names and attribute names are `Identifier` leaves, so they
//! take part in comparison the way callees and object keys do: rendering
//! `<UserCard>` instead of `<ProductCard>`, or passing `onClick` instead
//! of `onChange`, is a difference in what the code does. Intrinsic tags
//! (`div`, `span`) are markup and get a kind of their own. Text content and
//! string attribute values are string literals: `<p>Hi</p>` and
//! `<p>{"Hi"}</p>`, or `id="x"` and `id={"x"}`, are the same element.
//!
//! JSX used to convert to a single placeholder leaf, so two components
//! with the same hooks and entirely different markup were the same tree.

use oxc_ast::ast::{
    JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXChild, JSXElement, JSXElementName,
    JSXExpression, JSXFragment, JSXMemberExpression, JSXMemberExpressionObject,
};
use std::rc::Rc;

use super::expressions::expression_to_tree_node;
use super::{leaf, make_node, stamp, ConversionContext};
use crate::tree::TreeNode;

pub(super) fn jsx_element_to_tree_node(
    element: &JSXElement,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    let mut node = make_node("JSXElement", "JSXElement");
    node.add_child(element_name(&element.opening_element.name, cx));
    for attribute in &element.opening_element.attributes {
        node.add_child(attribute_node(attribute, cx));
    }
    append_children(&mut node, &element.children, cx);
    stamp(Rc::new(node), element.span)
}

pub(super) fn jsx_fragment_to_tree_node(
    fragment: &JSXFragment,
    cx: &ConversionContext,
) -> Rc<TreeNode> {
    let mut node = make_node("JSXFragment", "JSXFragment");
    append_children(&mut node, &fragment.children, cx);
    stamp(Rc::new(node), fragment.span)
}

fn element_name(name: &JSXElementName, cx: &ConversionContext) -> Rc<TreeNode> {
    match name {
        JSXElementName::Identifier(ident) => leaf(ident.name.as_str(), "JSXIdentifier"),
        JSXElementName::IdentifierReference(ident) => {
            leaf(&cx.identifier_label(ident.span, &ident.name), "Identifier")
        }
        JSXElementName::NamespacedName(name) => leaf(
            &format!("{}:{}", name.namespace.name.as_str(), name.name.name.as_str()),
            "JSXIdentifier",
        ),
        JSXElementName::MemberExpression(member) => member_name(member, cx),
        JSXElementName::ThisExpression(_) => leaf("ThisExpression", "ThisExpression"),
    }
}

/// `<Menu.Item>`: the member access it is.
fn member_name(member: &JSXMemberExpression, cx: &ConversionContext) -> Rc<TreeNode> {
    let mut node = make_node(".", "StaticMemberExpression");
    node.add_child(match &member.object {
        JSXMemberExpressionObject::IdentifierReference(ident) => {
            leaf(&cx.identifier_label(ident.span, &ident.name), "Identifier")
        }
        JSXMemberExpressionObject::MemberExpression(inner) => member_name(inner, cx),
        JSXMemberExpressionObject::ThisExpression(_) => leaf("ThisExpression", "ThisExpression"),
    });
    node.add_child(leaf(member.property.name.as_str(), "Identifier"));
    Rc::new(node)
}

fn attribute_node(attribute: &JSXAttributeItem, cx: &ConversionContext) -> Rc<TreeNode> {
    match attribute {
        JSXAttributeItem::Attribute(attribute) => {
            let mut node = make_node("JSXAttribute", "JSXAttribute");
            let name = match &attribute.name {
                JSXAttributeName::Identifier(ident) => ident.name.to_string(),
                JSXAttributeName::NamespacedName(name) => {
                    format!("{}:{}", name.namespace.name.as_str(), name.name.name.as_str())
                }
            };
            node.add_child(leaf(&name, "Identifier"));
            // A bare attribute (`<input disabled />`) has no value node.
            let value = attribute.value.as_ref().and_then(|value| match value {
                JSXAttributeValue::StringLiteral(literal) => {
                    Some(leaf(&format!("\"{}\"", literal.value.as_str()), "StringLiteral"))
                }
                JSXAttributeValue::ExpressionContainer(container) => {
                    container_expression(&container.expression, cx)
                }
                JSXAttributeValue::Element(element) => Some(jsx_element_to_tree_node(element, cx)),
                JSXAttributeValue::Fragment(fragment) => {
                    Some(jsx_fragment_to_tree_node(fragment, cx))
                }
            });
            if let Some(value) = value {
                node.add_child(value);
            }
            stamp(Rc::new(node), attribute.span)
        }
        JSXAttributeItem::SpreadAttribute(spread) => {
            let mut node = make_node("JSXSpreadAttribute", "JSXSpreadAttribute");
            if let Some(argument) = expression_to_tree_node(&spread.argument, cx) {
                node.add_child(argument);
            }
            Rc::new(node)
        }
    }
}

/// The expression inside `{…}`; `None` for an empty container (`{}` or a
/// comment).
fn container_expression(
    expression: &JSXExpression,
    cx: &ConversionContext,
) -> Option<Rc<TreeNode>> {
    expression.as_expression().and_then(|expression| expression_to_tree_node(expression, cx))
}

fn append_children(
    parent: &mut TreeNode,
    children: &oxc_allocator::Vec<JSXChild>,
    cx: &ConversionContext,
) {
    for child in children {
        let node = match child {
            JSXChild::Text(text) => jsx_text_value(text.value.as_str())
                .map(|value| leaf(&format!("\"{value}\""), "StringLiteral")),
            JSXChild::Element(element) => Some(jsx_element_to_tree_node(element, cx)),
            JSXChild::Fragment(fragment) => Some(jsx_fragment_to_tree_node(fragment, cx)),
            JSXChild::ExpressionContainer(container) => {
                container_expression(&container.expression, cx)
            }
            JSXChild::Spread(spread) => {
                let mut node = make_node("SpreadElement", "SpreadElement");
                if let Some(argument) = expression_to_tree_node(&spread.expression, cx) {
                    node.add_child(argument);
                }
                Some(Rc::new(node))
            }
        };
        if let Some(node) = node {
            parent.add_child(node);
        }
    }
}

/// The string JSX text stands for, by the rule JSX compilers apply: every
/// line is trimmed (the first only at its end, the last only at its
/// start), blank lines are dropped, and what remains is joined by single
/// spaces. `None` when nothing remains — the whitespace between two tags
/// on separate lines is formatting, not content.
fn jsx_text_value(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    let pieces: Vec<&str> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let line = if index == 0 { line } else { line.trim_start_matches([' ', '\t']) };
            if index == last {
                line
            } else {
                line.trim_end_matches([' ', '\t'])
            }
        })
        .filter(|line| !line.is_empty())
        .collect();
    (!pieces.is_empty()).then(|| pieces.join(" "))
}

#[cfg(test)]
mod tests {
    use super::jsx_text_value;

    #[test]
    fn jsx_text_follows_the_compiler_whitespace_rule() {
        assert_eq!(jsx_text_value("Hello"), Some("Hello".to_string()));
        // Whitespace on a single line is content.
        assert_eq!(jsx_text_value(" Hello, "), Some(" Hello, ".to_string()));
        assert_eq!(jsx_text_value(" "), Some(" ".to_string()));
        // Line breaks and the indentation around them are not.
        assert_eq!(jsx_text_value("\n      "), None);
        assert_eq!(jsx_text_value("\n    Hello\n    world\n  "), Some("Hello world".to_string()));
        assert_eq!(jsx_text_value("Hello \r\n   world"), Some("Hello world".to_string()));
        assert_eq!(jsx_text_value("a \n\n b"), Some("a b".to_string()));
    }
}
