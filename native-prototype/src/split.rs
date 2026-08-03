use egui::{pos2, vec2, Pos2, Rect};

pub type PaneId = String;
pub type SplitId = u64;

pub const DIVIDER_SIZE: f32 = 4.0;
pub const MIN_SPLIT_RATIO: f32 = 0.1;
pub const MAX_SPLIT_RATIO: f32 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    /// A horizontal divider: the panes are arranged top and bottom.
    Horizontal,
    /// A vertical divider: the panes are arranged left and right.
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaneNode {
    Leaf {
        pane_id: PaneId,
    },
    Split {
        id: SplitId,
        direction: SplitDirection,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneViewport {
    pub pane_id: PaneId,
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DividerViewport {
    pub split_id: SplitId,
    pub direction: SplitDirection,
    pub rect: Rect,
    pub parent_rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutSnapshot {
    pub panes: Vec<PaneViewport>,
    pub dividers: Vec<DividerViewport>,
}

impl LayoutSnapshot {
    pub fn pane_at(&self, position: Pos2) -> Option<&PaneViewport> {
        self.panes.iter().find(|pane| pane.rect.contains(position))
    }

    pub fn pane(&self, pane_id: &str) -> Option<&PaneViewport> {
        self.panes.iter().find(|pane| pane.pane_id == pane_id)
    }

    pub fn divider_at(&self, position: Pos2) -> Option<DividerViewport> {
        self.dividers
            .iter()
            .copied()
            .find(|divider| divider.rect.contains(position))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosePaneResult {
    NotFound,
    LastPane,
    Closed { suggested_active: PaneId },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneTree {
    root: PaneNode,
    next_split_id: SplitId,
}

impl PaneTree {
    pub fn new(initial_pane_id: PaneId) -> Self {
        Self {
            root: PaneNode::Leaf {
                pane_id: initial_pane_id,
            },
            next_split_id: 1,
        }
    }

    pub fn root(&self) -> &PaneNode {
        &self.root
    }

    pub fn pane_count(&self) -> usize {
        pane_count(&self.root)
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut panes = Vec::with_capacity(self.pane_count());
        collect_panes(&self.root, &mut panes);
        panes
    }

    pub fn contains(&self, pane_id: &str) -> bool {
        contains_pane(&self.root, pane_id)
    }

    pub fn first_pane_id(&self) -> &str {
        first_pane_id(&self.root)
    }

    pub fn split(
        &mut self,
        target_pane_id: &str,
        direction: SplitDirection,
        new_pane_id: PaneId,
    ) -> bool {
        if self.contains(&new_pane_id) {
            return false;
        }
        let split_id = self.next_split_id;
        if !split_node(
            &mut self.root,
            target_pane_id,
            direction,
            new_pane_id,
            split_id,
        ) {
            return false;
        }
        self.next_split_id = self.next_split_id.wrapping_add(1).max(1);
        true
    }

    pub fn close(&mut self, pane_id: &str) -> ClosePaneResult {
        if !self.contains(pane_id) {
            return ClosePaneResult::NotFound;
        }
        if self.pane_count() == 1 {
            return ClosePaneResult::LastPane;
        }
        let Some((replacement, suggested_active)) = remove_pane(self.root.clone(), pane_id) else {
            return ClosePaneResult::NotFound;
        };
        self.root = replacement.expect("a multi-pane tree must retain one sibling");
        ClosePaneResult::Closed { suggested_active }
    }

    pub fn set_ratio(&mut self, split_id: SplitId, ratio: f32) -> bool {
        let Some(stored) = find_ratio_mut(&mut self.root, split_id) else {
            return false;
        };
        *stored = sanitize_ratio(ratio);
        true
    }

    pub fn ratio(&self, split_id: SplitId) -> Option<f32> {
        find_ratio(&self.root, split_id)
    }

    pub fn layout(&self, rect: Rect) -> LayoutSnapshot {
        let mut snapshot = LayoutSnapshot::default();
        layout_node(&self.root, finite_rect(rect), &mut snapshot);
        snapshot
    }

    pub fn ratio_for_pointer(divider: DividerViewport, position: Pos2) -> f32 {
        let ratio = match divider.direction {
            SplitDirection::Horizontal => {
                (position.y - divider.parent_rect.top()) / divider.parent_rect.height()
            }
            SplitDirection::Vertical => {
                (position.x - divider.parent_rect.left()) / divider.parent_rect.width()
            }
        };
        sanitize_ratio(ratio)
    }
}

fn sanitize_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
    } else {
        0.5
    }
}

fn finite_rect(rect: Rect) -> Rect {
    if !rect.min.is_finite() || !rect.max.is_finite() {
        return Rect::from_min_size(Pos2::ZERO, egui::Vec2::ZERO);
    }
    Rect::from_min_max(
        pos2(rect.min.x.min(rect.max.x), rect.min.y.min(rect.max.y)),
        pos2(rect.min.x.max(rect.max.x), rect.min.y.max(rect.max.y)),
    )
}

fn pane_count(node: &PaneNode) -> usize {
    match node {
        PaneNode::Leaf { .. } => 1,
        PaneNode::Split { first, second, .. } => pane_count(first) + pane_count(second),
    }
}

fn collect_panes(node: &PaneNode, output: &mut Vec<PaneId>) {
    match node {
        PaneNode::Leaf { pane_id } => output.push(pane_id.clone()),
        PaneNode::Split { first, second, .. } => {
            collect_panes(first, output);
            collect_panes(second, output);
        }
    }
}

fn contains_pane(node: &PaneNode, target: &str) -> bool {
    match node {
        PaneNode::Leaf { pane_id } => pane_id == target,
        PaneNode::Split { first, second, .. } => {
            contains_pane(first, target) || contains_pane(second, target)
        }
    }
}

fn first_pane_id(node: &PaneNode) -> &str {
    match node {
        PaneNode::Leaf { pane_id } => pane_id,
        PaneNode::Split { first, .. } => first_pane_id(first),
    }
}

fn split_node(
    node: &mut PaneNode,
    target: &str,
    direction: SplitDirection,
    new_pane_id: PaneId,
    split_id: SplitId,
) -> bool {
    match node {
        PaneNode::Leaf { pane_id } if pane_id == target => {
            let original = pane_id.clone();
            *node = PaneNode::Split {
                id: split_id,
                direction,
                ratio: 0.5,
                first: Box::new(PaneNode::Leaf { pane_id: original }),
                second: Box::new(PaneNode::Leaf {
                    pane_id: new_pane_id,
                }),
            };
            true
        }
        PaneNode::Split { first, second, .. } => {
            if split_node(first, target, direction, new_pane_id.clone(), split_id) {
                true
            } else {
                split_node(second, target, direction, new_pane_id, split_id)
            }
        }
        PaneNode::Leaf { .. } => false,
    }
}

fn remove_pane(node: PaneNode, target: &str) -> Option<(Option<PaneNode>, PaneId)> {
    match node {
        PaneNode::Leaf { pane_id } => (pane_id == target).then(|| (None, pane_id)),
        PaneNode::Split {
            id,
            direction,
            ratio,
            first,
            second,
        } => {
            if contains_pane(&first, target) {
                let (replacement, suggested_active) = remove_pane(*first, target)?;
                match replacement {
                    Some(first) => Some((
                        Some(PaneNode::Split {
                            id,
                            direction,
                            ratio,
                            first: Box::new(first),
                            second,
                        }),
                        suggested_active,
                    )),
                    None => {
                        let suggested = first_pane_id(&second).to_owned();
                        Some((Some(*second), suggested))
                    }
                }
            } else if contains_pane(&second, target) {
                let (replacement, suggested_active) = remove_pane(*second, target)?;
                match replacement {
                    Some(second) => Some((
                        Some(PaneNode::Split {
                            id,
                            direction,
                            ratio,
                            first,
                            second: Box::new(second),
                        }),
                        suggested_active,
                    )),
                    None => {
                        let suggested = first_pane_id(&first).to_owned();
                        Some((Some(*first), suggested))
                    }
                }
            } else {
                None
            }
        }
    }
}

fn find_ratio_mut(node: &mut PaneNode, target: SplitId) -> Option<&mut f32> {
    match node {
        PaneNode::Leaf { .. } => None,
        PaneNode::Split {
            id,
            ratio,
            first,
            second,
            ..
        } => {
            if *id == target {
                Some(ratio)
            } else {
                find_ratio_mut(first, target).or_else(|| find_ratio_mut(second, target))
            }
        }
    }
}

fn find_ratio(node: &PaneNode, target: SplitId) -> Option<f32> {
    match node {
        PaneNode::Leaf { .. } => None,
        PaneNode::Split {
            id,
            ratio,
            first,
            second,
            ..
        } => {
            if *id == target {
                Some(*ratio)
            } else {
                find_ratio(first, target).or_else(|| find_ratio(second, target))
            }
        }
    }
}

fn layout_node(node: &PaneNode, rect: Rect, output: &mut LayoutSnapshot) {
    match node {
        PaneNode::Leaf { pane_id } => output.panes.push(PaneViewport {
            pane_id: pane_id.clone(),
            rect,
        }),
        PaneNode::Split {
            id,
            direction,
            ratio,
            first,
            second,
        } => {
            let ratio = sanitize_ratio(*ratio);
            let (first_rect, divider_rect, second_rect) = match direction {
                SplitDirection::Horizontal => {
                    let usable = (rect.height() - DIVIDER_SIZE).max(0.0);
                    let first_height = usable * ratio;
                    let divider_top = rect.top() + first_height;
                    (
                        Rect::from_min_size(rect.min, vec2(rect.width(), first_height)),
                        Rect::from_min_size(
                            pos2(rect.left(), divider_top),
                            vec2(rect.width(), DIVIDER_SIZE.min(rect.height())),
                        ),
                        Rect::from_min_max(
                            pos2(rect.left(), (divider_top + DIVIDER_SIZE).min(rect.bottom())),
                            rect.max,
                        ),
                    )
                }
                SplitDirection::Vertical => {
                    let usable = (rect.width() - DIVIDER_SIZE).max(0.0);
                    let first_width = usable * ratio;
                    let divider_left = rect.left() + first_width;
                    (
                        Rect::from_min_size(rect.min, vec2(first_width, rect.height())),
                        Rect::from_min_size(
                            pos2(divider_left, rect.top()),
                            vec2(DIVIDER_SIZE.min(rect.width()), rect.height()),
                        ),
                        Rect::from_min_max(
                            pos2((divider_left + DIVIDER_SIZE).min(rect.right()), rect.top()),
                            rect.max,
                        ),
                    )
                }
            };
            output.dividers.push(DividerViewport {
                split_id: *id,
                direction: *direction,
                rect: divider_rect,
                parent_rect: rect,
            });
            layout_node(first, first_rect, output);
            layout_node(second, second_rect, output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(width: f32, height: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(width, height))
    }

    #[test]
    fn horizontal_is_top_bottom_and_vertical_is_left_right() {
        let mut horizontal = PaneTree::new("a".into());
        assert!(horizontal.split("a", SplitDirection::Horizontal, "b".into()));
        let layout = horizontal.layout(rect(100.0, 80.0));
        assert_eq!(layout.panes[0].rect.width(), 100.0);
        assert_eq!(layout.panes[0].rect.height(), 38.0);
        assert_eq!(layout.panes[1].rect.top(), 42.0);

        let mut vertical = PaneTree::new("a".into());
        assert!(vertical.split("a", SplitDirection::Vertical, "b".into()));
        let layout = vertical.layout(rect(100.0, 80.0));
        assert_eq!(layout.panes[0].rect.width(), 48.0);
        assert_eq!(layout.panes[1].rect.left(), 52.0);
        assert_eq!(layout.panes[1].rect.height(), 80.0);
    }

    #[test]
    fn nested_split_has_stable_unique_ids_and_leaf_order() {
        let mut tree = PaneTree::new("a".into());
        assert!(tree.split("a", SplitDirection::Vertical, "b".into()));
        assert!(tree.split("b", SplitDirection::Horizontal, "c".into()));
        assert_eq!(tree.pane_ids(), vec!["a", "b", "c"]);
        let layout = tree.layout(rect(100.0, 100.0));
        assert_eq!(layout.dividers.len(), 2);
        assert_ne!(layout.dividers[0].split_id, layout.dividers[1].split_id);
    }

    #[test]
    fn close_promotes_sibling_and_suggests_it_for_focus() {
        let mut tree = PaneTree::new("a".into());
        tree.split("a", SplitDirection::Vertical, "b".into());
        tree.split("b", SplitDirection::Horizontal, "c".into());

        assert_eq!(
            tree.close("b"),
            ClosePaneResult::Closed {
                suggested_active: "c".into()
            }
        );
        assert_eq!(tree.pane_ids(), vec!["a", "c"]);
        assert_eq!(
            tree.close("a"),
            ClosePaneResult::Closed {
                suggested_active: "c".into()
            }
        );
        assert_eq!(tree.pane_ids(), vec!["c"]);
        assert_eq!(tree.close("c"), ClosePaneResult::LastPane);
    }

    #[test]
    fn duplicate_and_missing_pane_ids_are_rejected() {
        let mut tree = PaneTree::new("a".into());
        assert!(!tree.split("missing", SplitDirection::Vertical, "b".into()));
        assert!(!tree.split("a", SplitDirection::Vertical, "a".into()));
        assert_eq!(tree.close("missing"), ClosePaneResult::NotFound);
    }

    #[test]
    fn ratios_are_sanitized_and_dividers_are_hit_tested() {
        let mut tree = PaneTree::new("a".into());
        tree.split("a", SplitDirection::Vertical, "b".into());
        assert!(tree.set_ratio(1, f32::NAN));
        assert_eq!(tree.ratio(1), Some(0.5));
        assert!(tree.set_ratio(1, 10.0));
        assert_eq!(tree.ratio(1), Some(MAX_SPLIT_RATIO));

        let layout = tree.layout(rect(100.0, 80.0));
        let divider = layout.dividers[0];
        assert_eq!(layout.divider_at(divider.rect.center()), Some(divider));
        assert_eq!(
            PaneTree::ratio_for_pointer(divider, pos2(10.0, 30.0)),
            MIN_SPLIT_RATIO
        );
    }
}
