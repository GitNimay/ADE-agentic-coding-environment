use std::path::{Path, PathBuf};

use ade_protocol::AppSnapshot;
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

const SCHEMA_VERSION: i64 = 1;
const SNAPSHOT_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("LOCALAPPDATA is unavailable")]
    LocalAppDataUnavailable,
    #[error("failed to create storage directory: {0}")]
    CreateDirectory(#[source] std::io::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("snapshot JSON operation failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported snapshot version {0}")]
    UnsupportedSnapshotVersion(i64),
}

pub struct Repository {
    connection: Connection,
}

impl Repository {
    /// Opens the default per-user database and initializes its schema.
    ///
    /// # Errors
    ///
    /// Returns an error if `LOCALAPPDATA` is unavailable, the directory cannot be created, or
    /// database initialization fails.
    pub fn open_default() -> Result<Self, StorageError> {
        let path = default_database_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StorageError::CreateDirectory)?;
        }
        Self::open(path)
    }

    /// Opens an explicit database path, useful for tests and embedding.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or migrated.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        let repository = Self { connection };
        repository.migrate()?;
        Ok(repository)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY);\n\
             CREATE TABLE IF NOT EXISTS app_snapshot (\n\
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\n\
                 version INTEGER NOT NULL,\n\
                 json TEXT NOT NULL\n\
             );",
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (?1)",
            [SCHEMA_VERSION],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Atomically replaces the persisted application snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if JSON serialization or the database transaction fails.
    pub fn save_snapshot(&self, snapshot: &AppSnapshot) -> Result<(), StorageError> {
        let json = serde_json::to_string(snapshot)?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO app_snapshot(singleton, version, json) VALUES (1, ?1, ?2)\n\
             ON CONFLICT(singleton) DO UPDATE SET version = excluded.version, json = excluded.json",
            params![SNAPSHOT_VERSION, json],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Loads the persisted snapshot, if one exists.
    ///
    /// # Errors
    ///
    /// Returns an error for database or JSON failures and unsupported snapshot versions.
    pub fn load_snapshot(&self) -> Result<Option<AppSnapshot>, StorageError> {
        let row: Option<(i64, String)> = self
            .connection
            .query_row(
                "SELECT version, json FROM app_snapshot WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((version, json)) = row else {
            return Ok(None);
        };
        if version != SNAPSHOT_VERSION {
            return Err(StorageError::UnsupportedSnapshotVersion(version));
        }
        Ok(Some(serde_json::from_str(&json)?))
    }

    /// Returns the latest initialized schema version.
    ///
    /// # Errors
    ///
    /// Returns an error if the migration table cannot be queried.
    pub fn schema_version(&self) -> Result<i64, StorageError> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }
}

/// Returns `LOCALAPPDATA/ADE/ade.db` without creating it.
///
/// # Errors
///
/// Returns [`StorageError::LocalAppDataUnavailable`] when `LOCALAPPDATA` is unset.
pub fn default_database_path() -> Result<PathBuf, StorageError> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or(StorageError::LocalAppDataUnavailable)?;
    Ok(PathBuf::from(base).join("ADE").join("ade.db"))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use ade_core::{SessionStatus, Workspace};
    use ade_protocol::{PaneSnapshot, WorkspaceSnapshot};

    use super::*;

    fn temporary_database() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("ade-storage-{}-{unique}.db", std::process::id()))
    }

    #[test]
    fn initializes_schema() {
        let path = temporary_database();
        let repository = Repository::open(&path).unwrap();
        assert_eq!(repository.schema_version().unwrap(), SCHEMA_VERSION);
        drop(repository);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn snapshot_round_trips() {
        let path = temporary_database();
        let repository = Repository::open(&path).unwrap();
        let workspace = Workspace::new("test", PathBuf::from(r"C:\work"));
        let snapshot = AppSnapshot {
            active_workspace_id: Some(workspace.id),
            workspaces: vec![WorkspaceSnapshot {
                id: workspace.id,
                name: workspace.name,
                root: workspace.root_directory.clone(),
                layout: workspace.layout,
                active_pane_id: workspace.active_pane_id,
            }],
            panes: vec![PaneSnapshot {
                id: workspace.active_pane_id,
                workspace_id: workspace.id,
                status: SessionStatus::Running,
                cwd: workspace.root_directory,
                process_label: "pwsh.exe".to_owned(),
                cols: 80,
                rows: 24,
            }],
        };
        repository.save_snapshot(&snapshot).unwrap();
        assert_eq!(repository.load_snapshot().unwrap(), Some(snapshot));
        drop(repository);
        std::fs::remove_file(path).unwrap();
    }
}
