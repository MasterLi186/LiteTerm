/// Direction of a split between two panes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

/// A node in the binary split tree.
#[derive(Debug)]
pub enum SplitNode {
    /// A terminal pane with a unique identifier.
    Leaf { pane_id: usize },
    /// An internal split containing two children.
    Split {
        direction: SplitDirection,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    /// Count the number of leaf panes in this subtree.
    fn pane_count(&self) -> usize {
        match self {
            SplitNode::Leaf { .. } => 1,
            SplitNode::Split { first, second, .. } => {
                first.pane_count() + second.pane_count()
            }
        }
    }

    /// Collect all pane IDs in this subtree.
    fn pane_ids(&self) -> Vec<usize> {
        match self {
            SplitNode::Leaf { pane_id } => vec![*pane_id],
            SplitNode::Split { first, second, .. } => {
                let mut ids = first.pane_ids();
                ids.extend(second.pane_ids());
                ids
            }
        }
    }

    /// Replace the leaf with `target_pane_id` by a Split node containing
    /// the original leaf and a new leaf with `new_pane_id`.
    /// Returns `true` if the target was found and split.
    fn split_at(&mut self, target_pane_id: usize, direction: SplitDirection, new_pane_id: usize) -> bool {
        match self {
            SplitNode::Leaf { pane_id } if *pane_id == target_pane_id => {
                let original = Box::new(SplitNode::Leaf { pane_id: *pane_id });
                let new_leaf = Box::new(SplitNode::Leaf { pane_id: new_pane_id });
                *self = SplitNode::Split {
                    direction,
                    first: original,
                    second: new_leaf,
                };
                true
            }
            SplitNode::Split { first, second, .. } => {
                first.split_at(target_pane_id, direction, new_pane_id)
                    || second.split_at(target_pane_id, direction, new_pane_id)
            }
            _ => false,
        }
    }

    /// Remove the leaf with `pane_id`. Returns `Some(sibling)` if this node
    /// was a Split that contained the target, meaning the caller should
    /// replace this node with the returned sibling. Returns `None` otherwise.
    fn close_leaf(&mut self, pane_id: usize) -> Option<SplitNode> {
        match self {
            SplitNode::Leaf { .. } => None,
            SplitNode::Split { first, second, .. } => {
                // Check if one of the direct children is the target leaf.
                let first_is_target = matches!(first.as_ref(), SplitNode::Leaf { pane_id: id } if *id == pane_id);
                let second_is_target = matches!(second.as_ref(), SplitNode::Leaf { pane_id: id } if *id == pane_id);

                if first_is_target {
                    // Remove first, promote second.
                    // We need to take ownership of second. Use a placeholder.
                    let placeholder = SplitNode::Leaf { pane_id: 0 };
                    let sibling = std::mem::replace(second.as_mut(), placeholder);
                    Some(sibling)
                } else if second_is_target {
                    let placeholder = SplitNode::Leaf { pane_id: 0 };
                    let sibling = std::mem::replace(first.as_mut(), placeholder);
                    Some(sibling)
                } else {
                    // Recurse into children. If a child returns a replacement,
                    // swap that child with the replacement.
                    if let Some(replacement) = first.close_leaf(pane_id) {
                        *first = Box::new(replacement);
                        return None;
                    }
                    if let Some(replacement) = second.close_leaf(pane_id) {
                        *second = Box::new(replacement);
                        return None;
                    }
                    None
                }
            }
        }
    }
}

/// A binary tree representing nested split panes.
///
/// Each leaf holds a `pane_id` (an opaque identifier for a terminal pane).
/// Internal nodes represent a horizontal or vertical split between two
/// subtrees.
pub struct SplitTree {
    root: SplitNode,
}

impl SplitTree {
    /// Create a new tree containing a single pane.
    pub fn new(initial_pane_id: usize) -> Self {
        SplitTree {
            root: SplitNode::Leaf { pane_id: initial_pane_id },
        }
    }

    /// Returns `true` if the tree is a single leaf (no splits).
    pub fn is_leaf(&self) -> bool {
        matches!(self.root, SplitNode::Leaf { .. })
    }

    /// Count the total number of panes (leaves) in the tree.
    pub fn pane_count(&self) -> usize {
        self.root.pane_count()
    }

    /// Collect all pane IDs in the tree.
    pub fn pane_ids(&self) -> Vec<usize> {
        self.root.pane_ids()
    }

    /// Split the pane identified by `target_pane_id` in the given direction,
    /// creating a new pane with `new_pane_id`.
    ///
    /// The target leaf is replaced by a Split node whose first child is the
    /// original leaf and whose second child is the new leaf.
    pub fn split(&mut self, target_pane_id: usize, direction: SplitDirection, new_pane_id: usize) {
        self.root.split_at(target_pane_id, direction, new_pane_id);
    }

    /// Close the pane identified by `pane_id`.
    ///
    /// The parent Split node is replaced by the remaining sibling.
    /// If the tree has only one pane, this is a no-op.
    pub fn close(&mut self, pane_id: usize) {
        if let Some(replacement) = self.root.close_leaf(pane_id) {
            self.root = replacement;
        }
    }
}
