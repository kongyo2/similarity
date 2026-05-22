use std::cell::Cell;
use std::rc::Rc;

/// AST node used by the structural-distance comparators.
///
/// `subtree_size` lazily caches the total node count of `self` plus its
/// descendants. It's measured in O(n) the first time it's requested and
/// O(1) thereafter — the APTED/cutoff code calls it inside hot loops so
/// recomputing each time pushed pathological inputs into multi-minute
/// territory. We use `Cell` for interior mutability because `TreeNode` is
/// wrapped in `Rc` (no `&mut` access) and the cached value never escapes
/// the same logical tree — so `Cell` is enough; no need for `RefCell`'s
/// extra borrow tracking.
#[derive(Debug)]
pub struct TreeNode {
    pub label: String,
    pub value: String,
    pub children: Vec<Rc<TreeNode>>,
    pub id: usize,
    subtree_size: Cell<Option<usize>>,
}

impl Clone for TreeNode {
    fn clone(&self) -> Self {
        TreeNode {
            label: self.label.clone(),
            value: self.value.clone(),
            children: self.children.clone(),
            id: self.id,
            subtree_size: Cell::new(self.subtree_size.get()),
        }
    }
}

impl TreeNode {
    #[must_use]
    pub fn new(label: String, value: String, id: usize) -> Self {
        TreeNode { label, value, children: Vec::new(), id, subtree_size: Cell::new(None) }
    }

    pub fn add_child(&mut self, child: Rc<TreeNode>) {
        // Adding a child invalidates the cached subtree size.
        self.subtree_size.set(None);
        self.children.push(child);
    }

    #[must_use]
    pub fn get_subtree_size(&self) -> usize {
        if let Some(size) = self.subtree_size.get() {
            return size;
        }
        let mut size = 1;
        for child in &self.children {
            size += child.get_subtree_size();
        }
        self.subtree_size.set(Some(size));
        size
    }
}
