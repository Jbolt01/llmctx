//! Session persistence utilities.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::domain::model::SelectionItem;

const SESSION_DIR: &str = ".llmctx";
const SESSION_FILE: &str = "session.json";

/// Snapshot of interactive UI state persisted between sessions.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Previously selected items restored into the selection manager.
    pub selections: Vec<SelectionRecord>,
    /// Path of the file that was focused when the session closed.
    pub focused_path: Option<String>,
    /// Active file tree filter.
    pub filter: Option<String>,
    /// User configured model override if any.
    pub model: Option<String>,
}

/// Serializable representation of a [`SelectionItem`].
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SelectionRecord {
    pub path: String,
    pub range: Option<(usize, usize)>,
    pub note: Option<String>,
}

impl From<&SelectionItem> for SelectionRecord {
    fn from(value: &SelectionItem) -> Self {
        Self {
            path: value.path.display().to_string(),
            range: value.range,
            note: value.note.clone(),
        }
    }
}

impl SelectionRecord {
    /// Convert the record back into a domain [`SelectionItem`].
    pub fn into_selection_item(self) -> SelectionItem {
        SelectionItem {
            path: PathBuf::from(self.path),
            range: self.range,
            note: self.note,
        }
    }
}

/// Persists UI state to a session file under `.llmctx/`.
#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
    path: PathBuf,
}

impl SessionStore {
    /// Create a new store rooted at the provided directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let path = root.join(SESSION_DIR).join(SESSION_FILE);
        Self { root, path }
    }

    /// Location of the persisted session file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the most recently persisted session snapshot.
    pub fn load(&self) -> Result<Option<SessionSnapshot>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let data = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read session file at {}", self.path.display()))?;
        let snapshot = serde_json::from_str(&data)
            .with_context(|| format!("invalid session data in {}", self.path.display()))?;
        Ok(Some(snapshot))
    }

    /// Persist the provided snapshot to disk, creating parent directories as needed.
    pub fn save(&self, snapshot: &SessionSnapshot) -> Result<()> {
        let dir = self.path.parent().unwrap_or(&self.root);
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create session directory {}", dir.display()))?;

        let data = serde_json::to_string_pretty(snapshot)
            .context("failed to serialize session snapshot")?;
        fs::write(&self.path, data)
            .with_context(|| format!("failed to write session file to {}", self.path.display()))?;
        Ok(())
    }
}
