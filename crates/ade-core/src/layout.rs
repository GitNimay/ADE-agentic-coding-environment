use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::PaneId;

const DEFAULT_SPLIT_RATIO: f32 = 0.5;
const MIN_SPLIT_RATIO: f32 = 0.1;
const MAX_SPLIT_RATIO: f32 = 0.9;

pub const MAX_TERMINALS_PER_WORKSPACE: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Columns,
    Rows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Right,
    Down,
    Left,
    Up,
}

impl SplitDirection {
    #[must_use]
    pub const fn axis(self) -> SplitAxis {
        match self {
            Self::Right | Self::Left => SplitAxis::Columns,
            Self::Down | Self::Up => SplitAxis::Rows,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Empty,
    Pane {
        pane_id: PaneId,
    },
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<Self>,
        second: Box<Self>,
    },
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum LayoutError {
    #[error("pane {0} does not exist in the layout")]
    PaneNotFound(PaneId),
    #[error("pane {0} already exists in the layout")]
    DuplicatePane(PaneId),
    #[error("the final pane in a layout cannot be closed")]
    CannotCloseFinalPane,
    #[error("split ratio must be finite and between 0.1 and 0.9")]
    InvalidRatio,
}

impl LayoutNode {
    #[must_use]
    pub const fn pane(pane_id: PaneId) -> Self {
        Self::Pane { pane_id }
    }

    #[must_use]
    pub fn contains(&self, target: PaneId) -> bool {
        match self {
            Self::Empty => false,
            Self::Pane { pane_id } => *pane_id == target,
            Self::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    #[must_use]
    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        self.collect_panes(&mut panes);
        panes
    }

    /// Splits `target`, placing `new_pane` to its right or below it.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::PaneNotFound`] when `target` is absent, or
    /// [`LayoutError::DuplicatePane`] when `new_pane` is already present.
    pub fn split(
        &mut self,
        target: PaneId,
        new_pane: PaneId,
        direction: SplitDirection,
    ) -> Result<(), LayoutError> {
        if self.contains(new_pane) {
            return Err(LayoutError::DuplicatePane(new_pane));
        }

        let Some(target_node) = self.find_mut(target) else {
            return Err(LayoutError::PaneNotFound(target));
        };
        let original = std::mem::replace(target_node, Self::pane(target));
        *target_node = Self::Split {
            axis: direction.axis(),
            ratio: DEFAULT_SPLIT_RATIO,
            first: Box::new(original),
            second: Box::new(Self::pane(new_pane)),
        };
        Ok(())
    }

    /// Removes `target` and collapses its parent split.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::PaneNotFound`] when `target` is absent, or
    /// [`LayoutError::CannotCloseFinalPane`] when it is the only pane.
    pub fn close(&mut self, target: PaneId) -> Result<(), LayoutError> {
        if matches!(self, Self::Pane { pane_id } if *pane_id == target) {
            return Err(LayoutError::CannotCloseFinalPane);
        }
        if Self::remove_from_split(self, target) {
            Ok(())
        } else {
            Err(LayoutError::PaneNotFound(target))
        }
    }

    /// Updates the split at `path` to use `ratio` for its first child.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::InvalidRatio`] for an invalid path, a path ending in a pane, or a
    /// non-finite ratio outside the supported range.
    pub fn set_ratio(&mut self, path: &[usize], ratio: f32) -> Result<(), LayoutError> {
        if !ratio.is_finite() || !(MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO).contains(&ratio) {
            return Err(LayoutError::InvalidRatio);
        }

        let mut node = self;
        for branch in path {
            match (node, branch) {
                (Self::Split { first, .. }, 0) => node = first,
                (Self::Split { second, .. }, 1) => node = second,
                _ => return Err(LayoutError::InvalidRatio),
            }
        }
        match node {
            Self::Split {
                ratio: node_ratio, ..
            } => {
                *node_ratio = ratio;
                Ok(())
            }
            Self::Empty | Self::Pane { .. } => Err(LayoutError::InvalidRatio),
        }
    }

    /// Checks ratio bounds and pane identifier uniqueness throughout the tree.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::InvalidRatio`] or [`LayoutError::DuplicatePane`] when an invariant
    /// is violated.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let mut panes = HashSet::new();
        self.validate_inner(&mut panes)
    }

    fn collect_panes(&self, panes: &mut Vec<PaneId>) {
        match self {
            Self::Empty => {}
            Self::Pane { pane_id } => panes.push(*pane_id),
            Self::Split { first, second, .. } => {
                first.collect_panes(panes);
                second.collect_panes(panes);
            }
        }
    }

    fn find_mut(&mut self, target: PaneId) -> Option<&mut Self> {
        match self {
            Self::Empty => None,
            Self::Pane { pane_id } => (*pane_id == target).then_some(self),
            Self::Split { first, second, .. } => {
                first.find_mut(target).or_else(|| second.find_mut(target))
            }
        }
    }

    fn remove_from_split(node: &mut Self, target: PaneId) -> bool {
        let Self::Split { first, second, .. } = node else {
            return false;
        };

        if matches!(first.as_ref(), Self::Pane { pane_id } if *pane_id == target) {
            *node = std::mem::replace(second.as_mut(), Self::pane(target));
            return true;
        }
        if matches!(second.as_ref(), Self::Pane { pane_id } if *pane_id == target) {
            *node = std::mem::replace(first.as_mut(), Self::pane(target));
            return true;
        }
        Self::remove_from_split(first, target) || Self::remove_from_split(second, target)
    }

    /// Returns the pane adjacent to `current` in the given `direction`, if one exists.
    ///
    /// Left/Right navigation follows `Columns` splits; Up/Down follows `Rows` splits.
    /// The method walks from `current` toward the root looking for the nearest ancestor
    /// split with a matching axis, then returns the sibling's first leaf pane.
    #[must_use]
    pub fn find_adjacent(&self, current: PaneId, direction: SplitDirection) -> Option<PaneId> {
        let target_axis = direction.axis();
        let forward = matches!(direction, SplitDirection::Right | SplitDirection::Down);
        let mut path = Vec::new();
        self.collect_path(current, &mut path)?;

        while let Some(child_index) = path.pop() {
            let parent = self.node_at(&path)?;

            if let Self::Split { axis, .. } = parent
                && *axis == target_axis
            {
                let dominated = child_index == 0;
                if forward && !dominated {
                    return None;
                }
                if !forward && dominated {
                    continue;
                }
                let mut current_path = path.clone();
                current_path.push(child_index);
                let current_subtree = self.node_at(&current_path)?;
                let current_panes = current_subtree.pane_ids();
                let position = current_panes.iter().position(|&id| id == current)?;

                let sibling_index = 1 - child_index;
                let mut sibling_path = path.clone();
                sibling_path.push(sibling_index);
                let sibling_subtree = self.node_at(&sibling_path)?;
                let sibling_panes = sibling_subtree.pane_ids();
                let target_index = position.min(sibling_panes.len().saturating_sub(1));
                return sibling_panes.get(target_index).copied();
            }
        }

        None
    }

    fn collect_path(&self, target: PaneId, path: &mut Vec<usize>) -> Option<()> {
        match self {
            Self::Empty => None,
            Self::Pane { pane_id } => {
                if *pane_id == target {
                    Some(())
                } else {
                    None
                }
            }
            Self::Split { first, second, .. } => {
                path.push(0);
                if first.collect_path(target, path).is_some() {
                    return Some(());
                }
                path.pop();
                path.push(1);
                if second.collect_path(target, path).is_some() {
                    return Some(());
                }
                path.pop();
                None
            }
        }
    }

    fn node_at(&self, path: &[usize]) -> Option<&Self> {
        let mut node = self;
        for &branch in path {
            match (node, branch) {
                (Self::Split { first, .. }, 0) => node = first,
                (Self::Split { second, .. }, 1) => node = second,
                _ => return None,
            }
        }
        Some(node)
    }

    fn validate_inner(&self, panes: &mut HashSet<PaneId>) -> Result<(), LayoutError> {
        match self {
            Self::Empty => Ok(()),
            Self::Pane { pane_id } => {
                if panes.insert(*pane_id) {
                    Ok(())
                } else {
                    Err(LayoutError::DuplicatePane(*pane_id))
                }
            }
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if !ratio.is_finite() || !(MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO).contains(ratio) {
                    return Err(LayoutError::InvalidRatio);
                }
                first.validate_inner(panes)?;
                second.validate_inner(panes)
            }
        }
    }
}

/// Builds the managed workspace arrangement for up to six panes.
///
/// One to three panes use a single row. Four to six panes use two rows,
/// with the larger row placed first when the count is odd.
#[must_use]
pub fn managed_terminal_layout(panes: &[PaneId]) -> LayoutNode {
    match panes.len() {
        0 => LayoutNode::Empty,
        1..=3 => pane_row(panes),
        _ => {
            let first_row_len = panes.len().div_ceil(2).min(3);
            LayoutNode::Split {
                axis: SplitAxis::Rows,
                ratio: DEFAULT_SPLIT_RATIO,
                first: Box::new(pane_row(&panes[..first_row_len])),
                second: Box::new(pane_row(&panes[first_row_len..])),
            }
        }
    }
}

fn pane_row(panes: &[PaneId]) -> LayoutNode {
    match panes {
        [] => LayoutNode::Empty,
        [pane_id] => LayoutNode::pane(*pane_id),
        [first, rest @ ..] => LayoutNode::Split {
            axis: SplitAxis::Columns,
            ratio: 1.0 / f32::from(u16::try_from(panes.len()).expect("terminal row is too large")),
            first: Box::new(LayoutNode::pane(*first)),
            second: Box::new(pane_row(rest)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_close_collapse_the_tree() {
        let first = PaneId::new();
        let right = PaneId::new();
        let down = PaneId::new();
        let mut layout = LayoutNode::pane(first);

        layout.split(first, right, SplitDirection::Right).unwrap();
        layout.split(first, down, SplitDirection::Down).unwrap();
        assert_eq!(layout.pane_ids(), vec![first, down, right]);

        layout.close(down).unwrap();
        assert_eq!(layout.pane_ids(), vec![first, right]);
        layout.validate().unwrap();
    }

    #[test]
    fn find_adjacent_left_right() {
        let a = PaneId::new();
        let b = PaneId::new();
        let layout = LayoutNode::Split {
            axis: SplitAxis::Columns,
            ratio: 0.5,
            first: Box::new(LayoutNode::pane(a)),
            second: Box::new(LayoutNode::pane(b)),
        };

        assert_eq!(layout.find_adjacent(a, SplitDirection::Right), Some(b));
        assert_eq!(layout.find_adjacent(b, SplitDirection::Left), Some(a));
        assert_eq!(layout.find_adjacent(a, SplitDirection::Left), None);
        assert_eq!(layout.find_adjacent(b, SplitDirection::Right), None);
    }

    #[test]
    fn find_adjacent_up_down() {
        let a = PaneId::new();
        let b = PaneId::new();
        let layout = LayoutNode::Split {
            axis: SplitAxis::Rows,
            ratio: 0.5,
            first: Box::new(LayoutNode::pane(a)),
            second: Box::new(LayoutNode::pane(b)),
        };

        assert_eq!(layout.find_adjacent(a, SplitDirection::Down), Some(b));
        assert_eq!(layout.find_adjacent(b, SplitDirection::Up), Some(a));
        assert_eq!(layout.find_adjacent(a, SplitDirection::Up), None);
        assert_eq!(layout.find_adjacent(b, SplitDirection::Down), None);
    }

    #[test]
    fn find_adjacent_cross_axis_returns_none() {
        let a = PaneId::new();
        let b = PaneId::new();
        let layout = LayoutNode::Split {
            axis: SplitAxis::Columns,
            ratio: 0.5,
            first: Box::new(LayoutNode::pane(a)),
            second: Box::new(LayoutNode::pane(b)),
        };

        assert_eq!(layout.find_adjacent(a, SplitDirection::Up), None);
        assert_eq!(layout.find_adjacent(a, SplitDirection::Down), None);
    }

    #[test]
    fn find_adjacent_three_column_row() {
        let a = PaneId::new();
        let b = PaneId::new();
        let c = PaneId::new();
        let layout = LayoutNode::Split {
            axis: SplitAxis::Columns,
            ratio: 1.0 / 3.0,
            first: Box::new(LayoutNode::pane(a)),
            second: Box::new(LayoutNode::Split {
                axis: SplitAxis::Columns,
                ratio: 0.5,
                first: Box::new(LayoutNode::pane(b)),
                second: Box::new(LayoutNode::pane(c)),
            }),
        };

        assert_eq!(layout.find_adjacent(a, SplitDirection::Right), Some(b));
        assert_eq!(layout.find_adjacent(b, SplitDirection::Left), Some(a));
        assert_eq!(layout.find_adjacent(b, SplitDirection::Right), Some(c));
        assert_eq!(layout.find_adjacent(c, SplitDirection::Left), Some(b));
    }

    #[test]
    fn find_adjacent_2x2_grid() {
        let tl = PaneId::new();
        let tr = PaneId::new();
        let bl = PaneId::new();
        let br = PaneId::new();
        let layout = LayoutNode::Split {
            axis: SplitAxis::Rows,
            ratio: 0.5,
            first: Box::new(LayoutNode::Split {
                axis: SplitAxis::Columns,
                ratio: 0.5,
                first: Box::new(LayoutNode::pane(tl)),
                second: Box::new(LayoutNode::pane(tr)),
            }),
            second: Box::new(LayoutNode::Split {
                axis: SplitAxis::Columns,
                ratio: 0.5,
                first: Box::new(LayoutNode::pane(bl)),
                second: Box::new(LayoutNode::pane(br)),
            }),
        };

        assert_eq!(layout.find_adjacent(tl, SplitDirection::Right), Some(tr));
        assert_eq!(layout.find_adjacent(tl, SplitDirection::Down), Some(bl));
        assert_eq!(layout.find_adjacent(br, SplitDirection::Left), Some(bl));
        assert_eq!(layout.find_adjacent(br, SplitDirection::Up), Some(tr));
        assert_eq!(layout.find_adjacent(tr, SplitDirection::Left), Some(tl));
        assert_eq!(layout.find_adjacent(tr, SplitDirection::Down), Some(br));
        assert_eq!(layout.find_adjacent(bl, SplitDirection::Right), Some(br));
        assert_eq!(layout.find_adjacent(bl, SplitDirection::Up), Some(tl));
    }

    #[test]
    fn final_pane_cannot_be_closed() {
        let pane = PaneId::new();
        let mut layout = LayoutNode::pane(pane);

        assert_eq!(layout.close(pane), Err(LayoutError::CannotCloseFinalPane));
    }

    #[test]
    fn empty_layout_has_no_panes_and_is_valid() {
        let layout = LayoutNode::Empty;

        assert!(layout.pane_ids().is_empty());
        layout.validate().unwrap();
    }

    #[test]
    fn duplicate_panes_are_rejected() {
        let pane = PaneId::new();
        let layout = LayoutNode::Split {
            axis: SplitAxis::Columns,
            ratio: 0.5,
            first: Box::new(LayoutNode::pane(pane)),
            second: Box::new(LayoutNode::pane(pane)),
        };

        assert_eq!(layout.validate(), Err(LayoutError::DuplicatePane(pane)));
    }

    #[test]
    fn layout_round_trips_through_json() {
        let first = PaneId::new();
        let second = PaneId::new();
        let mut layout = LayoutNode::pane(first);
        layout.split(first, second, SplitDirection::Right).unwrap();

        let json = serde_json::to_string(&layout).unwrap();
        let restored: LayoutNode = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, layout);
        restored.validate().unwrap();
    }

    #[test]
    fn managed_terminal_layout_uses_expected_grid_for_each_supported_count() {
        let panes: Vec<_> = (0..MAX_TERMINALS_PER_WORKSPACE)
            .map(|_| PaneId::new())
            .collect();

        assert_grid(&managed_terminal_layout(&panes[..0]), &panes, &[]);
        assert_grid(&managed_terminal_layout(&panes[..1]), &panes, &[1]);
        assert_grid(&managed_terminal_layout(&panes[..2]), &panes, &[2]);
        assert_grid(&managed_terminal_layout(&panes[..3]), &panes, &[3]);
        assert_grid(&managed_terminal_layout(&panes[..4]), &panes, &[2, 2]);
        assert_grid(&managed_terminal_layout(&panes[..5]), &panes, &[3, 2]);
        assert_grid(&managed_terminal_layout(&panes[..6]), &panes, &[3, 3]);
    }

    fn assert_grid(layout: &LayoutNode, panes: &[PaneId], row_lengths: &[usize]) {
        layout.validate().unwrap();
        assert_eq!(
            layout.pane_ids(),
            panes[..row_lengths.iter().sum::<usize>()]
        );

        match row_lengths {
            [] => assert!(matches!(layout, LayoutNode::Empty)),
            [row_length] => assert_row(layout, &panes[..*row_length]),
            [first_row_length, second_row_length] => {
                let LayoutNode::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } = layout
                else {
                    panic!("expected a two-row terminal grid");
                };
                assert_eq!(*axis, SplitAxis::Rows);
                assert!((*ratio - 0.5).abs() < f32::EPSILON);
                assert_row(first, &panes[..*first_row_length]);
                assert_row(
                    second,
                    &panes[*first_row_length..*first_row_length + *second_row_length],
                );
            }
            _ => panic!("managed terminal layouts support at most two rows"),
        }
    }

    fn assert_row(layout: &LayoutNode, panes: &[PaneId]) {
        match panes {
            [pane_id] => assert_eq!(layout, &LayoutNode::pane(*pane_id)),
            [first_pane, remaining @ ..] => {
                let LayoutNode::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } = layout
                else {
                    panic!("expected a terminal row");
                };
                assert_eq!(*axis, SplitAxis::Columns);
                let pane_count = u16::try_from(panes.len()).unwrap();
                assert!((*ratio - 1.0 / f32::from(pane_count)).abs() < f32::EPSILON);
                assert_eq!(first.as_ref(), &LayoutNode::pane(*first_pane));
                assert_row(second, remaining);
            }
            [] => assert!(matches!(layout, LayoutNode::Empty)),
        }
    }
}
