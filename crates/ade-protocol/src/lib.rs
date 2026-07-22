use std::io::{self, Read, Write};
use std::path::PathBuf;

use ade_core::{LayoutNode, PaneId, SessionStatus, SplitDirection, WorkspaceId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Versioned<T> {
    pub protocol_version: u32,
    pub message: T,
}

impl<T> Versioned<T> {
    #[must_use]
    pub const fn new(message: T) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientRequest {
    Attach,
    GetSnapshot,
    CreateWorkspace {
        name: String,
        root: PathBuf,
    },
    RenameWorkspace {
        workspace_id: WorkspaceId,
        name: String,
    },
    CloseWorkspace {
        workspace_id: WorkspaceId,
    },
    SplitPane {
        workspace_id: WorkspaceId,
        target: PaneId,
        direction: SplitDirection,
    },
    ClosePane {
        pane_id: PaneId,
    },
    FocusWorkspace {
        workspace_id: WorkspaceId,
    },
    FocusPane {
        pane_id: PaneId,
    },
    UpdateLayout {
        workspace_id: WorkspaceId,
        layout: LayoutNode,
    },
    Input {
        pane_id: PaneId,
        data: Vec<u8>,
    },
    Resize {
        pane_id: PaneId,
        cols: u16,
        rows: u16,
    },
    ReportCwd {
        pane_id: PaneId,
        cwd: PathBuf,
    },
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Attached {
        snapshot: AppSnapshot,
    },
    TerminalOutput {
        pane_id: PaneId,
        data: Vec<u8>,
    },
    WorkspaceUpdated {
        snapshot: AppSnapshot,
    },
    PaneStatus {
        pane_id: PaneId,
        status: SessionStatus,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub active_workspace_id: Option<WorkspaceId>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub panes: Vec<PaneSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub id: WorkspaceId,
    pub name: String,
    pub root: PathBuf,
    pub layout: LayoutNode,
    pub active_pane_id: PaneId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneSnapshot {
    pub id: PaneId,
    pub workspace_id: WorkspaceId,
    pub status: SessionStatus,
    pub cwd: PathBuf,
    pub process_label: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("frame length {0} exceeds the {MAX_FRAME_SIZE} byte limit")]
    TooLarge(usize),
    #[error("frame JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

/// Writes one JSON value prefixed by its little-endian `u32` byte length.
///
/// # Errors
///
/// Returns an error when serialization or I/O fails, or when the encoded value exceeds
/// [`MAX_FRAME_SIZE`].
pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<(), FrameError> {
    let json = serde_json::to_vec(value)?;
    if json.len() > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge(json.len()));
    }
    let length = u32::try_from(json.len()).map_err(|_| FrameError::TooLarge(json.len()))?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&json)?;
    writer.flush()?;
    Ok(())
}

/// Reads one length-delimited JSON value, rejecting oversized frames before allocation.
///
/// # Errors
///
/// Returns an error when I/O or deserialization fails, or when the declared frame length exceeds
/// [`MAX_FRAME_SIZE`].
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, FrameError> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge(length));
    }
    let mut json = vec![0_u8; length];
    reader.read_exact(&mut json)?;
    Ok(serde_json::from_slice(&json)?)
}

#[must_use]
pub fn pipe_name_for_user(user: &str) -> String {
    let safe_user: String = user
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    format!(r"\\.\pipe\ade-{safe_user}")
}

#[must_use]
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_owned());
    pipe_name_for_user(&user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_message_round_trips() {
        let request = Versioned::new(ClientRequest::Resize {
            pane_id: PaneId::new(),
            cols: 120,
            rows: 40,
        });
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request).unwrap();
        let decoded: Versioned<ClientRequest> = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn oversized_length_is_rejected_before_reading_body() {
        let bytes = u32::try_from(MAX_FRAME_SIZE + 1).unwrap().to_le_bytes();
        let error = read_frame::<_, serde_json::Value>(&mut bytes.as_slice()).unwrap_err();
        assert!(matches!(error, FrameError::TooLarge(_)));
    }

    #[test]
    fn pipe_user_component_is_sanitized() {
        assert_eq!(
            pipe_name_for_user(r"domain\a user"),
            r"\\.\pipe\ade-domain_a_user"
        );
    }
}
