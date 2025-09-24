//! File tree component and state management.

use std::collections::{HashMap, HashSet};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::scan::{FileMetadata, ScanResult, SkipReason};

/// Maintains the navigable state of the file tree.
#[derive(Debug, Default, Clone)]
pub struct FileTreeState {
    entries: Vec<TreeEntry>,
    visible: Vec<usize>,
    selected: usize,
    expanded: HashSet<String>,
    filter: String,
    filter_active: bool,
    root_label: String,
}

impl FileTreeState {
    /// Construct state from a scan result.
    pub fn from_scan(result: &ScanResult) -> Self {
        let mut state = Self {
            entries: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            expanded: HashSet::new(),
            filter: String::new(),
            filter_active: false,
            root_label: result
                .root
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| result.root.display().to_string()),
        };
        state.rebuild_entries(result);
        state
    }

    fn rebuild_entries(&mut self, result: &ScanResult) {
        let mut entries = Vec::with_capacity(result.files.len());
        let mut index_map: HashMap<String, usize> = HashMap::new();

        for meta in &result.files {
            let key = meta.display_path.clone();
            let depth = meta.display_path.matches('/').count();
            let name = display_name(&meta.display_path);
            let parent_key = parent_key(&meta.display_path);
            let parent = parent_key.as_ref().and_then(|p| index_map.get(p).copied());

            let entry = TreeEntry {
                metadata: meta.clone(),
                name,
                depth,
                parent,
                has_children: false,
            };
            let idx = entries.len();
            entries.push(entry);
            index_map.insert(key.clone(), idx);

            if let Some(parent_idx) = parent
                && let Some(parent_entry) = entries.get_mut(parent_idx)
            {
                parent_entry.has_children = true;
            }
        }

        // Expand first level directories by default for better discoverability.
        self.expanded.clear();
        for entry in &entries {
            if entry.depth == 0 && entry.metadata.is_dir {
                self.expanded.insert(entry.metadata.display_path.clone());
            }
        }

        self.entries = entries;
        self.visible.clear();
        self.selected = 0;
        self.refresh_visible();
    }

    /// Provide read-only access to the currently selected metadata.
    pub fn selected_metadata(&self) -> Option<&FileMetadata> {
        self.visible
            .get(self.selected)
            .and_then(|idx| self.entries.get(*idx))
            .map(|entry| &entry.metadata)
    }

    /// Highlight the provided path if it exists in the tree.
    pub fn focus_path(&mut self, display_path: &str) {
        if let Some((index, _)) = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.metadata.display_path == display_path)
        {
            self.expand_to(index);
        }
    }

    fn expand_to(&mut self, index: usize) {
        let mut cursor = Some(index);
        while let Some(idx) = cursor {
            if let Some(entry) = self.entries.get(idx) {
                if entry.metadata.is_dir {
                    self.expanded.insert(entry.metadata.display_path.clone());
                }
                cursor = entry.parent;
            } else {
                break;
            }
        }

        self.refresh_visible();
        if let Some(pos) = self.visible.iter().position(|idx| *idx == index) {
            self.selected = pos;
        }
    }

    /// Advance selection to the next item if possible.
    pub fn select_next(&mut self) {
        if self.selected + 1 < self.visible.len() {
            self.selected += 1;
        }
    }

    /// Move selection to the previous item if possible.
    pub fn select_previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Expand the currently selected directory or activate its first child.
    pub fn expand_or_open(&mut self) {
        if let Some(index) = self.selected_entry_index()
            && self.entries[index].metadata.is_dir
        {
            let key = self.entries[index].metadata.display_path.clone();
            if !self.expanded.insert(key.clone()) {
                if let Some(first_child) = self.visible.iter().position(|idx| {
                    self.entries.get(*idx).and_then(|item| item.parent) == Some(index)
                }) {
                    self.selected = first_child;
                }
            } else {
                self.refresh_visible();
            }
        }
    }

    /// Collapse the selected directory or move focus to its parent.
    pub fn collapse_or_parent(&mut self) {
        if let Some(index) = self.selected_entry_index() {
            let key = self.entries[index].metadata.display_path.clone();
            let parent = self.entries[index].parent;
            let is_dir = self.entries[index].metadata.is_dir;
            if is_dir && self.expanded.remove(&key) {
                self.refresh_visible();
            } else if let Some(parent_idx) = parent
                && let Some(pos) = self.visible.iter().position(|idx| *idx == parent_idx)
            {
                self.selected = pos;
            }
        }
    }

    /// Toggle the expansion state of the selected directory.
    pub fn toggle_expansion(&mut self) {
        if let Some(index) = self.selected_entry_index()
            && self.entries[index].metadata.is_dir
        {
            let key = self.entries[index].metadata.display_path.clone();
            if !self.expanded.remove(&key) {
                self.expanded.insert(key);
            }
            self.refresh_visible();
        }
    }

    /// Activate incremental filter editing.
    pub fn begin_filter(&mut self) {
        self.filter_active = true;
    }

    /// Deactivate the filter editing mode.
    pub fn end_filter(&mut self) {
        self.filter_active = false;
    }

    /// Whether filter mode is currently active.
    pub fn is_filter_active(&self) -> bool {
        self.filter_active
    }

    /// Append a character to the filter string and refresh visibility.
    pub fn push_filter_char(&mut self, ch: char) {
        self.filter.push(ch);
        self.refresh_visible();
    }

    /// Remove the most recent filter character.
    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.refresh_visible();
    }

    /// Clear the active filter.
    pub fn clear_filter(&mut self) {
        if !self.filter.is_empty() {
            self.filter.clear();
            self.refresh_visible();
        }
    }

    /// Replace the filter contents.
    pub fn set_filter<S: Into<String>>(&mut self, pattern: S) {
        self.filter = pattern.into();
        self.refresh_visible();
    }

    /// Retrieve the active filter string.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    fn refresh_visible(&mut self) {
        self.visible.clear();
        if self.entries.is_empty() {
            return;
        }

        let lower_filter = self.filter.to_ascii_lowercase();
        let mut matches = vec![lower_filter.is_empty(); self.entries.len()];

        if !lower_filter.is_empty() {
            for (idx, entry) in self.entries.iter().enumerate() {
                if entry
                    .metadata
                    .display_path
                    .to_ascii_lowercase()
                    .contains(&lower_filter)
                {
                    matches[idx] = true;
                    let mut parent = entry.parent;
                    while let Some(p) = parent {
                        matches[p] = true;
                        parent = self.entries[p].parent;
                    }
                }
            }
        }

        for (idx, entry) in self.entries.iter().enumerate() {
            if !matches[idx] {
                continue;
            }
            if self.ancestors_expanded(idx, &matches) {
                self.visible.push(idx);
            }

            if entry.metadata.is_dir && !self.filter.is_empty() {
                self.expanded.insert(entry.metadata.display_path.clone());
            }
        }

        if self.selected >= self.visible.len() {
            self.selected = self.visible.len().saturating_sub(1);
        }
    }

    fn ancestors_expanded(&self, mut idx: usize, matches: &[bool]) -> bool {
        while let Some(parent_idx) = self.entries[idx].parent {
            let parent = &self.entries[parent_idx];
            if !parent.metadata.is_dir {
                idx = parent_idx;
                continue;
            }
            if !self.is_expanded(parent_idx, matches) {
                return false;
            }
            idx = parent_idx;
        }
        true
    }

    fn is_expanded(&self, idx: usize, matches: &[bool]) -> bool {
        let key = &self.entries[idx].metadata.display_path;
        if !self.filter.is_empty() && matches[idx] {
            true
        } else {
            self.expanded.contains(key)
        }
    }

    fn selected_entry_index(&self) -> Option<usize> {
        self.visible.get(self.selected).copied()
    }

    /// Iterate over entries that should be displayed in the UI.
    fn iter_visible(&self) -> impl Iterator<Item = (usize, usize, &TreeEntry)> {
        self.visible
            .iter()
            .enumerate()
            .filter_map(|(display_idx, entry_idx)| {
                self.entries
                    .get(*entry_idx)
                    .map(|entry| (display_idx, *entry_idx, entry))
            })
    }

    /// Index of the currently highlighted item within the visible list.
    pub fn selected_index(&self) -> Option<usize> {
        if self.visible.is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    /// Number of items currently visible in the file tree.
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// Whether a path is currently expanded.
    pub fn is_path_expanded(&self, path: &str) -> bool {
        self.expanded.contains(path)
    }

    /// Expose the root label for rendering.
    pub fn root_label(&self) -> &str {
        &self.root_label
    }
}

