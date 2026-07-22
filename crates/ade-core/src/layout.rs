use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::PaneId;

const DEFAULT_SPLIT_RATIO: f32 = 0.5;
const MIN_SPLIT_RATIO: f32 = 0.1;
const MAX_SPLIT_RATIO: f32 = 0.9;

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
}

impl SplitDirection {
    const fn axis(self) -> SplitAxis {
        match self {
            Self::Right => SplitAxis::Columns,
            Self::Down => SplitAxis::Rows,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
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
            Self::Pane { .. } => Err(LayoutError::InvalidRatio),
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
            Self::Pane { pane_id } => panes.push(*pane_id),
            Self::Split { first, second, .. } => {
                first.collect_panes(panes);
                second.collect_panes(panes);
            }
        }
    }

    fn find_mut(&mut self, target: PaneId) -> Option<&mut Self> {
        match self {
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

    fn validate_inner(&self, panes: &mut HashSet<PaneId>) -> Result<(), LayoutError> {
        match self {
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
    fn final_pane_cannot_be_closed() {
        let pane = PaneId::new();
        let mut layout = LayoutNode::pane(pane);

        assert_eq!(layout.close(pane), Err(LayoutError::CannotCloseFinalPane));
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
}
