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
///
/// `source_span` carries the byte range the node was parsed from. The
/// subtree-fingerprint overlap detector uses it to report meaningful
/// `(start_line, end_line)` ranges instead of placeholder values derived
/// from internal node ids.
#[derive(Debug)]
pub struct TreeNode {
    pub label: String,
    pub value: String,
    pub children: Vec<Rc<TreeNode>>,
    pub id: usize,
    /// (start_byte, end_byte) for the source slice this node was parsed
    /// from. `0,0` indicates an absent or synthetic node — callers should
    /// treat that as "no source position available" rather than "byte
    /// range [0, 0]".
    pub source_span: (u32, u32),
    subtree_size: Cell<Option<usize>>,
}

impl Clone for TreeNode {
    fn clone(&self) -> Self {
        TreeNode {
            label: self.label.clone(),
            value: self.value.clone(),
            children: self.children.clone(),
            id: self.id,
            source_span: self.source_span,
            subtree_size: Cell::new(self.subtree_size.get()),
        }
    }
}

impl TreeNode {
    #[must_use]
    pub fn new(label: String, value: String, id: usize) -> Self {
        TreeNode {
            label,
            value,
            children: Vec::new(),
            id,
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
