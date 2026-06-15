use guishell::ui::split::{SplitDirection, SplitTree};

#[test]
fn test_initial_tree_is_single_pane() {
    let tree = SplitTree::new(0);
    assert_eq!(tree.pane_count(), 1);
    assert!(tree.is_leaf());
}

#[test]
fn test_split_horizontal() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Horizontal, 1);
    assert_eq!(tree.pane_count(), 2);
    assert!(!tree.is_leaf());
}

#[test]
fn test_split_vertical_then_close() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Vertical, 1);
    assert_eq!(tree.pane_count(), 2);
    tree.close(1);
    assert_eq!(tree.pane_count(), 1);
}

#[test]
fn test_nested_split() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Horizontal, 1);
    tree.split(1, SplitDirection::Vertical, 2);
    assert_eq!(tree.pane_count(), 3);
}

#[test]
fn test_all_pane_ids() {
    let mut tree = SplitTree::new(0);
    tree.split(0, SplitDirection::Horizontal, 1);
    tree.split(1, SplitDirection::Vertical, 2);
    let mut ids = tree.pane_ids();
    ids.sort();
    assert_eq!(ids, vec![0, 1, 2]);
}