#[derive(Debug, Clone)]
struct TreeEntry {
    metadata: FileMetadata,
    name: String,
    depth: usize,
    parent: Option<usize>,
    has_children: bool,
}

/// Ratatui component responsible for rendering the file tree view.
#[derive(Debug, Default)]
pub struct FileTree;

impl FileTree {
    /// Render the file tree to the provided frame.
    pub fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        state: &FileTreeState,
        has_focus: bool,
        selected_paths: &HashSet<String>,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Workspace · {}", state.root_label()));
        frame.render_widget(block.clone(), area);

        let inner = block.inner(area);
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);

        let filter_text = if state.filter().is_empty() {
            "⌕ filter (press /)".to_string()
        } else {
            format!("⌕ {}", state.filter())
        };

        let mut filter_style = Style::default().fg(Color::Gray);
        if state.is_filter_active() {
            filter_style = filter_style.add_modifier(Modifier::BOLD).fg(Color::Cyan);
        }

        let filter_line = Paragraph::new(filter_text).style(filter_style);
        frame.render_widget(filter_line, layout[0]);

        if state.visible_len() == 0 {
            let placeholder = Paragraph::new("No files match filter").style(
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            );
            frame.render_widget(placeholder, layout[1]);
            return;
        }

        let mut items = Vec::with_capacity(state.visible_len());
        for (display_idx, _index, entry) in state.iter_visible() {
            let mut spans = Vec::new();
            spans.push(Span::raw("  ".repeat(entry.depth)));

            if entry.metadata.is_dir {
                let symbol = if state.is_path_expanded(&entry.metadata.display_path) {
                    "▾"
                } else if entry.has_children {
                    "▸"
                } else {
                    "·"
                };
                spans.push(Span::styled(
                    format!("{} ", symbol),
                    Style::default().fg(Color::Yellow),
                ));
            } else {
                spans.push(Span::styled("• ", Style::default().fg(Color::Gray)));
            }

            let mut name_style = Style::default();
            if selected_paths.contains(&entry.metadata.display_path) {
                name_style = name_style.fg(Color::Cyan).add_modifier(Modifier::BOLD);
            }

            if let Some(reason) = entry.metadata.skipped {
                let label = match reason {
                    SkipReason::LargeFile => "(large)",
                    SkipReason::BinaryFile => "(binary)",
                };
                spans.push(Span::styled(
                    entry.name.clone(),
                    name_style.fg(Color::DarkGray),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(label, Style::default().fg(Color::Yellow)));
            } else {
                spans.push(Span::styled(entry.name.clone(), name_style));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if display_idx % 2 == 1 {
                item = item.style(Style::default().bg(Color::Rgb(24, 24, 24)));
            }
            items.push(item);
        }

        let mut list_state = ratatui::widgets::ListState::default();
        if let Some(selected) = state.selected_index() {
            list_state.select(Some(selected));
        }

        let highlight_style = if has_focus {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Gray)
                .add_modifier(Modifier::BOLD)
        };

        let list = List::new(items)
            .block(Block::default())
            .highlight_style(highlight_style)
            .highlight_symbol("▸ ");

        frame.render_stateful_widget(list, layout[1], &mut list_state);
    }
}

