use std::path::PathBuf;

use ade_core::{
    LayoutError, LayoutNode, MAX_TERMINALS_PER_WORKSPACE, PaneId, SessionStatus, Workspace,
    WorkspaceId, managed_terminal_layout,
};
use ade_protocol::{AppSnapshot, ClientRequest, PaneSnapshot, WorkspaceSnapshot};
use ade_storage::{Repository, StorageError};
use thiserror::Error;

#[cfg(windows)]
mod windows;

const DEFAULT_COLUMNS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("workspace {0} does not exist")]
    WorkspaceNotFound(WorkspaceId),
    #[error("workspace {0} has no active pane")]
    WorkspaceHasNoActivePane(WorkspaceId),
    #[error("workspace {workspace_id} is limited to {limit} terminals")]
    TerminalLimitReached {
        workspace_id: WorkspaceId,
        limit: usize,
    },
    #[error("pane {0} does not exist")]
    PaneNotFound(PaneId),
    #[error("layout mutation failed: {0}")]
    Layout(#[from] LayoutError),
    #[error("workspace name cannot be empty")]
    EmptyWorkspaceName,
    #[error("terminal size must be non-zero")]
    InvalidTerminalSize,
    #[error("PTY failed: {0}")]
    Pty(#[from] ade_pty::PtyError),
    #[error("protocol failed: {0}")]
    Protocol(#[from] ade_protocol::FrameError),
    #[error("daemon I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(windows)]
    #[error("Windows API failed: {0}")]
    Windows(#[from] ::windows::core::Error),
    #[error("client protocol version {received} is unsupported; expected {expected}")]
    ProtocolVersion { received: u32, expected: u32 },
    #[error("daemon is only supported on Windows")]
    UnsupportedPlatform,
    #[error("the per-user ADE daemon is already running")]
    AlreadyRunning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateAction {
    None,
    SpawnPane(PaneId),
    ClosePane(PaneId),
    ClosePanes(Vec<PaneId>),
    Input {
        pane_id: PaneId,
        data: Vec<u8>,
    },
    Resize {
        pane_id: PaneId,
        cols: u16,
        rows: u16,
    },
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateChange {
    pub action: StateAction,
    pub persist: bool,
    pub publish_snapshot: bool,
}

impl StateChange {
    const fn runtime(action: StateAction) -> Self {
        Self {
            action,
            persist: false,
            publish_snapshot: false,
        }
    }

    const fn mutation(action: StateAction) -> Self {
        Self {
            action,
            persist: true,
            publish_snapshot: true,
        }
    }
}

/// Authoritative, transport-independent daemon model.
pub struct DaemonState {
    snapshot: AppSnapshot,
}

impl DaemonState {
    #[must_use]
    pub fn new(mut snapshot: AppSnapshot) -> Self {
        // Persisted processes cannot survive daemon restarts. Runtime startup replaces these.
        for pane in &mut snapshot.panes {
            pane.status = SessionStatus::Starting;
        }
        Self { snapshot }
    }

    #[must_use]
    pub const fn snapshot(&self) -> &AppSnapshot {
        &self.snapshot
    }

    /// Returns pane metadata by identifier.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::PaneNotFound`] when the identifier is unknown.
    pub fn pane(&self, pane_id: PaneId) -> Result<&PaneSnapshot, DaemonError> {
        self.snapshot
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .ok_or(DaemonError::PaneNotFound(pane_id))
    }

    /// Updates the observed root-process status for a pane.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::PaneNotFound`] when the identifier is unknown.
    pub fn set_pane_status(
        &mut self,
        pane_id: PaneId,
        status: SessionStatus,
    ) -> Result<(), DaemonError> {
        let pane = self
            .snapshot
            .panes
            .iter_mut()
            .find(|pane| pane.id == pane_id)
            .ok_or(DaemonError::PaneNotFound(pane_id))?;
        pane.status = status;
        Ok(())
    }

    /// Stores a terminal-reported working directory and returns whether it changed.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::PaneNotFound`] when the identifier is unknown.
    pub fn set_pane_cwd(&mut self, pane_id: PaneId, cwd: PathBuf) -> Result<bool, DaemonError> {
        let pane = self
            .snapshot
            .panes
            .iter_mut()
            .find(|pane| pane.id == pane_id)
            .ok_or(DaemonError::PaneNotFound(pane_id))?;
        if pane.cwd == cwd {
            return Ok(false);
        }
        pane.cwd = cwd;
        Ok(true)
    }

    /// Validates and applies one client request, returning its runtime side effect.
    ///
    /// # Errors
    ///
    /// Returns an error when an ID is unknown or the requested state mutation is invalid.
    #[allow(clippy::too_many_lines)]
    pub fn handle(
        &mut self,
        request: ClientRequest,
        process_label: &str,
    ) -> Result<StateChange, DaemonError> {
        match request {
            ClientRequest::Attach | ClientRequest::GetSnapshot => {
                Ok(StateChange::runtime(StateAction::None))
            }
            ClientRequest::CreateWorkspace { name, root } => {
                validate_name(&name)?;
                let workspace = Workspace::new(name, root.clone());
                let pane_id = workspace
                    .active_pane_id
                    .ok_or(DaemonError::WorkspaceHasNoActivePane(workspace.id))?;
                self.snapshot.panes.push(PaneSnapshot {
                    id: pane_id,
                    workspace_id: workspace.id,
                    status: SessionStatus::Starting,
                    cwd: root,
                    process_label: process_label.to_owned(),
                    cols: DEFAULT_COLUMNS,
                    rows: DEFAULT_ROWS,
                });
                self.snapshot.active_workspace_id = Some(workspace.id);
                self.snapshot.workspaces.push(workspace_snapshot(workspace));
                Ok(StateChange::mutation(StateAction::SpawnPane(pane_id)))
            }
            ClientRequest::RenameWorkspace { workspace_id, name } => {
                validate_name(&name)?;
                self.workspace_mut(workspace_id)?.name = name;
                Ok(StateChange::mutation(StateAction::None))
            }
            ClientRequest::CloseWorkspace { workspace_id } => {
                let index = self
                    .snapshot
                    .workspaces
                    .iter()
                    .position(|workspace| workspace.id == workspace_id)
                    .ok_or(DaemonError::WorkspaceNotFound(workspace_id))?;
                let workspace = self.snapshot.workspaces.remove(index);
                let panes = workspace.layout.pane_ids();
                self.snapshot
                    .panes
                    .retain(|pane| pane.workspace_id != workspace_id);
                if self.snapshot.active_workspace_id == Some(workspace_id) {
                    self.snapshot.active_workspace_id = self
                        .snapshot
                        .workspaces
                        .get(index.min(self.snapshot.workspaces.len().saturating_sub(1)))
                        .map(|workspace| workspace.id);
                }
                Ok(StateChange::mutation(StateAction::ClosePanes(panes)))
            }
            ClientRequest::CreatePane { workspace_id } => {
                let workspace = self.workspace_mut(workspace_id)?;
                if workspace.active_pane_id.is_some() {
                    return Ok(StateChange::runtime(StateAction::None));
                }
                let pane_id = PaneId::new();
                workspace.layout = LayoutNode::pane(pane_id);
                workspace.active_pane_id = Some(pane_id);
                let root = workspace.root.clone();
                self.snapshot.panes.push(PaneSnapshot {
                    id: pane_id,
                    workspace_id,
                    status: SessionStatus::Starting,
                    cwd: root,
                    process_label: process_label.to_owned(),
                    cols: DEFAULT_COLUMNS,
                    rows: DEFAULT_ROWS,
                });
                Ok(StateChange::mutation(StateAction::SpawnPane(pane_id)))
            }
            ClientRequest::SplitPane {
                workspace_id,
                target,
                direction: _,
            } => {
                let target_pane = self.pane(target)?.clone();
                if target_pane.workspace_id != workspace_id {
                    return Err(DaemonError::PaneNotFound(target));
                }
                let pane_id = PaneId::new();
                let workspace = self.workspace_mut(workspace_id)?;
                let mut panes = workspace.layout.pane_ids();
                if panes.len() >= MAX_TERMINALS_PER_WORKSPACE {
                    return Err(DaemonError::TerminalLimitReached {
                        workspace_id,
                        limit: MAX_TERMINALS_PER_WORKSPACE,
                    });
                }
                panes.push(pane_id);
                workspace.layout = managed_terminal_layout(&panes);
                workspace.active_pane_id = Some(pane_id);
                self.snapshot.panes.push(PaneSnapshot {
                    id: pane_id,
                    workspace_id,
                    status: SessionStatus::Starting,
                    cwd: target_pane.cwd,
                    process_label: process_label.to_owned(),
                    cols: target_pane.cols,
                    rows: target_pane.rows,
                });
                Ok(StateChange::mutation(StateAction::SpawnPane(pane_id)))
            }
            ClientRequest::ClosePane { pane_id } => {
                let Some(workspace_id) = self
                    .snapshot
                    .panes
                    .iter()
                    .find(|pane| pane.id == pane_id)
                    .map(|pane| pane.workspace_id)
                else {
                    return Ok(StateChange::runtime(StateAction::None));
                };
                let workspace = self.workspace_mut(workspace_id)?;
                if workspace.layout.pane_ids().len() == 1 {
                    workspace.layout = LayoutNode::Empty;
                    workspace.active_pane_id = None;
                } else {
                    let mut panes = workspace.layout.pane_ids();
                    panes.retain(|candidate| *candidate != pane_id);
                    workspace.layout = managed_terminal_layout(&panes);
                    if workspace.active_pane_id == Some(pane_id) {
                        workspace.active_pane_id = panes.first().copied();
                    }
                }
                self.snapshot.panes.retain(|pane| pane.id != pane_id);
                Ok(StateChange::mutation(StateAction::ClosePane(pane_id)))
            }
            ClientRequest::FocusWorkspace { workspace_id } => {
                self.workspace(workspace_id)?;
                self.snapshot.active_workspace_id = Some(workspace_id);
                Ok(StateChange::mutation(StateAction::None))
            }
            ClientRequest::FocusPane { pane_id } => {
                let workspace_id = self.pane(pane_id)?.workspace_id;
                self.workspace_mut(workspace_id)?.active_pane_id = Some(pane_id);
                self.snapshot.active_workspace_id = Some(workspace_id);
                Ok(StateChange::mutation(StateAction::None))
            }
            ClientRequest::UpdateLayout {
                workspace_id,
                layout,
            } => {
                layout.validate()?;
                let workspace = self.workspace_mut(workspace_id)?;
                let mut expected = workspace.layout.pane_ids();
                let mut received = layout.pane_ids();
                expected.sort_unstable();
                received.sort_unstable();
                if expected != received {
                    return Err(DaemonError::Layout(LayoutError::PaneNotFound(
                        expected
                            .into_iter()
                            .find(|pane| !received.contains(pane))
                            .or_else(|| received.first().copied())
                            .or(workspace.active_pane_id)
                            .unwrap_or_else(PaneId::new),
                    )));
                }
                workspace.layout = layout;
                Ok(StateChange::mutation(StateAction::None))
            }
            ClientRequest::Input { pane_id, data } => {
                let Some(pane) = self.snapshot.panes.iter().find(|pane| pane.id == pane_id) else {
                    return Ok(StateChange::runtime(StateAction::None));
                };
                let action = if pane_has_live_session(&pane.status) {
                    StateAction::Input { pane_id, data }
                } else {
                    StateAction::None
                };
                Ok(StateChange::runtime(action))
            }
            ClientRequest::Resize {
                pane_id,
                cols,
                rows,
            } => {
                if cols == 0 || rows == 0 {
                    return Err(DaemonError::InvalidTerminalSize);
                }
                let Some(pane) = self
                    .snapshot
                    .panes
                    .iter_mut()
                    .find(|pane| pane.id == pane_id)
                else {
                    return Ok(StateChange::runtime(StateAction::None));
                };
                pane.cols = cols;
                pane.rows = rows;
                let action = if pane_has_live_session(&pane.status) {
                    StateAction::Resize {
                        pane_id,
                        cols,
                        rows,
                    }
                } else {
                    StateAction::None
                };
                Ok(StateChange::mutation(action))
            }
            ClientRequest::ReportCwd { pane_id, cwd } => {
                self.set_pane_cwd(pane_id, cwd)?;
                Ok(StateChange::mutation(StateAction::None))
            }
            ClientRequest::Shutdown => Ok(StateChange::runtime(StateAction::Shutdown)),
        }
    }

    fn workspace(&self, workspace_id: WorkspaceId) -> Result<&WorkspaceSnapshot, DaemonError> {
        self.snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or(DaemonError::WorkspaceNotFound(workspace_id))
    }

    fn workspace_mut(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<&mut WorkspaceSnapshot, DaemonError> {
        self.snapshot
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or(DaemonError::WorkspaceNotFound(workspace_id))
    }
}

fn workspace_snapshot(workspace: Workspace) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        id: workspace.id,
        name: workspace.name,
        root: workspace.root_directory,
        layout: workspace.layout,
        active_pane_id: workspace.active_pane_id,
    }
}

fn validate_name(name: &str) -> Result<(), DaemonError> {
    if name.trim().is_empty() {
        Err(DaemonError::EmptyWorkspaceName)
    } else {
        Ok(())
    }
}

fn pane_has_live_session(status: &SessionStatus) -> bool {
    matches!(status, SessionStatus::Starting | SessionStatus::Running)
}

/// Runs the persistent per-user daemon until a client requests shutdown.
///
/// # Errors
///
/// Returns an error when storage, named-pipe transport, or terminal initialization fails.
pub fn run_daemon() -> Result<(), DaemonError> {
    #[cfg(windows)]
    {
        windows::run()
    }
    #[cfg(not(windows))]
    {
        Err(DaemonError::UnsupportedPlatform)
    }
}

fn load_state(repository: &Repository) -> Result<DaemonState, DaemonError> {
    Ok(DaemonState::new(
        repository.load_snapshot()?.unwrap_or_default(),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ade_core::SplitDirection;

    use super::*;

    #[test]
    fn workspace_and_pane_mutations_preserve_authoritative_snapshot() {
        let mut state = DaemonState::new(AppSnapshot::default());
        let change = state
            .handle(
                ClientRequest::CreateWorkspace {
                    name: "one".to_owned(),
                    root: PathBuf::from(r"C:\work"),
                },
                "pwsh.exe",
            )
            .unwrap();
        assert!(change.persist);
        let workspace = state.snapshot().workspaces[0].clone();
        let first = workspace.active_pane_id.unwrap();

        state
            .handle(
                ClientRequest::SplitPane {
                    workspace_id: workspace.id,
                    target: first,
                    direction: SplitDirection::Right,
                },
                "pwsh.exe",
            )
            .unwrap();
        assert_eq!(state.snapshot().panes.len(), 2);
        let second = state.snapshot().workspaces[0].active_pane_id.unwrap();

        state
            .handle(ClientRequest::ClosePane { pane_id: second }, "pwsh.exe")
            .unwrap();
        assert_eq!(state.snapshot().panes.len(), 1);
        assert_eq!(state.snapshot().workspaces[0].active_pane_id, Some(first));
    }

    #[test]
    fn persisted_running_panes_restart_as_starting() {
        let mut snapshot = AppSnapshot::default();
        let workspace = Workspace::new("one", PathBuf::from(r"C:\work"));
        let pane_id = workspace.active_pane_id.unwrap();
        snapshot.panes.push(PaneSnapshot {
            id: pane_id,
            workspace_id: workspace.id,
            status: SessionStatus::Running,
            cwd: workspace.root_directory.clone(),
            process_label: "pwsh.exe".to_owned(),
            cols: DEFAULT_COLUMNS,
            rows: DEFAULT_ROWS,
        });
        snapshot.workspaces.push(workspace_snapshot(workspace));
        let state = DaemonState::new(snapshot);
        assert_eq!(state.snapshot().panes[0].status, SessionStatus::Starting);
    }

    #[test]
    fn layout_ratio_name_and_cwd_updates_are_persisted_in_state() {
        let mut state = DaemonState::new(AppSnapshot::default());
        state
            .handle(
                ClientRequest::CreateWorkspace {
                    name: "one".to_owned(),
                    root: PathBuf::from(r"C:\work"),
                },
                "pwsh.exe",
            )
            .unwrap();
        let workspace = state.snapshot().workspaces[0].clone();
        state
            .handle(
                ClientRequest::SplitPane {
                    workspace_id: workspace.id,
                    target: workspace.active_pane_id.unwrap(),
                    direction: SplitDirection::Right,
                },
                "pwsh.exe",
            )
            .unwrap();
        let mut layout = state.snapshot().workspaces[0].layout.clone();
        layout.set_ratio(&[], 0.7).unwrap();
        state
            .handle(
                ClientRequest::UpdateLayout {
                    workspace_id: workspace.id,
                    layout,
                },
                "pwsh.exe",
            )
            .unwrap();
        state
            .handle(
                ClientRequest::RenameWorkspace {
                    workspace_id: workspace.id,
                    name: "renamed".to_owned(),
                },
                "pwsh.exe",
            )
            .unwrap();
        state
            .handle(
                ClientRequest::ReportCwd {
                    pane_id: workspace.active_pane_id.unwrap(),
                    cwd: PathBuf::from(r"C:\work\nested"),
                },
                "pwsh.exe",
            )
            .unwrap();

        assert_eq!(state.snapshot().workspaces[0].name, "renamed");
        assert_eq!(
            state.snapshot().panes[0].cwd,
            PathBuf::from(r"C:\work\nested")
        );
        assert!(matches!(
            state.snapshot().workspaces[0].layout,
            ade_core::LayoutNode::Split { ratio, .. } if (ratio - 0.7).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn closing_workspace_removes_its_panes_and_selects_a_remaining_workspace() {
        let mut state = DaemonState::new(AppSnapshot::default());
        for name in ["one", "two"] {
            state
                .handle(
                    ClientRequest::CreateWorkspace {
                        name: name.to_owned(),
                        root: PathBuf::from(format!(r"C:\{name}")),
                    },
                    "pwsh.exe",
                )
                .unwrap();
        }
        let closed = state.snapshot().workspaces[1].clone();
        let change = state
            .handle(
                ClientRequest::CloseWorkspace {
                    workspace_id: closed.id,
                },
                "pwsh.exe",
            )
            .unwrap();

        assert_eq!(
            change.action,
            StateAction::ClosePanes(closed.layout.pane_ids())
        );
        assert_eq!(state.snapshot().workspaces.len(), 1);
        assert_eq!(state.snapshot().panes.len(), 1);
        assert_eq!(
            state.snapshot().active_workspace_id,
            Some(state.snapshot().workspaces[0].id)
        );
    }

    #[test]
    fn final_pane_can_be_closed_and_recreated_without_removing_workspace() {
        let mut state = DaemonState::new(AppSnapshot::default());
        state
            .handle(
                ClientRequest::CreateWorkspace {
                    name: "one".to_owned(),
                    root: PathBuf::from(r"C:\work"),
                },
                "pwsh.exe",
            )
            .unwrap();
        let workspace_id = state.snapshot().workspaces[0].id;
        let pane_id = state.snapshot().workspaces[0].active_pane_id.unwrap();

        let close = state
            .handle(ClientRequest::ClosePane { pane_id }, "pwsh.exe")
            .unwrap();
        assert_eq!(close.action, StateAction::ClosePane(pane_id));
        assert!(close.persist);
        assert_eq!(state.snapshot().workspaces.len(), 1);
        assert!(state.snapshot().panes.is_empty());
        assert!(matches!(
            state.snapshot().workspaces[0].layout,
            LayoutNode::Empty
        ));
        assert_eq!(state.snapshot().workspaces[0].active_pane_id, None);

        let repeated_close = state
            .handle(ClientRequest::ClosePane { pane_id }, "pwsh.exe")
            .unwrap();
        assert_eq!(repeated_close.action, StateAction::None);

        let create = state
            .handle(ClientRequest::CreatePane { workspace_id }, "pwsh.exe")
            .unwrap();
        assert!(matches!(create.action, StateAction::SpawnPane(_)));
        assert_eq!(state.snapshot().panes.len(), 1);
        assert!(state.snapshot().workspaces[0].active_pane_id.is_some());
    }

    #[test]
    fn workspace_reflows_panes_and_rejects_a_seventh_terminal() {
        let mut state = DaemonState::new(AppSnapshot::default());
        state
            .handle(
                ClientRequest::CreateWorkspace {
                    name: "one".to_owned(),
                    root: PathBuf::from(r"C:\work"),
                },
                "pwsh.exe",
            )
            .unwrap();
        let workspace_id = state.snapshot().workspaces[0].id;

        for _ in 1..MAX_TERMINALS_PER_WORKSPACE {
            let target = state.snapshot().workspaces[0].active_pane_id.unwrap();
            state
                .handle(
                    ClientRequest::SplitPane {
                        workspace_id,
                        target,
                        direction: SplitDirection::Down,
                    },
                    "pwsh.exe",
                )
                .unwrap();
        }

        let workspace = &state.snapshot().workspaces[0];
        assert_eq!(
            workspace.layout.pane_ids().len(),
            MAX_TERMINALS_PER_WORKSPACE
        );
        assert!(matches!(
            workspace.layout,
            LayoutNode::Split {
                axis: ade_core::SplitAxis::Rows,
                ..
            }
        ));

        let target = workspace.active_pane_id.unwrap();
        let error = state
            .handle(
                ClientRequest::SplitPane {
                    workspace_id,
                    target,
                    direction: SplitDirection::Right,
                },
                "pwsh.exe",
            )
            .unwrap_err();
        assert!(matches!(error, DaemonError::TerminalLimitReached { .. }));
        assert_eq!(state.snapshot().panes.len(), MAX_TERMINALS_PER_WORKSPACE);

        state
            .handle(ClientRequest::ClosePane { pane_id: target }, "pwsh.exe")
            .unwrap();
        assert_eq!(state.snapshot().workspaces[0].layout.pane_ids().len(), 5);
    }

    #[test]
    fn exited_panes_do_not_dispatch_to_closed_runtime_channels() {
        let mut state = DaemonState::new(AppSnapshot::default());
        state
            .handle(
                ClientRequest::CreateWorkspace {
                    name: "one".to_owned(),
                    root: PathBuf::from(r"C:\work"),
                },
                "pwsh.exe",
            )
            .unwrap();
        let pane_id = state.snapshot().panes[0].id;
        state
            .set_pane_status(pane_id, SessionStatus::Exited { exit_code: 2 })
            .unwrap();

        let input = state
            .handle(
                ClientRequest::Input {
                    pane_id,
                    data: b"ignored".to_vec(),
                },
                "pwsh.exe",
            )
            .unwrap();
        assert_eq!(input.action, StateAction::None);

        let resize = state
            .handle(
                ClientRequest::Resize {
                    pane_id,
                    cols: 120,
                    rows: 40,
                },
                "pwsh.exe",
            )
            .unwrap();
        assert_eq!(resize.action, StateAction::None);
        assert!(resize.persist);
        assert_eq!(state.snapshot().panes[0].cols, 120);
        assert_eq!(state.snapshot().panes[0].rows, 40);

        let close = state
            .handle(ClientRequest::ClosePane { pane_id }, "pwsh.exe")
            .unwrap();
        assert_eq!(close.action, StateAction::ClosePane(pane_id));
        assert!(state.snapshot().panes.is_empty());
        assert!(matches!(
            state.snapshot().workspaces[0].layout,
            LayoutNode::Empty
        ));

        let stale_input = state
            .handle(
                ClientRequest::Input {
                    pane_id,
                    data: b"ignored".to_vec(),
                },
                "pwsh.exe",
            )
            .unwrap();
        assert_eq!(stale_input.action, StateAction::None);

        let stale_resize = state
            .handle(
                ClientRequest::Resize {
                    pane_id,
                    cols: 120,
                    rows: 40,
                },
                "pwsh.exe",
            )
            .unwrap();
        assert_eq!(stale_resize.action, StateAction::None);
        assert!(!stale_resize.persist);
    }
}
