//! Managing selections and context bundles.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::app::tokens::{BundleTokenSummary, TokenEstimator};
use crate::domain::model::{ContextBundle, SelectionItem};

/// Tracks the active selection set and produces export-ready bundles.
#[derive(Debug, Default, Clone)]
pub struct SelectionManager {
    items: Vec<SelectionItem>,
    model: Option<String>,
}

impl SelectionManager {
    /// Create an empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of tracked selections.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether any selections exist.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Replace the associated model for bundle generation.
    pub fn set_model<S: Into<String>>(&mut self, model: S) {
        self.model = Some(model.into());
    }

    /// Clear the configured model, falling back to estimator defaults.
    pub fn clear_model(&mut self) {
        self.model = None;
    }

    /// Access the active selections.
    pub fn items(&self) -> &[SelectionItem] {
        &self.items
    }

    /// Append or merge a selection.
    ///
    /// Entire file selections replace any previous ranges for the same path. Ranged selections are
    /// merged when they overlap or touch to keep the bundle compact while preserving insertion
    /// order.
    pub fn add_selection(
        &mut self,
        path: impl Into<PathBuf>,
        range: Option<(usize, usize)>,
        note: Option<String>,
    ) -> SelectionItem {
        let item = SelectionItem {
            path: path.into(),
            range: range.map(normalize_range),
            note: note.and_then(clean_note),
        };

        match item.range {
            None => self.insert_entire_file(item),
            Some(range) => self.insert_range(item, range),
        }
    }

    /// Remove a specific selection. When `range` is `None`, all selections for the file are
    /// cleared.
    pub fn remove_selection(&mut self, path: &Path, range: Option<(usize, usize)>) -> bool {
        let original_len = self.items.len();
        match range.map(normalize_range) {
            None => self.items.retain(|item| item.path != path),
            Some(target) => self.items.retain(|item| {
                if item.path != path {
                    return true;
                }
                item.range != Some(target)
            }),
        }
        self.items.len() != original_len
    }

    /// Update the note associated with a selection. Returns `true` when a matching selection is
    /// found.
    pub fn set_note(
        &mut self,
        path: &Path,
        range: Option<(usize, usize)>,
        note: Option<String>,
    ) -> bool {
        let normalized = range.map(normalize_range);
        let note = note.and_then(clean_note);

        if let Some(item) = self.items.iter_mut().find(|item| {
            item.path == path
                && match (item.range, normalized) {
                    (None, None) => true,
                    (Some(existing), Some(target)) => existing == target,
                    _ => false,
                }
        }) {
            item.note = note;
            return true;
        }

        false
    }

    /// Remove all selections.
    pub fn clear(&mut self) {
        self.items.clear();
        self.model = None;
    }

    /// Build a [`ContextBundle`] from the tracked selections, using an optional override model.
    pub fn to_bundle_with_model(&self, override_model: Option<String>) -> ContextBundle {
        ContextBundle {
            items: self.items.clone(),
            model: override_model.or_else(|| self.model.clone()),
        }
    }

    /// Build a [`ContextBundle`] using the internally configured model (if any).
    pub fn to_bundle(&self) -> ContextBundle {
        self.to_bundle_with_model(None)
    }

    /// Estimate tokens for the active bundle using the provided estimator.
    pub fn summarize_tokens(
        &self,
        estimator: &TokenEstimator,
    ) -> Result<Option<BundleTokenSummary>> {
        if self.items.is_empty() {
            return Ok(None);
        }
        let bundle = self.to_bundle();
        estimator.estimate_bundle(&bundle).map(Some)
    }

    fn insert_entire_file(&mut self, mut item: SelectionItem) -> SelectionItem {
        let mut insert_at = None;
        let mut preserved_note = item.note.clone();

        let mut index = 0;
        while index < self.items.len() {
            if self.items[index].path == item.path {
                if insert_at.is_none() {
                    insert_at = Some(index);
                }
                if preserved_note.is_none() {
                    preserved_note = self.items[index].note.clone();
                }
                self.items.remove(index);
            } else {
                index += 1;
            }
        }

        if item.note.is_none() {
            item.note = preserved_note;
        }

        let position = insert_at.unwrap_or(self.items.len());
        self.items.insert(position, item.clone());
        item
    }

