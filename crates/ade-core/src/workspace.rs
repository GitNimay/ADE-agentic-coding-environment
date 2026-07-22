use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{LayoutNode, PaneId, WorkspaceId};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Running,
    Exited { exit_code: u32 },
    FailedToStart { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub root_directory: PathBuf,
    pub layout: LayoutNode,
    pub active_pane_id: Option<PaneId>,
}

impl Workspace {
    #[must_use]
    pub fn new(name: impl Into<String>, root_directory: PathBuf) -> Self {
        let active_pane_id = PaneId::new();
        Self {
            id: WorkspaceId::new(),
            name: name.into(),
            root_directory,
            layout: LayoutNode::pane(active_pane_id),
            active_pane_id: Some(active_pane_id),
        }
    }
}