fn display_name(display_path: &str) -> String {
    std::path::Path::new(display_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| display_path.to_string())
}

fn parent_key(display_path: &str) -> Option<String> {
    std::path::Path::new(display_path)
        .parent()
        .and_then(|parent| {
            if parent.as_os_str().is_empty() {
                None
            } else {
                Some(parent.to_string_lossy().to_string())
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashSet;
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::app::scan::{FileMetadata, ScanResult};

    #[test]
    fn renders_tree_for_basic_scan() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let scan = sample_scan();
        let state = FileTreeState::from_scan(&scan);
        let component = FileTree;
        let selected = HashSet::new();

        terminal
            .draw(|frame| {
                let area = frame.size();
                component.render(frame, area, &state, true, &selected);
            })
            .unwrap();
    }

    fn sample_scan() -> ScanResult {
        let root = PathBuf::from("/tmp/workspace");
        let files = vec![
            FileMetadata {
                path: root.join("src"),
                display_path: "src".into(),
                is_dir: true,
                size: None,
                modified: None,
                language: None,
                skipped: None,
            },
            FileMetadata {
                path: root.join("src/lib.rs"),
                display_path: "src/lib.rs".into(),
                is_dir: false,
                size: Some(42),
                modified: None,
                language: Some("rust".into()),
                skipped: None,
            },
            FileMetadata {
                path: root.join("README.md"),
                display_path: "README.md".into(),
                is_dir: false,
                size: Some(10),
                modified: None,
                language: Some("markdown".into()),
                skipped: None,
            },
        ];

        ScanResult { files, root }
    }
}
