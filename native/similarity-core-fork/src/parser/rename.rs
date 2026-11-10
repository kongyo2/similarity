//! Alpha-renaming: canonical names for the symbols a fragment declares.

use oxc_ast::ast::Program;
use oxc_semantic::SemanticBuilder;
use oxc_span::GetSpan;
use std::collections::HashMap;
use std::rc::Rc;

use super::RenameMap;
use crate::tree::TreeNode;

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
///
/// `aliases` are `(alias, binding)` pairs: a named function expression
/// bound to a variable (`const factorial = function recur(n) {…}`) is
/// reachable under both names, but the comparison fragment can only
/// declare one of them. A free reference spelled like the other name is
/// the function itself, so it shares the binding's canonical name.
pub(super) fn build_rename_map(program: &Program, aliases: &[(String, String)]) -> RenameMap {
    let semantic = SemanticBuilder::new().build(program).semantic;
    let scoping = semantic.scoping();
    let nodes = semantic.nodes();

    let mut symbols: Vec<_> = scoping.symbol_ids().collect();
    symbols.sort_by_key(|&symbol_id| {
        let span = scoping.symbol_span(symbol_id);
        (span.start, span.end)
    });

    let mut map = RenameMap::new();
    for (ordinal, symbol_id) in symbols.into_iter().enumerate() {
        let canonical: Rc<str> = Rc::from(format!("§{ordinal}"));
        let declaration = scoping.symbol_span(symbol_id);
        map.insert((declaration.start, declaration.end), Rc::clone(&canonical));
        // `var total = 0; … var total = 1;` declares one symbol twice; the
        // second binding is the same variable and takes the same name.
        for redeclaration in scoping.symbol_redeclarations(symbol_id) {
            map.insert((redeclaration.span.start, redeclaration.span.end), Rc::clone(&canonical));
        }
        for &reference_id in scoping.get_resolved_reference_ids(symbol_id) {
            let reference = scoping.get_reference(reference_id);
            let span = nodes.get_node(reference.node_id()).kind().span();
            map.insert((span.start, span.end), Rc::clone(&canonical));
        }
    }

    for (alias, binding) in aliases {
        let Some(symbol_id) = scoping.get_root_binding(oxc_span::Ident::from(binding.as_str()))
        else {
            continue;
        };
        let declaration = scoping.symbol_span(symbol_id);
        let Some(canonical) = map.get(&(declaration.start, declaration.end)).cloned() else {
            continue;
        };
        for (name, reference_ids) in scoping.root_unresolved_references() {
            if name.as_str() != alias.as_str() {
                continue;
            }
            for &reference_id in reference_ids {
                let reference = scoping.get_reference(reference_id);
                let span = nodes.get_node(reference.node_id()).kind().span();
                map.insert((span.start, span.end), Rc::clone(&canonical));
            }
        }
    }
    map
}

/// Rebuild the tree, renumbering every `§N` token (in identifier-carrying
/// labels) by first depth-first occurrence.
pub(super) fn relabel_canonical_ordinals(root: &Rc<TreeNode>) -> Rc<TreeNode> {
    fn remap_label(label: &str, mapping: &mut HashMap<String, String>) -> String {
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
            if token.len() == '§'.len_utf8() {
                result.push('§');
                continue;
            }
            let next = mapping.len();
            let renamed = mapping.entry(token).or_insert_with(|| format!("§{next}"));
            result.push_str(renamed);
        }
        result
    }

    fn rebuild(node: &TreeNode, mapping: &mut HashMap<String, String>) -> TreeNode {
        let relabel = matches!(
            node.kind,
            "Identifier" | "BindingIdentifier" | "FunctionDeclaration" | "ClassDeclaration"
        );
        let label = if relabel { remap_label(&node.label, mapping) } else { node.label.clone() };
        let mut fresh = TreeNode::new(label, node.kind);
        fresh.source_span = node.source_span;
        for child in &node.children {
            fresh.add_child(Rc::new(rebuild(child, mapping)));
        }
        fresh
    }

    Rc::new(rebuild(root, &mut HashMap::new()))
}
