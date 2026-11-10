use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// AST node used by the structural-distance comparators.
///
/// A node is identified by two strings: its `kind` (the syntactic category,
/// `Identifier`, `BinaryExpression`, …) and its `label` (what tells it apart
/// from other nodes of that kind: an identifier's name, an operator, a
/// literal's text). Kinds are a closed set of names chosen by the converter,
/// so they are `&'static str`; labels are data and owned.
///
/// `subtree_size` lazily caches the total node count of `self` plus its
/// descendants. It's measured in O(n) the first time it's requested and
/// O(1) thereafter — the comparators call it inside hot loops. `Cell` is
/// enough for the interior mutability: the node sits behind an `Rc` and
/// the cached value never escapes it.
#[derive(Debug)]
pub struct TreeNode {
    pub label: String,
    pub kind: &'static str,
    pub children: Vec<Rc<TreeNode>>,
    /// `(start_byte, end_byte)` of the source slice this node was converted
    /// from. `(0, 0)` marks a synthetic node without a source position —
    /// not the byte range `[0, 0)`.
    pub source_span: (u32, u32),
    subtree_size: Cell<Option<usize>>,
}

impl Clone for TreeNode {
    fn clone(&self) -> Self {
        TreeNode {
            label: self.label.clone(),
            kind: self.kind,
            children: self.children.clone(),
            source_span: self.source_span,
            subtree_size: Cell::new(self.subtree_size.get()),
        }
    }
}

impl TreeNode {
    #[must_use]
    pub fn new(label: impl Into<String>, kind: &'static str) -> Self {
        TreeNode {
            label: label.into(),
            kind,
            children: Vec::new(),
            source_span: (0, 0),
            subtree_size: Cell::new(None),
        }
    }

    pub fn add_child(&mut self, child: Rc<TreeNode>) {
        // Adding a child invalidates the cached subtree size.
        self.subtree_size.set(None);
        self.children.push(child);
    }

    pub fn set_source_span(&mut self, start: u32, end: u32) {
        self.source_span = (start, end);
    }

    /// Whether the node carries a real source position.
    #[must_use]
    pub fn has_source_span(&self) -> bool {
        self.source_span != (0, 0)
    }

    /// Number of nodes in this subtree, the node itself included.
    #[must_use]
    pub fn subtree_size(&self) -> usize {
        if let Some(size) = self.subtree_size.get() {
            return size;
        }
        let size = 1 + self.children.iter().map(|child| child.subtree_size()).sum::<usize>();
        self.subtree_size.set(Some(size));
        size
    }

    /// Hash of the subtree's shape: every node's label and kind, and how
    /// the nodes nest. Source positions take no part, so two subtrees
    /// converted from different places hash alike when they are the same
    /// code. Stable across runs and platforms (fixed-key SipHash).
    #[must_use]
    pub fn structural_hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash_structure(&mut hasher);
        hasher.finish()
    }

    /// Feed the subtree's shape into `hasher` (see [`Self::structural_hash`]).
    pub fn hash_structure(&self, hasher: &mut impl Hasher) {
        self.label.hash(hasher);
        self.kind.hash(hasher);
        self.children.len().hash(hasher);
        for child in &self.children {
            child.hash_structure(hasher);
        }
    }

    /// Whether two subtrees have the same shape, labels and kinds.
    #[must_use]
    pub fn structurally_equals(&self, other: &TreeNode) -> bool {
        self.label == other.label
            && self.kind == other.kind
            && self.children.len() == other.children.len()
            && self.children.iter().zip(&other.children).all(|(a, b)| a.structurally_equals(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(label: &str, kind: &'static str) -> Rc<TreeNode> {
        Rc::new(TreeNode::new(label, kind))
    }

    #[test]
    fn subtree_size_counts_every_node_and_tracks_additions() {
        let mut root = TreeNode::new("root", "Program");
        assert_eq!(root.subtree_size(), 1);
        let mut branch = TreeNode::new("+", "BinaryExpression");
        branch.add_child(leaf("a", "Identifier"));
        branch.add_child(leaf("b", "Identifier"));
        root.add_child(Rc::new(branch));
        assert_eq!(root.subtree_size(), 4);
        root.add_child(leaf("c", "Identifier"));
        assert_eq!(root.subtree_size(), 5, "the cached size is invalidated by add_child");
    }

    #[test]
    fn structural_hash_ignores_positions_but_not_shape() {
        let build = |label: &str, span: (u32, u32)| {
            let mut node = TreeNode::new("call", "CallExpression");
            node.set_source_span(span.0, span.1);
            node.add_child(leaf(label, "Identifier"));
            node
        };
        let first = build("run", (0, 5));
        let moved = build("run", (40, 45));
        let renamed = build("stop", (0, 5));
        assert_eq!(first.structural_hash(), moved.structural_hash());
        assert!(first.structurally_equals(&moved));
        assert_ne!(first.structural_hash(), renamed.structural_hash());
        assert!(!first.structurally_equals(&renamed));
        // A label and a kind that swap places are different nodes.
        assert_ne!(
            TreeNode::new("Identifier", "x").structural_hash(),
            TreeNode::new("x", "Identifier").structural_hash()
        );
    }
}
