//! Git integration utilities.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

/// Lightweight wrapper around [`gix::Repository`] discovery for metadata extraction.
#[derive(Default)]
pub struct GitClient {
    repo: Option<gix::Repository>,
}

impl GitClient {
    /// Attempt to locate a git repository starting from `path`.
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let repo = gix::discover(path).ok();
        Ok(Self { repo })
    }

    /// Retrieve repository metadata if discovery succeeded.
    pub fn metadata(&self) -> Option<GitMetadata> {
        let repo = self.repo.as_ref()?;
        let branch = repo.head_name().ok().flatten().map(|name| name.to_string());

        let commit = repo.head_id().ok().map(|id| id.detach().to_string());

        let root = repo
            .work_dir()
            .map(Path::to_path_buf)
            .or_else(|| repo.path().parent().map(Path::to_path_buf))?;

        Some(GitMetadata {
            branch,
            commit,
            root,
        })
    }
}

/// Basic information about the repository used in export templates.
#[derive(Debug, Clone, Serialize)]
pub struct GitMetadata {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub root: PathBuf,
}

/// Convenience helper to retrieve metadata directly from a path.
pub fn metadata_for_path(path: &Path) -> Option<GitMetadata> {
    GitClient::discover(path)
        .ok()
        .and_then(|client| client.metadata())
}