    fn insert_range(
        &mut self,
        mut item: SelectionItem,
        mut range: (usize, usize),
    ) -> SelectionItem {
        let mut merged_indices: Vec<usize> = Vec::new();
        let mut inherited_note = item.note.clone();

        for (idx, existing) in self.items.iter().enumerate() {
            if existing.path != item.path {
                continue;
            }

            match existing.range {
                None => {
                    let mut updated = existing.clone();
                    if item.note.is_some() {
                        updated.note = item.note.clone();
                        self.items[idx] = updated.clone();
                    }
                    return updated;
                }
                Some(existing_range) => {
                    if ranges_mergeable(range, existing_range) {
                        merged_indices.push(idx);
                        range = (range.0.min(existing_range.0), range.1.max(existing_range.1));
                        if inherited_note.is_none() {
                            inherited_note = existing.note.clone();
                        }
                    }
                }
            }
        }

        if !merged_indices.is_empty() {
            for idx in merged_indices.iter().rev() {
                self.items.remove(*idx);
            }
        }

        item.range = Some(range);
        if item.note.is_none() {
            item.note = inherited_note;
        }

        let position = self
            .items
            .iter()
            .enumerate()
            .find(|(_, existing)| {
                existing.path == item.path
                    && existing
                        .range
                        .map(|existing_range| existing_range.0 > range.1)
                        .unwrap_or(false)
            })
            .map(|(idx, _)| idx)
            .unwrap_or_else(|| {
                self.items
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, existing)| existing.path == item.path)
                    .map(|(idx, _)| idx + 1)
                    .unwrap_or(self.items.len())
            });

        self.items.insert(position, item.clone());
        item
    }
}

fn normalize_range(range: (usize, usize)) -> (usize, usize) {
    let start = range.0.min(range.1).max(1);
    let end = range.0.max(range.1).max(1);
    (start, end)
}

fn ranges_mergeable(a: (usize, usize), b: (usize, usize)) -> bool {
    let (a_start, a_end) = a;
    let (b_start, b_end) = b;
    a_start <= b_end.saturating_add(1) && b_start <= a_end.saturating_add(1)
}

fn clean_note(note: String) -> Option<String> {
    let trimmed = note.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    use tempfile::NamedTempFile;

    #[test]
    fn adds_entire_file_and_replaces_existing_ranges() {
        let mut manager = SelectionManager::new();
        let path: PathBuf = "src/lib.rs".into();

        manager.add_selection(path.clone(), Some((5, 10)), None);
        manager.add_selection(path.clone(), Some((15, 20)), Some("note".into()));
        assert_eq!(manager.len(), 2);

        manager.add_selection(path.clone(), None, None);
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.items()[0].range, None);
        assert_eq!(manager.items()[0].note, Some("note".into()));
    }

    #[test]
    fn merges_overlapping_ranges_and_preserves_order() {
        let mut manager = SelectionManager::new();
        let path: PathBuf = "src/lib.rs".into();

        let first = manager.add_selection(path.clone(), Some((5, 10)), None);
        assert_eq!(first.range, Some((5, 10)));

        let merged = manager.add_selection(path.clone(), Some((9, 15)), None);
        assert_eq!(merged.range, Some((5, 15)));
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn set_note_updates_existing_selection() {
        let mut manager = SelectionManager::new();
        let path: PathBuf = "src/lib.rs".into();
        manager.add_selection(path.clone(), Some((1, 3)), None);

        assert!(manager.set_note(&path, Some((1, 3)), Some("important".into())));
        assert_eq!(manager.items()[0].note.as_deref(), Some("important"));
    }

    #[test]
    fn summarize_tokens_returns_none_when_empty() {
        let manager = SelectionManager::new();
        let estimator = TokenEstimator::new(Default::default());
        assert!(manager.summarize_tokens(&estimator).unwrap().is_none());
    }

    #[test]
    fn summarize_tokens_reads_ranges() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "line one").unwrap();
        writeln!(file, "line two").unwrap();
        writeln!(file, "line three").unwrap();

        let mut manager = SelectionManager::new();
        manager.add_selection(file.path(), Some((2, 3)), None);

        let estimator = TokenEstimator::new(Default::default());
        let summary = manager.summarize_tokens(&estimator).unwrap().unwrap();
        assert_eq!(summary.items.len(), 1);
        assert!(summary.total_tokens > 0);
    }
}
